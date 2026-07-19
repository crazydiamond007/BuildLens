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
use tracing::{error, warn};
use uuid::Uuid;

use crate::{
    audit::{self, AuditContext},
    auth::{OrganizationRole, Principal, require_organization_role},
    error::AppError,
    github,
    github_api::{self, GitHubApi, GitHubRepository},
    github_app, installations, repository_sync,
    state::AppState,
};

pub async fn discover(
    State(state): State<AppState>,
    principal: Principal,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<DiscoverResponse>, AppError> {
    // Discovery lists what the workspace's GitHub App installation can see, so
    // it is scoped to an organization the caller belongs to. Keep it on the
    // revocable browser session rather than an API token.
    principal.require_session()?;
    require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;

    let install_url = install_url(&state);
    // Prefer the workspace's own linked installation, minting a fresh token so a
    // repository added since the last mint is not hidden by a stale scope. If
    // nothing is linked yet, try to self-heal from the caller's own installation
    // before giving up: a missed post-install setup redirect must not strand a
    // workspace whose App is in fact already installed. Only then fall back to the
    // install affordance, which is an action the user can still take.
    let token =
        match github_api::organization_installation_token_fresh(&state, organization_id).await? {
            Some(token) => token,
            None => match reconcile_installation(&state, &principal, organization_id).await? {
                Some(token) => token,
                None => {
                    return Ok(Json(DiscoverResponse {
                        installed: false,
                        install_url,
                        repositories: Vec::new(),
                    }));
                }
            },
        };

    let github_repositories = GitHubApi::new(&state, &token)
        .installation_repositories()
        .await?;
    let tracked = sqlx::query(
        "SELECT id, github_repo_id, organization_id, tracking_enabled
         FROM repositories
         WHERE organization_id = $1 AND deleted_at IS NULL",
    )
    .bind(organization_id)
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

    Ok(Json(DiscoverResponse {
        installed: true,
        install_url,
        repositories: github_repositories
            .into_iter()
            .map(|repository| {
                let tracking = tracked.get(&repository.id).cloned();
                DiscoveredRepository {
                    github: repository,
                    tracking,
                }
            })
            .collect(),
    }))
}

/// The browser URL that starts installing (or reconfiguring) the App. Points at
/// github.com regardless of the API base, because that is where a person clicks.
fn install_url(state: &AppState) -> String {
    format!(
        "https://github.com/apps/{}/installations/new",
        state.config.github_app_slug
    )
}

/// Recovers a workspace whose GitHub App installation exists on GitHub but was
/// never linked here - typically because the post-install setup redirect never
/// reached the gateway (it was down at install time, or the App's setup URL was
/// unset). Without this, such a workspace is stranded on the install prompt even
/// though the App is installed, because the only other path to a link is that one
/// redirect.
///
/// The caller's own `/user/installations` is the authorization boundary, exactly
/// as the setup callback uses it: an installation the user cannot see is never
/// linked. A single installation is linked to their personal workspace and a
/// token returned; zero or several are left for the explicit install flow, which
/// can disambiguate. Idempotent - a no-op once the workspace is already linked.
async fn reconcile_installation(
    state: &AppState,
    principal: &Principal,
    organization_id: Uuid,
) -> Result<Option<String>, AppError> {
    let Some(installation_id) = caller_single_installation(state, principal.user_id).await? else {
        return Ok(None);
    };
    let installation = github_app::fetch_installation(state, installation_id).await?;
    installations::upsert(&state.db, &installation).await?;
    installations::link_to_personal_organization(&state.db, principal.user_id, installation_id)
        .await?;
    // Only the workspace that actually received the link ends up with a token; a
    // caller viewing some other workspace still sees the install prompt.
    github_api::organization_installation_token(state, organization_id).await
}

/// The single App installation the caller controls, or `None` if we cannot tell.
///
/// This is a best-effort input to self-healing, so every "cannot tell" reason
/// collapses to `None` rather than an error: no connected account, a token GitHub
/// rejects (e.g. one issued before the App migration, which cannot read
/// `/user/installations`), or a count other than exactly one. The caller then
/// falls back to the install prompt instead of failing the whole page - a stale
/// token must not turn discovery into a 502.
async fn caller_single_installation(
    state: &AppState,
    user_id: Uuid,
) -> Result<Option<i64>, AppError> {
    let Ok(user_token) = github_api::user_access_token(state, user_id).await else {
        return Ok(None);
    };
    match github::user_installation_ids(state, &user_token).await {
        Ok(ids) => match ids.as_slice() {
            [installation_id] => Ok(Some(*installation_id)),
            _ => Ok(None),
        },
        Err(error) => {
            warn!(%user_id, ?error, "could not list the caller's installations; skipping self-heal");
            Ok(None)
        }
    }
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

    // Repository access rides on the workspace's App installation now, not on the
    // caller's own GitHub permissions. No installation means nothing to track. A
    // fresh token is minted so a repository added to the App moments ago is not
    // rejected as "not part of your installation" by a token scoped before it.
    let Some(token) =
        github_api::organization_installation_token_fresh(&state, organization_id).await?
    else {
        return Err(AppError::bad_request(
            "install the BuildLens GitHub App on this workspace before tracking repositories",
        ));
    };
    let api = GitHubApi::new(&state, &token);
    // The installation token can only read repositories the App was granted, so a
    // 404 means the user has not added this one to the App. Anything else (rate
    // limit, 5xx) propagates as-is rather than masquerading as a user error.
    let Some(repository) = api.repository_opt(github_repository_id).await? else {
        return Err(AppError::bad_request(
            "this repository is not part of your BuildLens installation; add it to the app on GitHub first",
        ));
    };

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

    // No webhook to register: the App receives events for every installed
    // repository at its single App-level webhook. Enabling tracking is now purely
    // a BuildLens-side decision about which of those repositories to store.
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
    // Nothing to remove on GitHub: the App keeps its installation-level webhook
    // and simply keeps sending events, which the receiver now ignores for an
    // untracked repository. Fully cutting access is done by editing the app's
    // repositories (or uninstalling it) on GitHub.
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

/// Discovery's answer. When `installed` is false the repository list is empty and
/// `install_url` is where the browser goes to install the App.
#[derive(Serialize)]
pub struct DiscoverResponse {
    installed: bool,
    install_url: String,
    repositories: Vec<DiscoveredRepository>,
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
    sync_status: &'static str,
}
