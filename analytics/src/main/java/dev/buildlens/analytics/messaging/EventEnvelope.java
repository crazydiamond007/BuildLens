package dev.buildlens.analytics.messaging;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import tools.jackson.databind.JsonNode;
import java.time.OffsetDateTime;
import java.util.UUID;

@JsonIgnoreProperties(ignoreUnknown = true)
public record EventEnvelope(
        UUID id,
        String type,
        int version,
        OffsetDateTime occurredAt,
        Aggregate aggregate,
        UUID organizationId,
        UUID repositoryId,
        JsonNode data) {
    @JsonIgnoreProperties(ignoreUnknown = true)
    public record Aggregate(String type, UUID id) {}
}
