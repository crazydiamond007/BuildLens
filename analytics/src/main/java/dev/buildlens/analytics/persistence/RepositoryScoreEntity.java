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
@Table(name = "repository_scores")
public class RepositoryScoreEntity {
    @Id private UUID id;
    @Column(name = "repository_id", nullable = false) private UUID repositoryId;
    @Column(name = "window_days", nullable = false) private int windowDays;
    @Column(name = "overall_score", nullable = false, precision = 5, scale = 2) private BigDecimal overallScore;
    @Column(name = "reliability_score", precision = 5, scale = 2) private BigDecimal reliabilityScore;
    @Column(name = "velocity_score", precision = 5, scale = 2) private BigDecimal velocityScore;
    @Column(name = "quality_score", precision = 5, scale = 2) private BigDecimal qualityScore;
    @Column(name = "efficiency_score", precision = 5, scale = 2) private BigDecimal efficiencyScore;
    private String grade;
    @JdbcTypeCode(SqlTypes.JSON)
    @Column(nullable = false, columnDefinition = "jsonb")
    private String breakdown;
    @Column(name = "computed_at", nullable = false) private OffsetDateTime computedAt;
    @Column(name = "created_at", nullable = false) private OffsetDateTime createdAt;

    protected RepositoryScoreEntity() {}
}
