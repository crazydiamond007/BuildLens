import asyncio
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from decimal import Decimal
from typing import Any
from uuid import UUID

import asyncpg
import boto3

from buildlens_ai.config import Settings
from buildlens_ai.errors import GroundingError, ReportGenerationError
from buildlens_ai.models import Evidence, ReportClaim, ReportOutput
from buildlens_ai.redaction import ExtractedRange, extract_failure_ranges, redact_data


@dataclass(frozen=True)
class ContextBundle:
    prompt_data: dict[str, Any]
    job_ids: frozenset[UUID]
    step_ids: frozenset[UUID]
    test_keys: frozenset[str]
    log_ranges: tuple[ExtractedRange, ...]
    metric_keys: frozenset[str]


class ContextLoader:
    def __init__(self, pool: asyncpg.Pool, settings: Settings) -> None:
        self.pool = pool
        self.settings = settings
        self.s3 = boto3.client(
            "s3",
            endpoint_url=settings.s3_endpoint,
            region_name=settings.s3_region,
            aws_access_key_id=settings.s3_access_key.get_secret_value(),
            aws_secret_access_key=settings.s3_secret_key.get_secret_value(),
        )

    async def load(self, claim: ReportClaim) -> ContextBundle:
        if claim.workflow_run_id is not None:
            return await self._load_workflow(claim)
        return await self._load_repository(claim)

    async def _load_workflow(self, claim: ReportClaim) -> ContextBundle:
        run = await self.pool.fetchrow(
            """
            SELECT wr.id, wr.github_run_id, wr.run_attempt, wr.run_number, wr.name,
                   wr.event, wr.status, wr.conclusion, wr.head_sha, wr.head_branch,
                   wr.is_default_branch, wr.created_at_github, wr.run_started_at,
                   wr.completed_at, wr.queued_duration_ms, wr.duration_ms,
                   r.full_name AS repository
            FROM workflow_runs wr JOIN repositories r ON r.id = wr.repository_id
            WHERE wr.id = $1 AND wr.repository_id = $2
            """,
            claim.workflow_run_id,
            claim.repository_id,
        )
        if run is None:
            raise ReportGenerationError("workflow run disappeared before report generation")
        jobs = await self.pool.fetch(
            """
            SELECT id, name, status, conclusion, runner_name, runner_group_name,
                   started_at, completed_at, duration_ms
            FROM workflow_jobs WHERE workflow_run_id = $1 ORDER BY started_at, id
            """,
            claim.workflow_run_id,
        )
        steps = await self.pool.fetch(
            """
            SELECT ws.id, ws.workflow_job_id, ws.number, ws.name, ws.status,
                   ws.conclusion, ws.started_at, ws.completed_at, ws.duration_ms
            FROM workflow_steps ws
            JOIN workflow_jobs wj ON wj.id = ws.workflow_job_id
            WHERE wj.workflow_run_id = $1 ORDER BY wj.id, ws.number
            """,
            claim.workflow_run_id,
        )
        tests = await self.pool.fetch(
            """
            SELECT test_key, suite, classname, name, status, duration_ms,
                   failure_type, failure_message, workflow_job_id
            FROM test_results WHERE workflow_run_id = $1
            ORDER BY CASE status WHEN 'error' THEN 0 WHEN 'failed' THEN 1 ELSE 2 END,
                     test_key
            LIMIT 500
            """,
            claim.workflow_run_id,
        )
        log_ranges = await self._load_log_ranges(claim.workflow_run_id)
        prompt_data = {
            "report_kind": claim.kind,
            "workflow_run": _record(run),
            "jobs": [_record(row) for row in jobs],
            "steps": [_record(row) for row in steps],
            "test_results": [_record(row) for row in tests],
            "log_excerpts": [
                {
                    "source": item.source,
                    "start_line": item.start_line,
                    "end_line": item.end_line,
                    "text": item.text,
                }
                for item in log_ranges
            ],
        }
        return ContextBundle(
            prompt_data=redact_data(prompt_data),
            job_ids=frozenset(row["id"] for row in jobs),
            step_ids=frozenset(row["id"] for row in steps),
            test_keys=frozenset(row["test_key"] for row in tests),
            log_ranges=tuple(log_ranges),
            metric_keys=frozenset(),
        )

    async def _load_repository(self, claim: ReportClaim) -> ContextBundle:
        since = datetime.now(UTC) - timedelta(days=30)
        repository = await self.pool.fetchrow(
            """
            SELECT id, organization_id, full_name, default_branch
            FROM repositories WHERE id = $1 AND organization_id = $2 AND deleted_at IS NULL
            """,
            claim.repository_id,
            claim.organization_id,
        )
        if repository is None:
            raise ReportGenerationError("repository disappeared before report generation")
        dora = await self.pool.fetch(
            """
            SELECT id, granularity, period_start, period_end, deployment_count,
                   deployment_frequency, lead_time_p50_seconds, lead_time_p90_seconds,
                   change_failure_rate, failed_deployment_count, mttr_p50_seconds,
                   mttr_p90_seconds, performance_band, sample_size
            FROM dora_metrics
            WHERE repository_id = $1 AND period_end >= $2::date
            ORDER BY period_start DESC, granularity LIMIT 30
            """,
            claim.repository_id,
            since.date(),
        )
        repository_scores = await self.pool.fetch(
            """
            SELECT id, window_days, overall_score, reliability_score, velocity_score,
                   quality_score, efficiency_score, grade, breakdown, computed_at
            FROM repository_scores WHERE repository_id = $1
            ORDER BY computed_at DESC LIMIT 7
            """,
            claim.repository_id,
        )
        flaky = await self.pool.fetch(
            """
            SELECT id, test_key, suite, classname, name, total_runs, failed_runs,
                   flip_count, flake_rate, is_flaky, last_failed_at
            FROM flaky_tests WHERE repository_id = $1 AND is_flaky
            ORDER BY flake_rate DESC, test_key LIMIT 50
            """,
            claim.repository_id,
        )
        build_scores = await self.pool.fetch(
            """
            SELECT bs.id, bs.workflow_run_id, bs.score, bs.duration_score,
                   bs.reliability_score, bs.flakiness_score, bs.computed_at
            FROM build_scores bs
            WHERE bs.repository_id = $1 AND bs.computed_at >= $2
            ORDER BY bs.computed_at DESC LIMIT 50
            """,
            claim.repository_id,
            since,
        )
        metric_rows = [*dora, *repository_scores, *flaky, *build_scores]
        metric_keys = frozenset(f"metric:{row['id']}" for row in metric_rows)
        prompt_data = {
            "report_kind": claim.kind,
            "repository": _record(repository),
            "window_start": since,
            "dora_metrics": [_metric_record(row) for row in dora],
            "repository_scores": [_metric_record(row) for row in repository_scores],
            "flaky_tests": [_metric_record(row) for row in flaky],
            "build_scores": [_metric_record(row) for row in build_scores],
        }
        return ContextBundle(
            prompt_data=redact_data(prompt_data),
            job_ids=frozenset(),
            step_ids=frozenset(),
            test_keys=frozenset(row["test_key"] for row in flaky),
            log_ranges=(),
            metric_keys=metric_keys,
        )

    async def _load_log_ranges(self, workflow_run_id: UUID) -> list[ExtractedRange]:
        row = await self.pool.fetchrow(
            """
            SELECT storage_bucket, object_key, size_bytes
            FROM build_logs
            WHERE workflow_run_id = $1 AND workflow_job_id IS NULL
            ORDER BY created_at DESC LIMIT 1
            """,
            workflow_run_id,
        )
        if row is None or row["size_bytes"] > self.settings.ai_max_log_download_bytes:
            return []
        archive = await asyncio.to_thread(
            self._download_object,
            row["storage_bucket"],
            row["object_key"],
        )
        return await asyncio.to_thread(
            extract_failure_ranges,
            archive,
            max_lines=self.settings.ai_max_log_lines,
            max_prompt_bytes=self.settings.ai_max_log_prompt_bytes,
        )

    def _download_object(self, bucket: str, key: str) -> bytes:
        response = self.s3.get_object(Bucket=bucket, Key=key)
        body = response["Body"]
        try:
            data = body.read(self.settings.ai_max_log_download_bytes + 1)
        finally:
            body.close()
        if len(data) > self.settings.ai_max_log_download_bytes:
            raise ReportGenerationError("build log exceeds the download limit")
        return data


