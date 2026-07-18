//! Persistence for GitHub App installations - the durable link between an
//! installation on GitHub and the BuildLens workspace it feeds.
//!
//! Both entry points write through here: the setup callback
//! (`routes/auth::github_setup`) when a user finishes installing, and the
//! `installation` / `installation_repositories` webhooks when they later change
//! the install on GitHub. Keeping the SQL in one place means those two paths
//! cannot drift apart.

use sqlx::PgPool;
use uuid::Uuid;

use crate::{error::AppError, github_app::Installation};

/// Records (or refreshes) an installation from freshly fetched metadata.
pub async fn upsert(db: &PgPool, installation: &Installation) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO github_installations
            (installation_id, account_login, account_id, target_type,
             repository_selection, suspended_at)
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $6 THEN now() ELSE NULL END)
         ON CONFLICT (installation_id) DO UPDATE SET
             account_login = EXCLUDED.account_login,
             account_id = EXCLUDED.account_id,
             target_type = EXCLUDED.target_type,
             repository_selection = EXCLUDED.repository_selection,
             -- Preserve the original suspension moment while still suspended;
             -- clear it once GitHub reports the installation active again.
             suspended_at = CASE
                 WHEN $6 THEN COALESCE(github_installations.suspended_at, now())
                 ELSE NULL
             END",
    )
    .bind(installation.installation_id)
    .bind(&installation.account_login)
    .bind(installation.account_id)
    .bind(&installation.target_type)
    .bind(&installation.repository_selection)
    .bind(installation.suspended)
    .execute(db)
    .await?;
    Ok(())
}

/// Points a user's personal workspace at an installation, moving the link off
/// any other workspace first so the `UNIQUE` column never collides.
///
/// The personal workspace created at sign-in is the default home for an
/// installation. An installation on a GitHub *organization* is, for now, still
/// linked to the installer's own workspace; routing those to a dedicated team
/// workspace is a later refinement, not a correctness gap.
pub async fn link_to_personal_organization(
    db: &PgPool,
    user_id: Uuid,
    installation_id: i64,
) -> Result<Uuid, AppError> {
    let organization_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM organizations
         WHERE created_by = $1 AND kind = 'personal' AND deleted_at IS NULL
         ORDER BY created_at
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AppError::internal("signed-in user has no personal workspace"))?;

    let mut transaction = db.begin().await?;
    sqlx::query(
        "UPDATE organizations SET github_installation_id = NULL
         WHERE github_installation_id = $1 AND id <> $2",
    )
    .bind(installation_id)
    .bind(organization_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE organizations SET github_installation_id = $2 WHERE id = $1")
        .bind(organization_id)
        .bind(installation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(organization_id)
}

/// Flips the suspended flag. A suspended installation cannot mint tokens, so its
/// repositories go quiet until GitHub reports it active again.
pub async fn set_suspended(
    db: &PgPool,
    installation_id: i64,
    suspended: bool,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE github_installations
         SET suspended_at = CASE
                 WHEN $2 THEN COALESCE(suspended_at, now())
                 ELSE NULL
             END
         WHERE installation_id = $1",
    )
    .bind(installation_id)
    .bind(suspended)
    .execute(db)
    .await?;
    Ok(())
}

/// Removes an installation. Any workspace pointed at it is unlinked automatically
/// by the `ON DELETE SET NULL` foreign key; its repositories are first stopped
/// so nothing keeps trying to fetch data the App can no longer reach.
pub async fn remove(db: &PgPool, installation_id: i64) -> Result<(), AppError> {
    let mut transaction = db.begin().await?;
    sqlx::query(
        "UPDATE repositories SET tracking_enabled = false
         WHERE tracking_enabled
           AND organization_id IN (
               SELECT id FROM organizations WHERE github_installation_id = $1
           )",
    )
    .bind(installation_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM github_installations WHERE installation_id = $1")
        .bind(installation_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Stops tracking repositories the App can no longer see, keyed by GitHub's
/// numeric repo id. Called when a user removes repositories from an installation.
pub async fn untrack_repositories(db: &PgPool, github_repo_ids: &[i64]) -> Result<(), AppError> {
    if github_repo_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE repositories SET tracking_enabled = false
         WHERE tracking_enabled AND github_repo_id = ANY($1)",
    )
    .bind(github_repo_ids)
    .execute(db)
    .await?;
    Ok(())
}
