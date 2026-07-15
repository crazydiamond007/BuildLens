package dev.buildlens.analytics.messaging;

import java.util.Set;

final class EventPolicy {
    private static final Set<String> SUPPORTED_TYPES =
            Set.of("workflow_run.completed", "deployment.recorded");

    private EventPolicy() {}

    static void validate(EventEnvelope envelope) {
        if (envelope.id() == null
                || envelope.type() == null
                || envelope.occurredAt() == null
                || envelope.aggregate() == null
                || envelope.aggregate().id() == null
                || envelope.organizationId() == null
                || envelope.repositoryId() == null) {
            throw new FatalEventException("event envelope is missing a required field");
        }
        if (envelope.version() != 1) {
            throw new FatalEventException("unsupported event version: " + envelope.version());
        }
        if (!SUPPORTED_TYPES.contains(envelope.type())) {
            throw new FatalEventException("unsupported event type: " + envelope.type());
        }
        String expectedAggregate = envelope.type().startsWith("workflow_run.")
                ? "workflow_run"
                : "deployment";
        if (!expectedAggregate.equals(envelope.aggregate().type())) {
            throw new FatalEventException("event aggregate type does not match event type");
        }
    }
}
