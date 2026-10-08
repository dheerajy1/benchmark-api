use serde::Serialize;

use crate::features::v1::benchmark::metrics::http_latency::round_two_decimals;

// Which way is an improvement, decided by the backend so no client has to infer it.
// These are API strings only, nothing in the database stores them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    LowerIsBetter,
    HigherIsBetter,
}

// A comparison of a whole number metric: milliseconds, bytes, counts
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IntegerMetric {
    pub baseline_value: Option<i64>,
    pub comparison_value: Option<i64>,
    pub absolute_difference: Option<i64>,
    pub percentage_change: Option<f64>,
    pub direction: Direction,
}

// A comparison of a decimal metric: percentages, latencies, throughput
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DecimalMetric {
    pub baseline_value: Option<f64>,
    pub comparison_value: Option<f64>,
    pub absolute_difference: Option<f64>,
    pub percentage_change: Option<f64>,
    pub direction: Direction,
}

// Both sides must exist, a missing side is never treated as zero
pub fn integer(
    baseline: Option<u64>,
    comparison: Option<u64>,
    direction: Direction,
) -> IntegerMetric {
    let baseline = baseline.and_then(|value| i64::try_from(value).ok());
    let comparison = comparison.and_then(|value| i64::try_from(value).ok());

    let (absolute_difference, percentage_change) = match (baseline, comparison) {
        (Some(baseline), Some(comparison)) => (
            comparison.checked_sub(baseline),
            percentage_change(baseline as f64, comparison as f64),
        ),
        _ => (None, None),
    };

    IntegerMetric {
        baseline_value: baseline,
        comparison_value: comparison,
        absolute_difference,
        percentage_change,
        direction,
    }
}

pub fn decimal(
    baseline: Option<f64>,
    comparison: Option<f64>,
    direction: Direction,
) -> DecimalMetric {
    let (absolute_difference, percentage_change) = match (baseline, comparison) {
        (Some(baseline), Some(comparison)) => (
            // Adding 0.0 turns a rounded -0.0 into 0.0
            Some(round_two_decimals(comparison - baseline) + 0.0),
            percentage_change(baseline, comparison),
        ),
        _ => (None, None),
    };

    DecimalMetric {
        baseline_value: baseline,
        comparison_value: comparison,
        absolute_difference,
        percentage_change,
        direction,
    }
}

// ((comparison - baseline) / baseline) * 100, null when the baseline is zero
fn percentage_change(baseline: f64, comparison: f64) -> Option<f64> {
    if baseline == 0.0 {
        return None;
    }

    Some(round_two_decimals((comparison - baseline) / baseline * 100.0) + 0.0)
}
