//! Iteration loop watcher: detects repeated gate-fail retries without progress.
//!
//! When the same plan repeatedly cycles through gate failures without
//! advancing to a later phase, this watcher fires a critical signal to abort.

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Maximum implementer attempts before firing.
pub const MAX_IMPLEMENTER_ATTEMPTS: usize = 3;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "iteration-loop";
const PLAN_ID_TAG: &str = "plan_id";
const PLAN_EVENT_FIELD: &str = "event";

/// Detects repeated gate-fail cycles for a single plan.
#[derive(Debug, Clone)]
pub struct IterationLoopWatcher {
    /// Max implementer restarts before firing.
    max_attempts: usize,
}

impl Default for IterationLoopWatcher {
    fn default() -> Self {
        Self {
            max_attempts: MAX_IMPLEMENTER_ATTEMPTS,
        }
    }
}

impl IterationLoopWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_attempts: usize) -> Self {
        Self { max_attempts }
    }
}

fn signal_plan_id(signal: &Signal) -> Option<String> {
    signal.tag(PLAN_ID_TAG).map(str::to_owned).or_else(|| {
        if signal.kind != Kind::PlanPhase {
            return None;
        }

        signal
            .body
            .as_json::<serde_json::Value>()
            .ok()
            .and_then(|body| {
                body.get(PLAN_ID_TAG)
                    .and_then(|plan_id| plan_id.as_str())
                    .map(str::to_owned)
            })
    })
}

fn latest_plan_id(stream: &[Signal]) -> Option<String> {
    stream.iter().rev().find_map(signal_plan_id)
}

fn plan_event(signal: &Signal) -> Option<String> {
    if signal.kind != Kind::PlanPhase {
        return None;
    }

    signal
        .body
        .as_json::<serde_json::Value>()
        .ok()
        .and_then(|body| {
            body.get(PLAN_EVENT_FIELD)
                .and_then(|event| event.as_str())
                .map(str::to_owned)
        })
}

impl Policy for IterationLoopWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let Some(plan_id) = latest_plan_id(stream) else {
            return Vec::new();
        };

        let mut gate_failures = 0usize;

        for signal in stream {
            if signal_plan_id(signal).as_deref() != Some(plan_id.as_str()) {
                continue;
            }

            if signal.kind != Kind::PlanPhase {
                continue;
            }

            match plan_event(signal).as_deref() {
                Some("GateFailed") => {
                    gate_failures += 1;
                    if gate_failures >= self.max_attempts {
                        return vec![
                            Signal::builder(Kind::Custom("conductor.intervention".into()))
                                .body(Body::text(format!(
                                    "plan {plan_id} repeated gate failures {gate_failures} times without progress"
                                )))
                                .tag("watcher", WATCHER_NAME)
                                .tag("severity", "critical")
                                .tag("plan_id", plan_id)
                                .tag("attempts", gate_failures.to_string())
                                .build(),
                        ];
                    }
                }
                Some("GatePassed")
                | Some("ImplementationDone")
                | Some("ReviewApproved")
                | Some("DocRevisionDone")
                | Some("MergeSucceeded")
                | Some("VerifyPassed") => {
                    gate_failures = 0;
                }
                _ => {}
            }
        }

        Vec::new()
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_phase_signal(event: &str) -> Signal {
        Signal::builder(Kind::PlanPhase)
            .body(Body::Json(serde_json::json!({
                "plan_id": "plan-1",
                "event": event,
            })))
            .tag(PLAN_ID_TAG, "plan-1")
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = IterationLoopWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = IterationLoopWatcher::default();
        let stream = vec![
            plan_phase_signal("GateFailed"),
            plan_phase_signal("GateFailed"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_fires() {
        let w = IterationLoopWatcher::default();
        let stream = vec![
            plan_phase_signal("GateFailed"),
            plan_phase_signal("GateFailed"),
            plan_phase_signal("GateFailed"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
        assert_eq!(out[0].tag("severity"), Some("critical"));
    }

    #[test]
    fn non_restart_signals_ignored() {
        let w = IterationLoopWatcher::default();
        let stream = vec![
            plan_phase_signal("ImplementationDone"),
            plan_phase_signal("GatePassed"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn mixed_signals_only_count_restarts() {
        let w = IterationLoopWatcher::new(2);
        let stream = vec![
            plan_phase_signal("GatePassed"),
            plan_phase_signal("GateFailed"),
            plan_phase_signal("AutoFixDone"),
            plan_phase_signal("GateFailed"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn custom_threshold() {
        let w = IterationLoopWatcher::new(5);
        let stream: Vec<Signal> = (0..4).map(|_| plan_phase_signal("GateFailed")).collect();
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
        let mut stream5 = stream;
        stream5.push(plan_phase_signal("GateFailed"));
        let out = w.decide(&stream5, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
