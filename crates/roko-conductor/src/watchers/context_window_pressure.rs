//! Context window pressure watcher: fires when token usage exceeds threshold.
//!
//! # STATUS: GATED
//!
//! Only active when `conductor.context_pressure_enabled = true` in `roko.toml`.
//! Extended in 2026-05 to read `context_window` from `ModelProfile` config for
//! non-Anthropic models (via a precomputed map passed at construction time).
//! Uses a lookback window (`PRESSURE_LOOKBACK = 3`) over recent `TokenUsage`
//! signals instead of checking only the most recent signal.
//!
//! Emits `conductor.intervention` signals; `orchestrate.rs` must subscribe to
//! react. Enable only after wiring a subscriber in the runner event loop.
//!
//! The watcher requires `Kind::TokenUsage` signals in the conductor's signal
//! stream. These are emitted by the orchestrator after each agent dispatch
//! (via `emit_conductor_signal`). Without those signals, the watcher is inert
//! but harmless.
//!
//! Monitors `TokenUsage` signals derived from agent efficiency events and
//! fires when usage exceeds [`MAX_CONTEXT_USAGE_RATIO`].

use std::collections::HashMap;

use roko_core::{Body, Context, Engram, Kind, React};
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

/// Recent `TokenUsage` signal lookback window size.
///
/// Uses the maximum utilization over this window to prevent noisy
/// alternating high/low utilization from triggering repeatedly.
pub const PRESSURE_LOOKBACK: usize = 3;

/// Fires when context window usage exceeds [`MAX_CONTEXT_USAGE_RATIO`].
///
/// Examines the last [`PRESSURE_LOOKBACK`] `TokenUsage` signals and takes
/// the maximum utilization ratio. If it exceeds the threshold, fires a warning.
///
/// Optionally carries a precomputed map of model slug to context window size
/// (populated from `ModelProfile.context_window` at construction time) so that
/// non-Anthropic models can also be monitored.
#[derive(Debug, Clone)]
pub struct ContextWindowPressureWatcher {
    /// Maximum ratio before firing.
    max_ratio: f64,
    /// Precomputed map: model slug (lowercased) -> context window tokens.
    /// Built from `RokoConfig.models` at conductor construction time.
    configured_windows: HashMap<String, u64>,
}

impl Default for ContextWindowPressureWatcher {
    fn default() -> Self {
        Self {
            max_ratio: MAX_CONTEXT_USAGE_RATIO,
            configured_windows: HashMap::new(),
        }
    }
}

impl ContextWindowPressureWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub fn new(max_ratio: f64) -> Self {
        Self {
            max_ratio,
            configured_windows: HashMap::new(),
        }
    }

    /// Create with a custom threshold and precomputed context window map.
    ///
    /// The map keys should be lowercased model slugs; values are context
    /// window sizes in tokens. Build this from `RokoConfig.models` when
    /// constructing the conductor.
    #[must_use]
    pub fn with_context_windows(max_ratio: f64, windows: HashMap<String, u64>) -> Self {
        Self {
            max_ratio,
            configured_windows: windows,
        }
    }
}

