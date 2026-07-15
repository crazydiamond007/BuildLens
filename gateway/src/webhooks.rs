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
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use tokio::{sync::watch, time};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::{
    crypto::verify_github_signature,
    error::AppError,
    github_api::{
        self, GitHubApi, GitHubJob, GitHubPullRequest, GitHubWorkflow, GitHubWorkflowRun,
    },
    junit::{self, JunitCapture},
    repository_sync::upsert_pull_request,
    state::AppState,
    workflow_ingest,
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
    let context = match delivery.github_repo_id {
        Some(github_repo_id) => fetch_repo_context(state, github_repo_id).await?,
        None => None,
    };
    let Some(context) = context else {
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
    let mut outcome = Outcome::ignored();
    match delivery.event_type.as_str() {
        "push" => {
            outcome.applied = apply_push(&mut transaction, context.id, &delivery.payload).await?
        }
        "pull_request" => {
            outcome.applied =
                apply_pull_request(&mut transaction, context.id, &delivery.payload).await?
        }
        "pull_request_review" => {
            outcome.applied =
                apply_pull_request_review(&mut transaction, context.id, &delivery.payload).await?
        }
        "workflow_run" => {
            outcome = apply_workflow_run(&mut transaction, &context, &delivery.payload).await?
        }
        "workflow_job" => {
            outcome.applied =
                apply_workflow_job(&mut transaction, context.id, &delivery.payload).await?
        }
        _ => {}
    };
    sqlx::query(
        "UPDATE webhook_deliveries
         SET repository_id = $2, status = $3, processed_at = now(), error = $4
         WHERE id = $1",
    )
    .bind(delivery.id)
    .bind(context.id)
    .bind(if outcome.applied {
        "processed"
    } else {
        "ignored"
    })
    .bind((!outcome.applied).then_some("event type or action is not handled in Phase 4"))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    // Post-commit, best-effort: pull the completed run's logs into MinIO. Its
    // own task, so a slow multi-megabyte download never holds up the queue, and
    // its failure never un-does the facts that were just committed.
    if let Some(capture) = outcome.log_capture {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = store_run_logs(&state, capture.clone()).await {
                warn!(error = ?error, "workflow log capture failed");
            }
            let junit_capture = JunitCapture {
                repository_id: capture.repository_id,
                workflow_run_id: capture.workflow_run_id,
                owner: capture.owner,
                name: capture.name,
                github_run_id: capture.github_run_id,
                executed_at: capture.executed_at,
            };
            if let Err(error) = junit::capture_run_tests(&state, junit_capture).await {
                warn!(error = ?error, "JUnit artifact capture failed");
            }
        });
    }
    Ok(())
}

/// What `apply_delivery` needs to know about the tracked repository a delivery
/// belongs to: its id (for FKs), its owning org (for event envelopes), its
/// default branch (to stamp `is_default_branch`), and its `owner/name` (to call
/// GitHub for logs).
struct RepoContext {
    id: Uuid,
    organization_id: Uuid,
    default_branch: String,
    owner_login: String,
    name: String,
}

async fn fetch_repo_context(
    state: &AppState,
    github_repo_id: i64,
) -> Result<Option<RepoContext>, AppError> {
    let row = sqlx::query(
        "SELECT id, organization_id, default_branch, owner_login, name
         FROM repositories
         WHERE github_repo_id = $1 AND tracking_enabled AND deleted_at IS NULL",
    )
    .bind(github_repo_id)
    .fetch_optional(&state.db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(RepoContext {
        id: row.try_get("id")?,
        organization_id: row.try_get("organization_id")?,
        default_branch: row.try_get("default_branch")?,
        owner_login: row.try_get("owner_login")?,
        name: row.try_get("name")?,
    }))
}

/// The result of applying one delivery: whether it changed anything, and an
/// optional request to fetch logs after the transaction commits.
struct Outcome {
    applied: bool,
    log_capture: Option<LogCapture>,
}

impl Outcome {
    fn ignored() -> Self {
        Self {
            applied: false,
            log_capture: None,
        }
    }
}

