//! Iteration loop watcher: detects implementation retry exhaustion.
//!
//! When an implementer has retried [`MAX_IMPLEMENTER_ATTEMPTS`] times
//! without advancing past the Implementing phase, this watcher fires
//! a critical signal to abort.

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Maximum implementer attempts before firing.
pub const MAX_IMPLEMENTER_ATTEMPTS: usize = 3;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "iteration-loop";

/// Tag key on plan-phase signals indicating an implementation restart.
pub const IMPL_RESTART_TAG: &str = "impl_restart";

/// Detects implementation iteration loops.
///
/// Counts `PlanPhase` signals tagged with `impl_restart=true` (emitted
/// when the executor restarts an implementer). Fires when the count
/// reaches [`MAX_IMPLEMENTER_ATTEMPTS`].
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

impl Policy for IterationLoopWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let restart_count = stream
            .iter()
            .filter(|s| {
                s.kind == Kind::PlanPhase && s.tag(IMPL_RESTART_TAG) == Some("true")
            })
            .count();

        if restart_count >= self.max_attempts {
            vec![Signal::builder(Kind::Custom(
                "conductor.intervention".into(),
            ))
            .body(Body::text(format!(
                "implementer restarted {restart_count} times without advancing"
            )))
            .tag("watcher", WATCHER_NAME)
            .tag("severity", "critical")
            .tag("attempts", restart_count.to_string())
            .build()]
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

    fn restart_signal() -> Signal {
        Signal::builder(Kind::PlanPhase)
            .body(Body::text("restarting implementer"))
            .tag(IMPL_RESTART_TAG, "true")
            .build()
    }

    fn other_phase_signal() -> Signal {
        Signal::builder(Kind::PlanPhase)
            .body(Body::text("advancing to gating"))
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
        let stream = vec![restart_signal(), restart_signal()]; // 2 < 3
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_fires() {
        let w = IterationLoopWatcher::default();
        let stream = vec![restart_signal(), restart_signal(), restart_signal()];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
        assert_eq!(out[0].tag("severity"), Some("critical"));
    }

    #[test]
    fn non_restart_signals_ignored() {
        let w = IterationLoopWatcher::default();
        let stream = vec![other_phase_signal(), other_phase_signal(), other_phase_signal()];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn mixed_signals_only_count_restarts() {
        let w = IterationLoopWatcher::new(2);
        let stream = vec![
            other_phase_signal(),
            restart_signal(),
            other_phase_signal(),
            restart_signal(),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn custom_threshold() {
        let w = IterationLoopWatcher::new(5);
        let stream: Vec<Signal> = (0..4).map(|_| restart_signal()).collect();
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
        let mut stream5 = stream;
        stream5.push(restart_signal());
        let out = w.decide(&stream5, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
