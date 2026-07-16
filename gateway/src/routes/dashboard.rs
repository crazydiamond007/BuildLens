use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{
    auth::{OrganizationRole, Principal, require_organization_role},
    error::AppError,
    state::AppState,
};

const RECENT_RUN_LIMIT: i64 = 50;
const LIST_LIMIT: i64 = 50;

pub async fn overview(
    State(state): State<AppState>,
    principal: Principal,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<DashboardResponse>, AppError> {
    let role = require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;

    let organization = organization_header(&state.db, organization_id, role).await?;
    let (dora, repositories, runs, flaky_tests, recommendations, reports) = tokio::try_join!(
        dora_series(&state.db, organization_id, None),
        repository_summaries(&state.db, organization_id),
        recent_runs(&state.db, organization_id, None, RECENT_RUN_LIMIT),
        flaky_test_rows(&state.db, organization_id, None, LIST_LIMIT),
        recommendation_rows(&state.db, organization_id, None, LIST_LIMIT),
        report_rows(&state.db, organization_id, None, LIST_LIMIT),
    )?;

    Ok(Json(DashboardResponse {
        organization,
        dora,
        repositories,
        recent_runs: runs,
        flaky_tests,
        recommendations,
        reports,
    }))
}

pub async fn repository(
    State(state): State<AppState>,
    principal: Principal,
    Path((organization_id, repository_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<RepositoryInsightsResponse>, AppError> {
    require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;

    let summary = repository_summary(&state.db, organization_id, repository_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let (dora, scores, runs, flaky_tests, recommendations, reports) = tokio::try_join!(
        dora_series(&state.db, organization_id, Some(repository_id)),
        score_series(&state.db, repository_id),
        recent_runs(
            &state.db,
            organization_id,
            Some(repository_id),
            RECENT_RUN_LIMIT,
        ),
        flaky_test_rows(&state.db, organization_id, Some(repository_id), LIST_LIMIT,),
        recommendation_rows(&state.db, organization_id, Some(repository_id), LIST_LIMIT,),
        report_rows(&state.db, organization_id, Some(repository_id), LIST_LIMIT,),
    )?;

    Ok(Json(RepositoryInsightsResponse {
        repository: summary,
        dora,
        scores,
        recent_runs: runs,
        flaky_tests,
        recommendations,
        reports,
    }))
}

pub async fn run(
    State(state): State<AppState>,
    principal: Principal,
    Path((organization_id, run_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<RunDetailResponse>, AppError> {
    require_organization_role(
        &state,
        &principal,
        organization_id,
        OrganizationRole::Viewer,
    )
    .await?;

    let run = run_header(&state.db, organization_id, run_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let (jobs, tests, report, log) = tokio::try_join!(
        job_rows(&state.db, run_id),
        test_rows(&state.db, run_id),
        run_report(&state.db, run_id),
        build_log(&state.db, run_id),
    )?;
    let recommendations = if let Some(report) = &report {
        recommendations_for_report(&state.db, report.id).await?
    } else {
        Vec::new()
    };

    Ok(Json(RunDetailResponse {
        run,
        jobs,
        tests,
        report,
        recommendations,
        log,
    }))
}

async fn organization_header(
    pool: &PgPool,
    organization_id: Uuid,
    role: OrganizationRole,
) -> Result<OrganizationHeader, AppError> {
    let row = sqlx::query(
        "SELECT id, slug::text AS slug, name, kind
         FROM organizations WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(OrganizationHeader {
        id: row.try_get("id")?,
        slug: row.try_get("slug")?,
        name: row.try_get("name")?,
        kind: row.try_get("kind")?,
        role: role.as_str(),
    })
}

async fn dora_series(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Option<Uuid>,
) -> Result<Vec<DoraMetric>, AppError> {
    sqlx::query(
        "SELECT id, repository_id, granularity, period_start, period_end,
                deployment_count, deployment_frequency::float8 AS deployment_frequency,
                lead_time_p50_seconds, lead_time_p90_seconds,
                change_failure_rate::float8 AS change_failure_rate,
                failed_deployment_count, mttr_p50_seconds, mttr_p90_seconds,
                performance_band, sample_size, computed_at
         FROM dora_metrics
         WHERE organization_id = $1
           AND (($2::uuid IS NULL AND repository_id IS NULL) OR repository_id = $2)
           AND granularity = 'weekly'
         ORDER BY period_start DESC LIMIT 16",
    )
    .bind(organization_id)
    .bind(repository_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(DoraMetric {
            id: row.try_get("id")?,
            repository_id: row.try_get("repository_id")?,
            granularity: row.try_get("granularity")?,
            period_start: row.try_get("period_start")?,
            period_end: row.try_get("period_end")?,
            deployment_count: row.try_get("deployment_count")?,
            deployment_frequency: row.try_get("deployment_frequency")?,
            lead_time_p50_seconds: row.try_get("lead_time_p50_seconds")?,
            lead_time_p90_seconds: row.try_get("lead_time_p90_seconds")?,
            change_failure_rate: row.try_get("change_failure_rate")?,
            failed_deployment_count: row.try_get("failed_deployment_count")?,
            mttr_p50_seconds: row.try_get("mttr_p50_seconds")?,
            mttr_p90_seconds: row.try_get("mttr_p90_seconds")?,
            performance_band: row.try_get("performance_band")?,
            sample_size: row.try_get("sample_size")?,
            computed_at: row.try_get("computed_at")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

async fn repository_summaries(
    pool: &PgPool,
    organization_id: Uuid,
) -> Result<Vec<RepositorySummary>, AppError> {
    sqlx::query(repository_summary_sql())
        .bind(organization_id)
        .bind(Option::<Uuid>::None)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(repository_summary_from_row)
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(AppError::from)
}

async fn repository_summary(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Uuid,
) -> Result<Option<RepositorySummary>, AppError> {
    sqlx::query(repository_summary_sql())
        .bind(organization_id)
        .bind(Some(repository_id))
        .fetch_optional(pool)
        .await?
        .map(repository_summary_from_row)
        .transpose()
        .map_err(AppError::from)
}

fn repository_summary_sql() -> &'static str {
    "SELECT r.id, r.full_name, r.description, r.default_branch, r.primary_language,
            r.tracking_enabled, r.is_private, r.html_url,
            latest.overall_score::float8 AS overall_score,
            latest.reliability_score::float8 AS reliability_score,
            latest.velocity_score::float8 AS velocity_score,
            latest.quality_score::float8 AS quality_score,
            latest.efficiency_score::float8 AS efficiency_score,
            latest.grade, latest.computed_at AS score_computed_at,
            COALESCE(stats.run_count, 0)::bigint AS run_count,
            COALESCE(stats.failure_count, 0)::bigint AS failure_count,
            stats.last_run_at,
            COALESCE(flaky.flaky_count, 0)::bigint AS flaky_count,
            COALESCE(recs.open_recommendations, 0)::bigint AS open_recommendations
     FROM repositories r
     LEFT JOIN LATERAL (
         SELECT overall_score, reliability_score, velocity_score, quality_score,
                efficiency_score, grade, computed_at
         FROM repository_scores WHERE repository_id = r.id
         ORDER BY computed_at DESC LIMIT 1
     ) latest ON true
     LEFT JOIN LATERAL (
         SELECT count(*) AS run_count,
                count(*) FILTER (WHERE conclusion IN ('failure','timed_out','startup_failure','action_required')) AS failure_count,
                max(created_at_github) AS last_run_at
         FROM workflow_runs
         WHERE repository_id = r.id AND created_at_github >= now() - interval '30 days'
     ) stats ON true
     LEFT JOIN LATERAL (
         SELECT count(*) AS flaky_count FROM flaky_tests
         WHERE repository_id = r.id AND is_flaky AND NOT is_quarantined
     ) flaky ON true
     LEFT JOIN LATERAL (
         SELECT count(*) AS open_recommendations FROM ai_recommendations
         WHERE repository_id = r.id AND status = 'open'
     ) recs ON true
     WHERE r.organization_id = $1 AND r.deleted_at IS NULL
       AND ($2::uuid IS NULL OR r.id = $2)
     ORDER BY latest.overall_score DESC NULLS LAST, r.full_name"
}

fn repository_summary_from_row(
    row: sqlx::postgres::PgRow,
) -> Result<RepositorySummary, sqlx::Error> {
    Ok(RepositorySummary {
        id: row.try_get("id")?,
        full_name: row.try_get("full_name")?,
        description: row.try_get("description")?,
        default_branch: row.try_get("default_branch")?,
        primary_language: row.try_get("primary_language")?,
        tracking_enabled: row.try_get("tracking_enabled")?,
        is_private: row.try_get("is_private")?,
        html_url: row.try_get("html_url")?,
        overall_score: row.try_get("overall_score")?,
        reliability_score: row.try_get("reliability_score")?,
        velocity_score: row.try_get("velocity_score")?,
        quality_score: row.try_get("quality_score")?,
        efficiency_score: row.try_get("efficiency_score")?,
        grade: row.try_get("grade")?,
        score_computed_at: row.try_get("score_computed_at")?,
        run_count: row.try_get("run_count")?,
        failure_count: row.try_get("failure_count")?,
        last_run_at: row.try_get("last_run_at")?,
        flaky_count: row.try_get("flaky_count")?,
        open_recommendations: row.try_get("open_recommendations")?,
    })
}

async fn recent_runs(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<RunSummary>, AppError> {
    sqlx::query(
        "SELECT wr.id, wr.repository_id, r.full_name AS repository, wr.run_number,
                wr.run_attempt, wr.name, wr.event, wr.status, wr.conclusion,
                wr.head_sha, wr.head_branch, wr.actor_login, wr.created_at_github,
                wr.run_started_at, wr.completed_at, wr.queued_duration_ms,
                wr.duration_ms, bs.score::float8 AS score
         FROM workflow_runs wr
         JOIN repositories r ON r.id = wr.repository_id
         LEFT JOIN build_scores bs ON bs.workflow_run_id = wr.id
         WHERE r.organization_id = $1 AND r.deleted_at IS NULL
           AND ($2::uuid IS NULL OR wr.repository_id = $2)
         ORDER BY wr.created_at_github DESC NULLS LAST, wr.created_at DESC
         LIMIT $3",
    )
    .bind(organization_id)
    .bind(repository_id)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(run_summary_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

fn run_summary_from_row(row: sqlx::postgres::PgRow) -> Result<RunSummary, sqlx::Error> {
    Ok(RunSummary {
        id: row.try_get("id")?,
        repository_id: row.try_get("repository_id")?,
        repository: row.try_get("repository")?,
        run_number: row.try_get("run_number")?,
        run_attempt: row.try_get("run_attempt")?,
        name: row.try_get("name")?,
        event: row.try_get("event")?,
        status: row.try_get("status")?,
        conclusion: row.try_get("conclusion")?,
        head_sha: row.try_get("head_sha")?,
        head_branch: row.try_get("head_branch")?,
        actor_login: row.try_get("actor_login")?,
        created_at: row.try_get("created_at_github")?,
        started_at: row.try_get("run_started_at")?,
        completed_at: row.try_get("completed_at")?,
        queued_duration_ms: row.try_get("queued_duration_ms")?,
        duration_ms: row.try_get("duration_ms")?,
        score: row.try_get("score")?,
    })
}

async fn score_series(pool: &PgPool, repository_id: Uuid) -> Result<Vec<ScorePoint>, AppError> {
    sqlx::query(
        "SELECT overall_score::float8 AS overall_score,
                reliability_score::float8 AS reliability_score,
                velocity_score::float8 AS velocity_score,
                quality_score::float8 AS quality_score,
                efficiency_score::float8 AS efficiency_score,
                grade, computed_at
         FROM repository_scores WHERE repository_id = $1
         ORDER BY computed_at DESC LIMIT 30",
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(ScorePoint {
            overall_score: row.try_get("overall_score")?,
            reliability_score: row.try_get("reliability_score")?,
            velocity_score: row.try_get("velocity_score")?,
            quality_score: row.try_get("quality_score")?,
            efficiency_score: row.try_get("efficiency_score")?,
            grade: row.try_get("grade")?,
            computed_at: row.try_get("computed_at")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

async fn flaky_test_rows(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<FlakyTest>, AppError> {
    sqlx::query(
        "SELECT ft.id, ft.repository_id, r.full_name AS repository, ft.test_key,
                ft.suite, ft.classname, ft.name, ft.window_days, ft.total_runs,
                ft.passed_runs, ft.failed_runs, ft.flip_count,
                ft.flake_rate::float8 AS flake_rate, ft.is_flaky,
                ft.is_quarantined, ft.last_seen_at, ft.last_failed_at, ft.computed_at
         FROM flaky_tests ft JOIN repositories r ON r.id = ft.repository_id
         WHERE r.organization_id = $1 AND ft.is_flaky
           AND ($2::uuid IS NULL OR ft.repository_id = $2)
         ORDER BY ft.is_quarantined, ft.flake_rate DESC, ft.flip_count DESC LIMIT $3",
    )
    .bind(organization_id)
    .bind(repository_id)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(FlakyTest {
            id: row.try_get("id")?,
            repository_id: row.try_get("repository_id")?,
            repository: row.try_get("repository")?,
            test_key: row.try_get("test_key")?,
            suite: row.try_get("suite")?,
            classname: row.try_get("classname")?,
            name: row.try_get("name")?,
            window_days: row.try_get("window_days")?,
            total_runs: row.try_get("total_runs")?,
            passed_runs: row.try_get("passed_runs")?,
            failed_runs: row.try_get("failed_runs")?,
            flip_count: row.try_get("flip_count")?,
            flake_rate: row.try_get("flake_rate")?,
            is_flaky: row.try_get("is_flaky")?,
            is_quarantined: row.try_get("is_quarantined")?,
            last_seen_at: row.try_get("last_seen_at")?,
            last_failed_at: row.try_get("last_failed_at")?,
            computed_at: row.try_get("computed_at")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

async fn recommendation_rows(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Recommendation>, AppError> {
    sqlx::query(
        "SELECT ar.id, ar.ai_report_id, ar.repository_id, r.full_name AS repository,
                ar.category, ar.severity, ar.title, ar.body_md, ar.evidence,
                ar.status, ar.created_at
         FROM ai_recommendations ar JOIN repositories r ON r.id = ar.repository_id
         WHERE r.organization_id = $1
           AND ar.status = 'open'
           AND ($2::uuid IS NULL OR ar.repository_id = $2)
         ORDER BY CASE ar.severity WHEN 'critical' THEN 0 WHEN 'high' THEN 1
                       WHEN 'medium' THEN 2 WHEN 'low' THEN 3 ELSE 4 END,
                  ar.created_at DESC LIMIT $3",
    )
    .bind(organization_id)
    .bind(repository_id)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(recommendation_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

fn recommendation_from_row(row: sqlx::postgres::PgRow) -> Result<Recommendation, sqlx::Error> {
    Ok(Recommendation {
        id: row.try_get("id")?,
        report_id: row.try_get("ai_report_id")?,
        repository_id: row.try_get("repository_id")?,
        repository: row.try_get("repository")?,
        category: row.try_get("category")?,
        severity: row.try_get("severity")?,
        title: row.try_get("title")?,
        body_md: row.try_get("body_md")?,
        evidence: row.try_get("evidence")?,
        status: row.try_get("status")?,
        created_at: row.try_get("created_at")?,
    })
}

async fn report_rows(
    pool: &PgPool,
    organization_id: Uuid,
    repository_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<AiReport>, AppError> {
    sqlx::query(
        "SELECT ar.id, ar.repository_id, r.full_name AS repository, ar.workflow_run_id,
                ar.kind, ar.status, ar.title, ar.summary, ar.content_md, ar.content,
                ar.model, ar.prompt_version, ar.input_tokens, ar.output_tokens,
                ar.cost_usd::float8 AS cost_usd, ar.latency_ms, ar.error,
                ar.requested_at, ar.completed_at
         FROM ai_reports ar LEFT JOIN repositories r ON r.id = ar.repository_id
         WHERE ar.organization_id = $1
           AND ($2::uuid IS NULL OR ar.repository_id = $2)
         ORDER BY ar.requested_at DESC LIMIT $3",
    )
    .bind(organization_id)
    .bind(repository_id)
    .bind(limit)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(report_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

fn report_from_row(row: sqlx::postgres::PgRow) -> Result<AiReport, sqlx::Error> {
    Ok(AiReport {
        id: row.try_get("id")?,
        repository_id: row.try_get("repository_id")?,
        repository: row.try_get("repository")?,
        workflow_run_id: row.try_get("workflow_run_id")?,
        kind: row.try_get("kind")?,
        status: row.try_get("status")?,
        title: row.try_get("title")?,
        summary: row.try_get("summary")?,
        content_md: row.try_get("content_md")?,
        content: row.try_get("content")?,
        model: row.try_get("model")?,
        prompt_version: row.try_get("prompt_version")?,
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        cost_usd: row.try_get("cost_usd")?,
        latency_ms: row.try_get("latency_ms")?,
        error: row.try_get("error")?,
        requested_at: row.try_get("requested_at")?,
        completed_at: row.try_get("completed_at")?,
    })
}

async fn run_header(
    pool: &PgPool,
    organization_id: Uuid,
    run_id: Uuid,
) -> Result<Option<RunHeader>, AppError> {
    sqlx::query(
        "SELECT wr.id, wr.repository_id, r.full_name AS repository, wr.github_run_id,
                wr.run_attempt, wr.run_number, wr.name, wr.event, wr.status,
                wr.conclusion, wr.head_sha, wr.head_branch, wr.actor_login,
                wr.triggering_actor_login, wr.is_default_branch,
                wr.created_at_github, wr.run_started_at, wr.completed_at,
                wr.queued_duration_ms, wr.duration_ms,
                bs.score::float8 AS score, bs.duration_score::float8 AS duration_score,
                bs.reliability_score::float8 AS reliability_score,
                bs.flakiness_score::float8 AS flakiness_score, bs.breakdown
         FROM workflow_runs wr JOIN repositories r ON r.id = wr.repository_id
         LEFT JOIN build_scores bs ON bs.workflow_run_id = wr.id
         WHERE wr.id = $1 AND r.organization_id = $2 AND r.deleted_at IS NULL",
    )
    .bind(run_id)
    .bind(organization_id)
    .fetch_optional(pool)
    .await?
    .map(|row| -> Result<RunHeader, sqlx::Error> {
        Ok(RunHeader {
            id: row.try_get("id")?,
            repository_id: row.try_get("repository_id")?,
            repository: row.try_get("repository")?,
            github_run_id: row.try_get("github_run_id")?,
            run_attempt: row.try_get("run_attempt")?,
            run_number: row.try_get("run_number")?,
            name: row.try_get("name")?,
            event: row.try_get("event")?,
            status: row.try_get("status")?,
            conclusion: row.try_get("conclusion")?,
            head_sha: row.try_get("head_sha")?,
            head_branch: row.try_get("head_branch")?,
            actor_login: row.try_get("actor_login")?,
            triggering_actor_login: row.try_get("triggering_actor_login")?,
            is_default_branch: row.try_get("is_default_branch")?,
            created_at: row.try_get("created_at_github")?,
            started_at: row.try_get("run_started_at")?,
            completed_at: row.try_get("completed_at")?,
            queued_duration_ms: row.try_get("queued_duration_ms")?,
            duration_ms: row.try_get("duration_ms")?,
            score: row.try_get("score")?,
            duration_score: row.try_get("duration_score")?,
            reliability_score: row.try_get("reliability_score")?,
            flakiness_score: row.try_get("flakiness_score")?,
            score_breakdown: row.try_get("breakdown")?,
        })
    })
    .transpose()
    .map_err(AppError::from)
}

async fn job_rows(pool: &PgPool, run_id: Uuid) -> Result<Vec<JobDetail>, AppError> {
    let job_rows = sqlx::query(
        "SELECT id, name, status, conclusion, runner_name, runner_group_name,
                labels, started_at, completed_at, duration_ms
         FROM workflow_jobs WHERE workflow_run_id = $1 ORDER BY started_at, id",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;
    let mut jobs = Vec::with_capacity(job_rows.len());
    let mut positions = HashMap::with_capacity(job_rows.len());
    for row in job_rows {
        let id: Uuid = row.try_get("id")?;
        positions.insert(id, jobs.len());
        jobs.push(JobDetail {
            id,
            name: row.try_get("name")?,
            status: row.try_get("status")?,
            conclusion: row.try_get("conclusion")?,
            runner_name: row.try_get("runner_name")?,
            runner_group_name: row.try_get("runner_group_name")?,
            labels: row.try_get("labels")?,
            started_at: row.try_get("started_at")?,
            completed_at: row.try_get("completed_at")?,
            duration_ms: row.try_get("duration_ms")?,
            steps: Vec::new(),
        });
    }

    for row in sqlx::query(
        "SELECT ws.id, ws.workflow_job_id, ws.number, ws.name, ws.status,
                ws.conclusion, ws.started_at, ws.completed_at, ws.duration_ms
         FROM workflow_steps ws JOIN workflow_jobs wj ON wj.id = ws.workflow_job_id
         WHERE wj.workflow_run_id = $1 ORDER BY wj.id, ws.number",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?
    {
        let job_id: Uuid = row.try_get("workflow_job_id")?;
        if let Some(position) = positions.get(&job_id) {
            jobs[*position].steps.push(StepDetail {
                id: row.try_get("id")?,
                number: row.try_get("number")?,
                name: row.try_get("name")?,
                status: row.try_get("status")?,
                conclusion: row.try_get("conclusion")?,
                started_at: row.try_get("started_at")?,
                completed_at: row.try_get("completed_at")?,
                duration_ms: row.try_get("duration_ms")?,
            });
        }
    }
    Ok(jobs)
}

async fn test_rows(pool: &PgPool, run_id: Uuid) -> Result<Vec<TestResult>, AppError> {
    sqlx::query(
        "SELECT id, workflow_job_id, test_key, suite, classname, name, status,
                duration_ms, failure_type, failure_message, executed_at
         FROM test_results WHERE workflow_run_id = $1
         ORDER BY CASE status WHEN 'error' THEN 0 WHEN 'failed' THEN 1 ELSE 2 END,
                  test_key LIMIT 500",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        Ok(TestResult {
            id: row.try_get("id")?,
            workflow_job_id: row.try_get("workflow_job_id")?,
            test_key: row.try_get("test_key")?,
            suite: row.try_get("suite")?,
            classname: row.try_get("classname")?,
            name: row.try_get("name")?,
            status: row.try_get("status")?,
            duration_ms: row.try_get("duration_ms")?,
            failure_type: row.try_get("failure_type")?,
            failure_message: row.try_get("failure_message")?,
            executed_at: row.try_get("executed_at")?,
        })
    })
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

async fn run_report(pool: &PgPool, run_id: Uuid) -> Result<Option<AiReport>, AppError> {
    sqlx::query(
        "SELECT ar.id, ar.repository_id, r.full_name AS repository, ar.workflow_run_id,
                ar.kind, ar.status, ar.title, ar.summary, ar.content_md, ar.content,
                ar.model, ar.prompt_version, ar.input_tokens, ar.output_tokens,
                ar.cost_usd::float8 AS cost_usd, ar.latency_ms, ar.error,
                ar.requested_at, ar.completed_at
         FROM ai_reports ar LEFT JOIN repositories r ON r.id = ar.repository_id
         WHERE ar.workflow_run_id = $1 ORDER BY ar.requested_at DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    .map(report_from_row)
    .transpose()
    .map_err(AppError::from)
}

async fn recommendations_for_report(
    pool: &PgPool,
    report_id: Uuid,
) -> Result<Vec<Recommendation>, AppError> {
    sqlx::query(
        "SELECT ar.id, ar.ai_report_id, ar.repository_id, r.full_name AS repository,
                ar.category, ar.severity, ar.title, ar.body_md, ar.evidence,
                ar.status, ar.created_at
         FROM ai_recommendations ar JOIN repositories r ON r.id = ar.repository_id
         WHERE ar.ai_report_id = $1 ORDER BY ar.created_at",
    )
    .bind(report_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(recommendation_from_row)
    .collect::<Result<Vec<_>, sqlx::Error>>()
    .map_err(AppError::from)
}

async fn build_log(pool: &PgPool, run_id: Uuid) -> Result<Option<BuildLog>, AppError> {
    sqlx::query(
        "SELECT id, size_bytes, content_type, content_encoding, line_count,
                expires_at, created_at
         FROM build_logs WHERE workflow_run_id = $1 AND workflow_job_id IS NULL
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    .map(|row| -> Result<BuildLog, sqlx::Error> {
        Ok(BuildLog {
            id: row.try_get("id")?,
            size_bytes: row.try_get("size_bytes")?,
            content_type: row.try_get("content_type")?,
            content_encoding: row.try_get("content_encoding")?,
            line_count: row.try_get("line_count")?,
            expires_at: row.try_get("expires_at")?,
            created_at: row.try_get("created_at")?,
        })
    })
    .transpose()
    .map_err(AppError::from)
}

#[derive(Serialize)]
pub struct DashboardResponse {
    organization: OrganizationHeader,
    dora: Vec<DoraMetric>,
    repositories: Vec<RepositorySummary>,
    recent_runs: Vec<RunSummary>,
    flaky_tests: Vec<FlakyTest>,
    recommendations: Vec<Recommendation>,
    reports: Vec<AiReport>,
}

#[derive(Serialize)]
pub struct RepositoryInsightsResponse {
    repository: RepositorySummary,
    dora: Vec<DoraMetric>,
    scores: Vec<ScorePoint>,
    recent_runs: Vec<RunSummary>,
    flaky_tests: Vec<FlakyTest>,
    recommendations: Vec<Recommendation>,
    reports: Vec<AiReport>,
}

#[derive(Serialize)]
pub struct RunDetailResponse {
    run: RunHeader,
    jobs: Vec<JobDetail>,
    tests: Vec<TestResult>,
    report: Option<AiReport>,
    recommendations: Vec<Recommendation>,
    log: Option<BuildLog>,
}

#[derive(Serialize)]
struct OrganizationHeader {
    id: Uuid,
    slug: String,
    name: String,
    kind: String,
    role: &'static str,
}

#[derive(Serialize)]
struct DoraMetric {
    id: Uuid,
    repository_id: Option<Uuid>,
    granularity: String,
    period_start: NaiveDate,
    period_end: NaiveDate,
    deployment_count: i32,
    deployment_frequency: Option<f64>,
    lead_time_p50_seconds: Option<i64>,
    lead_time_p90_seconds: Option<i64>,
    change_failure_rate: Option<f64>,
    failed_deployment_count: i32,
    mttr_p50_seconds: Option<i64>,
    mttr_p90_seconds: Option<i64>,
    performance_band: Option<String>,
    sample_size: i32,
    computed_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct RepositorySummary {
    id: Uuid,
    full_name: String,
    description: Option<String>,
    default_branch: String,
    primary_language: Option<String>,
    tracking_enabled: bool,
    is_private: bool,
    html_url: Option<String>,
    overall_score: Option<f64>,
    reliability_score: Option<f64>,
    velocity_score: Option<f64>,
    quality_score: Option<f64>,
    efficiency_score: Option<f64>,
    grade: Option<String>,
    score_computed_at: Option<DateTime<Utc>>,
    run_count: i64,
    failure_count: i64,
    last_run_at: Option<DateTime<Utc>>,
    flaky_count: i64,
    open_recommendations: i64,
}

#[derive(Serialize)]
struct RunSummary {
    id: Uuid,
    repository_id: Uuid,
    repository: String,
    run_number: i32,
    run_attempt: i32,
    name: Option<String>,
    event: String,
    status: String,
    conclusion: Option<String>,
    head_sha: String,
    head_branch: Option<String>,
    actor_login: Option<String>,
    created_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    queued_duration_ms: Option<i64>,
    duration_ms: Option<i64>,
    score: Option<f64>,
}

#[derive(Serialize)]
struct ScorePoint {
    overall_score: f64,
    reliability_score: Option<f64>,
    velocity_score: Option<f64>,
    quality_score: Option<f64>,
    efficiency_score: Option<f64>,
    grade: Option<String>,
    computed_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct FlakyTest {
    id: Uuid,
    repository_id: Uuid,
    repository: String,
    test_key: String,
    suite: Option<String>,
    classname: Option<String>,
    name: String,
    window_days: i32,
    total_runs: i32,
    passed_runs: i32,
    failed_runs: i32,
    flip_count: i32,
    flake_rate: f64,
    is_flaky: bool,
    is_quarantined: bool,
    last_seen_at: Option<DateTime<Utc>>,
    last_failed_at: Option<DateTime<Utc>>,
    computed_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct Recommendation {
    id: Uuid,
    report_id: Uuid,
    repository_id: Uuid,
    repository: String,
    category: String,
    severity: String,
    title: String,
    body_md: String,
    evidence: Value,
    status: String,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct AiReport {
    id: Uuid,
    repository_id: Option<Uuid>,
    repository: Option<String>,
    workflow_run_id: Option<Uuid>,
    kind: String,
    status: String,
    title: Option<String>,
    summary: Option<String>,
    content_md: Option<String>,
    content: Value,
    model: Option<String>,
    prompt_version: Option<String>,
    input_tokens: Option<i32>,
    output_tokens: Option<i32>,
    cost_usd: Option<f64>,
    latency_ms: Option<i32>,
    error: Option<String>,
    requested_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
struct RunHeader {
    id: Uuid,
    repository_id: Uuid,
    repository: String,
    github_run_id: i64,
    run_attempt: i32,
    run_number: i32,
    name: Option<String>,
    event: String,
    status: String,
    conclusion: Option<String>,
    head_sha: String,
    head_branch: Option<String>,
    actor_login: Option<String>,
    triggering_actor_login: Option<String>,
    is_default_branch: bool,
    created_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    queued_duration_ms: Option<i64>,
    duration_ms: Option<i64>,
    score: Option<f64>,
    duration_score: Option<f64>,
    reliability_score: Option<f64>,
    flakiness_score: Option<f64>,
    score_breakdown: Option<Value>,
}

#[derive(Serialize)]
struct JobDetail {
    id: Uuid,
    name: String,
    status: String,
    conclusion: Option<String>,
    runner_name: Option<String>,
    runner_group_name: Option<String>,
    labels: Vec<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    duration_ms: Option<i64>,
    steps: Vec<StepDetail>,
}

#[derive(Serialize)]
struct StepDetail {
    id: Uuid,
    number: i32,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    duration_ms: Option<i64>,
}

#[derive(Serialize)]
struct TestResult {
    id: Uuid,
    workflow_job_id: Option<Uuid>,
    test_key: String,
    suite: Option<String>,
    classname: Option<String>,
    name: String,
    status: String,
    duration_ms: Option<i64>,
    failure_type: Option<String>,
    failure_message: Option<String>,
    executed_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct BuildLog {
    id: Uuid,
    size_bytes: i64,
    content_type: String,
    content_encoding: Option<String>,
    line_count: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}
