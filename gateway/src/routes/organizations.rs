use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    audit::{self, AuditContext},
    auth::{OrganizationRole, Principal, require_organization_role},
    error::AppError,
    state::AppState,
};

pub async fn list(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Vec<OrganizationResponse>>, AppError> {
    let organizations = sqlx::query(
        "SELECT o.id, o.slug::text AS slug, o.name, o.kind, m.role, o.created_at
         FROM organization_members m
         JOIN organizations o ON o.id = m.organization_id
         WHERE m.user_id = $1
           AND o.deleted_at IS NULL
           AND ($2::uuid IS NULL OR o.id = $2)
         ORDER BY o.name",
    )
    .bind(principal.user_id)
    .bind(principal.token_organization_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(organization_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(organizations))
}

pub async fn create(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Json(request): Json<CreateOrganization>,
) -> Result<(StatusCode, Json<OrganizationResponse>), AppError> {
    principal.require_session()?;
    let name = request.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(AppError::bad_request(
            "organization name must be between 1 and 100 characters",
        ));
    }
    let slug = request
        .slug
        .as_deref()
        .map(str::to_owned)
        .unwrap_or_else(|| slugify(name));
    validate_slug(&slug)?;

    let organization_id = Uuid::now_v7();
    let mut transaction = state.db.begin().await?;
    let created_at = sqlx::query_scalar::<_, DateTime<Utc>>(
        "INSERT INTO organizations (id, slug, name, kind, created_by)
         VALUES ($1, $2, $3, 'team', $4)
         RETURNING created_at",
    )
    .bind(organization_id)
    .bind(&slug)
    .bind(name)
    .bind(principal.user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(map_slug_conflict)?;
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(principal.user_id)
    .execute(&mut *transaction)
    .await?;
    audit::write(
        &mut transaction,
        &principal,
        &context,
        Some(organization_id),
        "organization.created",
        Some("organization"),
        Some(organization_id),
        json!({"slug": slug}),
    )
    .await?;
    transaction.commit().await?;

    Ok((
        StatusCode::CREATED,
        Json(OrganizationResponse {
            id: organization_id,
            slug,
            name: name.to_string(),
            kind: "team".to_string(),
            role: "owner".to_string(),
            created_at,
        }),
    ))
}

pub async fn list_members(
    State(state): State<AppState>,
    principal: Principal,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Vec<MemberResponse>>, AppError> {
    require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;

    let members = sqlx::query(
        "SELECT u.id, u.email::text AS email, u.name, u.avatar_url, m.role, m.created_at
         FROM organization_members m
         JOIN users u ON u.id = m.user_id
         WHERE m.organization_id = $1 AND u.deleted_at IS NULL
         ORDER BY u.name NULLS LAST, u.email",
    )
    .bind(organization_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(member_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(members))
}

pub async fn add_member(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Path(organization_id): Path<Uuid>,
    Json(request): Json<AddMember>,
) -> Result<(StatusCode, Json<MemberResponse>), AppError> {
    principal.require_session()?;
    let mut transaction = state.db.begin().await?;
    lock_team_organization(&mut transaction, organization_id).await?;
    let actor_role = require_transaction_role(
        &mut transaction,
        organization_id,
        principal.user_id,
        OrganizationRole::Admin,
    )
    .await?;
    let target_role = OrganizationRole::parse(&request.role)
        .ok_or_else(|| AppError::bad_request("role must be owner, admin, member, or viewer"))?;
    if target_role.level() >= OrganizationRole::Admin.level()
        && actor_role != OrganizationRole::Owner
    {
        return Err(AppError::Forbidden);
    }

    let user = sqlx::query(
        "SELECT id, email::text AS email, name, avatar_url
         FROM users WHERE email = $1 AND is_active AND deleted_at IS NULL",
    )
    .bind(request.email.trim())
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::bad_request("no active BuildLens user has that email"))?;
    let user_id: Uuid = user.try_get("id")?;

    let previous_role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM organization_members
         WHERE organization_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?;

    if previous_role.as_deref() == Some("owner") && target_role != OrganizationRole::Owner {
        ensure_another_owner(&mut transaction, organization_id).await?;
    }

    let membership = sqlx::query(
        "INSERT INTO organization_members
            (organization_id, user_id, role, invited_by)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (organization_id, user_id) DO UPDATE SET role = EXCLUDED.role
         RETURNING id, created_at",
    )
    .bind(organization_id)
    .bind(user_id)
    .bind(target_role.as_str())
    .bind(principal.user_id)
    .fetch_one(&mut *transaction)
    .await?;
    let membership_id: Uuid = membership.try_get("id")?;

    let action = if previous_role.is_some() {
        "organization_membership.role_changed"
    } else {
        "organization_membership.added"
    };
    audit::write(
        &mut transaction,
        &principal,
        &context,
        Some(organization_id),
        action,
        Some("organization_membership"),
        Some(membership_id),
        json!({"previous_role": previous_role, "role": target_role.as_str()}),
    )
    .await?;
    transaction.commit().await?;

    Ok((
        if previous_role.is_some() {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(MemberResponse {
            user_id,
            email: user.try_get("email")?,
            name: user.try_get("name")?,
            avatar_url: user.try_get("avatar_url")?,
            role: target_role.as_str().to_string(),
            created_at: membership.try_get("created_at")?,
        }),
    ))
}

pub async fn remove_member(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Path((organization_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    principal.require_session()?;
    let mut transaction = state.db.begin().await?;
    lock_team_organization(&mut transaction, organization_id).await?;
    let actor_role = require_transaction_role(
        &mut transaction,
        organization_id,
        principal.user_id,
        OrganizationRole::Admin,
    )
    .await?;
    let target = sqlx::query(
        "SELECT id, role FROM organization_members
         WHERE organization_id = $1 AND user_id = $2 FOR UPDATE",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let membership_id: Uuid = target.try_get("id")?;
    let target_role = OrganizationRole::parse(target.try_get::<String, _>("role")?.as_str())
        .ok_or_else(|| AppError::internal("organization membership contains an invalid role"))?;

    if target_role.level() >= OrganizationRole::Admin.level()
        && actor_role != OrganizationRole::Owner
    {
        return Err(AppError::Forbidden);
    }
    if target_role == OrganizationRole::Owner {
        ensure_another_owner(&mut transaction, organization_id).await?;
    }

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(organization_id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    audit::write(
        &mut transaction,
        &principal,
        &context,
        Some(organization_id),
        "organization_membership.removed",
        Some("organization_membership"),
        Some(membership_id),
        json!({"role": target_role.as_str()}),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn lock_team_organization(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
) -> Result<(), AppError> {
    let kind = sqlx::query_scalar::<_, String>(
        "SELECT kind FROM organizations
         WHERE id = $1 AND deleted_at IS NULL
         FOR UPDATE",
    )
    .bind(organization_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    if kind != "team" {
        return Err(AppError::bad_request(
            "personal organizations cannot have additional members",
        ));
    }
    Ok(())
}

async fn require_transaction_role(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    user_id: Uuid,
    minimum: OrganizationRole,
) -> Result<OrganizationRole, AppError> {
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM organization_members
         WHERE organization_id = $1 AND user_id = $2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await?
    .and_then(|role| OrganizationRole::parse(&role))
    .ok_or(AppError::Forbidden)?;
    if role.level() < minimum.level() {
        return Err(AppError::Forbidden);
    }
    Ok(role)
}

async fn ensure_another_owner(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
) -> Result<(), AppError> {
    let owners = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM organization_members
         WHERE organization_id = $1 AND role = 'owner'",
    )
    .bind(organization_id)
    .fetch_one(&mut **transaction)
    .await?;
    if owners <= 1 {
        return Err(AppError::conflict(
            "an organization must retain at least one owner",
        ));
    }
    Ok(())
}

fn organization_from_row(row: sqlx::postgres::PgRow) -> Result<OrganizationResponse, sqlx::Error> {
    Ok(OrganizationResponse {
        id: row.try_get("id")?,
        slug: row.try_get("slug")?,
        name: row.try_get("name")?,
        kind: row.try_get("kind")?,
        role: row.try_get("role")?,
        created_at: row.try_get("created_at")?,
    })
}

fn member_from_row(row: sqlx::postgres::PgRow) -> Result<MemberResponse, sqlx::Error> {
    Ok(MemberResponse {
        user_id: row.try_get("id")?,
        email: row.try_get("email")?,
        name: row.try_get("name")?,
        avatar_url: row.try_get("avatar_url")?,
        role: row.try_get("role")?,
        created_at: row.try_get("created_at")?,
    })
}

fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut previous_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            output.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !output.is_empty() {
            output.push('-');
            previous_dash = true;
        }
    }
    output.trim_matches('-').chars().take(63).collect()
}

fn validate_slug(slug: &str) -> Result<(), AppError> {
    let valid = (3..=63).contains(&slug.len())
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err(AppError::bad_request(
            "slug must be 3-63 lowercase letters, numbers, or hyphens",
        ));
    }
    Ok(())
}

fn map_slug_conflict(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .and_then(|database| database.code())
        .is_some_and(|code| code == "23505")
    {
        AppError::conflict("that organization slug is already in use")
    } else {
        AppError::from(error)
    }
}

#[derive(Deserialize)]
pub struct CreateOrganization {
    name: String,
    slug: Option<String>,
}

#[derive(Deserialize)]
pub struct AddMember {
    email: String,
    role: String,
}

#[derive(Serialize)]
pub struct OrganizationResponse {
    id: Uuid,
    slug: String,
    name: String,
    kind: String,
    role: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct MemberResponse {
    user_id: Uuid,
    email: String,
    name: Option<String>,
    avatar_url: Option<String>,
    role: String,
    created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_collapses_non_slug_characters() {
        assert_eq!(
            slugify("  Platform & Reliability  "),
            "platform-reliability"
        );
        assert!(validate_slug("platform-reliability").is_ok());
        assert!(validate_slug("Not Valid").is_err());
    }
}
