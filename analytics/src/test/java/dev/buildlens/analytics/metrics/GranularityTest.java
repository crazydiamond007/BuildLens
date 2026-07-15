package dev.buildlens.analytics.metrics;

import static org.assertj.core.api.Assertions.assertThat;

import java.time.LocalDate;
import org.junit.jupiter.api.Test;

class GranularityTest {
    @Test
    void weeksAreMondayThroughSunday() {
        var period = Granularity.WEEKLY.periodContaining(LocalDate.of(2026, 7, 15));
        assertThat(period.start()).isEqualTo(LocalDate.of(2026, 7, 13));
        assertThat(period.end()).isEqualTo(LocalDate.of(2026, 7, 19));
    }

    @Test
    void monthUsesCalendarBoundaries() {
        var period = Granularity.MONTHLY.periodContaining(LocalDate.of(2024, 2, 12));
        assertThat(period.start()).isEqualTo(LocalDate.of(2024, 2, 1));
        assertThat(period.end()).isEqualTo(LocalDate.of(2024, 2, 29));
    }
}
