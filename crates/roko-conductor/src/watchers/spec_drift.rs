//! Spec drift watcher: fires when spec-drift metric exceeds threshold.
//!
//! Monitors `Metric` signals tagged `name=spec_drift` for accumulated
//! drift between the original spec and the implementation. Fires when
//! the drift exceeds [`MAX_SPEC_DRIFT_RATIO`].

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Maximum acceptable spec drift ratio (0.0 to 1.0).
pub const MAX_SPEC_DRIFT_RATIO: f64 = 0.25;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "spec-drift";

/// Tag key identifying the metric name on `Metric` signals.
pub const METRIC_NAME_TAG: &str = "name";
/// Expected metric name value for spec-drift readings.
pub const SPEC_DRIFT_METRIC: &str = "spec_drift";
/// Tag key for the numeric drift value.
pub const METRIC_VALUE_TAG: &str = "value";

/// Fires when spec drift exceeds [`MAX_SPEC_DRIFT_RATIO`].
///
/// Examines the most recent `Metric` signal with `name=spec_drift`
/// and fires if the drift value exceeds the threshold.
#[derive(Debug, Clone)]
pub struct SpecDriftWatcher {
    /// Maximum drift ratio before firing.
    max_drift: f64,
}

impl Default for SpecDriftWatcher {
    fn default() -> Self {
        Self {
            max_drift: MAX_SPEC_DRIFT_RATIO,
        }
    }
}

impl SpecDriftWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_drift: f64) -> Self {
        Self { max_drift }
    }
}

impl Policy for SpecDriftWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        // Find the most recent spec-drift metric.
        let latest = stream
            .iter()
            .rev()
            .find(|s| s.kind == Kind::Metric && s.tag(METRIC_NAME_TAG) == Some(SPEC_DRIFT_METRIC));

        let Some(signal) = latest else {
            return Vec::new();
        };

        let drift: f64 = signal
            .tag(METRIC_VALUE_TAG)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);

        if drift > self.max_drift {
            vec![
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "spec drift {drift:.1}% exceeds threshold {:.0}%",
                        self.max_drift * 100.0
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("drift", format!("{drift:.3}"))
                    .build(),
            ]
        } else {
            Vec::new()
        }
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drift_signal(value: f64) -> Signal {
        Signal::builder(Kind::Metric)
            .body(Body::text("spec drift reading"))
            .tag(METRIC_NAME_TAG, SPEC_DRIFT_METRIC)
            .tag(METRIC_VALUE_TAG, &format!("{value}"))
            .build()
    }

    fn unrelated_metric() -> Signal {
        Signal::builder(Kind::Metric)
            .body(Body::text("other metric"))
            .tag(METRIC_NAME_TAG, "cpu_usage")
            .tag(METRIC_VALUE_TAG, "0.99")
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = SpecDriftWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = SpecDriftWatcher::default();
        let stream = vec![drift_signal(0.20)]; // 20% < 25%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_no_fire() {
        let w = SpecDriftWatcher::default();
        let stream = vec![drift_signal(0.25)]; // exactly 25%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_threshold_fires() {
        let w = SpecDriftWatcher::default();
        let stream = vec![drift_signal(0.30)]; // 30% > 25%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn uses_most_recent() {
        let w = SpecDriftWatcher::default();
        let stream = vec![
            drift_signal(0.50), // old — above threshold
            drift_signal(0.10), // most recent — below threshold
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn unrelated_metrics_ignored() {
        let w = SpecDriftWatcher::default();
        let stream = vec![unrelated_metric()];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn custom_threshold() {
        let w = SpecDriftWatcher::new(0.10);
        let stream = vec![drift_signal(0.15)]; // 15% > 10%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
