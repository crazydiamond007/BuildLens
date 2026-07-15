package dev.buildlens.analytics.persistence;

import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.Id;
import jakarta.persistence.Table;
import java.math.BigDecimal;
import java.time.OffsetDateTime;
import java.util.UUID;
import org.hibernate.annotations.JdbcTypeCode;
import org.hibernate.type.SqlTypes;

@Entity
@Table(name = "build_scores")
public class BuildScoreEntity {
    @Id private UUID id;
    @Column(name = "workflow_run_id", nullable = false, unique = true) private UUID workflowRunId;
    @Column(name = "repository_id", nullable = false) private UUID repositoryId;
    @Column(nullable = false, precision = 5, scale = 2) private BigDecimal score;
    @Column(name = "duration_score", precision = 5, scale = 2) private BigDecimal durationScore;
    @Column(name = "reliability_score", precision = 5, scale = 2) private BigDecimal reliabilityScore;
    @Column(name = "flakiness_score", precision = 5, scale = 2) private BigDecimal flakinessScore;
    @JdbcTypeCode(SqlTypes.JSON)
    @Column(nullable = false, columnDefinition = "jsonb")
    private String breakdown;
    @Column(name = "computed_at", nullable = false) private OffsetDateTime computedAt;
    @Column(name = "created_at", nullable = false) private OffsetDateTime createdAt;
    @Column(name = "updated_at", nullable = false) private OffsetDateTime updatedAt;

    protected BuildScoreEntity() {}
}
