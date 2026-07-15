from decimal import Decimal

import pytest

from buildlens_ai.errors import ReportGenerationError
from buildlens_ai.llm import calculate_cost


def test_calculates_opus_cost_including_cache_rates() -> None:
    assert calculate_cost(
        "claude-opus-4-8",
        input_tokens=1000,
        output_tokens=100,
        cache_write_tokens=200,
        cache_read_tokens=300,
    ) == Decimal("0.008900")


def test_rejects_unpriced_model_so_cap_cannot_be_bypassed() -> None:
    with pytest.raises(ReportGenerationError, match="no pricing configured"):
        calculate_cost("unknown", input_tokens=1, output_tokens=1)
