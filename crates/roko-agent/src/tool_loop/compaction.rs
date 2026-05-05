//! Gentle tool-result compaction for long conversations.
//!
//! This keeps the most recent tool-call groups intact while truncating
//! older verbose tool results in place so their `tool_call_id`s remain
//! available to the backend.

use roko_core::defaults::{
    DEFAULT_RECENT_TOOL_GROUPS_TO_KEEP, DEFAULT_TOOL_RESULT_COMPACTION_THRESHOLD_CHARS,
    DEFAULT_TOOL_RESULT_PREVIEW_CHARS,
};
use serde_json::Value;

const RECENT_TOOL_GROUPS_TO_KEEP: usize = DEFAULT_RECENT_TOOL_GROUPS_TO_KEEP;
const TOOL_RESULT_COMPACTION_THRESHOLD_CHARS: usize =
    DEFAULT_TOOL_RESULT_COMPACTION_THRESHOLD_CHARS;
const TOOL_RESULT_PREVIEW_CHARS: usize = DEFAULT_TOOL_RESULT_PREVIEW_CHARS;

/// Truncate verbose tool results outside the most recent tool-call groups.
///
/// Tool result messages are grouped by contiguous `role="tool"` runs. The
/// newest two groups are preserved verbatim. Older groups keep their existing
/// message objects and `tool_call_id`s, but oversized `content` strings are
/// replaced with a short preview and a total character count.
pub fn compact_tool_results(messages: &mut Vec<Value>) {
    let groups = tool_result_groups(messages);
    let compact_until = groups.len().saturating_sub(RECENT_TOOL_GROUPS_TO_KEEP);

    for (start, end) in groups.into_iter().take(compact_until) {
        for message in &mut messages[start..end] {
            compact_tool_message(message);
        }
    }
}

fn tool_result_groups(messages: &[Value]) -> Vec<(usize, usize)> {
    let mut groups = Vec::new();
    let mut current_start = None;

    for (idx, message) in messages.iter().enumerate() {
        if is_tool_message(message) {
            current_start.get_or_insert(idx);
            continue;
        }

        if let Some(start) = current_start.take() {
            groups.push((start, idx));
        }
    }

    if let Some(start) = current_start {
        groups.push((start, messages.len()));
    }

    groups
}

fn is_tool_message(message: &Value) -> bool {
    matches!(message.get("role").and_then(Value::as_str), Some("tool"))
}

fn compact_tool_message(message: &mut Value) {
    let Some(content) = message.get("content").and_then(Value::as_str) else {
        return;
    };

    let total_chars = content.chars().count();
    if total_chars <= TOOL_RESULT_COMPACTION_THRESHOLD_CHARS {
        return;
    }

    let replacement = format!(
        "{}... [truncated, {total_chars} chars total]",
        truncate_chars(content, TOOL_RESULT_PREVIEW_CHARS),
    );
    if let Some(object) = message.as_object_mut() {
        object.insert("content".to_string(), Value::String(replacement));
    }
}

fn truncate_chars(content: &str, limit: usize) -> &str {
    match content.char_indices().nth(limit) {
        Some((idx, _)) => &content[..idx],
        None => content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_tool_call(id: &str) -> Value {
        serde_json::json!({
            "role": "assistant",
            "tool_calls": [{"id": id, "type": "function"}],
        })
    }

    fn tool_message(id: &str, content: &str) -> Value {
        serde_json::json!({
            "role": "tool",
            "tool_call_id": id,
            "content": content,
        })
    }

    #[test]
    fn tool_result_compaction_truncates_only_old_groups() {
        let old_content = "a".repeat(600);
        let recent_content = "b".repeat(600);
        let newest_content = "c".repeat(600);
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
            assistant_tool_call("old"),
            tool_message("old", &old_content),
            assistant_tool_call("recent"),
            tool_message("recent", &recent_content),
            assistant_tool_call("newest"),
            tool_message("newest", &newest_content),
        ];

        compact_tool_results(&mut messages);

        let old_result = messages[3]["content"].as_str().expect("old content");
        assert!(old_result.starts_with(&"a".repeat(200)));
        assert!(old_result.ends_with("[truncated, 600 chars total]"));
        assert_eq!(messages[3]["tool_call_id"], "old");
        assert_eq!(messages[5]["content"], recent_content);
        assert_eq!(messages[7]["content"], newest_content);
    }

    #[test]
    fn tool_result_compaction_preserves_two_most_recent_groups() {
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
            assistant_tool_call("first"),
            tool_message("first", &"x".repeat(700)),
            assistant_tool_call("second"),
            tool_message("second", &"y".repeat(700)),
        ];

        compact_tool_results(&mut messages);

        assert_eq!(messages[3]["content"], "x".repeat(700));
        assert_eq!(messages[5]["content"], "y".repeat(700));
    }

    #[test]
    fn tool_result_compaction_respects_character_boundaries() {
        let unicode = "é".repeat(550);
        let mut messages = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "usr"}),
            assistant_tool_call("old"),
            tool_message("old", &unicode),
            assistant_tool_call("recent"),
            tool_message("recent", "ok"),
            assistant_tool_call("newest"),
            tool_message("newest", "ok"),
        ];

        compact_tool_results(&mut messages);

        let compacted = messages[3]["content"].as_str().expect("compacted content");
        assert_eq!(compacted.chars().take(200).count(), 200);
        assert!(compacted.contains("[truncated, 550 chars total]"));
    }
}
