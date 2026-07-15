import json
from uuid import UUID

import pytest

from buildlens_ai.errors import InvalidEventError
from buildlens_ai.events import parse_event

EVENT_ID = "01912a3b-4c5d-7e8f-9a0b-1c2d3e4f5061"


def event(**changes: object) -> bytes:
    payload = {
        "id": EVENT_ID,
        "type": "workflow_run.completed",
        "version": 1,
        "occurred_at": "2026-07-15T12:00:00Z",
        "aggregate": {
            "type": "workflow_run",
            "id": "01912a3b-0000-7000-8000-000000000001",
        },
        "organization_id": "01912a3b-0000-7000-8000-0000000000aa",
        "repository_id": "01912a3b-0000-7000-8000-0000000000bb",
        "data": {},
        "future_field": "ignored",
    }
    payload.update(changes)
    return json.dumps(payload).encode()


def test_accepts_supported_envelope_and_ignores_unknown_fields() -> None:
    parsed = parse_event(event(), EVENT_ID, "workflow_run.completed", 100_000)
    assert parsed.id == UUID(EVENT_ID)


@pytest.mark.parametrize(
    ("body", "message_id", "routing_key", "message"),
    [
        (event(version=2), EVENT_ID, "workflow_run.completed", "unsupported event version"),
        (event(), None, "workflow_run.completed", "message_id is required"),
        (
            event(),
            "01912a3b-4c5d-7e8f-9a0b-1c2d3e4f5999",
            "workflow_run.completed",
            "does not match envelope id",
        ),
        (event(), EVENT_ID, "deployment.recorded", "routing key does not match"),
    ],
)
def test_rejects_poison_messages(
    body: bytes, message_id: str | None, routing_key: str, message: str
) -> None:
    with pytest.raises(InvalidEventError, match=message):
        parse_event(body, message_id, routing_key, 100_000)


def test_rejects_oversized_message_before_parsing() -> None:
    with pytest.raises(InvalidEventError, match="exceeds"):
        parse_event(event(), EVENT_ID, "workflow_run.completed", 10)
