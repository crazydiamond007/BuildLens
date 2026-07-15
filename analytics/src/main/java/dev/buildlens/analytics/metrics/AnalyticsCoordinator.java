package dev.buildlens.analytics.metrics;

import java.time.LocalDate;
import java.time.OffsetDateTime;
import java.util.UUID;
import org.springframework.stereotype.Service;

@Service
public class AnalyticsCoordinator {
    private final BuildScoreService buildScores;
    private final DoraService dora;
    private final FlakyTestService flakyTests;
    private final RepositoryScoreService repositoryScores;

    public AnalyticsCoordinator(
            BuildScoreService buildScores,
            DoraService dora,
            FlakyTestService flakyTests,
            RepositoryScoreService repositoryScores) {
        this.buildScores = buildScores;
        this.dora = dora;
        this.flakyTests = flakyTests;
        this.repositoryScores = repositoryScores;
    }

    public void workflowRunCompleted(UUID workflowRunId, UUID repositoryId, OffsetDateTime occurredAt) {
        if (buildScores.recompute(workflowRunId, repositoryId) == null) {
            throw new IllegalArgumentException("event workflow run does not exist or is not completed");
        }
        flakyTests.recompute(repositoryId, occurredAt);
        repositoryScores.recompute(repositoryId, occurredAt);
    }

    public void deploymentRecorded(UUID deploymentId, UUID repositoryId, OffsetDateTime occurredAt) {
        dora.recompute(repositoryId, dora.deploymentDate(deploymentId, repositoryId));
        flakyTests.recompute(repositoryId, occurredAt);
        repositoryScores.recompute(repositoryId, occurredAt);
    }

    public void recomputeRepository(UUID repositoryId, OffsetDateTime asOf) {
        dora.recompute(repositoryId, asOf.toLocalDate());
        flakyTests.recompute(repositoryId, asOf);
        repositoryScores.recompute(repositoryId, asOf);
    }

    public void scheduledRecompute(UUID repositoryId, OffsetDateTime asOf, int lookbackDays) {
        LocalDate first = asOf.toLocalDate().minusDays(lookbackDays - 1L);
        for (LocalDate date = first; !date.isAfter(asOf.toLocalDate()); date = date.plusDays(1)) {
            dora.recompute(repositoryId, date);
        }
        buildScores.recomputeRecent(repositoryId, asOf, lookbackDays);
        flakyTests.recompute(repositoryId, asOf);
        repositoryScores.recompute(repositoryId, asOf);
    }
}
