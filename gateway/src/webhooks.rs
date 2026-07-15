use std::time::Duration;

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use tokio::{sync::watch, time};
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    crypto::verify_github_signature, error::AppError, github_api::GitHubPullRequest,
    repository_sync::upsert_pull_request, state::AppState,
};

const DELIVERY_HEADER: &str = "x-github-delivery";
const EVENT_HEADER: &str = "x-github-event";
const SIGNATURE_HEADER: &str = "x-hub-signature-256";
const MAX_ATTEMPTS: i32 = 5;

pub async fn receive(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let delivery_id = required_header(&headers, DELIVERY_HEADER)?
        .parse::<Uuid>()
        .map_err(|_| AppError::bad_request("X-GitHub-Delivery must be a UUID"))?;
    let event_type = required_header(&headers, EVENT_HEADER)?;
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    // Signature verification deliberately happens before serde sees the body.
    let signature_valid = verify_github_signature(
        &state.config.github_webhook_secret,
        body.as_ref(),
        signature,
    );
    let payload = serde_json::from_slice::<Value>(&body)
        .unwrap_or_else(|_| json!({"_unparseable_body_base64": STANDARD.encode(body.as_ref())}));
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let github_repo_id = payload.pointer("/repository/id").and_then(Value::as_i64);
    let status = if signature_valid {
        "received"
    } else {
        "ignored"
    };
    let error = (!signature_valid).then_some("webhook signature is invalid");
    let inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO webhook_deliveries
            (github_delivery_id, event_type, action, github_repo_id, repository_id,
             signature_valid, status, payload, error, processed_at)
         VALUES ($1, $2, $3, $4,
                 (SELECT id FROM repositories WHERE github_repo_id = $4),
                 $5, $6, $7, $8,
                 CASE WHEN $5 THEN NULL ELSE now() END)
         ON CONFLICT (github_delivery_id) DO NOTHING
         RETURNING id",
    )
    .bind(delivery_id)
    .bind(event_type)
    .bind(action)
    .bind(github_repo_id)
    .bind(signature_valid)
    .bind(status)
    .bind(payload)
    .bind(error)
    .fetch_optional(&state.db)
    .await?;

    if !signature_valid {
        return Err(AppError::InvalidWebhookSignature);
    }
    Ok((
        if inserted.is_some() {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(json!({"accepted": inserted.is_some()})),
    )
        .into_response())
}

pub async fn run_processor(state: AppState, mut shutdown: watch::Receiver<bool>) {
    if let Err(recovery_error) = sqlx::query(
        "UPDATE webhook_deliveries
         SET status = 'failed', error = 'processor restarted during delivery'
         WHERE status = 'processing'",
    )
    .execute(&state.db)
    .await
    {
        error!(error = %recovery_error, "could not recover interrupted webhook deliveries");
    }
    let mut interval = time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    info!("webhook processor stopped");
                    return;
                }
            }
            _ = interval.tick() => {
                loop {
                    match process_one(&state).await {
                        Ok(true) => continue,
                        Ok(false) => break,
                        Err(process_error) => {
                            error!(error = ?process_error, "webhook processor iteration failed");
                            break;
                        }
                    }
                }
            }
        }
    }
}

async fn process_one(state: &AppState) -> Result<bool, AppError> {
    let Some(delivery) = claim_delivery(state).await? else {
        return Ok(false);
    };
    let result = apply_delivery(state, &delivery).await;
    if let Err(process_error) = result {
        let mut detail = format!("{process_error:?}");
        detail.truncate(2000);
        sqlx::query(
            "UPDATE webhook_deliveries SET status = 'failed', error = $2
             WHERE id = $1",
        )
        .bind(delivery.id)
        .bind(detail)
        .execute(&state.db)
        .await?;
        error!(delivery_id = %delivery.id, error = ?process_error, "webhook processing failed");
    }
    Ok(true)
}

