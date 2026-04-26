//! Time overrun watcher: fires when a task crosses the 80% timeout threshold.
//!
//! Monitors task completion signals emitted by the orchestrator and compares
//! the elapsed runtime against the task's declared `timeout_secs`.

use roko_core::{Body, Context, Engram, Kind, React};
use serde::Deserialize;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "time-overrun";

/// Custom conductor signal kind carrying completed task timing data.
pub const TASK_OUTPUT_KIND: &str = "conductor.agent_output";

/// Fraction of the timeout that triggers the early warning.
pub const ALERT_THRESHOLD: f64 = 0.80;

/// Fires when the latest task output exceeds 80% of its timeout.
#[derive(Debug, Clone, Default)]
pub struct TimeOverrunWatcher;

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
struct TaskTimingEvent {
    plan_id: String,
    task: String,
    duration_ms: u64,
    timeout_secs: u64,
}

impl TimeOverrunWatcher {
    /// Create a new instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

fn task_is_timing_event(signal: &Engram) -> bool {
    matches!(signal.kind, Kind::Custom(ref kind) if kind == TASK_OUTPUT_KIND)
}

fn extract_timing_event(signal: &Engram) -> Option<TaskTimingEvent> {
    if !task_is_timing_event(signal) {
        return None;
    }

    signal.body.as_json::<TaskTimingEvent>().ok()
}

fn exceeds_threshold(duration_ms: u64, timeout_secs: u64) -> bool {
    if timeout_secs == 0 {
        return false;
    }

    let timeout_ms = timeout_secs.saturating_mul(1000);
    duration_ms.saturating_mul(5) > timeout_ms.saturating_mul(4)
}

impl React for TimeOverrunWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        let Some(signal) = stream
            .iter()
            .rev()
            .find(|signal| task_is_timing_event(signal))
        else {
            return Vec::new();
        };

        let Some(event) = extract_timing_event(signal) else {
            return Vec::new();
        };

        if !exceeds_threshold(event.duration_ms, event.timeout_secs) {
            return Vec::new();
        }

        let timeout_ms = event.timeout_secs.saturating_mul(1000);
        let ratio = if timeout_ms > 0 {
            event.duration_ms as f64 / timeout_ms as f64
        } else {
            0.0
        };

        vec![
            Engram::builder(Kind::Custom("conductor.intervention".into()))
                .body(Body::text(format!(
                    "task {} exceeded 80% of timeout: {}ms of {}ms",
                    event.task, event.duration_ms, timeout_ms
                )))
                .tag("watcher", WATCHER_NAME)
                .tag("severity", "warning")
                .tag("plan_id", event.plan_id)
                .tag("task_id", event.task)
                .tag("duration_ms", event.duration_ms.to_string())
                .tag("timeout_secs", event.timeout_secs.to_string())
                .tag("threshold", ALERT_THRESHOLD.to_string())
                .tag("ratio", format!("{ratio:.3}"))
                .build(),
        ]
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_signal(task: &str, duration_ms: u64, timeout_secs: u64) -> Engram {
        let event = TaskTimingEvent {
            plan_id: "plan-1".into(),
            task: task.into(),
            duration_ms,
            timeout_secs,
        };

        Engram::builder(Kind::Custom(TASK_OUTPUT_KIND.into()))
            .body(Body::from_json(&event).expect("serialize timing event"))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = TimeOverrunWatcher::new();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![task_signal("task-1", 7_999, 10)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_no_fire() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![task_signal("task-1", 8_000, 10)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_threshold_fires() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![task_signal("task-1", 8_001, 10)];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
        assert_eq!(out[0].tag("task_id"), Some("task-1"));
    }

    #[test]
    fn uses_most_recent_task_signal() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![
            task_signal("task-1", 20_000, 10),
            task_signal("task-2", 1_000, 10),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn zero_timeout_no_fire() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![task_signal("task-1", 1_000, 0)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn non_task_signal_ignored() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("task finished"))
                .build(),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }
}
