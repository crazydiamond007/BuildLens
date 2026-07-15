//! Turning GitHub Actions facts into rows, and emitting the events that tell
//! analytics to recompute.
//!
//! Shared by two callers: the webhook processor (live `workflow_run` /
//! `workflow_job` deliveries) and the backfill (`repository_sync`). Both land in
//! the same upserts so a run ingested from history and the same run arriving by
//! webhook converge on one row — keyed `(github_run_id, run_attempt)`, because
//! every re-run attempt is its own row (invariant #6).
//!
//! Two events are emitted here, and only on a *transition* into the completed
//! state, so a replayed webhook does not re-fire them (the outbox already makes
//! delivery at-least-once; this keeps the common case at exactly-once):
//!   - `workflow_run.completed` — the DORA / flaky-test trigger.
//!   - `deployment.recorded` — when a successful default-branch run is inferred
//!     to be a production deployment.

use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    error::AppError,
    events::{self, OutboxEvent},
    github_api::{GitHubJob, GitHubWorkflow, GitHubWorkflowRun},
};

/// Events are inferred as production deployments only for these run events.
/// A green pull_request build is CI, not a deploy; a push/release/manual run to
/// the default branch is the thing that ships.
const DEPLOY_EVENTS: [&str; 3] = ["push", "release", "workflow_dispatch"];
const INFERRED_ENVIRONMENT: &str = "production";

/// Ingests one workflow run (and its workflow), returning the run's row id.
/// `workflow` is the nested object from a `workflow_run` webhook; the backfill
/// passes the same thing from the workflows endpoint. Emits
/// `workflow_run.completed` (and possibly `deployment.recorded`) when the run
/// first reaches the completed state.
pub async fn ingest_workflow_run(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    organization_id: Uuid,
    default_branch: &str,
    run: &GitHubWorkflowRun,
    workflow: Option<&GitHubWorkflow>,
) -> Result<Uuid, AppError> {
    // Prefer the workflow object off the webhook; otherwise resolve the FK from
    // a workflow synced earlier. Null is fine — the FK is nullable on purpose.
    let workflow_id = match workflow {
        Some(workflow) => Some(upsert_workflow(transaction, repository_id, workflow).await?),
        None => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM workflows
                 WHERE repository_id = $1 AND github_workflow_id = $2",
            )
            .bind(repository_id)
            .bind(run.workflow_id)
            .fetch_optional(&mut **transaction)
            .await?
        }
    };

    let is_default_branch = run.head_branch.as_deref() == Some(default_branch);
    // The run object has no completed_at; updated_at is the completion time once
    // the run is done.
    let completed_at: Option<DateTime<Utc>> = if run.status == "completed" {
        run.updated_at
    } else {
        None
    };

    // Soft FKs (invariant #3): resolve if we already hold the commit / PR, leave
    // NULL otherwise. A later backfill pass fills them in.
    let head_commit_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM commits WHERE repository_id = $1 AND sha = $2",
    )
    .bind(repository_id)
    .bind(&run.head_sha)
    .fetch_optional(&mut **transaction)
    .await?;
    let pull_request_id = match run.pull_requests.first() {
        Some(pr) => {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM pull_requests WHERE repository_id = $1 AND number = $2",
            )
            .bind(repository_id)
            .bind(pr.number)
            .fetch_optional(&mut **transaction)
            .await?
        }
        None => None,
    };

    let previous_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM workflow_runs WHERE github_run_id = $1 AND run_attempt = $2",
    )
    .bind(run.id)
    .bind(run.run_attempt)
    .fetch_optional(&mut **transaction)
    .await?;

    let run_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO workflow_runs
            (repository_id, workflow_id, github_run_id, run_attempt, run_number, name,
             event, status, conclusion, head_sha, head_branch, head_commit_id,
             pull_request_id, actor_login, triggering_actor_login, is_default_branch,
             created_at_github, run_started_at, completed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                 $17, $18, $19)
         ON CONFLICT (github_run_id, run_attempt) DO UPDATE SET
             workflow_id = COALESCE(EXCLUDED.workflow_id, workflow_runs.workflow_id),
             run_number = EXCLUDED.run_number,
             name = EXCLUDED.name,
             event = EXCLUDED.event,
             status = EXCLUDED.status,
             conclusion = EXCLUDED.conclusion,
             head_sha = EXCLUDED.head_sha,
             head_branch = EXCLUDED.head_branch,
             head_commit_id = COALESCE(EXCLUDED.head_commit_id, workflow_runs.head_commit_id),
             pull_request_id = COALESCE(EXCLUDED.pull_request_id, workflow_runs.pull_request_id),
             actor_login = EXCLUDED.actor_login,
             triggering_actor_login = EXCLUDED.triggering_actor_login,
             is_default_branch = EXCLUDED.is_default_branch,
             created_at_github = EXCLUDED.created_at_github,
             run_started_at = EXCLUDED.run_started_at,
             completed_at = COALESCE(EXCLUDED.completed_at, workflow_runs.completed_at)
         RETURNING id",
    )
    .bind(repository_id)
    .bind(workflow_id)
    .bind(run.id)
    .bind(run.run_attempt)
    .bind(run.run_number)
    .bind(&run.name)
    .bind(&run.event)
    .bind(&run.status)
    .bind(&run.conclusion)
    .bind(&run.head_sha)
    .bind(&run.head_branch)
    .bind(head_commit_id)
    .bind(pull_request_id)
    .bind(run.actor.as_ref().map(|a| a.login.as_str()))
    .bind(run.triggering_actor.as_ref().map(|a| a.login.as_str()))
    .bind(is_default_branch)
    .bind(run.created_at)
    .bind(run.run_started_at)
    .bind(completed_at)
    .fetch_one(&mut **transaction)
    .await?;

    let newly_completed =
        run.status == "completed" && previous_status.as_deref() != Some("completed");
    if newly_completed {
        emit_run_completed(
            transaction,
            organization_id,
            repository_id,
            run_id,
            workflow_id,
            run,
            is_default_branch,
            completed_at,
        )
        .await?;

        if is_default_branch
            && run.conclusion.as_deref() == Some("success")
            && DEPLOY_EVENTS.contains(&run.event.as_str())
        {
            infer_deployment(
                transaction,
                repository_id,
                organization_id,
                run_id,
                run,
                completed_at,
            )
            .await?;
        }
    }

    Ok(run_id)
}

