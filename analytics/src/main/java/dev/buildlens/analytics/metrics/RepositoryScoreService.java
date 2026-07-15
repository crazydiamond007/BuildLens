package dev.buildlens.analytics.metrics;

import tools.jackson.core.JacksonException;
import tools.jackson.databind.ObjectMapper;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.time.temporal.ChronoUnit;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;

@Service
public class RepositoryScoreService {
    private final JdbcTemplate jdbc;
    private final ObjectMapper objectMapper;
    private final int windowDays;

    public RepositoryScoreService(
            JdbcTemplate jdbc,
            ObjectMapper objectMapper,
            @Value("${analytics.recompute.window-days}") int windowDays) {
        this.jdbc = jdbc;
        this.objectMapper = objectMapper;
        this.windowDays = windowDays;
    }

    @Transactional
    public void recompute(UUID repositoryId, OffsetDateTime asOf) {
        OffsetDateTime computedAt = asOf.withOffsetSameInstant(ZoneOffset.UTC).truncatedTo(ChronoUnit.DAYS);
        OffsetDateTime from = computedAt.plusDays(1).minusDays(windowDays);
        OffsetDateTime until = computedAt.plusDays(1);

        Map<String, Object> builds = jdbc.queryForMap(
                """
                SELECT count(*) AS total,
                       count(*) FILTER (WHERE conclusion = 'success') AS successful
                FROM workflow_runs
                WHERE repository_id = ? AND status = 'completed'
                  AND completed_at >= ? AND completed_at < ?
                """,
                repositoryId,
                from,
                until);
        int totalBuilds = number(builds, "total").intValue();
        int successfulBuilds = number(builds, "successful").intValue();
        double reliability = totalBuilds == 0 ? 50 : 100.0 * successfulBuilds / totalBuilds;

        Integer deployments = jdbc.queryForObject(
                """
                SELECT count(*) FROM deployments
                WHERE repository_id = ? AND is_production
                  AND lower(status) IN ('success', 'successful')
                  AND COALESCE(deployed_at, started_at, created_at) >= ?
                  AND COALESCE(deployed_at, started_at, created_at) < ?
                """,
                Integer.class,
                repositoryId,
                from,
                until);
        int deploymentCount = deployments == null ? 0 : deployments;
        double velocity = MetricMath.clamp(100.0 * deploymentCount / windowDays);

        Map<String, Object> tests = jdbc.queryForMap(
                """
                SELECT count(*) AS known,
                       count(*) FILTER (WHERE is_flaky) AS flaky
                FROM flaky_tests WHERE repository_id = ? AND window_days = ?
                """,
                repositoryId,
                windowDays);
        int knownTests = number(tests, "known").intValue();
        int flakyTests = number(tests, "flaky").intValue();
        double quality = knownTests == 0 ? 50 : 100.0 * (1.0 - (double) flakyTests / knownTests);

        Number averageDuration = jdbc.queryForObject(
                """
                SELECT avg(bs.duration_score)
                FROM build_scores bs
                JOIN workflow_runs wr ON wr.id = bs.workflow_run_id
                WHERE bs.repository_id = ? AND wr.completed_at >= ? AND wr.completed_at < ?
                """,
                Number.class,
                repositoryId,
                from,
                until);
        double efficiency = averageDuration == null ? 50 : averageDuration.doubleValue();
        double overall = MetricMath.clamp(
                reliability * 0.35 + velocity * 0.25 + quality * 0.20 + efficiency * 0.20);

        Map<String, Object> breakdown = new LinkedHashMap<>();
        breakdown.put("model_version", 1);
        breakdown.put("weights", Map.of(
                "reliability", 0.35, "velocity", 0.25, "quality", 0.20, "efficiency", 0.20));
        breakdown.put("completed_builds", totalBuilds);
        breakdown.put("successful_builds", successfulBuilds);
        breakdown.put("production_deployments", deploymentCount);
        breakdown.put("known_tests", knownTests);
        breakdown.put("flaky_tests", flakyTests);
        breakdown.put("missing_signal_baseline", 50);

        jdbc.update(
                """
                INSERT INTO repository_scores
                    (repository_id, window_days, overall_score, reliability_score,
                     velocity_score, quality_score, efficiency_score, grade,
                     breakdown, computed_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?::jsonb, ?)
                ON CONFLICT (repository_id, window_days, computed_at) DO UPDATE SET
                    overall_score = EXCLUDED.overall_score,
                    reliability_score = EXCLUDED.reliability_score,
                    velocity_score = EXCLUDED.velocity_score,
                    quality_score = EXCLUDED.quality_score,
                    efficiency_score = EXCLUDED.efficiency_score,
                    grade = EXCLUDED.grade,
                    breakdown = EXCLUDED.breakdown
                """,
                repositoryId,
                windowDays,
                overall,
                reliability,
                velocity,
                quality,
                efficiency,
                MetricMath.grade(overall),
                json(breakdown),
                computedAt);
    }

    private static Number number(Map<String, Object> row, String key) {
        return (Number) row.get(key);
    }

    private String json(Object value) {
        try {
            return objectMapper.writeValueAsString(value);
        } catch (JacksonException exception) {
            throw new IllegalStateException("could not serialize repository score", exception);
        }
    }
}
