import asyncio
import logging

from buildlens_ai.config import Settings
from buildlens_ai.context import ContextLoader
from buildlens_ai.database import Database
from buildlens_ai.llm import ClaudeReports
from buildlens_ai.processor import ReportProcessor
from buildlens_ai.rabbit import RabbitConsumer
from buildlens_ai.scheduler import ReportScheduler

LOG = logging.getLogger(__name__)


class Runtime:
    def __init__(
        self,
        settings: Settings,
        database: Database,
        claude: ClaudeReports,
        processor: ReportProcessor,
        rabbit: RabbitConsumer,
        scheduler: ReportScheduler,
    ) -> None:
        self.settings = settings
        self.database = database
        self.claude = claude
        self.processor = processor
        self.rabbit = rabbit
        self.scheduler = scheduler
        self.scheduler_task: asyncio.Task[None] | None = None
        self.background_tasks: set[asyncio.Task[None]] = set()

    @classmethod
    async def start(cls, settings: Settings) -> "Runtime":
        database = await Database.connect(settings)
        claude = ClaudeReports(settings)
        contexts = ContextLoader(database.pool, settings)
        processor = ReportProcessor(database, contexts, claude, settings)
        rabbit = RabbitConsumer(settings, processor)
        scheduler = ReportScheduler(database, processor, settings)
        runtime = cls(settings, database, claude, processor, rabbit, scheduler)
        try:
            await rabbit.start()
        except Exception:
            await claude.close()
            await database.close()
            raise
        runtime.scheduler_task = asyncio.create_task(scheduler.run(), name="ai-report-scheduler")
        return runtime

    def spawn(self, coroutine: object) -> None:
        if not asyncio.iscoroutine(coroutine):
            raise TypeError("background work must be a coroutine")
        task = asyncio.create_task(coroutine)
        self.background_tasks.add(task)
        task.add_done_callback(self.background_tasks.discard)

    async def close(self) -> None:
        self.scheduler.stop()
        if self.scheduler_task is not None:
            await self.scheduler_task
        if self.background_tasks:
            await asyncio.gather(*self.background_tasks, return_exceptions=True)
        await self.rabbit.close()
        await self.claude.close()
        await self.database.close()
        LOG.info("AI worker stopped")
