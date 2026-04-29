//! Tool-result message construction and conversation management (§36.56).
//!
//! Builds the initial conversation messages and appends tool results in
//! the shape the backend expects.

use crate::translate::RenderedResults;

/// Build the initial messages for a tool-loop turn: `[system, user]`.
///
/// Downstream code (prune, checkpoint) expects the system prompt at
/// index 0 and the user prompt at index 1.
#[must_use]
pub fn initial_messages(system: &str, user: &str) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "role": "system", "content": system }),
        serde_json::json!({ "role": "user", "content": user }),
    ]
}

/// Build initial messages with few-shot examples inserted before the user turn.
///
/// The ordering is intentionally `[system, examples..., user]` so pruning and
/// checkpoint code can still rely on the system prompt being first while small
/// models get tool-call demonstrations before the actual request.
#[must_use]
pub fn initial_messages_with_few_shot(
    system: &str,
    user: &str,
    few_shot_messages: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut messages = Vec::with_capacity(few_shot_messages.len() + 2);
    messages.push(serde_json::json!({ "role": "system", "content": system }));
    messages.extend(few_shot_messages.iter().cloned());
    messages.push(serde_json::json!({ "role": "user", "content": user }));
    messages
}

/// Append rendered tool results to the conversation message history.
///
/// How the results are appended depends on the [`RenderedResults`]
/// variant:
///
/// - [`JsonMessages`](RenderedResults::JsonMessages) — each element of
///   the JSON array becomes a separate message.
/// - [`TextBlock`](RenderedResults::TextBlock) — a single user-role
///   message wrapping the observation text.
/// - [`HandledByBackend`](RenderedResults::HandledByBackend) — no-op
///   (the backend owns the tool-result loop, e.g. Claude CLI).
pub fn append_results(messages: &mut Vec<serde_json::Value>, rendered: RenderedResults) {
    match rendered {
        RenderedResults::JsonMessages(arr) => {
            if let Some(msgs) = arr.as_array() {
                for msg in msgs {
                    messages.push(msg.clone());
                }
            } else {
                // Single object rather than array — push as-is.
                messages.push(arr);
            }
        }
        RenderedResults::TextBlock(text) => {
            messages.push(serde_json::json!({
                "role": "user",
                "content": text,
            }));
        }
        RenderedResults::HandledByBackend => {
            // Backend drives its own loop; nothing to append.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_messages_has_system_and_user() {
        let msgs = initial_messages("you are a bot", "hello");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "you are a bot");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "hello");
    }

    #[test]
    fn initial_messages_with_few_shot_keeps_system_first_and_user_last() {
        let examples = vec![
            serde_json::json!({"role": "assistant", "content": "I will call a tool."}),
            serde_json::json!({"role": "tool", "tool_call_id": "call-1", "content": "ok"}),
        ];
        let msgs = initial_messages_with_few_shot("sys", "usr", &examples);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "sys");
        assert_eq!(msgs[1]["content"], "I will call a tool.");
        assert_eq!(msgs[2]["tool_call_id"], "call-1");
        assert_eq!(msgs[3]["role"], "user");
        assert_eq!(msgs[3]["content"], "usr");
    }

    #[test]
    fn append_json_messages_adds_each_element() {
        let mut msgs = initial_messages("sys", "usr");
        let rendered = RenderedResults::JsonMessages(serde_json::json!([
            {"role": "tool", "tool_call_id": "c1", "content": "ok"},
            {"role": "tool", "tool_call_id": "c2", "content": "ok2"},
        ]));
        append_results(&mut msgs, rendered);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[2]["tool_call_id"], "c1");
        assert_eq!(msgs[3]["tool_call_id"], "c2");
    }

    #[test]
    fn append_json_single_object_pushed_as_is() {
        let mut msgs = initial_messages("sys", "usr");
        let rendered = RenderedResults::JsonMessages(serde_json::json!(
            {"role": "tool", "tool_call_id": "c1", "content": "ok"}
        ));
        append_results(&mut msgs, rendered);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["tool_call_id"], "c1");
    }

    #[test]
    fn append_text_block_wraps_as_user_message() {
        let mut msgs = initial_messages("sys", "usr");
        let rendered = RenderedResults::TextBlock("Observation: file contents\n".into());
        append_results(&mut msgs, rendered);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"], "user");
        assert!(msgs[2]["content"].as_str().unwrap().contains("Observation"));
    }

    #[test]
    fn append_handled_by_backend_is_noop() {
        let mut msgs = initial_messages("sys", "usr");
        let original_len = msgs.len();
        append_results(&mut msgs, RenderedResults::HandledByBackend);
        assert_eq!(msgs.len(), original_len);
    }
}
