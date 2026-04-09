//! Context window pressure watcher: fires when token usage exceeds threshold.
//!
//! Monitors `TokenUsage` signals derived from agent efficiency events and
//! fires when usage exceeds [`MAX_CONTEXT_USAGE_RATIO`].

use roko_core::{Body, Context, Kind, Policy, Signal};
use roko_learn::efficiency::AgentEfficiencyEvent;

/// Maximum context window utilization ratio (0.0 to 1.0) before firing.
pub const MAX_CONTEXT_USAGE_RATIO: f64 = 0.80;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "context-window-pressure";

/// Tag key on token-usage signals for used tokens.
pub const TOKENS_USED_TAG: &str = "tokens_used";
/// Tag key on token-usage signals for total window size.
pub const TOKENS_TOTAL_TAG: &str = "tokens_total";
/// Tag key on token-usage signals for the model slug.
pub const MODEL_TAG: &str = "model";

const SMALL_CONTEXT_WINDOW_TOKENS: u64 = 200_000;
const OPUS_CONTEXT_WINDOW_TOKENS: u64 = 1_000_000;

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
        let latest = stream.iter().rev().find(|s| s.kind == Kind::TokenUsage);

        let Some(signal) = latest else {
            return Vec::new();
        };

        let Some((used, total)) = extract_usage(signal) else {
            return Vec::new();
        };

        if total <= 0.0 {
            return Vec::new();
        }

        let ratio = used / total;

        if ratio > self.max_ratio {
            vec![
                Signal::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "context window {:.0}% full ({used:.0}/{total:.0} tokens)",
                        ratio * 100.0
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("ratio", format!("{ratio:.3}"))
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

fn extract_usage(signal: &Signal) -> Option<(f64, f64)> {
    if let Ok(event) = signal.body.as_json::<AgentEfficiencyEvent>() {
        if let Some(total) = context_window_tokens(&event.model) {
            return Some((event.total_prompt_tokens as f64, total as f64));
        }
    }

    let used = signal.tag(TOKENS_USED_TAG)?.parse().ok()?;
    if let Some(total) = signal
        .tag(TOKENS_TOTAL_TAG)
        .and_then(|v| v.parse::<f64>().ok())
    {
        return Some((used, total));
    }

    let total = signal
        .tag(MODEL_TAG)
        .and_then(context_window_tokens)
        .map(|total| total as f64)?;

    Some((used, total))
}

fn context_window_tokens(model: &str) -> Option<u64> {
    let model = model.to_ascii_lowercase();
    if model.contains("opus") {
        Some(OPUS_CONTEXT_WINDOW_TOKENS)
    } else if model.contains("haiku") || model.contains("sonnet") {
        Some(SMALL_CONTEXT_WINDOW_TOKENS)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::OperatingFrequency;
    use roko_learn::efficiency::AgentEfficiencyEvent;

    fn efficiency_event_signal(model: &str, prompt_tokens: u64) -> Signal {
        let event = AgentEfficiencyEvent {
            agent_id: "agent-1".into(),
            role: "Implementer".into(),
            backend: "claude".into(),
            model: model.into(),
            plan_id: "plan-1".into(),
            task_id: "task-1".into(),
            input_tokens: prompt_tokens,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.5,
            cost_usd_without_cache: 0.5,
            prompt_sections: Vec::new(),
            total_prompt_tokens: prompt_tokens,
            system_prompt_tokens: 100,
            tools_available: 8,
            tools_used: 2,
            tool_calls: Vec::new(),
            wall_time_ms: 1_000,
            duration_ms: 1_000,
            time_to_first_token_ms: 100,
            was_warm_start: false,
            iteration: 1,
            gate_passed: true,
            outcome: "success".into(),
            gate_errors: Vec::new(),
            model_used: model.into(),
            frequency: OperatingFrequency::Theta,
            strategy_attempted: "none".into(),
            timestamp: "2026-04-09T00:00:00Z".into(),
        };

        Signal::builder(Kind::TokenUsage)
            .body(Body::from_json(&event).expect("serialize event"))
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
        let stream = vec![efficiency_event_signal("claude-sonnet-4-6", 150_000)]; // 75%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn at_threshold_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![efficiency_event_signal("claude-haiku-4-5", 160_000)]; // exactly 80%
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn above_threshold_fires() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![efficiency_event_signal("claude-opus-4-6", 850_000)]; // 85%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn uses_most_recent_signal() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            efficiency_event_signal("claude-opus-4-6", 900_000), // 90% — old
            efficiency_event_signal("claude-sonnet-4-6", 50_000), // 25% — most recent
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn zero_total_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            Signal::builder(Kind::TokenUsage)
                .body(Body::text("usage"))
                .tag(TOKENS_USED_TAG, "0")
                .tag(MODEL_TAG, "unknown-model")
                .build(),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn custom_threshold() {
        let w = ContextWindowPressureWatcher::new(0.50);
        let stream = vec![efficiency_event_signal("claude-sonnet-4-6", 120_000)]; // 60% > 50%
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
