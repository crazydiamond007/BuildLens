package dev.buildlens.analytics.persistence;

import jakarta.persistence.Column;
import jakarta.persistence.Entity;
import jakarta.persistence.Id;
import jakarta.persistence.Table;
import java.math.BigDecimal;
import java.time.OffsetDateTime;
import java.util.UUID;

@Entity
@Table(name = "flaky_tests")
public class FlakyTestEntity {
    @Id private UUID id;
    @Column(name = "repository_id", nullable = false) private UUID repositoryId;
    @Column(name = "test_key", nullable = false) private String testKey;
    private String suite;
    private String classname;
    @Column(nullable = false) private String name;
    @Column(name = "window_days", nullable = false) private int windowDays;
    @Column(name = "total_runs", nullable = false) private int totalRuns;
    @Column(name = "passed_runs", nullable = false) private int passedRuns;
    @Column(name = "failed_runs", nullable = false) private int failedRuns;
    @Column(name = "flip_count", nullable = false) private int flipCount;
    @Column(name = "flake_rate", nullable = false, precision = 5, scale = 4) private BigDecimal flakeRate;
    @Column(name = "is_flaky", nullable = false) private boolean flaky;
    @Column(name = "is_quarantined", nullable = false) private boolean quarantined;
    @Column(name = "first_seen_at") private OffsetDateTime firstSeenAt;
    @Column(name = "last_seen_at") private OffsetDateTime lastSeenAt;
    @Column(name = "last_failed_at") private OffsetDateTime lastFailedAt;
    @Column(name = "computed_at", nullable = false) private OffsetDateTime computedAt;
    @Column(name = "created_at", nullable = false) private OffsetDateTime createdAt;
    @Column(name = "updated_at", nullable = false) private OffsetDateTime updatedAt;

    protected FlakyTestEntity() {}
}
