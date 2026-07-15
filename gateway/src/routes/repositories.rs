use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::Row;
use tracing::error;
use uuid::Uuid;

use crate::{
    audit::{self, AuditContext},
    auth::{OrganizationRole, Principal, require_organization_role},
    error::AppError,
    github_api::{self, GitHubApi, GitHubRepository},
    repository_sync,
    state::AppState,
};

pub async fn discover(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Vec<DiscoveredRepository>>, AppError> {
    // Discovery exposes repositories that are not yet inside any BuildLens
    // organization, so an organization-scoped API token cannot authorize it.
    // Keep this on the user's revocable browser session.
    principal.require_session()?;
    let token = github_api::access_token(&state, principal.user_id).await?;
    let github_repositories = GitHubApi::new(&state, &token).repositories().await?;
    let tracked = sqlx::query(
        "SELECT r.id, r.github_repo_id, r.organization_id, r.tracking_enabled
         FROM repositories r
         JOIN organization_members m ON m.organization_id = r.organization_id
         JOIN organizations o ON o.id = r.organization_id
         WHERE m.user_id = $1 AND r.deleted_at IS NULL AND o.deleted_at IS NULL
           AND ($2::uuid IS NULL OR r.organization_id = $2)",
    )
    .bind(principal.user_id)
    .bind(principal.token_organization_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| {
        Ok((
            row.try_get::<i64, _>("github_repo_id")?,
            TrackingSummary {
                repository_id: row.try_get("id")?,
                organization_id: row.try_get("organization_id")?,
                tracking_enabled: row.try_get("tracking_enabled")?,
            },
        ))
    })
    .collect::<Result<HashMap<_, _>, sqlx::Error>>()?;

    Ok(Json(
        github_repositories
            .into_iter()
            .map(|repository| {
                let tracking = tracked.get(&repository.id).cloned();
                DiscoveredRepository {
                    github: repository,
                    tracking,
                }
            })
            .collect(),
    ))
}

pub async fn list(
    State(state): State<AppState>,
    principal: Principal,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Vec<RepositoryResponse>>, AppError> {
    require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;
    let repositories = sqlx::query(
        "SELECT id, organization_id, github_repo_id, owner_login, name, description,
                default_branch, is_private, is_archived, is_fork, primary_language,
                html_url, tracking_enabled, github_created_at, github_pushed_at,
                created_at, updated_at
         FROM repositories
         WHERE organization_id = $1 AND deleted_at IS NULL
         ORDER BY owner_login, name",
    )
    .bind(organization_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(repository_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(Json(repositories))
}

pub async fn enable_tracking(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Path((organization_id, github_repository_id)): Path<(Uuid, i64)>,
) -> Result<(StatusCode, Json<TrackingResponse>), AppError> {
    principal.require_session()?;
    require_organization_role(&state, &principal, organization_id, OrganizationRole::Admin).await?;

    let token = github_api::access_token(&state, principal.user_id).await?;
    let api = GitHubApi::new(&state, &token);
    let repository = api.repository(github_repository_id).await?;
    if repository
        .permissions
        .as_ref()
        .is_some_and(|permissions| !permissions.admin)
    {
        return Err(AppError::bad_request(
            "GitHub repository admin permission is required to register its webhook",
        ));
    }

    let existing_organization = sqlx::query_scalar::<_, Uuid>(
        "SELECT organization_id FROM repositories
         WHERE github_repo_id = $1 AND deleted_at IS NULL",
    )
    .bind(repository.id)
    .fetch_optional(&state.db)
    .await?;
    if existing_organization.is_some_and(|existing| existing != organization_id) {
        return Err(AppError::conflict(
            "this GitHub repository is already assigned to another BuildLens organization",
        ));
    }

    // GitHub is called before tracking is enabled. A failed hook registration
    // therefore cannot leave a repository claiming to be current when it has
    // no way to receive updates. Repeating this operation is safe because
    // ensure_webhook first looks for the configured callback URL.
    api.ensure_webhook(&repository.owner.login, &repository.name)
        .await?;

    let mut transaction = state.db.begin().await?;
    let row = sqlx::query(
        "INSERT INTO repositories
            (organization_id, github_repo_id, owner_login, name, description,
             default_branch, is_private, is_archived, is_fork, primary_language,
             html_url, tracking_enabled, github_created_at, github_pushed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, true, $12, $13)
         ON CONFLICT (github_repo_id) DO UPDATE SET
             owner_login = EXCLUDED.owner_login,
             name = EXCLUDED.name,
             description = EXCLUDED.description,
             default_branch = EXCLUDED.default_branch,
             is_private = EXCLUDED.is_private,
             is_archived = EXCLUDED.is_archived,
             is_fork = EXCLUDED.is_fork,
             primary_language = EXCLUDED.primary_language,
             html_url = EXCLUDED.html_url,
             tracking_enabled = true,
             github_created_at = EXCLUDED.github_created_at,
             github_pushed_at = EXCLUDED.github_pushed_at,
             deleted_at = NULL
         WHERE repositories.organization_id = EXCLUDED.organization_id
         RETURNING id, organization_id, github_repo_id, owner_login, name, description,
                   default_branch, is_private, is_archived, is_fork, primary_language,
                   html_url, tracking_enabled, github_created_at, github_pushed_at,
                   created_at, updated_at",
    )
    .bind(organization_id)
    .bind(repository.id)
    .bind(&repository.owner.login)
    .bind(&repository.name)
    .bind(&repository.description)
    .bind(&repository.default_branch)
    .bind(repository.is_private)
    .bind(repository.archived)
    .bind(repository.fork)
    .bind(&repository.language)
    .bind(&repository.html_url)
    .bind(repository.created_at)
    .bind(repository.pushed_at)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| {
        AppError::conflict(
            "this GitHub repository was assigned to another organization concurrently",
        )
    })?;
    let response = repository_from_row(row)?;
    audit::write(
        &mut transaction,
        &principal,
        &context,
        Some(organization_id),
        "repository.tracking_enabled",
        Some("repository"),
        Some(response.id),
        json!({"github_repo_id": repository.id, "full_name": repository.full_name}),
    )
    .await?;
    transaction.commit().await?;

    let state_for_sync = state.clone();
    let repository_id = response.id;
    tokio::spawn(async move {
        if let Err(sync_error) =
            repository_sync::sync_repository(state_for_sync, repository_id, token).await
        {
            error!(%repository_id, error = ?sync_error, "initial repository sync failed");
        }
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(TrackingResponse {
            repository: response,
            webhook_registered: true,
            sync_status: "queued",
        }),
    ))
}

pub async fn disable_tracking(
    State(state): State<AppState>,
    principal: Principal,
    context: AuditContext,
    Path((organization_id, repository_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    principal.require_session()?;
    require_organization_role(&state, &principal, organization_id, OrganizationRole::Admin).await?;
    let row = sqlx::query(
        "SELECT owner_login, name FROM repositories
         WHERE id = $1 AND organization_id = $2 AND deleted_at IS NULL",
    )
    .bind(repository_id)
    .bind(organization_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let owner: String = row.try_get("owner_login")?;
    let name: String = row.try_get("name")?;
    let token = github_api::access_token(&state, principal.user_id).await?;
    GitHubApi::new(&state, &token)
        .remove_webhook(&owner, &name)
        .await?;

    let mut transaction = state.db.begin().await?;
    let changed = sqlx::query(
        "UPDATE repositories SET tracking_enabled = false
         WHERE id = $1 AND organization_id = $2 AND tracking_enabled",
    )
    .bind(repository_id)
    .bind(organization_id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    audit::write(
        &mut transaction,
        &principal,
        &context,
        Some(organization_id),
        "repository.tracking_disabled",
        Some("repository"),
        Some(repository_id),
        json!({"full_name": format!("{owner}/{name}")}),
    )
    .await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn repository_from_row(row: sqlx::postgres::PgRow) -> Result<RepositoryResponse, sqlx::Error> {
    Ok(RepositoryResponse {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        github_repo_id: row.try_get("github_repo_id")?,
        owner_login: row.try_get("owner_login")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        default_branch: row.try_get("default_branch")?,
        is_private: row.try_get("is_private")?,
        is_archived: row.try_get("is_archived")?,
        is_fork: row.try_get("is_fork")?,
        primary_language: row.try_get("primary_language")?,
        html_url: row.try_get("html_url")?,
        tracking_enabled: row.try_get("tracking_enabled")?,
        github_created_at: row.try_get("github_created_at")?,
        github_pushed_at: row.try_get("github_pushed_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

#[derive(Clone, Serialize)]
pub struct TrackingSummary {
    repository_id: Uuid,
    organization_id: Uuid,
    tracking_enabled: bool,
}

#[derive(Serialize)]
pub struct DiscoveredRepository {
    #[serde(flatten)]
    github: GitHubRepository,
    tracking: Option<TrackingSummary>,
}

#[derive(Serialize)]
pub struct RepositoryResponse {
    id: Uuid,
    organization_id: Uuid,
    github_repo_id: i64,
    owner_login: String,
    name: String,
    description: Option<String>,
    default_branch: String,
    is_private: bool,
    is_archived: bool,
    is_fork: bool,
    primary_language: Option<String>,
    html_url: Option<String>,
    tracking_enabled: bool,
    github_created_at: Option<DateTime<Utc>>,
    github_pushed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub struct TrackingResponse {
    repository: RepositoryResponse,
    webhook_registered: bool,
    sync_status: &'static str,
}
