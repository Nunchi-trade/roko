//! Conversation-history compaction with anchored iterative summarization.
//!
//! Older messages are compressed into a single summary message while:
//! - preserving configured anchor roles verbatim
//! - preserving tool-result errors verbatim
//! - carrying forward gate results and tool outcomes as structured JSON
//!
//! The compaction output is iterative: previously compacted summaries can be
//! compacted again without losing their structured `gate_results` or
//! `tool_outcomes`.

use roko_agent::Agent;
use roko_core::{Body, Context, Signal, Kind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tracing::warn;

/// Minimal conversation message shape used by compaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Conversation role (`system`, `user`, `assistant`, `tool`, ...).
    pub role: String,
    /// Human-readable content for the message.
    pub content: String,
    /// Optional structured payload preserved across compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ChatMessage {
    /// Construct a message with no structured payload.
    #[must_use]
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            data: None,
        }
    }

    /// Attach structured JSON data.
    #[must_use]
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// React controlling when and how conversation history is compacted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompactionPolicy {
    /// Compact when the compactable region occupies at least this fraction of
    /// the current context footprint.
    pub trigger_threshold: f64,
    /// Roles that are always preserved verbatim.
    pub anchor_roles: Vec<String>,
    /// Number of most-recent messages to preserve verbatim.
    pub preserve_last_n_turns: usize,
    /// Maximum token budget for the textual summary.
    pub summary_budget_tokens: usize,
}

/// Compact a conversation history into anchors + summary + recent turns.
///
/// The summarizer is used only for the textual digest. Structured gate results
/// and tool outcomes are extracted directly from the compacted region and
/// embedded in the summary payload so later compactions do not lose them.
#[must_use]
pub async fn compact_history(
    messages: &[ChatMessage],
    policy: &CompactionPolicy,
    summarizer: &dyn Agent,
) -> Vec<ChatMessage> {
    if messages.is_empty() {
        return Vec::new();
    }

    let recent_start = messages.len().saturating_sub(policy.preserve_last_n_turns);
    if recent_start == 0 {
        return messages.to_vec();
    }

    let anchor_roles: HashSet<&str> = policy.anchor_roles.iter().map(String::as_str).collect();

    let total_tokens = estimate_tokens(messages);
    let compactable_tokens = messages[..recent_start]
        .iter()
        .filter(|message| !is_anchor(message, &anchor_roles))
        .map(estimate_message_tokens)
        .sum::<usize>();

    if total_tokens == 0 || compactable_tokens == 0 {
        return messages.to_vec();
    }

    let compactable_fraction = compactable_tokens as f64 / total_tokens as f64;
    if compactable_fraction < policy.trigger_threshold.clamp(0.0, 1.0) {
        return messages.to_vec();
    }

    let anchors = messages[..recent_start]
        .iter()
        .filter(|message| is_anchor(message, &anchor_roles))
        .cloned()
        .collect::<Vec<_>>();
    let compactable = messages[..recent_start]
        .iter()
        .filter(|message| !is_anchor(message, &anchor_roles))
        .cloned()
        .collect::<Vec<_>>();

    if compactable.is_empty() {
        return messages.to_vec();
    }

    let gate_results = collect_gate_results(&compactable);
    let tool_outcomes = collect_tool_outcomes(&compactable);
    let summary_text =
        summarize_region(&compactable, policy.summary_budget_tokens, summarizer).await;

    let summary = ChatMessage::new("assistant", summary_text).with_data(json!({
        "kind": "history_summary",
        "compacted_messages": compactable.len(),
        "gate_results": gate_results,
        "tool_outcomes": tool_outcomes,
    }));

    let mut result = Vec::with_capacity(anchors.len() + 1 + messages.len() - recent_start);
    result.extend(anchors);
    result.push(summary);
    result.extend_from_slice(&messages[recent_start..]);
    result
}

fn is_anchor(message: &ChatMessage, anchor_roles: &HashSet<&str>) -> bool {
    anchor_roles.contains(message.role.as_str()) || is_error_tool_result(message)
}

fn is_error_tool_result(message: &ChatMessage) -> bool {
    if message.role != "tool" {
        return false;
    }

    message.data.as_ref().is_some_and(value_indicates_error)
        || contains_error_text(&message.content)
}

fn value_indicates_error(value: &Value) -> bool {
    value.get("error").and_then(Value::as_bool).unwrap_or(false)
        || value
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| matches!(status, "error" | "failed" | "failure"))
}

fn contains_error_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("error") || lower.contains("failed") || lower.contains("exception")
}

