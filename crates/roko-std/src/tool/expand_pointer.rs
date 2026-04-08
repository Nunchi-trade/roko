//! `expand_pointer` — meta-tool for retrieving full content by pointer ID.
//!
//! When a tool result exceeds the inline threshold, the dispatcher stores
//! the full payload in a [`PointerStore`] and returns a compact
//! [`MemoryPointer`] to the LLM. The LLM can then invoke
//! `expand_pointer(pointer_id)` to fetch the full content on demand.
//!
//! This module provides:
//!
//! - [`tool_def`] — the [`ToolDef`] describing the meta-tool
//! - [`ExpandPointerTool`] — a lightweight executor that resolves
//!   pointer IDs against an in-memory content map
//!
//! The content map is populated by the dispatcher / tool loop and
//! keyed by pointer ID. This avoids coupling the meta-tool to a
//! specific storage backend.

use std::collections::HashMap;

use roko_core::tool::{
    ToolCategory, ToolConcurrency, ToolDef, ToolError, ToolPermission, ToolResult, ToolSchema,
};

/// Canonical `snake_case` name.
pub const NAME: &str = "expand_pointer";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Retrieve the full content of a memory pointer by its ID. Use this when a tool result was truncated and you need the complete output.";

/// Build the [`ToolDef`] for `expand_pointer`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Meta,
        ToolPermission::read_only(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "pointer_id": {
                "type": "string",
                "description": "The pointer ID to expand (from a previous truncated tool result)."
            }
        },
        "required": ["pointer_id"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(10_000)
}

/// Meta-tool executor that resolves pointer IDs to full content.
///
/// The content store is an in-memory map populated by the dispatcher.
/// This avoids disk I/O on the hot path — the dispatcher pre-loads
/// pointer content from the [`PointerStore`] on demand.
#[derive(Debug, Clone)]
pub struct ExpandPointerTool {
    _private: (),
}

impl ExpandPointerTool {
    /// Construct a new instance.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Expand a pointer ID against the provided content store.
    ///
    /// Returns a [`ToolResult::Ok`] with the full content if the pointer
    /// exists, or a [`ToolResult::Err`] if the pointer ID is not found.
    #[must_use]
    pub fn expand(&self, pointer_id: &str, content_store: &HashMap<String, String>) -> ToolResult {
        content_store.get(pointer_id).map_or_else(
            || {
                ToolResult::Err(ToolError::Other(format!(
                    "pointer not found: {pointer_id}. The pointer may have been evicted or the ID is incorrect."
                )))
            },
            ToolResult::text,
        )
    }
}

impl Default for ExpandPointerTool {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_store() -> HashMap<String, String> {
        let mut store = HashMap::new();
        store.insert(
            "ptr-abc123".to_owned(),
            "This is the full content of a large tool result that was truncated.".to_owned(),
        );
        store.insert(
            "ptr-def456".to_owned(),
            r#"{"key": "value", "nested": {"data": [1, 2, 3]}}"#.to_owned(),
        );
        store.insert("ptr-empty".to_owned(), String::new());
        store
    }

    #[test]
    fn tool_def_has_correct_name_and_category() {
        let def = tool_def();
        assert_eq!(def.name, "expand_pointer");
        assert_eq!(def.category, ToolCategory::Meta);
        assert!(def.idempotent);
        assert_eq!(def.concurrency, ToolConcurrency::Parallel);
    }

    #[test]
    fn tool_def_schema_requires_pointer_id() {
        let def = tool_def();
        let schema = def.parameters.as_value();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("pointer_id")));
    }

    #[test]
    fn expand_found_returns_full_content() {
        let tool = ExpandPointerTool::new();
        let store = sample_store();
        let result = tool.expand("ptr-abc123", &store);
        assert!(result.is_ok());
        match result {
            ToolResult::Ok { content, .. } => {
                assert!(content.contains("full content of a large tool result"));
            }
            ToolResult::Err(_) => panic!("expected Ok"),
        }
    }

    #[test]
    fn expand_json_content_returns_correctly() {
        let tool = ExpandPointerTool::new();
        let store = sample_store();
        let result = tool.expand("ptr-def456", &store);
        assert!(result.is_ok());
        match result {
            ToolResult::Ok { content, .. } => {
                assert!(content.contains(r#""key": "value""#));
            }
            ToolResult::Err(_) => panic!("expected Ok"),
        }
    }

    #[test]
    fn expand_empty_content_returns_ok() {
        let tool = ExpandPointerTool::new();
        let store = sample_store();
        let result = tool.expand("ptr-empty", &store);
        assert!(result.is_ok());
        match result {
            ToolResult::Ok { content, .. } => {
                assert!(content.is_empty());
            }
            ToolResult::Err(_) => panic!("expected Ok"),
        }
    }

    #[test]
    fn expand_missing_pointer_returns_error() {
        let tool = ExpandPointerTool::new();
        let store = sample_store();
        let result = tool.expand("ptr-nonexistent", &store);
        assert!(result.is_err());
        match result {
            ToolResult::Err(ToolError::Other(msg)) => {
                assert!(msg.contains("pointer not found"));
                assert!(msg.contains("ptr-nonexistent"));
            }
            _ => panic!("expected ToolError::Other"),
        }
    }

    #[test]
    fn expand_empty_store_returns_error() {
        let tool = ExpandPointerTool::new();
        let store = HashMap::new();
        let result = tool.expand("any-id", &store);
        assert!(result.is_err());
    }

    #[test]
    fn expand_pointer_tool_default_works() {
        let tool = ExpandPointerTool::default();
        let store = sample_store();
        let result = tool.expand("ptr-abc123", &store);
        assert!(result.is_ok());
    }

    #[test]
    fn tool_def_serde_roundtrip() {
        let def = tool_def();
        let json = serde_json::to_string(&def).unwrap();
        let decoded: ToolDef = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, def.name);
        assert_eq!(decoded.category, def.category);
    }
}
