package dev.buildlens.analytics.metrics;

import java.time.OffsetDateTime;
import java.time.ZoneOffset;
import java.util.List;
import java.util.UUID;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.jdbc.core.JdbcTemplate;
import org.springframework.scheduling.annotation.Scheduled;
import org.springframework.stereotype.Component;

@Component
public class ScheduledRecompute {
    private static final Logger LOG = LoggerFactory.getLogger(ScheduledRecompute.class);

    private final JdbcTemplate jdbc;
    private final AnalyticsCoordinator coordinator;
    private final int lookbackDays;

    public ScheduledRecompute(
            JdbcTemplate jdbc,
            AnalyticsCoordinator coordinator,
            @Value("${analytics.recompute.lookback-days}") int lookbackDays) {
        this.jdbc = jdbc;
        this.coordinator = coordinator;
        this.lookbackDays = lookbackDays;
    }

    @Scheduled(cron = "${analytics.recompute.cron}", zone = "UTC")
    public void recomputeTrackedRepositories() {
        List<UUID> repositories = jdbc.queryForList(
                """
                SELECT id FROM repositories
                WHERE tracking_enabled AND deleted_at IS NULL
                ORDER BY id
                """,
                UUID.class);
        OffsetDateTime now = OffsetDateTime.now(ZoneOffset.UTC);
        for (UUID repositoryId : repositories) {
            try {
                coordinator.scheduledRecompute(repositoryId, now, lookbackDays);
            } catch (RuntimeException exception) {
                LOG.error("scheduled analytics recompute failed for repository {}", repositoryId, exception);
            }
        }
    }
}
