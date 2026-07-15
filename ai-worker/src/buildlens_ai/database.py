import json
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from datetime import datetime
from decimal import Decimal
from typing import cast
from uuid import UUID

import asyncpg

from buildlens_ai.config import Settings
from buildlens_ai.errors import InvalidEventError
from buildlens_ai.models import (
    EventEnvelope,
    RecommendationOutput,
    ReportClaim,
    ReportKind,
    ReportOutput,
)

FAILED_CONCLUSIONS = {"failure", "failed", "timed_out", "action_required", "startup_failure"}
SUCCESS_CONCLUSIONS = {"success"}
BUDGET_LOCK_KEY = "buildlens:ai:monthly-budget"


class Database:
    def __init__(self, pool: asyncpg.Pool, settings: Settings) -> None:
        self.pool = pool
        self.settings = settings

    @classmethod
    async def connect(cls, settings: Settings) -> "Database":
        pool = await asyncpg.create_pool(
            dsn=settings.ai_database_url,
            min_size=1,
            max_size=8,
            command_timeout=30,
            server_settings={"application_name": "buildlens-ai-worker", "timezone": "UTC"},
        )
        return cls(pool, settings)

    async def close(self) -> None:
        await self.pool.close()

    async def ping(self) -> None:
        await self.pool.fetchval("SELECT 1")

    async def event_report_kind(self, envelope: EventEnvelope) -> ReportKind | None:
        if envelope.type == "workflow_run.completed":
            row = await self.pool.fetchrow(
                """
                SELECT wr.status, wr.conclusion, wr.repository_id, r.organization_id
                FROM workflow_runs wr
                JOIN repositories r ON r.id = wr.repository_id
                WHERE wr.id = $1
                """,
                envelope.aggregate.id,
            )
            self._validate_scope(row, envelope, "workflow run")
            if row["status"] != "completed":
                raise InvalidEventError("workflow event points to a run that is not completed")
            conclusion = (row["conclusion"] or "").lower()
            if conclusion in FAILED_CONCLUSIONS:
                return "failure_analysis"
            if conclusion in SUCCESS_CONCLUSIONS and self.settings.ai_success_summaries_enabled:
                return "build_summary"
            return None

        row = await self.pool.fetchrow(
            """
            SELECT d.repository_id, r.organization_id, d.is_production
            FROM deployments d
            JOIN repositories r ON r.id = d.repository_id
            WHERE d.id = $1
            """,
            envelope.aggregate.id,
        )
        self._validate_scope(row, envelope, "deployment")
        if not row["is_production"]:
            raise InvalidEventError("deployment event does not point to a production deployment")
        return "repo_health"

    async def claim_event(
        self,
        envelope: EventEnvelope,
        kind: ReportKind | None,
    ) -> ReportClaim | None:
        model = self.model_for(kind) if kind is not None else None
        async with self.pool.acquire() as connection, connection.transaction():
            receipt = await connection.fetchrow(
                """
                INSERT INTO ai_event_receipts
                    (event_id, event_type, organization_id, repository_id,
                     aggregate_id, payload)
                VALUES ($1, $2, $3, $4, $5, $6::jsonb)
                ON CONFLICT (event_id) DO NOTHING
                RETURNING id
                """,
                envelope.id,
                envelope.type,
                envelope.organization_id,
                envelope.repository_id,
                envelope.aggregate.id,
                envelope.model_dump_json(),
            )
            if receipt is None:
                return None
            receipt_id = cast(UUID, receipt["id"])
            if kind is None:
                await connection.execute(
                    """
                    UPDATE ai_event_receipts
                    SET status = 'completed', processed_at = now()
                    WHERE id = $1
                    """,
                    receipt_id,
                )
                return None

            workflow_run_id = (
                envelope.aggregate.id if envelope.type == "workflow_run.completed" else None
            )
            report = await connection.fetchrow(
                """
                INSERT INTO ai_reports
                    (organization_id, repository_id, workflow_run_id, kind,
                     status, model, prompt_version)
                VALUES ($1, $2, $3, $4, 'pending', $5, $6)
                ON CONFLICT (workflow_run_id, kind) WHERE workflow_run_id IS NOT NULL
                DO NOTHING
                RETURNING id
                """,
                envelope.organization_id,
                envelope.repository_id,
                workflow_run_id,
                kind,
                model,
                self.settings.ai_prompt_version,
            )
            if report is None:
                await connection.execute(
                    """
                    UPDATE ai_event_receipts
                    SET status = 'completed', processed_at = now()
                    WHERE id = $1
                    """,
                    receipt_id,
                )
                return None
            report_id = cast(UUID, report["id"])
            await connection.execute(
                "UPDATE ai_event_receipts SET ai_report_id = $1 WHERE id = $2",
                report_id,
                receipt_id,
            )
            return ReportClaim(
                report_id=report_id,
                receipt_id=receipt_id,
                kind=kind,
                organization_id=envelope.organization_id,
                repository_id=envelope.repository_id,
                workflow_run_id=workflow_run_id,
                model=cast(str, model),
            )

    async def mark_processing(self, claim: ReportClaim) -> None:
        async with self.pool.acquire() as connection, connection.transaction():
            await connection.execute(
                """
                UPDATE ai_reports SET status = 'processing', error = NULL
                WHERE id = $1 AND status IN ('pending', 'failed', 'processing')
                """,
                claim.report_id,
            )
            if claim.receipt_id is not None:
                await connection.execute(
                    """
                    UPDATE ai_event_receipts
                    SET status = 'processing', attempts = attempts + 1,
                        locked_at = now(), last_error = NULL
                    WHERE id = $1
                    """,
                    claim.receipt_id,
                )

    async def complete_report(
        self,
        connection: asyncpg.Connection,
        claim: ReportClaim,
        output: ReportOutput,
        *,
        input_tokens: int,
        output_tokens: int,
        cost_usd: Decimal,
        latency_ms: int,
    ) -> None:
        async with connection.transaction():
            await connection.execute(
                "DELETE FROM ai_recommendations WHERE ai_report_id = $1",
                claim.report_id,
            )
            await connection.execute(
                """
                UPDATE ai_reports SET
                    status = 'completed', title = $2, summary = $3, content_md = $4,
                    content = $5::jsonb,
                    input_tokens = COALESCE(input_tokens, 0) + $6,
                    output_tokens = COALESCE(output_tokens, 0) + $7,
                    cost_usd = COALESCE(cost_usd, 0) + $8,
                    latency_ms = COALESCE(latency_ms, 0) + $9,
                    error = NULL, completed_at = now()
                WHERE id = $1
                """,
                claim.report_id,
                output.title,
                output.summary,
                output.content_md,
                output.model_dump_json(exclude={"recommendations"}),
                input_tokens,
                output_tokens,
                cost_usd,
                latency_ms,
            )
            for recommendation in output.recommendations:
                await self._insert_recommendation(connection, claim, recommendation)
            if claim.receipt_id is not None:
                await connection.execute(
                    """
                    UPDATE ai_event_receipts SET
                        status = 'completed', processed_at = now(), locked_at = NULL,
                        last_error = NULL
                    WHERE id = $1
                    """,
                    claim.receipt_id,
                )

    async def fail_report(
        self,
        claim: ReportClaim,
        error: str,
        *,
        connection: asyncpg.Connection | None = None,
        input_tokens: int | None = None,
        output_tokens: int | None = None,
        cost_usd: Decimal | None = None,
        latency_ms: int | None = None,
    ) -> None:
        if connection is not None:
            await self._fail_report(
                connection,
                claim,
                error,
                input_tokens=input_tokens,
                output_tokens=output_tokens,
                cost_usd=cost_usd,
                latency_ms=latency_ms,
            )
            return
        async with self.pool.acquire() as acquired:
            await self._fail_report(
                acquired,
                claim,
                error,
                input_tokens=input_tokens,
                output_tokens=output_tokens,
                cost_usd=cost_usd,
                latency_ms=latency_ms,
            )

    async def _fail_report(
        self,
        connection: asyncpg.Connection,
        claim: ReportClaim,
        error: str,
        *,
        input_tokens: int | None,
        output_tokens: int | None,
        cost_usd: Decimal | None,
        latency_ms: int | None,
    ) -> None:
        safe_error = error[:4000]
        async with connection.transaction():
            await connection.execute(
                """
                UPDATE ai_reports SET
                    status = 'failed', error = $2,
                    input_tokens = COALESCE(input_tokens, 0) + COALESCE($3::integer, 0),
                    output_tokens = COALESCE(output_tokens, 0) + COALESCE($4::integer, 0),
                    cost_usd = COALESCE(cost_usd, 0) + COALESCE($5::numeric, 0),
                    latency_ms = COALESCE(latency_ms, 0) + COALESCE($6::integer, 0)
                WHERE id = $1
                """,
                claim.report_id,
                safe_error,
                input_tokens,
                output_tokens,
                cost_usd,
                latency_ms,
            )
            if claim.receipt_id is not None:
                await connection.execute(
                    """
                    UPDATE ai_event_receipts SET
                        status = 'failed', processed_at = now(), locked_at = NULL,
                        last_error = $2
                    WHERE id = $1
                    """,
                    claim.receipt_id,
                    safe_error,
                )

    async def manual_retrigger(self, workflow_run_id: UUID, kind: ReportKind) -> ReportClaim:
        row = await self.pool.fetchrow(
            """
            SELECT wr.repository_id, r.organization_id, wr.conclusion
            FROM workflow_runs wr JOIN repositories r ON r.id = wr.repository_id
            WHERE wr.id = $1 AND wr.status = 'completed'
            """,
            workflow_run_id,
        )
        if row is None:
            raise InvalidEventError("completed workflow run not found")
        conclusion = (row["conclusion"] or "").lower()
        if kind == "failure_analysis" and conclusion not in FAILED_CONCLUSIONS:
            raise InvalidEventError("failure analysis requires a failed workflow run")
        if kind == "build_summary" and conclusion not in SUCCESS_CONCLUSIONS:
            raise InvalidEventError("build summary requires a successful workflow run")
        model = self.model_for(kind)
        async with self.pool.acquire() as connection, connection.transaction():
            report = await connection.fetchrow(
                """
                INSERT INTO ai_reports
                    (organization_id, repository_id, workflow_run_id, kind,
                     status, model, prompt_version)
                VALUES ($1, $2, $3, $4, 'pending', $5, $6)
                ON CONFLICT (workflow_run_id, kind) WHERE workflow_run_id IS NOT NULL
                DO UPDATE SET status = 'pending', model = EXCLUDED.model,
                              prompt_version = EXCLUDED.prompt_version,
                              error = NULL, completed_at = NULL
                WHERE ai_reports.status = 'failed'
                RETURNING id, status
                """,
                row["organization_id"],
                row["repository_id"],
                workflow_run_id,
                kind,
                model,
                self.settings.ai_prompt_version,
            )
            if report is None:
                existing = await connection.fetchrow(
                    "SELECT id, status FROM ai_reports WHERE workflow_run_id = $1 AND kind = $2",
                    workflow_run_id,
                    kind,
                )
                raise InvalidEventError(
                    f"report already exists with status {existing['status']} and is not retryable"
                )
            return ReportClaim(
                report_id=report["id"],
                kind=kind,
                organization_id=row["organization_id"],
                repository_id=row["repository_id"],
                workflow_run_id=workflow_run_id,
                model=model,
            )

    async def claim_scheduled(
        self,
        organization_id: UUID,
        repository_id: UUID,
        kind: ReportKind,
        period_start: datetime,
    ) -> ReportClaim | None:
        key = f"{repository_id}:{kind}:{period_start.isoformat()}"
        model = self.model_for(kind)
        async with self.pool.acquire() as connection, connection.transaction():
            await connection.execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", key)
            existing = await connection.fetchval(
                """
                SELECT id FROM ai_reports
                WHERE repository_id = $1 AND kind = $2 AND requested_at >= $3
                LIMIT 1
                """,
                repository_id,
                kind,
                period_start,
            )
            if existing is not None:
                return None
            report_id = await connection.fetchval(
                """
                INSERT INTO ai_reports
                    (organization_id, repository_id, kind, status, model, prompt_version)
                VALUES ($1, $2, $3, 'pending', $4, $5)
                RETURNING id
                """,
                organization_id,
                repository_id,
                kind,
                model,
                self.settings.ai_prompt_version,
            )
            return ReportClaim(
                report_id=report_id,
                kind=kind,
                organization_id=organization_id,
                repository_id=repository_id,
                model=model,
            )

    async def tracked_repositories(self) -> list[tuple[UUID, UUID]]:
        rows = await self.pool.fetch(
            """
            SELECT organization_id, id FROM repositories
            WHERE tracking_enabled AND deleted_at IS NULL ORDER BY id
            """
        )
        return [(row["organization_id"], row["id"]) for row in rows]

    async def claim_recoverable_reports(self, limit: int = 10) -> list[ReportClaim]:
        async with self.pool.acquire() as connection, connection.transaction():
            rows = await connection.fetch(
                """
                SELECT ar.id, ar.kind, ar.organization_id, ar.repository_id,
                       ar.workflow_run_id, ar.model, aer.id AS receipt_id
                FROM ai_reports ar
                LEFT JOIN ai_event_receipts aer ON aer.ai_report_id = ar.id
                WHERE ar.status = 'pending'
                   OR (ar.status = 'processing'
                       AND ar.updated_at < now() - ($1 * interval '1 second'))
                ORDER BY ar.requested_at
                FOR UPDATE OF ar SKIP LOCKED
                LIMIT $2
                """,
                self.settings.ai_recovery_after_seconds,
                limit,
            )
            claims: list[ReportClaim] = []
            for row in rows:
                await connection.execute(
                    "UPDATE ai_reports SET status = 'processing', error = NULL WHERE id = $1",
                    row["id"],
                )
                if row["receipt_id"] is not None:
                    await connection.execute(
                        """
                        UPDATE ai_event_receipts
                        SET status = 'processing', attempts = attempts + 1,
                            locked_at = now(), last_error = NULL
                        WHERE id = $1
                        """,
                        row["receipt_id"],
                    )
                claims.append(
                    ReportClaim(
                        report_id=row["id"],
                        receipt_id=row["receipt_id"],
                        kind=row["kind"],
                        organization_id=row["organization_id"],
                        repository_id=row["repository_id"],
                        workflow_run_id=row["workflow_run_id"],
                        model=row["model"],
                    )
                )
            return claims

    @asynccontextmanager
    async def monthly_budget_lock(self) -> AsyncIterator[asyncpg.Connection]:
        async with self.pool.acquire() as connection:
            await connection.execute(
                "SELECT pg_advisory_lock(hashtextextended($1, 0))", BUDGET_LOCK_KEY
            )
            try:
                yield connection
            finally:
                await connection.execute(
                    "SELECT pg_advisory_unlock(hashtextextended($1, 0))", BUDGET_LOCK_KEY
                )

    async def monthly_spend(self, connection: asyncpg.Connection) -> Decimal:
        value = await connection.fetchval(
            """
            SELECT COALESCE(sum(cost_usd), 0)
            FROM ai_reports
            WHERE requested_at >= date_trunc('month', now())
              AND requested_at < date_trunc('month', now()) + interval '1 month'
            """
        )
        return Decimal(value)

    def model_for(self, kind: ReportKind) -> str:
        if kind == "failure_analysis":
            return self.settings.ai_failure_model
        return self.settings.ai_summary_model

    @staticmethod
    def _validate_scope(
        row: asyncpg.Record | None,
        envelope: EventEnvelope,
        aggregate_name: str,
    ) -> None:
        if row is None:
            raise InvalidEventError(f"event {aggregate_name} does not exist")
        if row["repository_id"] != envelope.repository_id:
            raise InvalidEventError(f"event repository does not match its {aggregate_name}")
        if row["organization_id"] != envelope.organization_id:
            raise InvalidEventError(f"event organization does not match its {aggregate_name}")

    @staticmethod
    async def _insert_recommendation(
        connection: asyncpg.Connection,
        claim: ReportClaim,
        recommendation: RecommendationOutput,
    ) -> None:
        await connection.execute(
            """
            INSERT INTO ai_recommendations
                (ai_report_id, repository_id, category, severity, title, body_md, evidence)
            VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb)
            """,
            claim.report_id,
            claim.repository_id,
            recommendation.category,
            recommendation.severity,
            recommendation.title,
            recommendation.body_md,
            json.dumps(recommendation.evidence.model_dump(mode="json")),
        )
