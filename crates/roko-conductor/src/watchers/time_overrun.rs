//! Time overrun watcher: fires when a plan exceeds its phase timeout.
//!
//! Monitors `PlanPhase` signals for phase-entry timestamps and compares
//! the elapsed time against the configured phase timeout.

use crate::state_machine::phase_timeout;
use roko_core::{Body, Context, Kind, Policy, Signal, TaskComplexityBand};

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "time-overrun";

/// Tag key on `PlanPhase` signals for the current phase name.
pub const PHASE_TAG: &str = "phase";
/// Tag key on `PlanPhase` signals for the phase-entry timestamp (ms).
pub const PHASE_ENTERED_TAG: &str = "phase_entered_ms";
/// Tag key on `PlanPhase` signals for task complexity band.
pub const COMPLEXITY_TAG: &str = "complexity";

/// Fires when the current phase has exceeded its timeout.
///
/// Examines the most recent `PlanPhase` signal for `phase` and
/// `phase_entered_ms` tags, computes the elapsed time relative to
/// `ctx.now_ms`, and fires if it exceeds the configured timeout.
#[derive(Debug, Clone, Default)]
pub struct TimeOverrunWatcher;

impl TimeOverrunWatcher {
    /// Create a new instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Parse a `PhaseKind` from its kebab-case string representation.
fn parse_phase(s: &str) -> Option<roko_core::PhaseKind> {
    match s {
        "queued" => Some(roko_core::PhaseKind::Queued),
        "enriching" => Some(roko_core::PhaseKind::Enriching),
        "implementing" => Some(roko_core::PhaseKind::Implementing),
        "gating" => Some(roko_core::PhaseKind::Gating),
        "verifying" => Some(roko_core::PhaseKind::Verifying),
        "reviewing" => Some(roko_core::PhaseKind::Reviewing),
        "doc-revision" => Some(roko_core::PhaseKind::DocRevision),
        "auto-fixing" => Some(roko_core::PhaseKind::AutoFixing),
        "regenerating-verify" => Some(roko_core::PhaseKind::RegeneratingVerify),
        "merging" => Some(roko_core::PhaseKind::Merging),
        "complete" => Some(roko_core::PhaseKind::Complete),
        "done" => Some(roko_core::PhaseKind::Done),
        "failed" => Some(roko_core::PhaseKind::Failed),
        "skipped" => Some(roko_core::PhaseKind::Skipped),
        _ => None,
    }
}

/// Parse a `TaskComplexityBand` from its kebab-case string.
fn parse_complexity(s: &str) -> TaskComplexityBand {
    match s {
        "complex" => TaskComplexityBand::Complex,
        "standard" => TaskComplexityBand::Standard,
        _ => TaskComplexityBand::Fast,
    }
}

impl Policy for TimeOverrunWatcher {
    fn decide(&self, stream: &[Signal], ctx: &Context) -> Vec<Signal> {
        // Find the most recent PlanPhase signal.
        let Some(signal) = stream.iter().rev().find(|s| s.kind == Kind::PlanPhase) else {
            return Vec::new();
        };

        let Some(phase_str) = signal.tag(PHASE_TAG) else {
            return Vec::new();
        };

        let Some(phase) = parse_phase(phase_str) else {
            return Vec::new();
        };

        let Some(entered_ms) = signal
            .tag(PHASE_ENTERED_TAG)
            .and_then(|v| v.parse::<i64>().ok())
        else {
            return Vec::new();
        };

        let complexity = signal
            .tag(COMPLEXITY_TAG)
            .map_or(TaskComplexityBand::Standard, parse_complexity);

        let Some(timeout_secs) = phase_timeout(phase, complexity) else {
            return Vec::new();
        };

        #[allow(clippy::cast_sign_loss)]
        let elapsed_secs = ((ctx.now_ms - entered_ms).max(0) / 1000) as u64;

        if elapsed_secs > timeout_secs {
            vec![
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "phase {phase_str} exceeded timeout: {elapsed_secs}s > {timeout_secs}s"
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("phase", phase_str)
                    .tag("elapsed_secs", elapsed_secs.to_string())
                    .tag("timeout_secs", timeout_secs.to_string())
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

    fn phase_signal(phase: &str, entered_ms: i64) -> Signal {
        Signal::builder(Kind::PlanPhase)
            .body(Body::text("phase update"))
            .tag(PHASE_TAG, phase)
            .tag(PHASE_ENTERED_TAG, entered_ms.to_string())
            .build()
    }

    fn phase_signal_with_complexity(phase: &str, entered_ms: i64, complexity: &str) -> Signal {
        Signal::builder(Kind::PlanPhase)
            .body(Body::text("phase update"))
            .tag(PHASE_TAG, phase)
            .tag(PHASE_ENTERED_TAG, entered_ms.to_string())
            .tag(COMPLEXITY_TAG, complexity)
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = TimeOverrunWatcher::new();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn within_timeout_no_fire() {
        let w = TimeOverrunWatcher::new();
        // Implementing, standard = 300s timeout, elapsed = 100s
        let stream = vec![phase_signal("implementing", 0)];
        let ctx = Context::at(100_000); // 100s later
        assert!(w.decide(&stream, &ctx).is_empty());
    }

    #[test]
    fn exceeds_timeout_fires() {
        let w = TimeOverrunWatcher::new();
        // Implementing, standard = 300s timeout, elapsed = 400s
        let stream = vec![phase_signal("implementing", 0)];
        let ctx = Context::at(400_000); // 400s later
        let out = w.decide(&stream, &ctx);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn complex_implementing_longer_timeout() {
        let w = TimeOverrunWatcher::new();
        // Complex implementing = 600s timeout
        let stream = vec![phase_signal_with_complexity("implementing", 0, "complex")];
        let ctx = Context::at(500_000); // 500s — within 600s timeout
        assert!(w.decide(&stream, &ctx).is_empty());
    }

    #[test]
    fn fast_implementing_shorter_timeout() {
        let w = TimeOverrunWatcher::new();
        // Fast implementing = 120s timeout
        let stream = vec![phase_signal_with_complexity("implementing", 0, "fast")];
        let ctx = Context::at(150_000); // 150s > 120s timeout
        let out = w.decide(&stream, &ctx);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn terminal_phase_no_timeout() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![phase_signal("complete", 0)];
        let ctx = Context::at(999_999_000);
        assert!(w.decide(&stream, &ctx).is_empty());
    }

    #[test]
    fn missing_phase_tag_no_fire() {
        let w = TimeOverrunWatcher::new();
        let stream = vec![
            Signal::builder(Kind::PlanPhase)
                .body(Body::text("no tags"))
                .build(),
        ];
        assert!(w.decide(&stream, &Context::at(999_000)).is_empty());
    }

    #[test]
    fn merging_timeout_short() {
        let w = TimeOverrunWatcher::new();
        // Merging = 60s timeout
        let stream = vec![phase_signal("merging", 0)];
        let ctx = Context::at(70_000); // 70s > 60s
        let out = w.decide(&stream, &ctx);
        assert_eq!(out.len(), 1);
    }
}
