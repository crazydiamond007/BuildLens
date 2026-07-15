package dev.buildlens.analytics.metrics;

import java.math.BigDecimal;
import java.math.RoundingMode;
import java.time.Duration;
import java.time.LocalDate;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import java.util.UUID;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;

@Service
public class DoraService {
    private static final Set<String> FAILED = Set.of("failure", "failed", "error");
    private final JdbcTemplate jdbc;

    public DoraService(JdbcTemplate jdbc) {
        this.jdbc = jdbc;
    }

    @Transactional
    public void recompute(UUID repositoryId, LocalDate date) {
        UUID organizationId = jdbc.queryForObject(
                "SELECT organization_id FROM repositories WHERE id = ? AND deleted_at IS NULL",
                UUID.class,
                repositoryId);
        if (organizationId == null) {
            return;
        }
        for (Granularity granularity : Granularity.values()) {
            Granularity.Period period = granularity.periodContaining(date);
            write(repositoryId, organizationId, granularity, period);
            write(null, organizationId, granularity, period);
        }
    }

    public LocalDate deploymentDate(UUID deploymentId, UUID repositoryId) {
        List<OffsetDateTime> dates = jdbc.query(
                """
                SELECT COALESCE(deployed_at, started_at, created_at) AS effective_at
                FROM deployments WHERE id = ? AND repository_id = ?
                """,
                (rs, ignored) -> rs.getObject("effective_at", OffsetDateTime.class),
                deploymentId,
                repositoryId);
        if (dates.isEmpty()) {
            throw new IllegalArgumentException("event deployment does not match its repository");
        }
        return dates.getFirst().withOffsetSameInstant(ZoneOffset.UTC).toLocalDate();
    }

    private void write(
            UUID repositoryId,
            UUID organizationId,
            Granularity granularity,
            Granularity.Period period) {
        OffsetDateTime from = period.start().atStartOfDay().atOffset(ZoneOffset.UTC);
        OffsetDateTime until = period.end().plusDays(1).atStartOfDay().atOffset(ZoneOffset.UTC);
        List<Deployment> deployments = loadDeployments(repositoryId, organizationId, from, until);

        List<Long> leadTimes = new ArrayList<>();
        List<Long> recoveryTimes = new ArrayList<>();
        int failures = 0;
        for (Deployment deployment : deployments) {
            if (deployment.authoredAt() != null && !deployment.authoredAt().isAfter(deployment.at())) {
                leadTimes.add(Duration.between(deployment.authoredAt(), deployment.at()).toSeconds());
            }
            if (FAILED.contains(normalize(deployment.status()))) {
                failures++;
                OffsetDateTime recoveredAt = nextSuccess(deployment.repositoryId(), deployment.at());
                if (recoveredAt != null) {
                    recoveryTimes.add(Duration.between(deployment.at(), recoveredAt).toSeconds());
                }
            }
        }

        int count = deployments.size();
        BigDecimal frequency = BigDecimal.valueOf((double) count / period.days())
                .setScale(4, RoundingMode.HALF_UP);
        BigDecimal failureRate = count == 0
                ? null
                : BigDecimal.valueOf((double) failures / count).setScale(4, RoundingMode.HALF_UP);
        Long leadP50 = MetricMath.percentile(leadTimes, 0.50);
        Long leadP90 = MetricMath.percentile(leadTimes, 0.90);
        Long mttrP50 = MetricMath.percentile(recoveryTimes, 0.50);
        Long mttrP90 = MetricMath.percentile(recoveryTimes, 0.90);
        String band = performanceBand(count, frequency, leadP50, failureRate, mttrP50);

        String conflict = repositoryId == null
                ? "(organization_id, granularity, period_start) WHERE repository_id IS NULL"
                : "(repository_id, granularity, period_start) WHERE repository_id IS NOT NULL";
        String sql = """
                INSERT INTO dora_metrics
                    (organization_id, repository_id, granularity, period_start, period_end,
                     deployment_count, deployment_frequency, lead_time_p50_seconds,
                     lead_time_p90_seconds, change_failure_rate, failed_deployment_count,
                     mttr_p50_seconds, mttr_p90_seconds, performance_band, sample_size)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT %s DO UPDATE SET
                    period_end = EXCLUDED.period_end,
                    deployment_count = EXCLUDED.deployment_count,
                    deployment_frequency = EXCLUDED.deployment_frequency,
                    lead_time_p50_seconds = EXCLUDED.lead_time_p50_seconds,
                    lead_time_p90_seconds = EXCLUDED.lead_time_p90_seconds,
                    change_failure_rate = EXCLUDED.change_failure_rate,
                    failed_deployment_count = EXCLUDED.failed_deployment_count,
                    mttr_p50_seconds = EXCLUDED.mttr_p50_seconds,
                    mttr_p90_seconds = EXCLUDED.mttr_p90_seconds,
                    performance_band = EXCLUDED.performance_band,
                    sample_size = EXCLUDED.sample_size,
                    computed_at = now()
                """.formatted(conflict);
        jdbc.update(
                sql,
                organizationId,
                repositoryId,
                granularity.databaseValue(),
                period.start(),
                period.end(),
                count,
                frequency,
                leadP50,
                leadP90,
                failureRate,
                failures,
                mttrP50,
                mttrP90,
                band,
                count);
    }

