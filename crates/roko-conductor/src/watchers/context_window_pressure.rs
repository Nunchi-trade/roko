//! Context window pressure watcher: fires when token usage exceeds threshold.
//!
//! Monitors `TokenUsage` signals for context window utilization and fires
//! when usage exceeds [`MAX_CONTEXT_USAGE_RATIO`].

use roko_core::{Body, Context, Kind, Policy, Signal};

/// Maximum context window utilization ratio (0.0 to 1.0) before firing.
pub const MAX_CONTEXT_USAGE_RATIO: f64 = 0.80;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "context-window-pressure";

/// Tag key on token-usage signals for used tokens.
pub const TOKENS_USED_TAG: &str = "tokens_used";
/// Tag key on token-usage signals for total window size.
pub const TOKENS_TOTAL_TAG: &str = "tokens_total";

/// Fires when context window usage exceeds [`MAX_CONTEXT_USAGE_RATIO`].
///
/// Examines the most recent `TokenUsage` signal for
/// `tokens_used` / `tokens_total` ratio. If the ratio exceeds the
/// threshold, fires a warning.
#[derive(Debug, Clone)]
pub struct ContextWindowPressureWatcher {
    /// Maximum ratio before firing.
    max_ratio: f64,
}

impl Default for ContextWindowPressureWatcher {
    fn default() -> Self {
        Self {
            max_ratio: MAX_CONTEXT_USAGE_RATIO,
        }
    }
}

impl ContextWindowPressureWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_ratio: f64) -> Self {
        Self { max_ratio }
    }
}

impl Policy for ContextWindowPressureWatcher {
    fn decide(&self, stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        // Find the most recent TokenUsage signal.
        let latest = stream
            .iter()
            .rev()
            .find(|s| s.kind == Kind::TokenUsage);

        let Some(signal) = latest else {
            return Vec::new();
        };

        let used: f64 = signal
            .tag(TOKENS_USED_TAG)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let total: f64 = signal
            .tag(TOKENS_TOTAL_TAG)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);

        if total <= 0.0 {
            return Vec::new();
        }

        let ratio = used / total;

        if ratio > self.max_ratio {
            vec![Signal::builder(Kind::Custom(
                "conductor.intervention".into(),
            ))
            .body(Body::text(format!(
                "context window {:.0}% full ({used:.0}/{total:.0} tokens)",
                ratio * 100.0
            )))
            .tag("watcher", WATCHER_NAME)
            .tag("severity", "warning")
            .tag("ratio", format!("{ratio:.3}"))
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

    fn token_signal(used: u64, total: u64) -> Signal {
        Signal::builder(Kind::TokenUsage)
            .body(Body::text("usage"))
            .tag(TOKENS_USED_TAG, &used.to_string())
            .tag(TOKENS_TOTAL_TAG, &total.to_string())
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![token_signal(70_000, 100_000)]; // 70%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![token_signal(80_000, 100_000)]; // exactly 80%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_threshold_fires() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![token_signal(85_000, 100_000)]; // 85%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn uses_most_recent_signal() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            token_signal(90_000, 100_000), // 90% — old
            token_signal(50_000, 100_000), // 50% — most recent
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn zero_total_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![token_signal(0, 0)];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn custom_threshold() {
        let w = ContextWindowPressureWatcher::new(0.50);
        let stream = vec![token_signal(60_000, 100_000)]; // 60% > 50%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
