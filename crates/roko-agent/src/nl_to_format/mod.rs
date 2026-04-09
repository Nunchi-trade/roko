//! NL-to-Format two-pass pipeline.
//!
//! Some model/complexity combinations produce better results when the LLM
//! first generates a natural-language response (pass 1), then a second
//! extraction pass converts the NL into the target structured format
//! (pass 2). This module implements:
//!
//! - [`NlToFormatConverter`] — the two-pass extraction pipeline
//! - [`ConvertError`] — errors from the conversion process
//!
//! # When to use two-pass
//!
//! The routing module ([`routing`]) decides whether a given task should
//! use the two-pass pipeline or direct constrained decoding. The general
//! rule: Complex tasks on Premium models benefit from thinking in NL first.

pub mod delimiters;
pub mod routing;

use delimiters::{extract_between_delimiters, wrap_with_delimiters};

/// Errors produced during NL-to-structured-format conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// The extraction pass could not parse valid JSON from the NL response.
    ParseFailed(String),
    /// The extracted JSON does not conform to the target schema.
    SchemaViolation(String),
    /// The NL response was empty or whitespace-only.
    EmptyResponse,
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParseFailed(msg) => write!(f, "parse failed: {msg}"),
            Self::SchemaViolation(msg) => write!(f, "schema violation: {msg}"),
            Self::EmptyResponse => write!(f, "empty response"),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Orchestrates the two-pass NL-to-Format pipeline.
///
/// # Pipeline
///
/// 1. **Pass 1** (caller-side): The LLM generates a natural language response
///    describing the structured data it wants to emit. This pass is external —
///    the caller feeds the NL string into [`NlToFormatConverter::convert`].
///
/// 2. **Pass 2** (this module): Extracts structured JSON from the NL response
///    by looking for CRANE-style delimited blocks, then validates the result
///    against the target schema.
///
/// # Example
///
/// ```
/// use roko_agent::nl_to_format::NlToFormatConverter;
/// use serde_json::json;
///
/// let converter = NlToFormatConverter::new();
/// let nl = r#"Here is the result: <|json|>{"name":"Alice","age":30}<|/json|> done."#;
/// let schema = json!({"type": "object", "required": ["name"]});
/// let result = converter.convert(nl, &schema);
/// assert!(result.is_ok());
/// ```
#[derive(Debug, Clone)]
pub struct NlToFormatConverter {
    /// The delimiter tag used to wrap structured data in NL responses.
    tag: String,
}

impl Default for NlToFormatConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl NlToFormatConverter {
    /// Create a converter that looks for `<|json|>...<|/json|>` delimiters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tag: "json".to_string(),
        }
    }

    /// Create a converter with a custom delimiter tag.
    #[must_use]
    pub fn with_tag(tag: impl Into<String>) -> Self {
        Self { tag: tag.into() }
    }

    /// The delimiter tag this converter uses.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Build the extraction prompt that instructs the LLM to wrap its
    /// structured output in CRANE-style delimiters.
    ///
    /// This prompt is appended to the system message for pass 1 so the
    /// model knows how to format its answer for pass 2 extraction.
    #[must_use]
    pub fn extraction_prompt(&self, schema: &serde_json::Value) -> String {
        let schema_str = serde_json::to_string_pretty(schema).unwrap_or_default();
        let tag = &self.tag;
        format!(
            "When providing structured output, wrap the JSON in CRANE delimiters:\n\
             <|{tag}|>\n\
             <your JSON here>\n\
             <|/{tag}|>\n\n\
             The JSON must conform to this schema:\n```json\n{schema_str}\n```",
        )
    }

    /// Extract structured JSON from a natural-language response (pass 2).
    ///
    /// 1. Checks the response is non-empty.
    /// 2. Looks for CRANE-delimited content (`<|json|>...<|/json|>`).
    /// 3. If delimiters are not found, tries to parse the entire response as JSON.
    /// 4. Validates the parsed JSON against required fields in the schema.
    ///
    /// # Errors
    ///
    /// - [`ConvertError::EmptyResponse`] if `nl_response` is empty/whitespace.
    /// - [`ConvertError::ParseFailed`] if no valid JSON can be extracted.
    /// - [`ConvertError::SchemaViolation`] if required fields are missing.
    pub fn convert(
        &self,
        nl_response: &str,
        target_schema: &serde_json::Value,
    ) -> Result<serde_json::Value, ConvertError> {
        let trimmed = nl_response.trim();
        if trimmed.is_empty() {
            return Err(ConvertError::EmptyResponse);
        }

        // Try CRANE-delimited extraction first
        let json_str = if let Some(delimited) = extract_between_delimiters(trimmed, &self.tag) {
            delimited.to_string()
        } else {
            // Fallback: try to find a JSON object/array in the raw text
            find_json_in_text(trimmed).ok_or_else(|| {
                ConvertError::ParseFailed(
                    "no CRANE delimiters found and no JSON object detected in response".to_string(),
                )
            })?
        };

        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| ConvertError::ParseFailed(format!("invalid JSON: {e}")))?;

        // Validate required fields from schema
        validate_required_fields(&parsed, target_schema)?;

        Ok(parsed)
    }

    /// Convenience: wrap a value in CRANE delimiters for embedding in a
    /// prompt or test fixture.
    #[must_use]
    pub fn wrap(&self, content: &str) -> String {
        wrap_with_delimiters(content, &self.tag)
    }
}

