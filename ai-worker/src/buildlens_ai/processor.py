import logging
from decimal import Decimal

from buildlens_ai.config import Settings
from buildlens_ai.context import ContextLoader, validate_grounding
from buildlens_ai.database import Database
from buildlens_ai.errors import MonthlyCostCapError
from buildlens_ai.llm import ClaudeReports, GeneratedReport
from buildlens_ai.models import EventEnvelope, ReportClaim

LOG = logging.getLogger(__name__)


class ReportProcessor:
    def __init__(
        self,
        database: Database,
        contexts: ContextLoader,
        claude: ClaudeReports,
        settings: Settings,
    ) -> None:
        self.database = database
        self.contexts = contexts
        self.claude = claude
        self.settings = settings

    async def handle_event(self, envelope: EventEnvelope) -> None:
        kind = await self.database.event_report_kind(envelope)
        claim = await self.database.claim_event(envelope, kind)
        if claim is not None:
            await self.process(claim)

    async def process(self, claim: ReportClaim, *, already_processing: bool = False) -> None:
        if not already_processing:
            await self.database.mark_processing(claim)
        generated: GeneratedReport | None = None
        committed = False
        try:
            context = await self.contexts.load(claim)
            projected = await self.claude.projected_cost(claim, context.prompt_data)
            async with self.database.monthly_budget_lock() as connection:
                try:
                    spent = await self.database.monthly_spend(connection)
                    if spent + projected > self.settings.ai_monthly_cost_cap_usd:
                        raise MonthlyCostCapError(
                            f"monthly cost cap would be exceeded: spent={spent}, "
                            f"projected={projected}, cap={self.settings.ai_monthly_cost_cap_usd}"
                        )
                    generated = await self.claude.generate(claim, context.prompt_data)
                    validate_grounding(generated.output, context)
                    await self.database.complete_report(
                        connection,
                        claim,
                        generated.output,
                        input_tokens=generated.input_tokens,
                        output_tokens=generated.output_tokens,
                        cost_usd=generated.cost_usd,
                        latency_ms=generated.latency_ms,
                    )
                    committed = True
                except Exception as error:
                    await self.database.fail_report(
                        claim,
                        str(error),
                        connection=connection,
                        **_usage(generated),
                    )
                    LOG.exception(
                        "AI report failed report_id=%s kind=%s", claim.report_id, claim.kind
                    )
                    return
            LOG.info(
                "completed AI report report_id=%s kind=%s cost_usd=%s",
                claim.report_id,
                claim.kind,
                generated.cost_usd,
            )
        except Exception as error:
            if committed:
                LOG.warning(
                    "report committed before cleanup failure report_id=%s kind=%s",
                    claim.report_id,
                    claim.kind,
                    exc_info=True,
                )
                return
            LOG.exception("AI report failed report_id=%s kind=%s", claim.report_id, claim.kind)
            await self.database.fail_report(claim, str(error), **_usage(generated))

    async def recover(self) -> None:
        for claim in await self.database.claim_recoverable_reports():
            await self.process(claim, already_processing=True)


def _usage(generated: GeneratedReport | None) -> dict[str, int | Decimal]:
    if generated is None:
        return {}
    return {
        "input_tokens": generated.input_tokens,
        "output_tokens": generated.output_tokens,
        "cost_usd": generated.cost_usd,
        "latency_ms": generated.latency_ms,
    }