fn collect_gate_results(messages: &[ChatMessage]) -> Vec<Value> {
    collect_structured_values(messages, |message| {
        let mut results = Vec::new();
        if let Some(data) = &message.data {
            extend_values_from_key(data, "gate_results", &mut results);
            push_object_value(data, "gate_result", &mut results);
            if looks_like_gate_result(data) {
                results.push(data.clone());
            }
        }
        results
    })
}

fn collect_tool_outcomes(messages: &[ChatMessage]) -> Vec<Value> {
    collect_structured_values(messages, |message| {
        let mut results = Vec::new();
        if message.role == "tool" {
            results.push(json!({
                "role": message.role,
                "content": message.content,
                "data": message.data.clone(),
            }));
        }
        if let Some(data) = &message.data {
            extend_values_from_key(data, "tool_outcomes", &mut results);
            push_object_value(data, "tool_outcome", &mut results);
            if data
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "history_summary")
            {
                extend_values_from_key(data, "tool_outcomes", &mut results);
            }
        }
        results
    })
}

fn collect_structured_values<F>(messages: &[ChatMessage], extractor: F) -> Vec<Value>
where
    F: Fn(&ChatMessage) -> Vec<Value>,
{
    let mut seen = HashSet::new();
    let mut collected = Vec::new();

    for message in messages {
        for value in extractor(message) {
            let Ok(serialized) = serde_json::to_string(&value) else {
                continue;
            };
            if seen.insert(serialized) {
                collected.push(value);
            }
        }
    }

    collected
}

fn extend_values_from_key(value: &Value, key: &str, out: &mut Vec<Value>) {
    if let Some(array) = value.get(key).and_then(Value::as_array) {
        out.extend(array.iter().cloned());
    }
}

fn push_object_value(value: &Value, key: &str, out: &mut Vec<Value>) {
    if let Some(object) = value.get(key) {
        out.push(object.clone());
    }
}

fn looks_like_gate_result(value: &Value) -> bool {
    value.get("gate").and_then(Value::as_str).is_some()
        && value.get("passed").and_then(Value::as_bool).is_some()
}

fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

fn estimate_message_tokens(message: &ChatMessage) -> usize {
    serde_json::to_string(message).map_or(0, |text| text.len() / 4)
}

async fn summarize_region(
    compactable: &[ChatMessage],
    summary_budget_tokens: usize,
    summarizer: &dyn Agent,
) -> String {
    if summary_budget_tokens == 0 {
        return String::new();
    }

    let prompt = build_summary_prompt(compactable, summary_budget_tokens);
    let input = Signal::builder(Kind::Prompt)
        .body(Body::text(prompt))
        .build();
    let result = summarizer.run(&input, &Context::at(0)).await;

    let raw = if result.success {
        extract_summary_text(&result.output).unwrap_or_else(|| fallback_summary(compactable))
    } else {
        warn!("history summarizer failed; falling back to heuristic summary");
        fallback_summary(compactable)
    };

    truncate_to_budget(&raw, summary_budget_tokens)
}

fn build_summary_prompt(compactable: &[ChatMessage], summary_budget_tokens: usize) -> String {
    let mut rendered = String::new();
    for (idx, message) in compactable.iter().enumerate() {
        use std::fmt::Write as _;
        let _ = writeln!(
            rendered,
            "[{idx}] role={} content={}",
            message.role,
            message.content.replace('\n', "\\n")
        );
    }

    format!(
        "Summarize the earlier conversation in <= {summary_budget_tokens} tokens. \
Focus on requirements, decisions, constraints, completed work, and unresolved issues. \
Do not restate raw gate results or tool outputs; those are preserved separately.\n\n{rendered}"
    )
}

fn extract_summary_text(signal: &Signal) -> Option<String> {
    match &signal.body {
        Body::Text(text) => Some(text.clone()),
        Body::Json(value) => value
            .get("summary")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        Body::Empty | Body::Bytes(_) => None,
    }
}

fn fallback_summary(compactable: &[ChatMessage]) -> String {
    let mut lines = compactable
        .iter()
        .filter_map(|message| {
            let trimmed = message.content.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(format!("{}: {}", message.role, trimmed))
            }
        })
        .take(4)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        "Earlier conversation compacted.".to_string()
    } else {
        if compactable.len() > lines.len() {
            lines.push(format!(
                "... plus {} earlier messages.",
                compactable.len() - lines.len()
            ));
        }
        lines.join("\n")
    }
}

