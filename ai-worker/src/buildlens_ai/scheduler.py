import asyncio
import logging
from datetime import UTC, datetime, timedelta

from buildlens_ai.config import Settings
from buildlens_ai.database import Database
from buildlens_ai.processor import ReportProcessor

LOG = logging.getLogger(__name__)


class ReportScheduler:
    def __init__(self, database: Database, processor: ReportProcessor, settings: Settings) -> None:
        self.database = database
        self.processor = processor
        self.settings = settings
        self._stopped = asyncio.Event()

    async def run(self) -> None:
        while not self._stopped.is_set():
            try:
                await self.processor.recover()
                if self.settings.ai_scheduled_reports_enabled:
                    await self._claim_current_reports()
            except Exception:
                LOG.exception("scheduled AI report pass failed")
            try:
                await asyncio.wait_for(
                    self._stopped.wait(), timeout=self.settings.ai_schedule_interval_seconds
                )
            except TimeoutError:
                pass

    def stop(self) -> None:
        self._stopped.set()

    async def _claim_current_reports(self) -> None:
        now = datetime.now(UTC)
        day_start = now.replace(hour=0, minute=0, second=0, microsecond=0)
        week_start = day_start - timedelta(days=day_start.weekday())
        for organization_id, repository_id in await self.database.tracked_repositories():
            daily = await self.database.claim_scheduled(
                organization_id, repository_id, "repo_health", day_start
            )
            if daily is not None:
                await self.processor.process(daily)
            weekly = await self.database.claim_scheduled(
                organization_id, repository_id, "weekly_digest", week_start
            )
            if weekly is not None:
                await self.processor.process(weekly)
