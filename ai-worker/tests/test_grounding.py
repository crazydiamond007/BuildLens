from uuid import UUID

import pytest

from buildlens_ai.context import ContextBundle, validate_grounding
from buildlens_ai.errors import GroundingError
from buildlens_ai.models import Evidence, Finding, LogRange, ReportOutput
from buildlens_ai.redaction import ExtractedRange

JOB_ID = UUID("01912a3b-0000-7000-8000-000000000001")


def context() -> ContextBundle:
    return ContextBundle(
        prompt_data={},
        job_ids=frozenset({JOB_ID}),
        step_ids=frozenset(),
        test_keys=frozenset({"test-key"}),
        log_ranges=(ExtractedRange("job.txt", 10, 20, "error"),),
        metric_keys=frozenset({"metric:one"}),
    )


def output(evidence: Evidence) -> ReportOutput:
    return ReportOutput(
        title="Failure",
        summary="The build failed.",
        content_md="Evidence-backed report.",
        findings=[
            Finding(
                title="Test failed",
                severity="high",
                explanation="The supplied test result failed.",
                evidence=evidence,
            )
        ],
    )


def test_accepts_supplied_ids_and_subranges() -> None:
    validate_grounding(
        output(
            Evidence(
                job_ids=[JOB_ID],
                test_keys=["test-key"],
                log_ranges=[LogRange(source="job.txt", start_line=12, end_line=14)],
            )
        ),
        context(),
    )


def test_rejects_hallucinated_evidence() -> None:
    with pytest.raises(GroundingError, match="outside the supplied context"):
        validate_grounding(output(Evidence(test_keys=["invented"])), context())


def test_rejects_findings_without_evidence() -> None:
    with pytest.raises(GroundingError, match="has no evidence"):
        validate_grounding(output(Evidence()), context())
