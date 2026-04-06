//! Ollama constrained decoding — format injection and malformed-JSON tracking.
//!
//! Ollama supports a `format` parameter in its `/api/chat` request that
//! constrains the model's output to valid JSON matching a given schema.
//! This module provides:
//!
//! - [`OllamaFormatConfig`] — configuration for constrained decoding
//! - [`enforce_format`] — injects the `format` parameter into the request JSON
//! - [`MalformedJsonTracker`] — per-model malformed response counter that
//!   triggers bandit demotion after repeated failures
//!
//! # M21 / M22 compliance
//!
//! - **M21**: When tools are present, streaming must be disabled (`stream: false`)
//!   because Ollama drops tool calls in streaming mode (issues #9632, #12557).
//! - **M22**: The JSON schema is passed via the `format` parameter so Ollama's
//!   constrained decoding engine guarantees structurally valid output.

use std::collections::HashMap;

/// Configuration for Ollama constrained decoding.
///
/// When a schema is provided, it is injected into the `format` field of
/// the Ollama chat request. When tools are present in the request,
/// `stream_override` forces `stream: false` (M21).
#[derive(Debug, Clone, Default)]
pub struct OllamaFormatConfig {
    /// Optional JSON schema that constrains the model's output structure.
    /// Passed directly to Ollama's `format` parameter.
    pub schema: Option<serde_json::Value>,
    /// When true, override `stream` to `false` in the request.
    /// Should be set when tools are present (M21 compliance).
    pub stream_override: bool,
}

impl OllamaFormatConfig {
    /// Create a config with no schema and no stream override.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the JSON schema for constrained decoding.
    #[must_use]
    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = Some(schema);
        self
    }

    /// Enable stream override (force `stream: false`).
    #[must_use]
    pub const fn with_stream_override(mut self, override_stream: bool) -> Self {
        self.stream_override = override_stream;
        self
    }

    /// Returns true if this config has a schema to inject.
    #[must_use]
    pub const fn has_schema(&self) -> bool {
        self.schema.is_some()
    }
}

/// Inject the `format` parameter and enforce `stream: false` into an Ollama
/// API request body (as a mutable JSON value).
///
/// # Behavior
///
/// - If `config.schema` is `Some`, sets `request["format"]` to the schema value.
/// - If `config.stream_override` is true, forces `request["stream"] = false`.
/// - If tools are detected in the request (a non-empty `tools` array), also
///   forces `stream: false` regardless of `stream_override`.
///
/// # Panics
///
/// Does not panic. If `request` is not a JSON object, this is a no-op.
pub fn enforce_format(request: &mut serde_json::Value, config: &OllamaFormatConfig) {
    let Some(obj) = request.as_object_mut() else {
        return;
    };

    // M22: inject schema into the format parameter
    if let Some(schema) = &config.schema {
        obj.insert("format".to_string(), schema.clone());
    }

    // M21: force stream=false when tools are present
    let has_tools = obj
        .get("tools")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if config.stream_override || has_tools {
        obj.insert(
            "stream".to_string(),
            serde_json::Value::Bool(false),
        );
    }
}

/// Threshold of consecutive malformed JSON responses before a model should
/// be demoted in the bandit.
const DEMOTION_THRESHOLD: u32 = 3;

/// Tracks consecutive malformed JSON responses per model slug.
///
/// When a model produces `DEMOTION_THRESHOLD` (3) consecutive malformed
/// responses, [`MalformedJsonTracker::should_demote`] returns true, which
/// the caller should use to trigger bandit demotion to the next format in
/// the fallback chain.
#[derive(Debug, Default)]
pub struct MalformedJsonTracker {
    counts: HashMap<String, u32>,
}

impl MalformedJsonTracker {
    /// Create a new empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a malformed JSON response for `model`. Increments the
    /// consecutive failure counter.
    pub fn record_malformed(&mut self, model: &str) {
        let counter = self.counts.entry(model.to_string()).or_insert(0);
        *counter += 1;
    }

    /// Returns true if `model` has produced >= 3 consecutive malformed
    /// responses — the caller should demote this model in the bandit.
    #[must_use]
    pub fn should_demote(&self, model: &str) -> bool {
        self.counts
            .get(model)
            .is_some_and(|&c| c >= DEMOTION_THRESHOLD)
    }

    /// Reset the malformed counter for `model` after a successful parse.
    pub fn reset(&mut self, model: &str) {
        self.counts.remove(model);
    }

