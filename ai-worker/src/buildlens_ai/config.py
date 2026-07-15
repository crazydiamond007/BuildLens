from decimal import Decimal

from pydantic import Field, SecretStr, field_validator
from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(extra="ignore")

    ai_database_url: str
    rabbitmq_url: str
    anthropic_api_key: SecretStr
    ai_manual_trigger_token: SecretStr = Field(min_length=32)

    ai_port: int = Field(default=8082, ge=1, le=65535)
    ai_queue: str = "ai.reports"
    ai_failure_model: str = "claude-opus-4-8"
    ai_summary_model: str = "claude-haiku-4-5"
    ai_success_summaries_enabled: bool = False
    ai_scheduled_reports_enabled: bool = False
    ai_monthly_cost_cap_usd: Decimal = Field(default=Decimal("10.00"), gt=0)
    ai_prompt_version: str = "phase6-v1"
    ai_failure_max_tokens: int = Field(default=4096, ge=256, le=16_384)
    ai_summary_max_tokens: int = Field(default=2048, ge=256, le=8192)
    ai_max_event_bytes: int = Field(default=262_144, ge=1024)
    ai_max_log_download_bytes: int = Field(default=20 * 1024 * 1024, ge=1024)
    ai_max_log_prompt_bytes: int = Field(default=65_536, ge=1024)
    ai_max_log_lines: int = Field(default=200, ge=1, le=1000)
    ai_recovery_after_seconds: int = Field(default=900, ge=60)
    ai_schedule_interval_seconds: int = Field(default=3600, ge=60)

    s3_endpoint: str
    s3_region: str = "us-east-1"
    s3_access_key: SecretStr
    s3_secret_key: SecretStr
    s3_logs_bucket: str

    @field_validator(
        "ai_database_url",
        "rabbitmq_url",
        "ai_queue",
        "ai_failure_model",
        "ai_summary_model",
        "ai_prompt_version",
        "s3_endpoint",
        "s3_region",
        "s3_logs_bucket",
    )
    @classmethod
    def reject_blank(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("must not be blank")
        return value

    @field_validator(
        "anthropic_api_key", "ai_manual_trigger_token", "s3_access_key", "s3_secret_key"
    )
    @classmethod
    def reject_blank_secret(cls, value: SecretStr) -> SecretStr:
        if not value.get_secret_value().strip():
            raise ValueError("must not be blank")
        return value
