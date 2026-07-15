class InvalidEventError(ValueError):
    """The wire message violates the supported contract and must be dead-lettered."""


class GroundingError(ValueError):
    """The model cited evidence that was not present in the supplied context."""


class MonthlyCostCapError(RuntimeError):
    """The configured monthly API budget cannot admit another request."""


class ReportGenerationError(RuntimeError):
    """A claimed report could not be generated and was durably marked failed."""
