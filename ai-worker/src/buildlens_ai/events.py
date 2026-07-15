from uuid import UUID

from pydantic import ValidationError

from buildlens_ai.errors import InvalidEventError
from buildlens_ai.models import EventEnvelope

SUPPORTED_EVENTS = {
    "workflow_run.completed": "workflow_run",
    "deployment.recorded": "deployment",
}


def parse_event(
    body: bytes, message_id: str | None, routing_key: str, max_bytes: int
) -> EventEnvelope:
    if len(body) > max_bytes:
        raise InvalidEventError(f"event exceeds the {max_bytes}-byte limit")
    try:
        envelope = EventEnvelope.model_validate_json(body)
    except ValidationError as error:
        raise InvalidEventError("event envelope is malformed") from error
    if envelope.version != 1:
        raise InvalidEventError(f"unsupported event version: {envelope.version}")
    expected_aggregate = SUPPORTED_EVENTS.get(envelope.type)
    if expected_aggregate is None:
        raise InvalidEventError(f"unsupported event type: {envelope.type}")
    if routing_key != envelope.type:
        raise InvalidEventError("routing key does not match envelope type")
    if envelope.aggregate.type != expected_aggregate:
        raise InvalidEventError("aggregate type does not match envelope type")
    if message_id is None:
        raise InvalidEventError("AMQP message_id is required")
    try:
        parsed_message_id = UUID(message_id)
    except ValueError as error:
        raise InvalidEventError("AMQP message_id is not a UUID") from error
    if parsed_message_id != envelope.id:
        raise InvalidEventError("AMQP message_id does not match envelope id")
    return envelope