    /// Current consecutive malformed count for a model (0 if not tracked).
    #[must_use]
    pub fn count(&self, model: &str) -> u32 {
        self.counts.get(model).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── OllamaFormatConfig tests ─────────────────────────────────────

    #[test]
    fn config_default_has_no_schema() {
        let cfg = OllamaFormatConfig::new();
        assert!(!cfg.has_schema());
        assert!(!cfg.stream_override);
    }

    #[test]
    fn config_with_schema_reports_has_schema() {
        let cfg = OllamaFormatConfig::new()
            .with_schema(json!({"type": "object"}));
        assert!(cfg.has_schema());
    }

    #[test]
    fn config_builder_chains() {
        let cfg = OllamaFormatConfig::new()
            .with_schema(json!({"type": "string"}))
            .with_stream_override(true);
        assert!(cfg.has_schema());
        assert!(cfg.stream_override);
    }

    // ── enforce_format tests ─────────────────────────────────────────

    #[test]
    fn enforce_format_injects_schema() {
        let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
        let config = OllamaFormatConfig::new().with_schema(schema.clone());

        let mut request = json!({
            "model": "llama3.1:8b",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        });

        enforce_format(&mut request, &config);
        assert_eq!(request["format"], schema);
    }

    #[test]
    fn enforce_format_forces_stream_false_when_tools_present() {
        let config = OllamaFormatConfig::new();

        let mut request = json!({
            "model": "llama3.1:8b",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "get_weather"}}]
        });

        enforce_format(&mut request, &config);
        assert_eq!(request["stream"], json!(false));
    }

    #[test]
    fn enforce_format_forces_stream_false_with_stream_override() {
        let config = OllamaFormatConfig::new().with_stream_override(true);

        let mut request = json!({
            "model": "llama3.1:8b",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        });

        enforce_format(&mut request, &config);
        assert_eq!(request["stream"], json!(false));
    }

    #[test]
    fn enforce_format_no_op_without_tools_or_override() {
        let config = OllamaFormatConfig::new();

        let mut request = json!({
            "model": "llama3.1:8b",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        });

        enforce_format(&mut request, &config);
        // stream should remain true — no tools, no override
        assert_eq!(request["stream"], json!(true));
        // format should not be set
        assert!(request.get("format").is_none());
    }

    #[test]
    fn enforce_format_empty_tools_array_does_not_force_stream() {
        let config = OllamaFormatConfig::new();

        let mut request = json!({
            "model": "llama3.1:8b",
            "stream": true,
            "messages": [],
            "tools": []
        });

        enforce_format(&mut request, &config);
        assert_eq!(request["stream"], json!(true));
    }

    #[test]
    fn enforce_format_schema_plus_tools_both_applied() {
        let schema = json!({"type": "object"});
        let config = OllamaFormatConfig::new().with_schema(schema.clone());

        let mut request = json!({
            "model": "qwen3-32b",
            "stream": true,
            "messages": [{"role": "user", "content": "call tool"}],
            "tools": [{"type": "function", "function": {"name": "read_file"}}]
        });

        enforce_format(&mut request, &config);
        assert_eq!(request["format"], schema);
        assert_eq!(request["stream"], json!(false));
    }

    #[test]
    fn enforce_format_non_object_is_noop() {
        let config = OllamaFormatConfig::new()
            .with_schema(json!("string"))
            .with_stream_override(true);

        let mut request = json!("not an object");
        enforce_format(&mut request, &config);
        assert_eq!(request, json!("not an object"));
    }

    // ── MalformedJsonTracker tests ───────────────────────────────────

    #[test]
    fn tracker_starts_at_zero() {
        let tracker = MalformedJsonTracker::new();
        assert_eq!(tracker.count("llama3.1:8b"), 0);
        assert!(!tracker.should_demote("llama3.1:8b"));
    }

    #[test]
    fn tracker_increments_on_record() {
        let mut tracker = MalformedJsonTracker::new();
        tracker.record_malformed("qwen3-32b");
        assert_eq!(tracker.count("qwen3-32b"), 1);
        assert!(!tracker.should_demote("qwen3-32b"));
    }

    #[test]
    fn tracker_demotes_after_three_failures() {
        let mut tracker = MalformedJsonTracker::new();
        let model = "mistral-7b";
        tracker.record_malformed(model);
        tracker.record_malformed(model);
        assert!(!tracker.should_demote(model));
        tracker.record_malformed(model);
        assert!(tracker.should_demote(model));
    }

    #[test]
    fn tracker_reset_clears_count() {
        let mut tracker = MalformedJsonTracker::new();
        let model = "llama3.1:8b";
        tracker.record_malformed(model);
        tracker.record_malformed(model);
        tracker.record_malformed(model);
        assert!(tracker.should_demote(model));
        tracker.reset(model);
        assert!(!tracker.should_demote(model));
        assert_eq!(tracker.count(model), 0);
    }

    #[test]
    fn tracker_independent_per_model() {
        let mut tracker = MalformedJsonTracker::new();
        tracker.record_malformed("model-a");
        tracker.record_malformed("model-a");
        tracker.record_malformed("model-a");
        tracker.record_malformed("model-b");

        assert!(tracker.should_demote("model-a"));
        assert!(!tracker.should_demote("model-b"));
        assert_eq!(tracker.count("model-b"), 1);
    }

    #[test]
    fn tracker_demotes_beyond_threshold() {
        let mut tracker = MalformedJsonTracker::new();
        let model = "phi-4";
        for _ in 0..5 {
            tracker.record_malformed(model);
        }
        assert!(tracker.should_demote(model));
        assert_eq!(tracker.count(model), 5);
    }

    #[test]
    fn tracker_reset_unknown_model_is_noop() {
        let mut tracker = MalformedJsonTracker::new();
        tracker.reset("never-seen");
        assert_eq!(tracker.count("never-seen"), 0);
    }
}
