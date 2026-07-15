from datetime import datetime
from typing import Any, Literal
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field

ReportKind = Literal["build_summary", "failure_analysis", "weekly_digest", "repo_health"]
Severity = Literal["info", "low", "medium", "high", "critical"]
Category = Literal["performance", "reliability", "security", "cost", "testing", "maintainability"]


class Aggregate(BaseModel):
    model_config = ConfigDict(extra="ignore")

    type: str
    id: UUID


class EventEnvelope(BaseModel):
    model_config = ConfigDict(extra="ignore")

    id: UUID
    type: str
    version: int
    occurred_at: datetime
    aggregate: Aggregate
    organization_id: UUID
    repository_id: UUID
    data: dict[str, Any] = Field(default_factory=dict)


class LogRange(BaseModel):
    source: str
    start_line: int = Field(ge=1)
    end_line: int = Field(ge=1)


class Evidence(BaseModel):
    job_ids: list[UUID] = Field(default_factory=list)
    step_ids: list[UUID] = Field(default_factory=list)
    test_keys: list[str] = Field(default_factory=list)
    log_ranges: list[LogRange] = Field(default_factory=list)
    metric_keys: list[str] = Field(default_factory=list)


class Finding(BaseModel):
    title: str
    severity: Severity
    explanation: str
    evidence: Evidence


class RecommendationOutput(BaseModel):
    category: Category
    severity: Severity
    title: str
    body_md: str
    evidence: Evidence


class ReportOutput(BaseModel):
    title: str
    summary: str
    content_md: str
    findings: list[Finding] = Field(min_length=1)
    recommendations: list[RecommendationOutput] = Field(default_factory=list)


class ReportClaim(BaseModel):
    report_id: UUID
    receipt_id: UUID | None = None
    kind: ReportKind
    organization_id: UUID
    repository_id: UUID
    workflow_run_id: UUID | None = None
    model: str


class ManualRetriggerRequest(BaseModel):
    workflow_run_id: UUID
    kind: Literal["build_summary", "failure_analysis"]


class ManualRetriggerResponse(BaseModel):
    report_id: UUID
    status: str


class HealthResponse(BaseModel):
    status: Literal["ok", "degraded"]
    dependencies: dict[str, str] | None = None
