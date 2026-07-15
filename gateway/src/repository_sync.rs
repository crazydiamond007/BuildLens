use chrono::{DateTime, Utc};
use reqwest::Url;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    error::AppError,
    github_api::{GitHubApi, GitHubBranch, GitHubCommit, GitHubPullRequest},
    state::AppState,
};

pub async fn sync_repository(
    state: AppState,
    repository_id: Uuid,
    token: String,
) -> Result<(), AppError> {
    let row = sqlx::query(
        "SELECT owner_login, name, default_branch
         FROM repositories
         WHERE id = $1 AND tracking_enabled AND deleted_at IS NULL",
    )
    .bind(repository_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    let owner: String = row.try_get("owner_login")?;
    let name: String = row.try_get("name")?;
    let default_branch: String = row.try_get("default_branch")?;
    let api = GitHubApi::new(&state, &token);

    let mut errors = Vec::new();
    if let Err(error) =
        sync_branches(&state, &api, repository_id, &owner, &name, &default_branch).await
    {
        record_error(&state, repository_id, "branches", &error).await?;
        errors.push(format!("branches: {error:?}"));
    }
    if let Err(error) = sync_commits(&state, &api, repository_id, &owner, &name).await {
        record_error(&state, repository_id, "commits", &error).await?;
        errors.push(format!("commits: {error:?}"));
    }
    if let Err(error) = sync_pull_requests(&state, &api, repository_id, &owner, &name).await {
        record_error(&state, repository_id, "pull_requests", &error).await?;
        errors.push(format!("pull_requests: {error:?}"));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Upstream(errors.join("; ")))
    }
}

async fn sync_branches(
    state: &AppState,
    api: &GitHubApi<'_>,
    repository_id: Uuid,
    owner: &str,
    name: &str,
    default_branch: &str,
) -> Result<(), AppError> {
    let (cursor, etag) = begin_resource(state, repository_id, "branches").await?;
    let mut first_page = cursor.is_none();
    let mut url = match cursor {
        Some(cursor) => parse_cursor(&cursor)?,
        None => {
            let mut url = api.endpoint(&["repos", owner, name, "branches"])?;
            url.query_pairs_mut().append_pair("per_page", "100");
            url
        }
    };
    let mut request_etag = first_page.then_some(etag.as_deref()).flatten();
    loop {
        let page = api.page::<GitHubBranch>(url, request_etag).await?;
        if page.not_modified {
            return finish_resource(state, repository_id, "branches").await;
        }
        let next = page.next.as_ref().map(Url::as_str);
        let mut transaction = state.db.begin().await?;
        for branch in page.items {
            sqlx::query(
                "INSERT INTO branches
                    (repository_id, name, head_sha, is_default, is_protected)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (repository_id, name) DO UPDATE SET
                    head_sha = EXCLUDED.head_sha,
                    is_default = EXCLUDED.is_default,
                    is_protected = EXCLUDED.is_protected,
                    deleted_at = NULL",
            )
            .bind(repository_id)
            .bind(branch.name.as_str())
            .bind(branch.commit.sha.as_str())
            .bind(branch.name == default_branch)
            .bind(branch.protected)
            .execute(&mut *transaction)
            .await?;
        }
        checkpoint(
            &mut transaction,
            repository_id,
            "branches",
            next,
            first_page.then_some(page.etag.as_deref()).flatten(),
        )
        .await?;
        transaction.commit().await?;
        first_page = false;
        match page.next {
            Some(next) => {
                url = next;
                request_etag = None;
            }
            None => return finish_resource(state, repository_id, "branches").await,
        }
    }
}

async fn sync_commits(
    state: &AppState,
    api: &GitHubApi<'_>,
    repository_id: Uuid,
    owner: &str,
    name: &str,
) -> Result<(), AppError> {
    let (cursor, etag) = begin_resource(state, repository_id, "commits").await?;
    let mut first_page = cursor.is_none();
    let mut url = match cursor {
        Some(cursor) => parse_cursor(&cursor)?,
        None => {
            let mut url = api.endpoint(&["repos", owner, name, "commits"])?;
            url.query_pairs_mut().append_pair("per_page", "100");
            url
        }
    };
    let mut request_etag = first_page.then_some(etag.as_deref()).flatten();
    loop {
        let page = api.page::<GitHubCommit>(url, request_etag).await?;
        if page.not_modified {
            return finish_resource(state, repository_id, "commits").await;
        }
        let next = page.next.as_ref().map(Url::as_str);
        let mut transaction = state.db.begin().await?;
        for commit in page.items {
            upsert_commit(&mut transaction, repository_id, &commit).await?;
        }
        checkpoint(
            &mut transaction,
            repository_id,
            "commits",
            next,
            first_page.then_some(page.etag.as_deref()).flatten(),
        )
        .await?;
        transaction.commit().await?;
        first_page = false;
        match page.next {
            Some(next) => {
                url = next;
                request_etag = None;
            }
            None => return finish_resource(state, repository_id, "commits").await,
        }
    }
}

