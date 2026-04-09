//! Result truncation (§36.43) — cap oversized `Ok` content while
//! preserving UTF-8 char boundaries.
//!
//! Handler output that blows the per-turn context budget must be
//! truncated before it flows back into the LLM. The algorithm:
//!
//! 1. If the result is an `Err` or the content is already ≤ `max_bytes`,
//!    return it unchanged.
//! 2. Otherwise, slice `content[..max_bytes]` and `pop()` trailing bytes
//!    until the length is a valid UTF-8 char boundary
//!    (`while !s.is_char_boundary(s.len()) { s.pop(); }`). This guards
//!    against splitting a multibyte codepoint mid-sequence.
//! 3. Append a `"\n...[truncated]"` marker so the LLM can see the
//!    result was cut (the marker adds ~16 bytes on top of `max_bytes`
//!    — acceptable because the cap is a guideline, not a hard cutoff).
//! 4. Leaves `is_structured` / `artifacts` untouched.

use roko_core::tool::ToolResult;

/// Marker appended to truncated content (visible to the LLM).
pub const TRUNCATION_MARKER: &str = "\n...[truncated]";

/// Truncate oversized `Ok` content, preserving UTF-8 char boundaries.
///
/// `Err` variants are passed through unchanged, as are `Ok` variants
/// whose `content.len() <= max_bytes`.
#[must_use]
pub fn truncate_result(res: ToolResult, max_bytes: usize) -> ToolResult {
    match res {
        ToolResult::Ok {
            content,
            is_structured,
            artifacts,
        } if content.len() > max_bytes => {
            // Find the largest position ≤ `max_bytes` that is a valid
            // UTF-8 char boundary (walk backwards from `max_bytes`).
            // `String::truncate` panics on a non-boundary, so we can't
            // just call it with an arbitrary byte index — we must
            // locate a boundary first.
            let mut cut = max_bytes;
            while cut > 0 && !content.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut trimmed = content;
            trimmed.truncate(cut);
            trimmed.push_str(TRUNCATION_MARKER);
            ToolResult::Ok {
                content: trimmed,
                is_structured,
                artifacts,
            }
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolError, ToolResult};

    #[test]
    fn truncate_leaves_err_variants_untouched() {
        let err = ToolResult::err(ToolError::Cancelled);
        let out = truncate_result(err.clone(), 4);
        assert_eq!(out, err, "Err variants must pass through unchanged");
    }

    #[test]
    fn truncate_skips_under_cap() {
        let res = ToolResult::text("hello");
        let out = truncate_result(res.clone(), 16);
        assert_eq!(out, res, "content under cap must be unchanged");
    }

    #[test]
    fn truncate_preserves_utf8_with_marker() {
        // "日本語" is 9 bytes (3 codepoints × 3 bytes each) followed by
        // 200 bytes of ASCII filler. Cap at 5 bytes — that would slice
        // inside the second codepoint, so the truncator must pop bytes
        // back to a char boundary (down to 3 bytes, keeping just "日").
        let mut content = String::from("日本語");
        content.push_str(&"x".repeat(200));
        let res = ToolResult::text(content);
        let out = truncate_result(res, 5);
        match out {
            ToolResult::Ok { content: c, .. } => {
                assert!(c.is_char_boundary(c.len()), "must end on char boundary");
                assert!(c.contains(TRUNCATION_MARKER), "must contain marker");
                // The marker is appended after the (char-boundary-aligned)
                // prefix, so the output must also be valid UTF-8 overall.
                let _ = std::str::from_utf8(c.as_bytes())
                    .expect("truncated content must be valid UTF-8");
                // The first 3 bytes are "日"; "本" starts at byte 3 and
                // ends at byte 6, so with cap=5 we pop back to 3.
                assert!(c.starts_with("日"));
                assert!(!c.starts_with("日本"));
            }
            ToolResult::Err(e) => panic!("expected Ok, got Err: {e}"),
        }
    }
}