    private List<Deployment> loadDeployments(
            UUID repositoryId, UUID organizationId, OffsetDateTime from, OffsetDateTime until) {
        String scope = repositoryId == null ? "r.organization_id = ?" : "d.repository_id = ?";
        return jdbc.query(
                """
                SELECT d.repository_id, d.status,
                       COALESCE(d.deployed_at, d.started_at, d.created_at) AS effective_at,
                       COALESCE(direct_commit.authored_at, sha_commit.authored_at) AS authored_at
                FROM deployments d
                JOIN repositories r ON r.id = d.repository_id
                LEFT JOIN commits direct_commit ON direct_commit.id = d.commit_id
                LEFT JOIN commits sha_commit
                  ON d.commit_id IS NULL
                 AND sha_commit.repository_id = d.repository_id
                 AND sha_commit.sha = d.sha
                WHERE d.is_production
                  AND %s
                  AND COALESCE(d.deployed_at, d.started_at, d.created_at) >= ?
                  AND COALESCE(d.deployed_at, d.started_at, d.created_at) < ?
                ORDER BY effective_at
                """.formatted(scope),
                (rs, ignored) -> new Deployment(
                        rs.getObject("repository_id", UUID.class),
                        rs.getString("status"),
                        rs.getObject("effective_at", OffsetDateTime.class),
                        rs.getObject("authored_at", OffsetDateTime.class)),
                repositoryId == null ? organizationId : repositoryId,
                from,
                until);
    }

    private OffsetDateTime nextSuccess(UUID repositoryId, OffsetDateTime after) {
        List<OffsetDateTime> values = jdbc.query(
                """
                SELECT MIN(COALESCE(d.deployed_at, d.started_at, d.created_at)) AS recovered_at
                FROM deployments d
                WHERE d.is_production
                  AND d.repository_id = ?
                  AND lower(d.status) IN ('success', 'successful')
                  AND COALESCE(d.deployed_at, d.started_at, d.created_at) > ?
                """,
                (rs, ignored) -> rs.getObject("recovered_at", OffsetDateTime.class),
                repositoryId,
                after);
        return values.isEmpty() ? null : values.getFirst();
    }

    private static String performanceBand(
            int samples, BigDecimal frequency, Long leadP50, BigDecimal failureRate, Long mttrP50) {
        if (samples < 5) return null;
        double failures = failureRate == null ? 0 : failureRate.doubleValue();
        long lead = leadP50 == null ? Long.MAX_VALUE : leadP50;
        long mttr = mttrP50 == null ? 0 : mttrP50;
        if (frequency.doubleValue() >= 1 && lead <= 86_400 && failures <= 0.15 && mttr <= 3_600) {
            return "elite";
        }
        if (frequency.doubleValue() >= 1.0 / 7 && lead <= 604_800 && failures <= 0.20 && mttr <= 86_400) {
            return "high";
        }
        if (frequency.doubleValue() >= 1.0 / 30 && lead <= 2_592_000 && failures <= 0.30 && mttr <= 604_800) {
            return "medium";
        }
        return "low";
    }

    private static String normalize(String status) {
        return status == null ? "" : status.toLowerCase(Locale.ROOT);
    }

    private record Deployment(
            UUID repositoryId, String status, OffsetDateTime at, OffsetDateTime authoredAt) {}
}
