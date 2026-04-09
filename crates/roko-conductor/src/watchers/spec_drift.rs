//! Spec drift watcher: fires when a task's file edits drift outside its scope.
//!
//! Monitors `Metric` signals tagged `name=spec_drift` for accumulated
//! drift between the files a task declared it would write and the files
//! it actually changed. Fires when the drift exceeds
//! [`MAX_SPEC_DRIFT_RATIO`].

use roko_core::{Body, Context, Kind, Policy, Signal};
use serde::Deserialize;

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

#[derive(Debug, Clone, Default, Deserialize)]
struct SpecDriftEvent {
    #[serde(default)]
    plan_id: Option<String>,
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    write_files: Vec<String>,
    #[serde(default)]
    changed_files: Vec<String>,
    #[serde(default)]
    unexpected_files: Vec<String>,
    #[serde(default)]
    drift_ratio: Option<f64>,
}

fn path_is_allowed(path: &str, allowed_files: &[String]) -> bool {
    allowed_files.iter().any(|declared| {
        path == declared
            || path.starts_with(&format!("{declared}/"))
            || path.starts_with(&format!("{declared}\\"))
    })
}

impl SpecDriftEvent {
    fn unexpected_files(&self) -> Vec<String> {
        if !self.unexpected_files.is_empty() {
            return self.unexpected_files.clone();
        }

        self.changed_files
            .iter()
            .filter(|path| !path_is_allowed(path, &self.write_files))
            .cloned()
            .collect()
    }

    fn drift_ratio(&self) -> f64 {
        if let Some(ratio) = self.drift_ratio {
            return ratio;
        }

        let changed = self.changed_files.len();
        if changed == 0 {
            0.0
        } else {
            self.unexpected_files().len() as f64 / changed as f64
        }
    }
}

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

        let drift = signal
            .body
            .as_json::<SpecDriftEvent>()
            .ok()
            .map(|event| event.drift_ratio())
            .or_else(|| signal.tag(METRIC_VALUE_TAG).and_then(|v| v.parse().ok()))
            .unwrap_or(0.0);

        if drift > self.max_drift {
            let details = signal.body.as_json::<SpecDriftEvent>().ok();
            let plan_id = details
                .as_ref()
                .and_then(|event| event.plan_id.as_deref())
                .unwrap_or("unknown");
            let task_id = details
                .as_ref()
                .and_then(|event| event.task_id.as_deref())
                .unwrap_or("unknown");
            let unexpected_count = details
                .as_ref()
                .map_or(0, |event| event.unexpected_files().len());
            let changed_count = details
                .as_ref()
                .map_or(0, |event| event.changed_files.len());
            let write_count = details.as_ref().map_or(0, |event| event.write_files.len());
            vec![
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "spec drift {drift:.1}% exceeds threshold {:.0}% (plan={plan_id}, task={task_id}, changed={changed_count}, write_files={write_count}, unexpected={unexpected_count})",
                        self.max_drift * 100.0,
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("drift", format!("{drift:.3}"))
                    .tag("plan_id", plan_id)
                    .tag("task_id", task_id)
                    .tag("changed_files", changed_count.to_string())
                    .tag("write_files", write_count.to_string())
                    .tag("unexpected_files", unexpected_count.to_string())
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

    #[test]
    fn detailed_file_drift_fires_on_unexpected_changes() {
        let w = SpecDriftWatcher::default();
        let stream = vec![
            Signal::builder(Kind::Metric)
                .body(
                    Body::from_json(&serde_json::json!({
                        "plan_id": "plan-1",
                        "task_id": "task-1",
                        "write_files": ["src/lib.rs"],
                        "changed_files": ["src/lib.rs", "src/main.rs"],
                        "unexpected_files": ["src/main.rs"],
                        "drift_ratio": 0.5,
                    }))
                    .expect("serialize spec drift event"),
                )
                .tag(METRIC_NAME_TAG, SPEC_DRIFT_METRIC)
                .tag(METRIC_VALUE_TAG, "0.5")
                .build(),
        ];

        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }
}