async fn claim_delivery(state: &AppState) -> Result<Option<Delivery>, AppError> {
    let mut transaction = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT id, event_type, github_repo_id, payload
         FROM webhook_deliveries
         WHERE status IN ('received', 'failed')
           AND signature_valid
           AND attempts < $1
         ORDER BY received_at
         FOR UPDATE SKIP LOCKED
         LIMIT 1",
    )
    .bind(MAX_ATTEMPTS)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        transaction.commit().await?;
        return Ok(None);
    };
    let delivery = Delivery {
        id: row.try_get("id")?,
        event_type: row.try_get("event_type")?,
        github_repo_id: row.try_get("github_repo_id")?,
        payload: row.try_get("payload")?,
    };
    sqlx::query(
        "UPDATE webhook_deliveries
         SET status = 'processing', attempts = attempts + 1, error = NULL
         WHERE id = $1",
    )
    .bind(delivery.id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(delivery))
}

async fn apply_delivery(state: &AppState, delivery: &Delivery) -> Result<(), AppError> {
    let repository_id = match delivery.github_repo_id {
        Some(github_repo_id) => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM repositories
             WHERE github_repo_id = $1 AND tracking_enabled AND deleted_at IS NULL",
            )
            .bind(github_repo_id)
            .fetch_optional(&state.db)
            .await?
        }
        None => None,
    };
    let Some(repository_id) = repository_id else {
        sqlx::query(
            "UPDATE webhook_deliveries
             SET status = 'ignored', processed_at = now(),
                 error = 'repository is not tracked'
             WHERE id = $1",
        )
        .bind(delivery.id)
        .execute(&state.db)
        .await?;
        return Ok(());
    };

    let mut transaction = state.db.begin().await?;
    let applied = match delivery.event_type.as_str() {
        "push" => apply_push(&mut transaction, repository_id, &delivery.payload).await?,
        "pull_request" => {
            apply_pull_request(&mut transaction, repository_id, &delivery.payload).await?
        }
        "pull_request_review" => {
            apply_pull_request_review(&mut transaction, repository_id, &delivery.payload).await?
        }
        _ => false,
    };
    sqlx::query(
        "UPDATE webhook_deliveries
         SET repository_id = $2, status = $3, processed_at = now(), error = $4
         WHERE id = $1",
    )
    .bind(delivery.id)
    .bind(repository_id)
    .bind(if applied { "processed" } else { "ignored" })
    .bind((!applied).then_some("event type or action is not handled in Phase 3"))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn apply_push(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    payload: &Value,
) -> Result<bool, AppError> {
    let push: PushPayload = serde_json::from_value(payload.clone())?;
    let Some(branch_name) = push.reference.strip_prefix("refs/heads/") else {
        return Ok(false);
    };
    if push.deleted {
        sqlx::query(
            "UPDATE branches SET deleted_at = now()
             WHERE repository_id = $1 AND name = $2",
        )
        .bind(repository_id)
        .bind(branch_name)
        .execute(&mut **transaction)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO branches (repository_id, name, head_sha, is_default)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (repository_id, name) DO UPDATE SET
                 head_sha = EXCLUDED.head_sha,
                 is_default = EXCLUDED.is_default,
                 deleted_at = NULL",
        )
        .bind(repository_id)
        .bind(branch_name)
        .bind(&push.after)
        .bind(branch_name == push.repository.default_branch)
        .execute(&mut **transaction)
        .await?;
    }
    for commit in push.commits {
        upsert_push_commit(transaction, repository_id, &commit).await?;
    }
    Ok(true)
}

