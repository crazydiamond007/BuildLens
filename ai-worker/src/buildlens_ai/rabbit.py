import logging

import aio_pika
import asyncpg
from aio_pika.abc import AbstractRobustChannel, AbstractRobustConnection, AbstractRobustQueue

from buildlens_ai.config import Settings
from buildlens_ai.errors import InvalidEventError
from buildlens_ai.events import parse_event
from buildlens_ai.processor import ReportProcessor

LOG = logging.getLogger(__name__)
EVENTS_EXCHANGE = "buildlens.events"
DEAD_LETTER_EXCHANGE = "buildlens.events.dlx"


class RabbitConsumer:
    def __init__(self, settings: Settings, processor: ReportProcessor) -> None:
        self.settings = settings
        self.processor = processor
        self.connection: AbstractRobustConnection | None = None
        self.channel: AbstractRobustChannel | None = None
        self.queue: AbstractRobustQueue | None = None

    async def start(self) -> None:
        self.connection = await aio_pika.connect_robust(self.settings.rabbitmq_url)
        self.channel = await self.connection.channel()
        await self.channel.set_qos(prefetch_count=1)
        events = await self.channel.declare_exchange(
            EVENTS_EXCHANGE, aio_pika.ExchangeType.TOPIC, durable=True
        )
        await self.channel.declare_exchange(
            DEAD_LETTER_EXCHANGE, aio_pika.ExchangeType.TOPIC, durable=True
        )
        self.queue = await self.channel.declare_queue(
            self.settings.ai_queue,
            durable=True,
            arguments={"x-dead-letter-exchange": DEAD_LETTER_EXCHANGE},
        )
        await self.queue.bind(events, routing_key="workflow_run.*")
        await self.queue.bind(events, routing_key="deployment.*")
        await self.queue.consume(self._consume, no_ack=False)
        LOG.info("AI consumer ready queue=%s", self.settings.ai_queue)

    async def close(self) -> None:
        if self.connection is not None and not self.connection.is_closed:
            await self.connection.close()

    @property
    def ready(self) -> bool:
        return self.connection is not None and not self.connection.is_closed

    async def _consume(self, message: aio_pika.IncomingMessage) -> None:
        try:
            envelope = parse_event(
                message.body,
                message.message_id,
                message.routing_key or "",
                self.settings.ai_max_event_bytes,
            )
            await self.processor.handle_event(envelope)
            await message.ack()
            LOG.info("processed AI trigger event_id=%s type=%s", envelope.id, envelope.type)
        except InvalidEventError as error:
            LOG.warning("dead-lettering invalid AI event: %s", error)
            await message.reject(requeue=False)
        except (asyncpg.PostgresError, OSError, aio_pika.AMQPException):
            LOG.exception("requeueing AI event after transient dependency failure")
            await message.nack(requeue=True)
        except Exception:
            LOG.exception("dead-lettering poison AI event")
            await message.reject(requeue=False)
