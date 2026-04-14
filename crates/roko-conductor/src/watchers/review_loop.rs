//! Review loop watcher: detects repeated review rejects without progress.
//!
//! When the same plan is rejected repeatedly without advancing to a later
//! phase, this watcher fires a warning so the conductor can restart or
//! escalate.

use roko_core::{Body, Context, Engram, Kind, Policy};

/// Maximum times the same review feedback can appear before firing.
pub const MAX_REVIEW_CYCLES: usize = 3;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "review-loop";
const PLAN_ID_TAG: &str = "plan_id";
const PLAN_EVENT_FIELD: &str = "event";

/// Detects repeated review rejects for a single plan.
#[derive(Debug, Clone)]
pub struct ReviewLoopWatcher {
    /// Max identical review feedbacks before firing.
    max_cycles: usize,
}

impl Default for ReviewLoopWatcher {
    fn default() -> Self {
        Self {
            max_cycles: MAX_REVIEW_CYCLES,
        }
    }
}

impl ReviewLoopWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_cycles: usize) -> Self {
        Self { max_cycles }
    }
}

fn signal_plan_id(signal: &Engram) -> Option<String> {
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

fn latest_plan_id(stream: &[Engram]) -> Option<String> {
    stream.iter().rev().find_map(signal_plan_id)
}

fn plan_event(signal: &Engram) -> Option<String> {
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

impl Policy for ReviewLoopWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        let Some(plan_id) = latest_plan_id(stream) else {
            return Vec::new();
        };

        let mut review_rejects = 0usize;

        for s in stream {
            if signal_plan_id(s).as_deref() != Some(plan_id.as_str()) {
                continue;
            }

            match s.kind {
                Kind::PlanPhase => match plan_event(s).as_deref() {
                    Some("ReviewRejected") => {
                        review_rejects += 1;
                        if review_rejects >= self.max_cycles {
                            return vec![
                                Engram::builder(Kind::Custom(
                                    "conductor.intervention".into(),
                                ))
                                .body(Body::text(format!(
                                    "plan {plan_id} repeated review rejects {review_rejects} times without progress"
                                )))
                                .tag("watcher", WATCHER_NAME)
                                .tag("severity", "warning")
                                .tag("plan_id", plan_id)
                                .tag("count", review_rejects.to_string())
                                .build(),
                            ];
                        }
                    }
                    Some("ReviewApproved") | Some("DocRevisionDone") | Some("MergeSucceeded") => {
                        review_rejects = 0;
                    }
                    _ => {}
                },
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

    fn review_phase(event: &str) -> Engram {
        Engram::builder(Kind::PlanPhase)
            .body(Body::Json(serde_json::json!({
                "plan_id": "plan-1",
                "event": event,
            })))
            .tag(PLAN_ID_TAG, "plan-1")
            .build()
    }

    fn other_plan_phase(event: &str) -> Engram {
        Engram::builder(Kind::PlanPhase)
            .body(Body::Json(serde_json::json!({
                "plan_id": "plan-2",
                "event": event,
            })))
            .tag(PLAN_ID_TAG, "plan-2")
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = ReviewLoopWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn unique_reviews_no_fire() {
        let w = ReviewLoopWatcher::default();
        let stream = vec![
            review_phase("ReviewRejected"),
            review_phase("ReviewApproved"),
            review_phase("DocRevisionDone"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn repeated_reviews_fires() {
        let w = ReviewLoopWatcher::default();
        let stream = vec![
            review_phase("ReviewRejected"),
            review_phase("ReviewRejected"),
            review_phase("ReviewRejected"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn non_review_signals_ignored() {
        let w = ReviewLoopWatcher::default();
        let stream = vec![
            other_plan_phase("ImplementationDone"),
            other_plan_phase("GateFailed"),
            other_plan_phase("AutoFixDone"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = ReviewLoopWatcher::new(3);
        let stream = vec![
            review_phase("ReviewRejected"),
            review_phase("ReviewRejected"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn approved_review_resets_loop() {
        let w = ReviewLoopWatcher::new(2);
        let stream = vec![
            review_phase("ReviewRejected"),
            review_phase("ReviewApproved"),
            review_phase("ReviewRejected"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn interleaved_reviews_still_count() {
        let w = ReviewLoopWatcher::new(3);
        let stream = vec![
            review_phase("ReviewRejected"),
            other_plan_phase("GateFailed"),
            review_phase("ReviewRejected"),
            other_plan_phase("AutoFixDone"),
            review_phase("ReviewRejected"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
