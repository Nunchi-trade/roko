//! CRANE-style delimiters for structured data extraction.
//!
//! CRANE (Context-Rich Annotated Notation for Extraction) uses paired
//! delimiters to mark structured regions within natural-language text.
//! The format is: `<|tag|>content<|/tag|>`.
//!
//! This module provides:
//! - [`CraneDelimiters`] — defines a delimiter pair for a given tag
//! - [`wrap_with_delimiters`] — wraps content in CRANE delimiters
//! - [`extract_between_delimiters`] — extracts content between delimiters

/// A CRANE-style delimiter pair for a given tag.
///
/// # Format
///
/// ```text
/// <|tag|>content<|/tag|>
/// ```
///
/// The tag is case-sensitive. Common tags: `json`, `data`, `tool_call`,
/// `result`, `plan`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraneDelimiters {
    /// The tag name (e.g. `"json"`).
    pub tag: String,
    /// The opening delimiter (e.g. `"<|json|>"`).
    pub open: String,
    /// The closing delimiter (e.g. `"<|/json|>"`).
    pub close: String,
}

impl CraneDelimiters {
    /// Create a delimiter pair for the given tag.
    ///
    /// # Example
    ///
    /// ```
    /// use roko_agent::nl_to_format::delimiters::CraneDelimiters;
    ///
    /// let d = CraneDelimiters::new("json");
    /// assert_eq!(d.open, "<|json|>");
    /// assert_eq!(d.close, "<|/json|>");
    /// ```
    #[must_use]
    pub fn new(tag: impl Into<String>) -> Self {
        let tag = tag.into();
        let open = format!("<|{tag}|>");
        let close = format!("<|/{tag}|>");
        Self { tag, open, close }
    }

    /// Wrap `content` in this delimiter pair.
    #[must_use]
    pub fn wrap(&self, content: &str) -> String {
        format!("{}{}{}", self.open, content, self.close)
    }

    /// Extract content between this delimiter pair from `text`.
    ///
    /// Returns the first match if multiple pairs exist. Returns `None`
    /// if the delimiters are not found or are in the wrong order.
    #[must_use]
    pub fn extract<'a>(&self, text: &'a str) -> Option<&'a str> {
        extract_between_delimiters(text, &self.tag)
    }
}

/// Wrap `content` in CRANE-style delimiters using the given `tag`.
///
/// # Example
///
/// ```
/// use roko_agent::nl_to_format::delimiters::wrap_with_delimiters;
///
/// let wrapped = wrap_with_delimiters(r#"{"a":1}"#, "json");
/// assert_eq!(wrapped, r#"<|json|>{"a":1}<|/json|>"#);
/// ```
#[must_use]
pub fn wrap_with_delimiters(content: &str, tag: &str) -> String {
    format!("<|{tag}|>{content}<|/{tag}|>")
}

