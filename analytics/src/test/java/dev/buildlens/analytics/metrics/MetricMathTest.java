package dev.buildlens.analytics.metrics;

import static org.assertj.core.api.Assertions.assertThat;

import java.util.List;
import org.junit.jupiter.api.Test;

class MetricMathTest {
    @Test
    void interpolatesPercentilesWithoutUsingAMean() {
        assertThat(MetricMath.percentile(List.of(10L, 20L, 30L, 100L), 0.5)).isEqualTo(25L);
        assertThat(MetricMath.percentile(List.of(10L, 20L, 30L, 100L), 0.9)).isEqualTo(79L);
    }

    @Test
    void returnsNullForAnEmptyDistribution() {
        assertThat(MetricMath.percentile(List.of(), 0.5)).isNull();
    }
}