async fn upsert_push_commit(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    commit: &PushCommit,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO commits
            (repository_id, sha, message, author_name, author_email, author_login,
             committer_name, committer_email, authored_at, committed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
         ON CONFLICT (repository_id, sha) DO UPDATE SET
             message = EXCLUDED.message,
             author_name = EXCLUDED.author_name,
             author_email = EXCLUDED.author_email,
             author_login = EXCLUDED.author_login,
             committer_name = EXCLUDED.committer_name,
             committer_email = EXCLUDED.committer_email,
             authored_at = EXCLUDED.authored_at,
             committed_at = EXCLUDED.committed_at",
    )
    .bind(repository_id)
    .bind(&commit.id)
    .bind(&commit.message)
    .bind(&commit.author.name)
    .bind(&commit.author.email)
    .bind(&commit.author.username)
    .bind(&commit.committer.name)
    .bind(&commit.committer.email)
    .bind(commit.timestamp)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn apply_pull_request(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    payload: &Value,
) -> Result<bool, AppError> {
    let pull = serde_json::from_value::<GitHubPullRequest>(
        payload
            .get("pull_request")
            .cloned()
            .ok_or_else(|| AppError::bad_request("pull_request payload is missing the PR"))?,
    )?;
    let first_review_at = existing_first_review(transaction, repository_id, pull.number).await?;
    upsert_pull_request(transaction, repository_id, &pull, first_review_at).await?;
    Ok(true)
}

async fn apply_pull_request_review(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    payload: &Value,
) -> Result<bool, AppError> {
    let pull = serde_json::from_value::<GitHubPullRequest>(
        payload
            .get("pull_request")
            .cloned()
            .ok_or_else(|| AppError::bad_request("review payload is missing the PR"))?,
    )?;
    let review: ReviewPayload = serde_json::from_value(
        payload
            .get("review")
            .cloned()
            .ok_or_else(|| AppError::bad_request("review payload is missing the review"))?,
    )?;
    let existing = existing_first_review(transaction, repository_id, pull.number).await?;
    let first_review_at = match (existing, review.submitted_at) {
        (Some(existing), Some(submitted)) => Some(existing.min(submitted)),
        (existing, submitted) => existing.or(submitted),
    };
    upsert_pull_request(transaction, repository_id, &pull, first_review_at).await?;
    Ok(true)
}

async fn existing_first_review(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    number: i32,
) -> Result<Option<DateTime<Utc>>, AppError> {
    Ok(sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
        "SELECT first_review_at FROM pull_requests
         WHERE repository_id = $1 AND number = $2",
    )
    .bind(repository_id)
    .bind(number)
    .fetch_optional(&mut **transaction)
    .await?
    .flatten())
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, AppError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::bad_request(format!("{name} header is required")))
}

struct Delivery {
    id: Uuid,
    event_type: String,
    github_repo_id: Option<i64>,
    payload: Value,
}

#[derive(Deserialize)]
struct PushPayload {
    #[serde(rename = "ref")]
    reference: String,
    after: String,
    #[serde(default)]
    deleted: bool,
    repository: PushRepository,
    #[serde(default)]
    commits: Vec<PushCommit>,
}

#[derive(Deserialize)]
struct PushRepository {
    default_branch: String,
}

#[derive(Deserialize)]
struct PushCommit {
    id: String,
    message: String,
    timestamp: DateTime<Utc>,
    author: PushIdentity,
    committer: PushIdentity,
}

#[derive(Deserialize)]
struct PushIdentity {
    name: Option<String>,
    email: Option<String>,
    username: Option<String>,
}

#[derive(Deserialize)]
struct ReviewPayload {
    submitted_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_payload_recognizes_branch_and_commit() {
        let payload: PushPayload = serde_json::from_value(json!({
            "ref": "refs/heads/main",
            "after": "abc",
            "deleted": false,
            "repository": {"default_branch": "main"},
            "commits": [{
                "id": "abc",
                "message": "ship it",
                "timestamp": "2026-01-01T00:00:00Z",
                "author": {"name": "A", "email": "a@example.com", "username": "a"},
                "committer": {"name": "A", "email": "a@example.com", "username": "a"}
            }]
        }))
        .unwrap();
        assert_eq!(payload.reference, "refs/heads/main");
        assert_eq!(payload.commits.len(), 1);
    }
}