pub async fn upsert_workflow(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    workflow: &GitHubWorkflow,
) -> Result<Uuid, AppError> {
    let state = workflow.state.as_deref().unwrap_or("active");
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO workflows (repository_id, github_workflow_id, name, path, state)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (repository_id, github_workflow_id) DO UPDATE SET
             name = EXCLUDED.name,
             path = EXCLUDED.path,
             state = EXCLUDED.state,
             deleted_at = NULL
         RETURNING id",
    )
    .bind(repository_id)
    .bind(workflow.id)
    .bind(&workflow.name)
    .bind(&workflow.path)
    .bind(state)
    .fetch_one(&mut **transaction)
    .await
    .map_err(AppError::from)
}

/// Ingests one job and its steps. Returns `false` (and does nothing) when the
/// run it belongs to has not been ingested yet — a `workflow_job` can arrive
/// before its `workflow_run` (webhooks are unordered). The delivery is recorded
/// and ignored; the job re-arrives with the run, or the backfill catches it.
pub async fn ingest_workflow_job(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    job: &GitHubJob,
) -> Result<bool, AppError> {
    let Some(workflow_run_id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM workflow_runs WHERE github_run_id = $1 AND run_attempt = $2",
    )
    .bind(job.run_id)
    .bind(job.run_attempt)
    .fetch_optional(&mut **transaction)
    .await?
    else {
        return Ok(false);
    };

    let job_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO workflow_jobs
            (workflow_run_id, repository_id, github_job_id, run_attempt, name, status,
             conclusion, runner_id, runner_name, runner_group_name, labels, started_at,
             completed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (github_job_id) DO UPDATE SET
             status = EXCLUDED.status,
             conclusion = EXCLUDED.conclusion,
             runner_id = EXCLUDED.runner_id,
             runner_name = EXCLUDED.runner_name,
             runner_group_name = EXCLUDED.runner_group_name,
             labels = EXCLUDED.labels,
             started_at = EXCLUDED.started_at,
             completed_at = EXCLUDED.completed_at
         RETURNING id",
    )
    .bind(workflow_run_id)
    .bind(repository_id)
    .bind(job.id)
    .bind(job.run_attempt)
    .bind(&job.name)
    .bind(&job.status)
    .bind(&job.conclusion)
    .bind(job.runner_id)
    .bind(&job.runner_name)
    .bind(&job.runner_group_name)
    .bind(&job.labels)
    .bind(job.started_at)
    .bind(job.completed_at)
    .fetch_one(&mut **transaction)
    .await?;

    for step in &job.steps {
        sqlx::query(
            "INSERT INTO workflow_steps
                (workflow_job_id, number, name, status, conclusion, started_at, completed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (workflow_job_id, number) DO UPDATE SET
                 name = EXCLUDED.name,
                 status = EXCLUDED.status,
                 conclusion = EXCLUDED.conclusion,
                 started_at = EXCLUDED.started_at,
                 completed_at = EXCLUDED.completed_at",
        )
        .bind(job_id)
        .bind(step.number)
        .bind(&step.name)
        .bind(&step.status)
        .bind(&step.conclusion)
        .bind(step.started_at)
        .bind(step.completed_at)
        .execute(&mut **transaction)
        .await?;
    }

    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn emit_run_completed(
    transaction: &mut Transaction<'_, Postgres>,
    organization_id: Uuid,
    repository_id: Uuid,
    run_id: Uuid,
    workflow_id: Option<Uuid>,
    run: &GitHubWorkflowRun,
    is_default_branch: bool,
    completed_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    let data = json!({
        "workflow_run_id": run_id,
        "github_run_id": run.id,
        "run_attempt": run.run_attempt,
        "run_number": run.run_number,
        "workflow_id": workflow_id,
        "github_workflow_id": run.workflow_id,
        "name": run.name,
        "event": run.event,
        "conclusion": run.conclusion,
        "head_sha": run.head_sha,
        "head_branch": run.head_branch,
        "is_default_branch": is_default_branch,
        "run_started_at": run.run_started_at,
        "completed_at": completed_at,
    });
    events::enqueue(
        transaction,
        OutboxEvent::new(
            "workflow_run",
            run_id,
            "workflow_run.completed",
            organization_id,
            repository_id,
            data,
        ),
    )
    .await?;
    Ok(())
}

async fn infer_deployment(
    transaction: &mut Transaction<'_, Postgres>,
    repository_id: Uuid,
    organization_id: Uuid,
    run_id: Uuid,
    run: &GitHubWorkflowRun,
    completed_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    // The partial unique index (workflow_run_id, environment) WHERE inferred
    // makes this idempotent: a replayed completion does not create a second
    // deployment. DO NOTHING + RETURNING means we only emit an event when a row
    // is actually created.
    let deployment_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO deployments
            (repository_id, workflow_run_id, environment, sha, status, source,
             is_production, deployed_at, started_at)
         VALUES ($1, $2, $3, $4, 'success', 'workflow_inferred', true, $5, $6)
         ON CONFLICT (workflow_run_id, environment) WHERE source = 'workflow_inferred'
         DO NOTHING
         RETURNING id",
    )
    .bind(repository_id)
    .bind(run_id)
    .bind(INFERRED_ENVIRONMENT)
    .bind(&run.head_sha)
    .bind(completed_at)
    .bind(run.run_started_at)
    .fetch_optional(&mut **transaction)
    .await?;

    let Some(deployment_id) = deployment_id else {
        return Ok(());
    };

    let data = json!({
        "deployment_id": deployment_id,
        "workflow_run_id": run_id,
        "environment": INFERRED_ENVIRONMENT,
        "sha": run.head_sha,
        "is_production": true,
        "source": "workflow_inferred",
        "status": "success",
        "deployed_at": completed_at,
    });
    events::enqueue(
        transaction,
        OutboxEvent::new(
            "deployment",
            deployment_id,
            "deployment.recorded",
            organization_id,
            repository_id,
            data,
        ),
    )
    .await?;
    Ok(())
}
