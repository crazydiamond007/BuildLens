package dev.buildlens.analytics.messaging;

import static org.assertj.core.api.Assertions.assertThatCode;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

import java.time.OffsetDateTime;
import java.util.UUID;
import org.junit.jupiter.api.Test;

class EventPolicyTest {
    private static final UUID ID = UUID.fromString("0197f100-0000-7000-8000-000000000001");

    @Test
    void acceptsTheVersionOneWorkflowTrigger() {
        assertThatCode(() -> EventPolicy.validate(envelope("workflow_run.completed", 1, "workflow_run")))
                .doesNotThrowAnyException();
    }

    @Test
    void rejectsAnUnknownVersionForDeadLettering() {
        assertThatThrownBy(() -> EventPolicy.validate(envelope("workflow_run.completed", 2, "workflow_run")))
                .isInstanceOf(FatalEventException.class)
                .hasMessageContaining("unsupported event version");
    }

    @Test
    void rejectsATypeAggregateMismatch() {
        assertThatThrownBy(() -> EventPolicy.validate(envelope("deployment.recorded", 1, "workflow_run")))
                .isInstanceOf(FatalEventException.class)
                .hasMessageContaining("aggregate type");
    }

    private static EventEnvelope envelope(String type, int version, String aggregateType) {
        return new EventEnvelope(
                ID,
                type,
                version,
                OffsetDateTime.parse("2026-07-15T00:00:00Z"),
                new EventEnvelope.Aggregate(aggregateType, ID),
                ID,
                ID,
                null);
    }
}