/// Try to find the first JSON object or array substring in `text`.
fn find_json_in_text(text: &str) -> Option<String> {
    // Look for the first `{` that starts a valid JSON object
    for (i, ch) in text.char_indices() {
        if ch == '{' || ch == '[' {
            let closing = if ch == '{' { '}' } else { ']' };
            // Find the matching closing brace/bracket (simple depth tracking)
            let mut depth = 0i32;
            let mut in_string = false;
            let mut escape_next = false;
            for (j, c) in text[i..].char_indices() {
                if escape_next {
                    escape_next = false;
                    continue;
                }
                if c == '\\' && in_string {
                    escape_next = true;
                    continue;
                }
                if c == '"' {
                    in_string = !in_string;
                    continue;
                }
                if in_string {
                    continue;
                }
                if c == ch {
                    depth += 1;
                } else if c == closing {
                    depth -= 1;
                    if depth == 0 {
                        let candidate = &text[i..=(i + j)];
                        // Verify it actually parses
                        if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                            return Some(candidate.to_string());
                        }
                        break;
                    }
                }
            }
        }
    }
    None
}

/// Check that all `required` fields in the schema are present in `value`.
fn validate_required_fields(
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), ConvertError> {
    let Some(required) = schema.get("required").and_then(|r| r.as_array()) else {
        return Ok(());
    };

    let Some(obj) = value.as_object() else {
        if !required.is_empty() {
            return Err(ConvertError::SchemaViolation(
                "expected object but got non-object value".to_string(),
            ));
        }
        return Ok(());
    };

    for field in required {
        if let Some(name) = field.as_str() {
            if !obj.contains_key(name) {
                return Err(ConvertError::SchemaViolation(format!(
                    "missing required field: {name}"
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn convert_extracts_from_crane_delimiters() {
        let converter = NlToFormatConverter::new();
        let nl = r#"Here is the answer: <|json|>{"name":"Alice","age":30}<|/json|> That's all."#;
        let schema = json!({"type": "object"});
        let result = converter.convert(nl, &schema).unwrap();
        assert_eq!(result["name"], "Alice");
        assert_eq!(result["age"], 30);
    }

    #[test]
    fn convert_falls_back_to_json_detection() {
        let converter = NlToFormatConverter::new();
        let nl = r#"The result is {"x": 42, "y": "hello"} as requested."#;
        let schema = json!({"type": "object"});
        let result = converter.convert(nl, &schema).unwrap();
        assert_eq!(result["x"], 42);
        assert_eq!(result["y"], "hello");
    }

    #[test]
    fn convert_rejects_empty_response() {
        let converter = NlToFormatConverter::new();
        let schema = json!({"type": "object"});
        let err = converter.convert("", &schema).unwrap_err();
        assert_eq!(err, ConvertError::EmptyResponse);
    }

    #[test]
    fn convert_rejects_whitespace_only() {
        let converter = NlToFormatConverter::new();
        let schema = json!({"type": "object"});
        let err = converter.convert("   \n\t  ", &schema).unwrap_err();
        assert_eq!(err, ConvertError::EmptyResponse);
    }

    #[test]
    fn convert_rejects_no_json() {
        let converter = NlToFormatConverter::new();
        let nl = "This is just plain text with no structured data.";
        let schema = json!({"type": "object"});
        let err = converter.convert(nl, &schema).unwrap_err();
        assert!(matches!(err, ConvertError::ParseFailed(_)));
    }

    #[test]
    fn convert_validates_required_fields() {
        let converter = NlToFormatConverter::new();
        let nl = r#"<|json|>{"name":"Bob"}<|/json|>"#;
        let schema = json!({"type": "object", "required": ["name", "age"]});
        let err = converter.convert(nl, &schema).unwrap_err();
        assert!(matches!(err, ConvertError::SchemaViolation(_)));
        if let ConvertError::SchemaViolation(msg) = &err {
            assert!(msg.contains("age"), "expected 'age' in: {msg}");
        }
    }

    #[test]
    fn convert_passes_with_all_required_fields() {
        let converter = NlToFormatConverter::new();
        let nl = r#"<|json|>{"name":"Bob","age":25}<|/json|>"#;
        let schema = json!({"type": "object", "required": ["name", "age"]});
        let result = converter.convert(nl, &schema).unwrap();
        assert_eq!(result["name"], "Bob");
        assert_eq!(result["age"], 25);
    }

    #[test]
    fn convert_no_required_fields_passes_any_object() {
        let converter = NlToFormatConverter::new();
        let nl = r#"<|json|>{"x":1}<|/json|>"#;
        let schema = json!({"type": "object"});
        assert!(converter.convert(nl, &schema).is_ok());
    }

    #[test]
    fn convert_with_custom_tag() {
        let converter = NlToFormatConverter::with_tag("data");
        let nl = r#"Result: <|data|>{"key":"value"}<|/data|> end."#;
        let schema = json!({"type": "object"});
        let result = converter.convert(nl, &schema).unwrap();
        assert_eq!(result["key"], "value");
    }

    #[test]
    fn extraction_prompt_contains_schema_and_delimiters() {
        let converter = NlToFormatConverter::new();
        let schema = json!({"type": "object", "required": ["name"]});
        let prompt = converter.extraction_prompt(&schema);
        assert!(prompt.contains("<|json|>"));
        assert!(prompt.contains("<|/json|>"));
        assert!(prompt.contains("\"name\""));
    }

    #[test]
    fn wrap_produces_crane_delimited_string() {
        let converter = NlToFormatConverter::new();
        let wrapped = converter.wrap(r#"{"a":1}"#);
        assert_eq!(wrapped, r#"<|json|>{"a":1}<|/json|>"#);
    }

    #[test]
    fn convert_handles_nested_json() {
        let converter = NlToFormatConverter::new();
        let nl = r#"<|json|>{"outer":{"inner":[1,2,3]}}<|/json|>"#;
        let schema = json!({"type": "object", "required": ["outer"]});
        let result = converter.convert(nl, &schema).unwrap();
        assert_eq!(result["outer"]["inner"], json!([1, 2, 3]));
    }

    #[test]
    fn convert_error_display() {
        assert_eq!(ConvertError::EmptyResponse.to_string(), "empty response");
        assert_eq!(
            ConvertError::ParseFailed("bad".into()).to_string(),
            "parse failed: bad"
        );
        assert_eq!(
            ConvertError::SchemaViolation("missing x".into()).to_string(),
            "schema violation: missing x"
        );
    }

    #[test]
    fn default_converter_uses_json_tag() {
        let converter = NlToFormatConverter::default();
        assert_eq!(converter.tag(), "json");
    }

    #[test]
    fn find_json_in_text_with_surrounding_text() {
        let text = r#"prefix {"a": "b"} suffix"#;
        let found = find_json_in_text(text).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&found).unwrap();
        assert_eq!(parsed["a"], "b");
    }

    #[test]
    fn find_json_in_text_no_json() {
        assert!(find_json_in_text("no json here").is_none());
    }

    #[test]
    fn find_json_array_in_text() {
        let text = r#"result: [1, 2, 3] done"#;
        let found = find_json_in_text(text).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&found).unwrap();
        assert_eq!(parsed, json!([1, 2, 3]));
    }
}