async fn sync_pull_requests(
    state: &AppState,
    api: &GitHubApi<'_>,
    repository_id: Uuid,
    owner: &str,
    name: &str,
) -> Result<(), AppError> {
    let (cursor, etag) = begin_resource(state, repository_id, "pull_requests").await?;
    let mut first_page = cursor.is_none();
    let mut url = match cursor {
        Some(cursor) => parse_cursor(&cursor)?,
        None => {
            let mut url = api.endpoint(&["repos", owner, name, "pulls"])?;
            url.query_pairs_mut()
                .append_pair("state", "all")
                .append_pair("sort", "updated")
                .append_pair("direction", "desc")
                .append_pair("per_page", "100");
            url
        }
    };
    let mut request_etag = first_page.then_some(etag.as_deref()).flatten();
    loop {
        let page = api.page::<GitHubPullRequest>(url, request_etag).await?;
        if page.not_modified {
            return finish_resource(state, repository_id, "pull_requests").await;
        }
        let mut pulls_with_reviews = Vec::with_capacity(page.items.len());
        for listed_pull_request in page.items {
            // The list endpoint omits additions/deletions/file and commit
            // counts. Fetch the detail before persisting so initial sync does
            // not create permanently partial PR facts.
            let pull_request = api
                .pull_request(owner, name, listed_pull_request.number)
                .await?;
            let existing_review = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
                "SELECT first_review_at FROM pull_requests
                 WHERE repository_id = $1 AND number = $2",
            )
            .bind(repository_id)
            .bind(pull_request.number)
            .fetch_optional(&state.db)
            .await?
            .flatten();
            let first_review_at = match existing_review {
                Some(review) => Some(review),
                None => {
                    api.first_review_at(owner, name, pull_request.number)
                        .await?
                }
            };
            pulls_with_reviews.push((pull_request, first_review_at));
        }
        let next = page.next.as_ref().map(Url::as_str);
        let mut transaction = state.db.begin().await?;
        for (pull_request, first_review_at) in pulls_with_reviews {
            upsert_pull_request(
                &mut transaction,
                repository_id,
                &pull_request,
                first_review_at,
            )
            .await?;
        }
        checkpoint(
            &mut transaction,
            repository_id,
            "pull_requests",
            next,
            first_page.then_some(page.etag.as_deref()).flatten(),
        )
        .await?;
        transaction.commit().await?;
        first_page = false;
        match page.next {
            Some(next) => {
                url = next;
                request_etag = None;
            }
            None => return finish_resource(state, repository_id, "pull_requests").await,
        }
    }
}

async fn begin_resource(
    state: &AppState,
    repository_id: Uuid,
    resource: &str,
) -> Result<(Option<String>, Option<String>), AppError> {
    let row = sqlx::query(
        "INSERT INTO repository_sync_state (repository_id, resource, sync_status, last_error)
         VALUES ($1, $2, 'syncing', NULL)
         ON CONFLICT (repository_id, resource) DO UPDATE SET
             sync_status = 'syncing', last_error = NULL
         RETURNING cursor, etag",
    )
    .bind(repository_id)
    .bind(resource)
    .fetch_one(&state.db)
    .await?;
    Ok((row.try_get("cursor")?, row.try_get("etag")?))
}

async fn checkpoint(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    resource: &str,
    cursor: Option<&str>,
    etag: Option<&str>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE repository_sync_state SET
             cursor = $3,
             etag = COALESCE($4, etag),
             last_synced_at = now()
         WHERE repository_id = $1 AND resource = $2",
    )
    .bind(repository_id)
    .bind(resource)
    .bind(cursor)
    .bind(etag)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn finish_resource(
    state: &AppState,
    repository_id: Uuid,
    resource: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE repository_sync_state SET
             cursor = NULL, sync_status = 'idle', last_synced_at = now(),
             last_success_at = now(), last_error = NULL
         WHERE repository_id = $1 AND resource = $2",
    )
    .bind(repository_id)
    .bind(resource)
    .execute(&state.db)
    .await?;
    Ok(())
}

async fn record_error(
    state: &AppState,
    repository_id: Uuid,
    resource: &str,
    error: &AppError,
) -> Result<(), AppError> {
    let mut detail = format!("{error:?}");
    detail.truncate(2000);
    sqlx::query(
        "UPDATE repository_sync_state SET
             sync_status = 'error', last_synced_at = now(), last_error = $3
         WHERE repository_id = $1 AND resource = $2",
    )
    .bind(repository_id)
    .bind(resource)
    .bind(detail)
    .execute(&state.db)
    .await?;
    Ok(())
}

fn parse_cursor(cursor: &str) -> Result<Url, AppError> {
    Url::parse(cursor).map_err(|error| AppError::internal(format!("invalid sync cursor: {error}")))
}

