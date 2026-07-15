package dev.buildlens.analytics.persistence;

import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.Id;
import jakarta.persistence.Table;
import java.math.BigDecimal;
import java.time.LocalDate;
import java.time.OffsetDateTime;
import java.util.UUID;

@Entity
@Table(name = "dora_metrics")
public class DoraMetricEntity {
    @Id private UUID id;
    @Column(name = "organization_id", nullable = false) private UUID organizationId;
    @Column(name = "repository_id") private UUID repositoryId;
    @Column(nullable = false) private String granularity;
    @Column(name = "period_start", nullable = false) private LocalDate periodStart;
    @Column(name = "period_end", nullable = false) private LocalDate periodEnd;
    @Column(name = "deployment_count", nullable = false) private int deploymentCount;
    @Column(name = "deployment_frequency", precision = 10, scale = 4) private BigDecimal deploymentFrequency;
    @Column(name = "lead_time_p50_seconds") private Long leadTimeP50Seconds;
    @Column(name = "lead_time_p90_seconds") private Long leadTimeP90Seconds;
    @Column(name = "change_failure_rate", precision = 5, scale = 4) private BigDecimal changeFailureRate;
    @Column(name = "failed_deployment_count", nullable = false) private int failedDeploymentCount;
    @Column(name = "mttr_p50_seconds") private Long mttrP50Seconds;
    @Column(name = "mttr_p90_seconds") private Long mttrP90Seconds;
    @Column(name = "performance_band") private String performanceBand;
    @Column(name = "sample_size", nullable = false) private int sampleSize;
    @Column(name = "computed_at", nullable = false) private OffsetDateTime computedAt;
    @Column(name = "created_at", nullable = false) private OffsetDateTime createdAt;
    @Column(name = "updated_at", nullable = false) private OffsetDateTime updatedAt;

    protected DoraMetricEntity() {}
}
