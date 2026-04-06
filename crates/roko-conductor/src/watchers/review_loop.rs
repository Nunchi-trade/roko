//! Review loop watcher: detects review cycling.
//!
//! When the same review feedback appears [`MAX_REVIEW_CYCLES`] times,
//! the reviewer and implementer are looping without convergence. This
//! watcher fires a warning so the conductor can restart or escalate.

use roko_core::{Body, Context, Kind, Policy, Signal};
use std::collections::HashMap;

/// Maximum times the same review feedback can appear before firing.
pub const MAX_REVIEW_CYCLES: usize = 3;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "review-loop";

/// Tag value identifying review feedback signals in the stream.
pub const REVIEW_TAG_KEY: &str = "role";
/// Tag value identifying review signals.
pub const REVIEW_TAG_VALUE: &str = "reviewer";

/// Detects review cycling: same feedback repeated N times.
///
/// Scans all `AgentMessage` signals tagged `role=reviewer` and counts
/// duplicate body text. Fires when any single feedback text appears
/// [`MAX_REVIEW_CYCLES`] times.
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

/// Extract a comparable key from a review signal's body.
fn review_body_key(signal: &Signal) -> Option<String> {
    match &signal.body {
        Body::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        _ => None,
    }
}

impl Policy for ReviewLoopWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        let mut counts: HashMap<String, usize> = HashMap::new();

        for s in stream {
            // Only consider review signals.
            let is_review = s.kind == Kind::AgentMessage
                && s.tag(REVIEW_TAG_KEY) == Some(REVIEW_TAG_VALUE);
            if !is_review {
                continue;
            }
            if let Some(key) = review_body_key(s) {
                *counts.entry(key).or_insert(0) += 1;
            }
        }

        // Find the most repeated feedback.
        if let Some((feedback, count)) = counts.iter().max_by_key(|(_, c)| **c) {
            if *count >= self.max_cycles {
                return vec![Signal::builder(Kind::Custom(
                    "conductor.intervention".into(),
                ))
                .body(Body::text(format!(
                    "review feedback repeated {count} times: {}",
                    truncate(feedback, 80)
                )))
                .tag("watcher", WATCHER_NAME)
                .tag("severity", "warning")
                .tag("count", count.to_string())
                .build()];
            }
        }

        Vec::new()
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
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

    fn review_signal(text: &str) -> Signal {
        Signal::builder(Kind::AgentMessage)
            .body(Body::text(text))
            .tag(REVIEW_TAG_KEY, REVIEW_TAG_VALUE)
            .build()
    }

    fn non_review_signal(text: &str) -> Signal {
        Signal::builder(Kind::AgentMessage)
            .body(Body::text(text))
            .tag("role", "implementer")
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
            review_signal("fix the error handling"),
            review_signal("add tests for edge cases"),
            review_signal("rename the variable"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn repeated_reviews_fires() {
        let w = ReviewLoopWatcher::default();
        let stream = vec![
            review_signal("fix the error handling"),
            review_signal("fix the error handling"),
            review_signal("fix the error handling"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn non_review_signals_ignored() {
        let w = ReviewLoopWatcher::default();
        let stream = vec![
            non_review_signal("same text"),
            non_review_signal("same text"),
            non_review_signal("same text"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = ReviewLoopWatcher::new(3);
        let stream = vec![
            review_signal("same"),
            review_signal("same"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn whitespace_trimmed() {
        let w = ReviewLoopWatcher::new(2);
        let stream = vec![
            review_signal("  fix it  "),
            review_signal("fix it"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn interleaved_reviews_still_count() {
        let w = ReviewLoopWatcher::new(3);
        let stream = vec![
            review_signal("fix A"),
            review_signal("fix B"),
            review_signal("fix A"),
            review_signal("fix B"),
            review_signal("fix A"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        // "fix A" appeared 3 times
    }
}
