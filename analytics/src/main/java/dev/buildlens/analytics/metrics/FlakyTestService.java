package dev.buildlens.analytics.metrics;

import java.math.BigDecimal;
import java.math.RoundingMode;
import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.util.List;
import java.util.UUID;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.stereotype.Service;
import org.springframework.transaction.annotation.Transactional;

@Service
public class FlakyTestService {
    private final JdbcTemplate jdbc;
    private final int windowDays;

    public FlakyTestService(JdbcTemplate jdbc, @Value("${analytics.recompute.window-days}") int windowDays) {
        this.jdbc = jdbc;
        this.windowDays = windowDays;
    }

    @Transactional
    public void recompute(UUID repositoryId, OffsetDateTime asOf) {
        OffsetDateTime until = asOf.withOffsetSameInstant(ZoneOffset.UTC);
        OffsetDateTime from = until.minusDays(windowDays);
        List<TestSummary> summaries = jdbc.query(
                """
                WITH per_run AS (
                    SELECT tr.test_key, max(tr.suite) AS suite,
                           max(tr.classname) AS classname, max(tr.name) AS name,
                           tr.workflow_run_id,
                           COALESCE(wr.workflow_id::text, 'run:' || wr.github_run_id::text)
                               AS workflow_key,
                           wr.head_sha,
                           max(tr.executed_at) AS executed_at,
                           CASE WHEN bool_or(tr.status IN ('failed', 'error')) THEN 'failed'
                                WHEN bool_or(tr.status = 'passed') THEN 'passed'
                                ELSE 'skipped' END AS outcome
                    FROM test_results tr
                    JOIN workflow_runs wr ON wr.id = tr.workflow_run_id
                    WHERE tr.repository_id = ?
                      AND tr.executed_at >= ? AND tr.executed_at <= ?
                    GROUP BY tr.test_key, tr.workflow_run_id, wr.workflow_id,
                             wr.github_run_id, wr.head_sha
                ), ordered AS (
                    SELECT per_run.*,
                           lag(outcome) OVER (
                               PARTITION BY test_key, workflow_key, head_sha
                               ORDER BY executed_at, workflow_run_id) AS previous_outcome
                    FROM per_run
                    WHERE outcome <> 'skipped'
                )
                SELECT test_key,
                       max(suite) AS suite,
                       max(classname) AS classname,
                       max(name) AS name,
                       count(*) FILTER (WHERE outcome <> 'skipped') AS total_runs,
                       count(*) FILTER (WHERE outcome = 'passed') AS passed_runs,
                       count(*) FILTER (WHERE outcome = 'failed') AS failed_runs,
                       count(*) FILTER (
                           WHERE previous_outcome IS NOT NULL
                             AND previous_outcome <> outcome) AS flip_count,
                       count(*) FILTER (WHERE previous_outcome IS NOT NULL) AS transition_count,
                       min(executed_at) AS first_seen_at,
                       max(executed_at) AS last_seen_at,
                       max(executed_at) FILTER (WHERE outcome = 'failed') AS last_failed_at
                FROM ordered
                GROUP BY test_key
                """,
                (rs, ignored) -> new TestSummary(
                        rs.getString("test_key"),
                        rs.getString("suite"),
                        rs.getString("classname"),
                        rs.getString("name"),
                        rs.getInt("total_runs"),
                        rs.getInt("passed_runs"),
                        rs.getInt("failed_runs"),
                        rs.getInt("flip_count"),
                        rs.getInt("transition_count"),
                        rs.getObject("first_seen_at", OffsetDateTime.class),
                        rs.getObject("last_seen_at", OffsetDateTime.class),
                        rs.getObject("last_failed_at", OffsetDateTime.class)),
                repositoryId,
                from,
                until);

        for (TestSummary summary : summaries) {
            BigDecimal rate = BigDecimal.valueOf(summary.transitionCount() == 0
                            ? 0
                            : (double) summary.flipCount() / summary.transitionCount())
                    .setScale(4, RoundingMode.HALF_UP);
            jdbc.update(
                    """
                    INSERT INTO flaky_tests
                        (repository_id, test_key, suite, classname, name, window_days,
                         total_runs, passed_runs, failed_runs, flip_count, flake_rate,
                         is_flaky, first_seen_at, last_seen_at, last_failed_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    ON CONFLICT (repository_id, test_key) DO UPDATE SET
                        suite = EXCLUDED.suite,
                        classname = EXCLUDED.classname,
                        name = EXCLUDED.name,
                        window_days = EXCLUDED.window_days,
                        total_runs = EXCLUDED.total_runs,
                        passed_runs = EXCLUDED.passed_runs,
                        failed_runs = EXCLUDED.failed_runs,
                        flip_count = EXCLUDED.flip_count,
                        flake_rate = EXCLUDED.flake_rate,
                        is_flaky = EXCLUDED.is_flaky,
                        first_seen_at = EXCLUDED.first_seen_at,
                        last_seen_at = EXCLUDED.last_seen_at,
                        last_failed_at = EXCLUDED.last_failed_at,
                        computed_at = now()
                    """,
                    repositoryId,
                    summary.testKey(),
                    summary.suite(),
                    summary.classname(),
                    summary.name(),
                    windowDays,
                    summary.totalRuns(),
                    summary.passedRuns(),
                    summary.failedRuns(),
                    summary.flipCount(),
                    rate,
                    summary.flipCount() > 0,
                    summary.firstSeenAt(),
                    summary.lastSeenAt(),
                    summary.lastFailedAt());
        }

        jdbc.update(
                """
                UPDATE flaky_tests ft SET
                    total_runs = 0, passed_runs = 0, failed_runs = 0,
                    flip_count = 0, flake_rate = 0, is_flaky = false, computed_at = now()
                WHERE ft.repository_id = ?
                  AND NOT EXISTS (
                    SELECT 1 FROM test_results tr
                    WHERE tr.repository_id = ft.repository_id
                      AND tr.test_key = ft.test_key
                      AND tr.status IN ('passed', 'failed', 'error')
                      AND tr.executed_at >= ? AND tr.executed_at <= ?)
                """,
                repositoryId,
                from,
                until);
    }

    private record TestSummary(
            String testKey,
            String suite,
            String classname,
            String name,
            int totalRuns,
            int passedRuns,
            int failedRuns,
            int flipCount,
            int transitionCount,
            OffsetDateTime firstSeenAt,
            OffsetDateTime lastSeenAt,
            OffsetDateTime lastFailedAt) {}
}
