package dev.buildlens.analytics.metrics;

import tools.jackson.core.JacksonException;
import tools.jackson.databind.ObjectMapper;
import java.time.OffsetDateTime;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;

@Service
public class BuildScoreService {
    private final JdbcTemplate jdbc;
    private final ObjectMapper objectMapper;

    public BuildScoreService(JdbcTemplate jdbc, ObjectMapper objectMapper) {
        this.jdbc = jdbc;
        this.objectMapper = objectMapper;
    }

    @Transactional
    public UUID recompute(UUID workflowRunId) {
        return recompute(workflowRunId, null);
    }

    @Transactional
    public UUID recompute(UUID workflowRunId, UUID expectedRepositoryId) {
        List<RunFacts> runs = jdbc.query(
                """
                SELECT id, repository_id, conclusion, duration_ms
                FROM workflow_runs WHERE id = ? AND status = 'completed'
                """,
                (rs, ignored) -> new RunFacts(
                        rs.getObject("id", UUID.class),
                        rs.getObject("repository_id", UUID.class),
                        rs.getString("conclusion"),
                        (Long) rs.getObject("duration_ms")),
                workflowRunId);
        if (runs.isEmpty()) {
            return null;
        }
        RunFacts run = runs.getFirst();
        if (expectedRepositoryId != null && !expectedRepositoryId.equals(run.repositoryId())) {
            throw new IllegalArgumentException("event repository does not match its workflow run");
        }
        Number medianValue = jdbc.queryForObject(
                """
                SELECT percentile_cont(0.5) WITHIN GROUP (ORDER BY duration_ms)
                FROM workflow_runs
                WHERE repository_id = ? AND status = 'completed' AND duration_ms IS NOT NULL
                  AND completed_at >= now() - interval '30 days'
                """,
                Number.class,
                run.repositoryId());
        double durationScore = durationScore(run.durationMs(), medianValue);
        double reliabilityScore = reliabilityScore(run.conclusion());
        Map<String, Object> testCounts = jdbc.queryForMap(
                """
                SELECT count(*) FILTER (WHERE status IN ('failed', 'error')) AS failed,
                       count(*) FILTER (WHERE status IN ('passed', 'failed', 'error')) AS total
                FROM test_results WHERE workflow_run_id = ?
                """,
                workflowRunId);
        int failed = ((Number) testCounts.get("failed")).intValue();
        int total = ((Number) testCounts.get("total")).intValue();
        double flakinessScore = total == 0 ? 100 : 100.0 * (1.0 - (double) failed / total);
        double overall = MetricMath.clamp(
                reliabilityScore * 0.50 + durationScore * 0.30 + flakinessScore * 0.20);

        Map<String, Object> breakdown = new LinkedHashMap<>();
        breakdown.put("model_version", 1);
        breakdown.put("weights", Map.of("reliability", 0.50, "duration", 0.30, "tests", 0.20));
        breakdown.put("duration_ms", run.durationMs());
        breakdown.put("repository_duration_p50_ms", medianValue);
        breakdown.put("test_results", total);
        breakdown.put("failed_test_results", failed);

        jdbc.update(
                """
                INSERT INTO build_scores
                    (workflow_run_id, repository_id, score, duration_score,
                     reliability_score, flakiness_score, breakdown)
                VALUES (?, ?, ?, ?, ?, ?, ?::jsonb)
                ON CONFLICT (workflow_run_id) DO UPDATE SET
                    repository_id = EXCLUDED.repository_id,
                    score = EXCLUDED.score,
                    duration_score = EXCLUDED.duration_score,
                    reliability_score = EXCLUDED.reliability_score,
                    flakiness_score = EXCLUDED.flakiness_score,
                    breakdown = EXCLUDED.breakdown,
                    computed_at = now()
                """,
                workflowRunId,
                run.repositoryId(),
                overall,
                durationScore,
                reliabilityScore,
                flakinessScore,
                json(breakdown));
        return run.repositoryId();
    }

    @Transactional
    public void recomputeRecent(UUID repositoryId, OffsetDateTime asOf, int windowDays) {
        List<UUID> runIds = jdbc.queryForList(
                """
                SELECT id FROM workflow_runs
                WHERE repository_id = ? AND status = 'completed'
                  AND completed_at >= ? AND completed_at <= ?
                ORDER BY completed_at
                """,
                UUID.class,
                repositoryId,
                asOf.minusDays(windowDays),
                asOf);
        for (UUID runId : runIds) {
            recompute(runId);
        }
    }

    private static double reliabilityScore(String conclusion) {
        if (conclusion == null) return 25;
        return switch (conclusion) {
            case "success" -> 100;
            case "neutral", "skipped", "cancelled" -> 50;
            default -> 0;
        };
    }

    private static double durationScore(Long durationMs, Number medianValue) {
        if (durationMs == null || medianValue == null || medianValue.doubleValue() <= 0) return 100;
        double ratio = durationMs / medianValue.doubleValue();
        return MetricMath.clamp(100.0 / Math.max(1.0, ratio));
    }

    private String json(Object value) {
        try {
            return objectMapper.writeValueAsString(value);
        } catch (JacksonException exception) {
            throw new IllegalStateException("could not serialize score breakdown", exception);
        }
    }

    private record RunFacts(UUID id, UUID repositoryId, String conclusion, Long durationMs) {}
}