impl React for ContextWindowPressureWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        // Collect the last PRESSURE_LOOKBACK TokenUsage signals and compute
        // the maximum utilization ratio across them. This prevents alternating
        // high/low usage from firing repeatedly -- any recent high-pressure
        // state triggers once.
        let recent: Vec<&Engram> = stream
            .iter()
            .rev()
            .filter(|s| s.kind == Kind::TokenUsage)
            .take(PRESSURE_LOOKBACK)
            .collect();

        if recent.is_empty() {
            return Vec::new();
        }

        let mut max_ratio: f64 = 0.0;
        let mut max_used: f64 = 0.0;
        let mut max_total: f64 = 0.0;

        for signal in &recent {
            let Some((used, total)) = self.extract_usage(signal) else {
                continue;
            };
            if total <= 0.0 {
                continue;
            }
            let ratio = used / total;
            if ratio > max_ratio {
                max_ratio = ratio;
                max_used = used;
                max_total = total;
            }
        }

        if max_total <= 0.0 {
            return Vec::new();
        }

        if max_ratio > self.max_ratio {
            vec![
                Engram::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "context window {:.0}% full ({max_used:.0}/{max_total:.0} tokens)",
                        max_ratio * 100.0
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("ratio", format!("{max_ratio:.3}"))
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

impl ContextWindowPressureWatcher {
    fn extract_usage(&self, signal: &Engram) -> Option<(f64, f64)> {
        if let Ok(event) = signal.body.as_json::<AgentEfficiencyEvent>() {
            if let Some(total) = self.context_window_tokens(&event.model) {
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
            .and_then(|m| self.context_window_tokens(m))
            .map(|total| total as f64)?;

        Some((used, total))
    }

    /// Look up context window size for a model slug.
    ///
    /// Resolution order:
    /// 1. Precomputed map from `ModelProfile.context_window` config entries.
    /// 2. Hardcoded fallback for Anthropic models (opus, sonnet, haiku).
    fn context_window_tokens(&self, model: &str) -> Option<u64> {
        let model_lower = model.to_ascii_lowercase();

        // First: check configured model profiles (precomputed at construction).
        if let Some(&ctx) = self.configured_windows.get(&model_lower) {
            return Some(ctx);
        }

        // Fallback: hardcoded Anthropic models.
        if model_lower.contains("opus") {
            Some(OPUS_CONTEXT_WINDOW_TOKENS)
        } else if model_lower.contains("haiku") || model_lower.contains("sonnet") {
            Some(SMALL_CONTEXT_WINDOW_TOKENS)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::OperatingFrequency;
    use roko_learn::efficiency::AgentEfficiencyEvent;
    use std::collections::HashMap;

    fn efficiency_event_signal(model: &str, prompt_tokens: u64) -> Engram {
        let event = AgentEfficiencyEvent {
            agent_id: "agent-1".into(),
            role: "Implementer".into(),
            backend: "claude".into(),
            model: model.into(),
            plan_id: "plan-1".into(),
            task_id: "task-1".into(),
            attempt_id: format!("{model}:{prompt_tokens}"),
            input_tokens: prompt_tokens,
            output_tokens: 10,
            reasoning_tokens: 0,
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

        Engram::builder(Kind::TokenUsage)
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
    fn lookback_window_max_over_recent() {
        // With PRESSURE_LOOKBACK = 3, the watcher takes the max utilization
        // over the last 3 signals. Even if the most recent is low, a high
        // signal within the window still fires.
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            efficiency_event_signal("claude-opus-4-6", 900_000), // 90% — within lookback
            efficiency_event_signal("claude-sonnet-4-6", 50_000), // 25% — most recent
        ];
        // The 90% opus signal is within the lookback window, so it fires.
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn all_low_no_fire() {
        // All signals in the lookback window are below threshold.
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            efficiency_event_signal("claude-sonnet-4-6", 100_000), // 50%
            efficiency_event_signal("claude-sonnet-4-6", 120_000), // 60%
            efficiency_event_signal("claude-sonnet-4-6", 140_000), // 70%
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn old_signal_outside_lookback_ignored() {
        // The high signal is outside the lookback window (4th from end).
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            efficiency_event_signal("claude-opus-4-6", 900_000), // 90% — outside lookback
            efficiency_event_signal("claude-sonnet-4-6", 50_000), // 25%
            efficiency_event_signal("claude-sonnet-4-6", 50_000), // 25%
            efficiency_event_signal("claude-sonnet-4-6", 50_000), // 25%
        ];
        // Only the last 3 are examined; the 90% opus signal is outside.
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn zero_total_no_fire() {
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            Engram::builder(Kind::TokenUsage)
                .body(Body::text("usage"))
                .tag(TOKENS_USED_TAG, "0")
                .tag(MODEL_TAG, "mystery-model")
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

    #[test]
    fn configured_windows_overrides_hardcoded() {
        // A model not in the hardcoded list but with a configured window.
        let mut windows = HashMap::new();
        windows.insert("gemini-2.5-flash".to_string(), 100_000);
        let w = ContextWindowPressureWatcher::with_context_windows(0.80, windows);

        // 85% of 100k = 85_000 used -- above threshold
        let stream = vec![
            Engram::builder(Kind::TokenUsage)
                .body(Body::text("usage"))
                .tag(TOKENS_USED_TAG, "85000")
                .tag(MODEL_TAG, "gemini-2.5-flash")
                .build(),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn unknown_model_no_configured_window_no_fire() {
        // A model not in hardcoded or configured list returns None for total.
        let w = ContextWindowPressureWatcher::default();
        let stream = vec![
            Engram::builder(Kind::TokenUsage)
                .body(Body::text("usage"))
                .tag(TOKENS_USED_TAG, "50000")
                .tag(MODEL_TAG, "mystery-model-v99")
                .build(),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }
}
