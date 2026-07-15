package dev.buildlens.analytics.metrics;

import java.time.DayOfWeek;
import java.time.LocalDate;
import java.time.temporal.TemporalAdjusters;

public enum Granularity {
    DAILY,
    WEEKLY,
    MONTHLY;

    public Period periodContaining(LocalDate date) {
        return switch (this) {
            case DAILY -> new Period(date, date);
            case WEEKLY -> {
                LocalDate start = date.with(TemporalAdjusters.previousOrSame(DayOfWeek.MONDAY));
                yield new Period(start, start.plusDays(6));
            }
            case MONTHLY -> {
                LocalDate start = date.withDayOfMonth(1);
                yield new Period(start, start.with(TemporalAdjusters.lastDayOfMonth()));
            }
        };
    }

    public String databaseValue() {
        return name().toLowerCase(java.util.Locale.ROOT);
    }

    public record Period(LocalDate start, LocalDate end) {
        public long days() {
            return java.time.temporal.ChronoUnit.DAYS.between(start, end) + 1;
        }
    }
}