#[derive(Clone)]
struct LogCapture {
    repository_id: Uuid,
    workflow_run_id: Uuid,
    owner: String,
    name: String,
    github_run_id: i64,
    run_attempt: i32,
    executed_at: DateTime<Utc>,
}

async fn apply_workflow_run(
    transaction: &mut Transaction<'_, Postgres>,
    context: &RepoContext,
    payload: &Value,
) -> Result<Outcome, AppError> {
    let run = serde_json::from_value::<GitHubWorkflowRun>(
        payload
            .get("workflow_run")
            .cloned()
            .ok_or_else(|| AppError::bad_request("workflow_run payload is missing the run"))?,
    )?;
    // The workflow object rides along on the webhook; the backfill supplies it
    // separately. Absence is fine - the run's workflow_id FK is nullable.
    let workflow = payload
        .get("workflow")
        .cloned()
        .and_then(|value| serde_json::from_value::<GitHubWorkflow>(value).ok());

    let workflow_run_id = workflow_ingest::ingest_workflow_run(
        transaction,
        context.id,
        context.organization_id,
        &context.default_branch,
        &run,
        workflow.as_ref(),
    )
    .await?;

    // Capture logs only for completed runs, and only if we do not already hold
    // them, so a duplicate completion webhook does not re-download.
    let log_capture = if run.status == "completed" {
        let already = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM build_logs
             WHERE workflow_run_id = $1 AND workflow_job_id IS NULL)",
        )
        .bind(workflow_run_id)
        .fetch_one(&mut **transaction)
        .await?;
        (!already).then(|| LogCapture {
            repository_id: context.id,
            workflow_run_id,
            owner: context.owner_login.clone(),
            name: context.name.clone(),
            github_run_id: run.id,
            run_attempt: run.run_attempt,
            executed_at: run.updated_at.unwrap_or_else(Utc::now),
        })
    } else {
        None
    };

    Ok(Outcome {
        applied: true,
        log_capture,
    })
}

async fn apply_workflow_job(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    payload: &Value,
) -> Result<bool, AppError> {
    let job = serde_json::from_value::<GitHubJob>(
        payload
            .get("workflow_job")
            .cloned()
            .ok_or_else(|| AppError::bad_request("workflow_job payload is missing the job"))?,
    )?;
    workflow_ingest::ingest_workflow_job(transaction, repository_id, &job).await
}

/// Downloads a run's log archive and records its `build_logs` metadata row.
/// Best-effort: any failure here is logged and dropped, never surfaced to the
/// delivery, because the facts are already committed and logs are re-fetchable.
async fn store_run_logs(state: &AppState, capture: LogCapture) -> Result<(), AppError> {
    let Some(token) = github_api::repository_token(state, capture.repository_id).await? else {
        info!(
            repository_id = %capture.repository_id,
            "no usable GitHub token for log capture; skipping"
        );
        return Ok(());
    };
    let api = GitHubApi::new(state, &token);
    let Some(bytes) = api
        .run_logs_zip(&capture.owner, &capture.name, capture.github_run_id)
        .await?
    else {
        return Ok(());
    };

    let object_key = format!(
        "logs/{}/{}/attempt-{}/run.zip",
        capture.repository_id, capture.github_run_id, capture.run_attempt
    );
    if let Err(error) = state
        .log_store
        .put(&object_key, &bytes, "application/zip")
        .await
    {
        warn!(%object_key, error, "log upload to object storage failed");
        return Ok(());
    }

    let digest = Sha256::digest(&bytes).to_vec();
    sqlx::query(
        "INSERT INTO build_logs
            (repository_id, workflow_run_id, storage_bucket, object_key, size_bytes,
             content_type, sha256)
         VALUES ($1, $2, $3, $4, $5, 'application/zip', $6)
         ON CONFLICT (object_key) DO UPDATE SET
             size_bytes = EXCLUDED.size_bytes,
             sha256 = EXCLUDED.sha256",
    )
    .bind(capture.repository_id)
    .bind(capture.workflow_run_id)
    .bind(state.log_store.bucket_name())
    .bind(&object_key)
    .bind(bytes.len() as i64)
    .bind(&digest)
    .execute(&state.db)
    .await?;
    info!(%object_key, size = bytes.len(), "captured workflow run logs");
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
