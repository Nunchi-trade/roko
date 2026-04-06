//! Ghost turn watcher: detects agent turns with zero meaningful output.
//!
//! An agent that produces empty or whitespace-only output is burning tokens
//! without progress. After [`MAX_GHOST_TURNS`] consecutive ghost turns,
//! this watcher fires a restart signal.

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Maximum consecutive empty/whitespace-only agent outputs before firing.
pub const MAX_GHOST_TURNS: usize = 3;

/// Tag key used to mark an intervention signal from this watcher.
pub const WATCHER_NAME: &str = "ghost-turn";

/// Detects agent turns that produce no meaningful output.
///
/// Scans the signal stream for consecutive `AgentOutput` signals whose
/// body is empty or whitespace-only. Fires when the count reaches
/// [`MAX_GHOST_TURNS`].
#[derive(Debug, Clone)]
pub struct GhostTurnWatcher {
    /// Consecutive ghost turns before firing.
    max_ghost_turns: usize,
}

impl Default for GhostTurnWatcher {
    fn default() -> Self {
        Self {
            max_ghost_turns: MAX_GHOST_TURNS,
        }
    }
}

impl GhostTurnWatcher {
    /// Create a watcher with a custom threshold.
    #[must_use]
    pub const fn new(max_ghost_turns: usize) -> Self {
        Self { max_ghost_turns }
    }
}

/// Check if a signal body represents an empty/whitespace-only output.
fn is_ghost(signal: &Signal) -> bool {
    match &signal.body {
        Body::Empty => true,
        Body::Text(s) => s.trim().is_empty(),
        Body::Json(v) => {
            // A JSON string that's empty or whitespace
            v.as_str().is_some_and(|s| s.trim().is_empty())
        }
        Body::Bytes(b) => b.is_empty(),
    }
}

impl Policy for GhostTurnWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        // Count consecutive ghost turns from the end of the stream.
        let consecutive = stream
            .iter()
            .rev()
            .take_while(|s| s.kind == Kind::AgentOutput && is_ghost(s))
            .count();

        if consecutive >= self.max_ghost_turns {
            vec![Signal::builder(Kind::Custom("conductor.intervention".into()))
                .body(Body::text(format!(
                    "{consecutive} consecutive ghost turns detected"
                )))
                .tag("watcher", WATCHER_NAME)
                .tag("severity", "warning")
                .tag("consecutive", consecutive.to_string())
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

    fn ghost_signal() -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::text("   "))
            .build()
    }

    fn empty_signal() -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::empty())
            .build()
    }

    fn real_signal() -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::text("impl Foo { ... }"))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = GhostTurnWatcher::default();
        let out = w.decide(&[], &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn real_output_no_fire() {
        let w = GhostTurnWatcher::default();
        let stream = vec![real_signal(), real_signal()];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = GhostTurnWatcher::default();
        let stream = vec![ghost_signal(), ghost_signal()]; // 2 < 3
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn at_threshold_fires() {
        let w = GhostTurnWatcher::default();
        let stream = vec![ghost_signal(), ghost_signal(), ghost_signal()];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn empty_body_counts_as_ghost() {
        let w = GhostTurnWatcher::new(2);
        let stream = vec![empty_signal(), empty_signal()];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn real_output_breaks_consecutive_chain() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_signal(),
            ghost_signal(),
            ghost_signal(),
            real_signal(), // breaks the chain
            ghost_signal(),
            ghost_signal(),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty()); // only 2 consecutive at the end
    }

    #[test]
    fn non_agent_output_breaks_chain() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_signal(),
            ghost_signal(),
            Signal::builder(Kind::GateVerdict).body(Body::empty()).build(),
            ghost_signal(),
            ghost_signal(),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty()); // gate verdict breaks the chain
    }
}