/// Extract content between CRANE-style delimiters `<|tag|>...<|/tag|>`.
///
/// Returns the first match. Returns `None` if delimiters are absent,
/// malformed, or in the wrong order.
///
/// # Example
///
/// ```
/// use roko_agent::nl_to_format::delimiters::extract_between_delimiters;
///
/// let text = "prefix <|json|>{\"x\":1}<|/json|> suffix";
/// let content = extract_between_delimiters(text, "json").unwrap();
/// assert_eq!(content, r#"{"x":1}"#);
/// ```
#[must_use]
pub fn extract_between_delimiters<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<|{tag}|>");
    let close = format!("<|/{tag}|>");

    let start = text.find(&open)?;
    let content_start = start + open.len();
    let end = text[content_start..].find(&close)?;

    Some(&text[content_start..content_start + end])
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CraneDelimiters tests ────────────────────────────────────────

    #[test]
    fn crane_delimiters_new_formats_correctly() {
        let d = CraneDelimiters::new("json");
        assert_eq!(d.tag, "json");
        assert_eq!(d.open, "<|json|>");
        assert_eq!(d.close, "<|/json|>");
    }

    #[test]
    fn crane_delimiters_wrap() {
        let d = CraneDelimiters::new("data");
        assert_eq!(d.wrap("hello"), "<|data|>hello<|/data|>");
    }

    #[test]
    fn crane_delimiters_extract() {
        let d = CraneDelimiters::new("result");
        let text = "before <|result|>42<|/result|> after";
        assert_eq!(d.extract(text), Some("42"));
    }

    #[test]
    fn crane_delimiters_extract_missing() {
        let d = CraneDelimiters::new("json");
        assert_eq!(d.extract("no delimiters here"), None);
    }

    // ── wrap_with_delimiters tests ───────────────────────────────────

    #[test]
    fn wrap_basic() {
        assert_eq!(
            wrap_with_delimiters("content", "tag"),
            "<|tag|>content<|/tag|>"
        );
    }

    #[test]
    fn wrap_empty_content() {
        assert_eq!(
            wrap_with_delimiters("", "json"),
            "<|json|><|/json|>"
        );
    }

    #[test]
    fn wrap_json_content() {
        let json = r#"{"key":"value","num":42}"#;
        let wrapped = wrap_with_delimiters(json, "json");
        assert_eq!(
            wrapped,
            r#"<|json|>{"key":"value","num":42}<|/json|>"#
        );
    }

    #[test]
    fn wrap_multiline_content() {
        let content = "line1\nline2\nline3";
        let wrapped = wrap_with_delimiters(content, "plan");
        assert!(wrapped.starts_with("<|plan|>"));
        assert!(wrapped.ends_with("<|/plan|>"));
        assert!(wrapped.contains("line1\nline2\nline3"));
    }

    // ── extract_between_delimiters tests ─────────────────────────────

    #[test]
    fn extract_basic() {
        let text = "<|json|>hello<|/json|>";
        assert_eq!(extract_between_delimiters(text, "json"), Some("hello"));
    }

    #[test]
    fn extract_with_surrounding_text() {
        let text = "prefix <|data|>inner<|/data|> suffix";
        assert_eq!(extract_between_delimiters(text, "data"), Some("inner"));
    }

    #[test]
    fn extract_missing_open() {
        let text = "no open <|/json|>";
        assert_eq!(extract_between_delimiters(text, "json"), None);
    }

    #[test]
    fn extract_missing_close() {
        let text = "<|json|>no close";
        assert_eq!(extract_between_delimiters(text, "json"), None);
    }

    #[test]
    fn extract_wrong_tag() {
        let text = "<|data|>content<|/data|>";
        assert_eq!(extract_between_delimiters(text, "json"), None);
    }

    #[test]
    fn extract_first_match_only() {
        let text = "<|x|>first<|/x|> middle <|x|>second<|/x|>";
        assert_eq!(extract_between_delimiters(text, "x"), Some("first"));
    }

    #[test]
    fn extract_empty_content() {
        let text = "<|tag|><|/tag|>";
        assert_eq!(extract_between_delimiters(text, "tag"), Some(""));
    }

    #[test]
    fn extract_json_object() {
        let text = r#"Result: <|json|>{"name":"Alice","age":30}<|/json|> done."#;
        let content = extract_between_delimiters(text, "json").unwrap();
        assert_eq!(content, r#"{"name":"Alice","age":30}"#);
        // Verify it parses
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert_eq!(parsed["name"], "Alice");
    }

    #[test]
    fn roundtrip_wrap_then_extract() {
        let original = r#"{"status":"ok","count":5}"#;
        let wrapped = wrap_with_delimiters(original, "json");
        let extracted = extract_between_delimiters(&wrapped, "json").unwrap();
        assert_eq!(extracted, original);
    }

    #[test]
    fn extract_case_sensitive_tag() {
        let text = "<|JSON|>content<|/JSON|>";
        // lowercase "json" should NOT match uppercase "JSON"
        assert_eq!(extract_between_delimiters(text, "json"), None);
        assert_eq!(extract_between_delimiters(text, "JSON"), Some("content"));
    }
}
