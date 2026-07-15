import pytest
from pydantic import ValidationError

from buildlens_ai.config import Settings


def settings(**changes: object) -> Settings:
    values: dict[str, object] = {
        "ai_database_url": "postgresql://ai:password@localhost/buildlens",
        "rabbitmq_url": "amqp://user:password@localhost/vhost",
        "anthropic_api_key": "test-anthropic-key",
        "ai_manual_trigger_token": "a" * 32,
        "s3_endpoint": "http://localhost:9000",
        "s3_access_key": "minio-user",
        "s3_secret_key": "minio-password",
        "s3_logs_bucket": "buildlens-logs",
    }
    values.update(changes)
    return Settings(**values)  # type: ignore[arg-type]


def test_accepts_a_strong_manual_trigger_token() -> None:
    assert len(settings().ai_manual_trigger_token.get_secret_value()) == 32


def test_rejects_a_short_manual_trigger_token() -> None:
    short_token = "x" * 5
    with pytest.raises(ValidationError, match="at least 32"):
        settings(ai_manual_trigger_token=short_token)