def validate_grounding(output: ReportOutput, context: ContextBundle) -> None:
    for item in [*output.findings, *output.recommendations]:
        evidence = item.evidence
        if not _has_evidence(evidence):
            raise GroundingError(f"{item.title!r} has no evidence")
        unknown_jobs = set(evidence.job_ids) - context.job_ids
        unknown_steps = set(evidence.step_ids) - context.step_ids
        unknown_tests = set(evidence.test_keys) - context.test_keys
        unknown_metrics = set(evidence.metric_keys) - context.metric_keys
        if unknown_jobs or unknown_steps or unknown_tests or unknown_metrics:
            raise GroundingError(f"{item.title!r} cites evidence outside the supplied context")
        for cited in evidence.log_ranges:
            if not any(
                cited.source == supplied.source
                and cited.start_line >= supplied.start_line
                and cited.end_line <= supplied.end_line
                for supplied in context.log_ranges
            ):
                raise GroundingError(f"{item.title!r} cites an unavailable log range")


def _has_evidence(evidence: Evidence) -> bool:
    return any(
        (
            evidence.job_ids,
            evidence.step_ids,
            evidence.test_keys,
            evidence.log_ranges,
            evidence.metric_keys,
        )
    )


def _record(row: asyncpg.Record) -> dict[str, Any]:
    return {key: _value(value) for key, value in dict(row).items()}


def _metric_record(row: asyncpg.Record) -> dict[str, Any]:
    value = _record(row)
    value["metric_key"] = f"metric:{row['id']}"
    return value


def _value(value: Any) -> Any:
    if isinstance(value, (UUID, datetime, Decimal)):
        return str(value)
    return value
