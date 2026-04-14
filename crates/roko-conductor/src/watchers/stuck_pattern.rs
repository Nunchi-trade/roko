//! Stuck pattern watcher: detects repeated identical actions without progress.
//!
//! When the same agent action (tool call, message, etc.) appears
//! [`MAX_IDENTICAL_ACTIONS`] consecutive times, the agent is stuck in a
//! loop. This watcher fires a warning to trigger restart.

use roko_core::{Body, Context, Engram, Kind, Policy};

/// Maximum consecutive identical actions before firing.
pub const MAX_IDENTICAL_ACTIONS: usize = 4;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "stuck-pattern";

/// Kinds considered as "actions" for stuck detection.
const ACTION_KINDS: &[Kind] = &[Kind::AgentOutput, Kind::AgentMessage];

/// Detects repeated identical actions without progress.
///
/// Scans the signal stream for consecutive `AgentOutput` or `AgentMessage`
/// signals with identical body text. Fires when the count reaches
/// [`MAX_IDENTICAL_ACTIONS`].
#[derive(Debug, Clone)]
pub struct StuckPatternWatcher {
    /// Consecutive identical actions before firing.
    max_actions: usize,
}

impl Default for StuckPatternWatcher {
    fn default() -> Self {
        Self {
            max_actions: MAX_IDENTICAL_ACTIONS,
        }
    }
}

impl StuckPatternWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_actions: usize) -> Self {
        Self { max_actions }
    }
}

/// Check if a signal's kind is an "action" kind.
fn is_action(signal: &Engram) -> bool {
    ACTION_KINDS.contains(&signal.kind)
}

/// Extract a body fingerprint for comparison.
fn body_fingerprint(signal: &Engram) -> Option<String> {
    match &signal.body {
        Body::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Body::Json(v) => Some(v.to_string()),
        Body::Empty => None,
        Body::Bytes(b) => {
            if b.is_empty() {
                None
            } else {
                // Use length + first few bytes as fingerprint.
                Some(format!("bytes:{}/{:?}", b.len(), &b[..b.len().min(16)]))
            }
        }
    }
}

impl Policy for StuckPatternWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        // Walk backwards through action signals, counting consecutive identical ones.
        let mut consecutive = 0usize;
        let mut last_fingerprint: Option<String> = None;

        for signal in stream.iter().rev() {
            if !is_action(signal) {
                continue;
            }
            let fp = body_fingerprint(signal);
            match (&last_fingerprint, &fp) {
                (None, Some(_)) => {
                    last_fingerprint = fp;
                    consecutive = 1;
                }
                (Some(prev), Some(curr)) if prev == curr => {
                    consecutive += 1;
                }
                (Some(_), Some(_)) => {
                    // Different action — stop counting.
                    break;
                }
                _ => {} // Skip empty fingerprints.
            }
        }

        if consecutive >= self.max_actions {
            let desc = last_fingerprint.unwrap_or_default();
            vec![
                Engram::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "{consecutive} consecutive identical actions: {}",
                        truncate(&desc, 80)
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("consecutive", consecutive.to_string())
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

/// Truncate a string for display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let mut t = s[..max_len].to_owned();
        t.push_str("...");
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_signal(text: &str) -> Engram {
        Engram::builder(Kind::AgentOutput)
            .body(Body::text(text))
            .build()
    }

    fn message_signal(text: &str) -> Engram {
        Engram::builder(Kind::AgentMessage)
            .body(Body::text(text))
            .build()
    }

    fn gate_signal() -> Engram {
        Engram::builder(Kind::GateVerdict)
            .body(Body::text("pass"))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = StuckPatternWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn varied_actions_no_fire() {
        let w = StuckPatternWatcher::default();
        let stream = vec![
            action_signal("edit file A"),
            action_signal("edit file B"),
            action_signal("run tests"),
            action_signal("check output"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = StuckPatternWatcher::default();
        let stream = vec![
            action_signal("same action"),
            action_signal("same action"),
            action_signal("same action"),
        ]; // 3 < 4
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_fires() {
        let w = StuckPatternWatcher::default();
        let stream = vec![
            action_signal("same action"),
            action_signal("same action"),
            action_signal("same action"),
            action_signal("same action"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn different_action_kinds_both_counted() {
        let w = StuckPatternWatcher::new(3);
        let stream = vec![
            action_signal("same action"),
            message_signal("same action"),
            action_signal("same action"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn non_action_signals_skipped_in_chain() {
        let w = StuckPatternWatcher::new(3);
        let stream = vec![
            action_signal("same action"),
            gate_signal(), // Not an action — skipped
            action_signal("same action"),
            action_signal("same action"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn custom_threshold() {
        let w = StuckPatternWatcher::new(2);
        let stream = vec![action_signal("stuck"), action_signal("stuck")];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