fn truncate_to_budget(text: &str, budget_tokens: usize) -> String {
    // Conservative: 3.5 chars/token → multiply by 7/2 (≈3.5) to stay under budget.
    let max_chars = budget_tokens.saturating_mul(7) / 2;
    if text.len() <= max_chars {
        return text.to_string();
    }

    let mut truncated = String::new();
    for ch in text.chars() {
        if truncated.len() + ch.len_utf8() > max_chars.saturating_sub(3) {
            break;
        }
        truncated.push(ch);
    }
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_agent::MockAgent;

    fn gate_message(idx: usize, passed: bool) -> ChatMessage {
        ChatMessage::new("assistant", format!("gate result {idx}"))
            .with_data(json!({"gate_result": {"gate": format!("gate-{idx}"), "passed": passed}}))
    }

    fn message(role: &str, content: impl Into<String>) -> ChatMessage {
        ChatMessage::new(role, content)
    }

    fn collect_preserved_gate_results(messages: &[ChatMessage]) -> Vec<Value> {
        let mut out = Vec::new();
        for message in messages {
            if let Some(data) = &message.data {
                if let Some(gate_result) = data.get("gate_result") {
                    out.push(gate_result.clone());
                }
                if let Some(summary_results) = data.get("gate_results").and_then(Value::as_array) {
                    out.extend(summary_results.iter().cloned());
                }
            }
        }
        out
    }

    #[tokio::test]
    async fn context_compaction_compacts_fifty_messages_to_fifteen_and_preserves_gate_results() {
        let mut messages = vec![message("system", "You are Roko.")];
        let mut expected_gate_results = Vec::new();

        for idx in 0..49 {
            if idx % 5 == 0 {
                let gate = gate_message(idx, idx % 10 != 0);
                expected_gate_results.push(
                    gate.data
                        .as_ref()
                        .and_then(|data| data.get("gate_result"))
                        .cloned()
                        .expect("gate result"),
                );
                messages.push(gate);
            } else {
                let role = if idx % 2 == 0 { "user" } else { "assistant" };
                messages.push(message(role, format!("conversation message {idx}")));
            }
        }

        let compacted = compact_history(
            &messages,
            &CompactionPolicy {
                trigger_threshold: 0.70,
                anchor_roles: vec!["system".into()],
                preserve_last_n_turns: 13,
                summary_budget_tokens: 64,
            },
            &MockAgent::reply("Earlier work covered requirements, edits, and verification state."),
        )
        .await;

        assert_eq!(compacted.len(), 15);
        assert_eq!(compacted[0].role, "system");
        assert_eq!(
            compacted[1]
                .data
                .as_ref()
                .and_then(|data| data.get("kind"))
                .and_then(Value::as_str),
            Some("history_summary")
        );

        let preserved = collect_preserved_gate_results(&compacted);
        assert_eq!(preserved.len(), expected_gate_results.len());
        for gate_result in expected_gate_results {
            assert!(preserved.contains(&gate_result));
        }
    }

    #[tokio::test]
    async fn context_compaction_keeps_error_tool_results_as_anchors() {
        let messages = vec![
            message("system", "System anchor"),
            message("user", "first prompt"),
            ChatMessage::new("tool", "stderr: compile error")
                .with_data(json!({"tool_name": "cargo-check", "error": true})),
            message("assistant", "diagnosis"),
            message("user", "recent prompt"),
            message("assistant", "recent answer"),
        ];

        let compacted = compact_history(
            &messages,
            &CompactionPolicy {
                trigger_threshold: 0.20,
                anchor_roles: vec!["system".into()],
                preserve_last_n_turns: 2,
                summary_budget_tokens: 32,
            },
            &MockAgent::reply("Summary"),
        )
        .await;

        assert_eq!(compacted[0].role, "system");
        assert_eq!(compacted[1].role, "tool");
        assert!(
            compacted[1]
                .data
                .as_ref()
                .and_then(|data| data.get("error"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn context_compaction_returns_original_when_threshold_not_met() {
        let messages = vec![
            message("system", "System"),
            message("user", "u1"),
            message("assistant", "a1"),
            message("user", "u2"),
            message("assistant", "a2"),
        ];

        let compacted = compact_history(
            &messages,
            &CompactionPolicy {
                trigger_threshold: 0.90,
                anchor_roles: vec!["system".into()],
                preserve_last_n_turns: 3,
                summary_budget_tokens: 32,
            },
            &MockAgent::reply("Summary"),
        )
        .await;

        assert_eq!(compacted, messages);
    }
}
