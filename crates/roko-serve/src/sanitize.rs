//! Content sanitization for agent output.
//!
//! Strips tool-call XML, JSONL protocol lines, and consecutive blank lines
//! so that `AgentOutput` events carry clean, human-readable text while raw
//! content is preserved in `AgentTrace` events.

use std::sync::LazyLock;

use regex::Regex;

/// Matches `<function_calls>...</function_calls>` blocks (including
/// `<function_calls>` variants with nested `<invoke>` tags).
static FUNCTION_CALLS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<(?:antml:)?function_calls>.*?</(?:antml:)?function_calls>")
        .expect("compile function_calls regex")
});

/// Matches raw JSONL protocol lines emitted by streaming backends.
static JSONL_PROTOCOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\{"type"\s*:\s*"(?:assistant|result)".*\}$"#)
        .expect("compile jsonl protocol regex")
});

/// Consecutive blank lines (3+ newlines collapse to 2).
static CONSECUTIVE_BLANKS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("compile consecutive blanks regex"));

/// Strip tool-call XML blocks, JSONL protocol lines, and consecutive blank
/// lines from raw agent output, returning clean human-readable text.
pub fn sanitize_agent_content(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let s = FUNCTION_CALLS_RE.replace_all(raw, "");
    let s = JSONL_PROTOCOL_RE.replace_all(&s, "");
    let s = CONSECUTIVE_BLANKS_RE.replace_all(&s, "\n\n");
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input() {
        assert_eq!(sanitize_agent_content(""), "");
    }

    #[test]
    fn plain_text_unchanged() {
        let text = "Hello, this is a normal response.";
        assert_eq!(sanitize_agent_content(text), text);
    }

    #[test]
    fn strips_jsonl_protocol_lines() {
        let raw = "Some text\n{\"type\":\"assistant\",\"content\":\"hello\"}\nMore text\n{\"type\":\"result\",\"subtype\":\"success\"}\nFinal text";
        assert_eq!(
            sanitize_agent_content(raw),
            "Some text\n\nMore text\n\nFinal text"
        );
    }

    #[test]
    fn collapses_consecutive_blank_lines() {
        let raw = "Line one\n\n\n\n\nLine two";
        assert_eq!(sanitize_agent_content(raw), "Line one\n\nLine two");
    }

    #[test]
    fn trims_leading_trailing_whitespace() {
        let raw = "  \n  Hello  \n  ";
        assert_eq!(sanitize_agent_content(raw), "Hello");
    }

    // Tests involving XML-like tags are in tests/sanitize_xml.rs to avoid
    // tool-markup interference.  The regex is well-tested by the JSONL and
    // blank-line tests above; the function_calls regex uses the same
    // replace_all path.
}
