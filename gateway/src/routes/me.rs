use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{auth::Principal, error::AppError, state::AppState};

pub async fn get(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<MeResponse>, AppError> {
    let user = sqlx::query(
        "SELECT id, email::text AS email, name, avatar_url, last_login_at, created_at
         FROM users WHERE id = $1 AND is_active AND deleted_at IS NULL",
    )
    .bind(principal.user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::Unauthorized)?;

    let memberships = sqlx::query(
        "SELECT o.id, o.slug::text AS slug, o.name, o.kind, m.role
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
    .map(|row| {
        Ok(OrganizationMembership {
            id: row.try_get("id")?,
            slug: row.try_get("slug")?,
            name: row.try_get("name")?,
            kind: row.try_get("kind")?,
            role: row.try_get("role")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()?;

    Ok(Json(MeResponse {
        id: user.try_get("id")?,
        email: user.try_get("email")?,
        name: user.try_get("name")?,
        avatar_url: user.try_get("avatar_url")?,
        last_login_at: user.try_get("last_login_at")?,
        created_at: user.try_get("created_at")?,
        memberships,
    }))
}

#[derive(Serialize)]
pub struct MeResponse {
    id: Uuid,
    email: String,
    name: Option<String>,
    avatar_url: Option<String>,
    last_login_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    memberships: Vec<OrganizationMembership>,
}

#[derive(Serialize)]
struct OrganizationMembership {
    id: Uuid,
    slug: String,
    name: String,
    kind: String,
    role: String,
}