pub async fn upsert_commit(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    commit: &GitHubCommit,
) -> Result<(), AppError> {
    let authored_at = commit
        .commit
        .author
        .as_ref()
        .and_then(|signature| signature.date);
    let committed_at = commit
        .commit
        .committer
        .as_ref()
        .and_then(|signature| signature.date)
        .or(authored_at)
        .ok_or_else(|| AppError::Upstream("GitHub commit has no timestamp".to_string()))?;
    sqlx::query(
        "INSERT INTO commits
            (repository_id, sha, message, author_name, author_email, author_login,
             author_github_user_id, committer_name, committer_email, authored_at,
             committed_at, is_merge_commit)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (repository_id, sha) DO UPDATE SET
             message = EXCLUDED.message,
             author_name = EXCLUDED.author_name,
             author_email = EXCLUDED.author_email,
             author_login = EXCLUDED.author_login,
             author_github_user_id = EXCLUDED.author_github_user_id,
             committer_name = EXCLUDED.committer_name,
             committer_email = EXCLUDED.committer_email,
             authored_at = EXCLUDED.authored_at,
             committed_at = EXCLUDED.committed_at,
             is_merge_commit = EXCLUDED.is_merge_commit",
    )
    .bind(repository_id)
    .bind(&commit.sha)
    .bind(&commit.commit.message)
    .bind(
        commit
            .commit
            .author
            .as_ref()
            .and_then(|signature| signature.name.as_deref()),
    )
    .bind(
        commit
            .commit
            .author
            .as_ref()
            .and_then(|signature| signature.email.as_deref()),
    )
    .bind(commit.author.as_ref().map(|author| author.login.as_str()))
    .bind(commit.author.as_ref().map(|author| author.id))
    .bind(
        commit
            .commit
            .committer
            .as_ref()
            .and_then(|signature| signature.name.as_deref()),
    )
    .bind(
        commit
            .commit
            .committer
            .as_ref()
            .and_then(|signature| signature.email.as_deref()),
    )
    .bind(authored_at)
    .bind(committed_at)
    .bind(commit.parents.len() > 1)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub async fn upsert_pull_request(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    pull: &GitHubPullRequest,
    first_review_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO pull_requests
            (repository_id, github_pr_id, number, title, state, is_draft,
             author_login, author_github_user_id, head_ref, head_sha, base_ref,
             base_sha, merge_commit_sha, merged_by_login, additions, deletions,
             changed_files, commits_count, comments_count, review_comments_count,
             opened_at, first_review_at, closed_at, merged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                 $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
         ON CONFLICT (repository_id, number) DO UPDATE SET
             github_pr_id = EXCLUDED.github_pr_id,
             title = EXCLUDED.title,
             state = EXCLUDED.state,
             is_draft = EXCLUDED.is_draft,
             author_login = EXCLUDED.author_login,
             author_github_user_id = EXCLUDED.author_github_user_id,
             head_ref = EXCLUDED.head_ref,
             head_sha = EXCLUDED.head_sha,
             base_ref = EXCLUDED.base_ref,
             base_sha = EXCLUDED.base_sha,
             merge_commit_sha = EXCLUDED.merge_commit_sha,
             merged_by_login = EXCLUDED.merged_by_login,
             additions = COALESCE(EXCLUDED.additions, pull_requests.additions),
             deletions = COALESCE(EXCLUDED.deletions, pull_requests.deletions),
             changed_files = COALESCE(EXCLUDED.changed_files, pull_requests.changed_files),
             commits_count = COALESCE(EXCLUDED.commits_count, pull_requests.commits_count),
             comments_count = EXCLUDED.comments_count,
             review_comments_count = EXCLUDED.review_comments_count,
             first_review_at = LEAST(pull_requests.first_review_at, EXCLUDED.first_review_at),
             closed_at = EXCLUDED.closed_at,
             merged_at = EXCLUDED.merged_at",
    )
    .bind(repository_id)
    .bind(pull.id)
    .bind(pull.number)
    .bind(&pull.title)
    .bind(&pull.state)
    .bind(pull.draft)
    .bind(pull.user.as_ref().map(|user| user.login.as_str()))
    .bind(pull.user.as_ref().map(|user| user.id))
    .bind(&pull.head.name)
    .bind(&pull.head.sha)
    .bind(&pull.base.name)
    .bind(&pull.base.sha)
    .bind(&pull.merge_commit_sha)
    .bind(pull.merged_by.as_ref().map(|user| user.login.as_str()))
    .bind(pull.additions)
    .bind(pull.deletions)
    .bind(pull.changed_files)
    .bind(pull.commits)
    .bind(pull.comments)
    .bind(pull.review_comments)
    .bind(pull.opened_at)
    .bind(first_review_at)
    .bind(pull.closed_at)
    .bind(pull.merged_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
