package dev.buildlens.analytics.metrics;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;

final class MetricMath {
    private MetricMath() {}

    static Long percentile(List<Long> samples, double percentile) {
        if (samples.isEmpty()) {
            return null;
        }
        List<Long> sorted = new ArrayList<>(samples);
        sorted.sort(Comparator.naturalOrder());
        double position = percentile * (sorted.size() - 1);
        int lower = (int) Math.floor(position);
        int upper = (int) Math.ceil(position);
        if (lower == upper) {
            return sorted.get(lower);
        }
        double interpolated = sorted.get(lower)
                + (sorted.get(upper) - sorted.get(lower)) * (position - lower);
        return Math.round(interpolated);
    }

    static double clamp(double value) {
        return Math.max(0.0, Math.min(100.0, value));
    }

    static String grade(double score) {
        if (score >= 90) return "A";
        if (score >= 80) return "B";
        if (score >= 70) return "C";
        if (score >= 60) return "D";
        return "F";
    }
}
