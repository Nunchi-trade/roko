//! Context-growth guard (§36.55).
//!
//! When the conversation grows too large for the model's context window,
//! this module drops the oldest tool-result messages while preserving
//! the system prompt, user prompt, and the most recent results.

/// Default context token limit.  Callers should set this to ~80% of the
/// model's actual context window so the LLM has room to reply.
pub const DEFAULT_CONTEXT_TOKEN_LIMIT: usize = 102_400;

/// Number of messages always kept at the start (system + user).
const HEAD_KEEP: usize = 2;

/// Number of messages always kept at the tail (most recent context).
const TAIL_KEEP: usize = 3;

/// Rough estimate of token count: total JSON bytes / 4.
///
/// This is a cheap heuristic, not a real tokenizer.  Good enough for
/// budget-guard decisions where +/- 20% doesn't matter.
pub(crate) fn estimate_message_tokens(messages: &[serde_json::Value]) -> usize {
    messages
        .iter()
        .map(|m| serde_json::to_string(m).map_or(0, |s| s.len()))
        .sum::<usize>()
        / 4
}

/// Drop the oldest non-head, non-tail messages until the estimated
/// token count is within `token_limit`.
///
/// Preserves:
/// - The first [`HEAD_KEEP`] messages (system prompt, user prompt).
/// - The last [`TAIL_KEEP`] messages (most recent results/context).
///
/// If the conversation has `HEAD_KEEP + TAIL_KEEP` or fewer messages,
/// nothing is removed.
pub fn prune_if_needed(messages: &mut Vec<serde_json::Value>, token_limit: usize) {
    let min_len = HEAD_KEEP + TAIL_KEEP;
    if messages.len() <= min_len {
        return;
    }
    while estimate_message_tokens(messages) > token_limit && messages.len() > min_len {
        // Remove the oldest droppable message (first one after the head).
        messages.remove(HEAD_KEEP);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(content: &str) -> serde_json::Value {
        serde_json::json!({"role": "assistant", "content": content})
    }

    #[test]
    fn no_prune_when_under_limit() {
        let mut msgs = vec![msg("sys"), msg("user"), msg("a"), msg("b"), msg("c")];
        let original_len = msgs.len();
        prune_if_needed(&mut msgs, 1_000_000);
        assert_eq!(msgs.len(), original_len);
    }

    #[test]
    fn no_prune_when_at_minimum_size() {
        let mut msgs = vec![msg("sys"), msg("user"), msg("a"), msg("b"), msg("c")];
        // 5 messages = HEAD_KEEP(2) + TAIL_KEEP(3), can't prune further.
        prune_if_needed(&mut msgs, 0);
        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn prunes_oldest_middle_messages() {
        let mut msgs = vec![msg("sys"), msg("user")];
        for i in 0..20 {
            msgs.push(msg(&"x".repeat(200 + i)));
        }
        let before_len = msgs.len();
        prune_if_needed(&mut msgs, 100);
        assert!(msgs.len() < before_len, "should have pruned some messages");
        assert!(msgs.len() >= HEAD_KEEP + TAIL_KEEP);
        assert_eq!(msgs[0], msg("sys"));
        assert_eq!(msgs[1], msg("user"));
    }

    #[test]
    fn preserves_head_and_tail() {
        let mut msgs = vec![
            msg("system"),
            msg("user_prompt"),
            msg("old_1"),
            msg("old_2"),
            msg("old_3"),
            msg("old_4"),
            msg("recent_1"),
            msg("recent_2"),
            msg("recent_3"),
        ];
        prune_if_needed(&mut msgs, 0);
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0], msg("system"));
        assert_eq!(msgs[1], msg("user_prompt"));
        assert_eq!(msgs[2], msg("recent_1"));
        assert_eq!(msgs[3], msg("recent_2"));
        assert_eq!(msgs[4], msg("recent_3"));
    }
}
