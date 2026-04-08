//! Claude CLI translator (§36.30 + §36.31).
//!
//! Claude CLI is a **hosted** backend: it runs tools inside its own
//! process and streams `tool_use` blocks back as stream-json events.
//! Roko's translator has two responsibilities:
//!
//! 1. **Outbound** — map canonical `snake_case` tool names to Claude's
//!    `PascalCase` names via [`roko_core::tool::aliases`] and join them
//!    into a `--tools=Read,Edit,Bash` CLI flag.
//! 2. **Inbound** — walk a sequence of stream-json events, pick out
//!    `content_block_start` events whose `content_block.type == "tool_use"`,
//!    and rebuild each one as a canonical [`ToolCall`].
//!
//! Tool *results* never flow back through this translator: the CLI owns
//! its own loop, so [`Translator::render_results`] returns
//! [`RenderedResults::HandledByBackend`] unconditionally.

use roko_core::tool::aliases::{canonical_of_claude, claude_of_canonical};
use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};

use super::{BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError};

/// Translator for the Anthropic Claude CLI backend.
///
/// Zero-sized — all behaviour lives in the trait impl. Construct with
/// `ClaudeTranslator` or `ClaudeTranslator::default()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeTranslator;

impl Translator for ClaudeTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::AnthropicBlocks
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let csv = tools
            .iter()
            .filter_map(|t| claude_of_canonical(&t.name))
            .collect::<Vec<_>>()
            .join(",");
        RenderedTools::CliFlag(csv)
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::StreamJson(events) = response else {
            return Err(TranslatorError::Malformed("expected stream-json".into()));
        };

        let mut calls = Vec::new();
        for event in events {
            // Only consider `content_block_start` events — the CLI emits
            // other event kinds (message_start, content_block_delta, …)
            // that don't carry a tool_use block.
            if event.get("type").and_then(serde_json::Value::as_str) != Some("content_block_start")
            {
                continue;
            }
            // Structural field: if present-and-declared a content block
            // but missing `content_block`, the stream is malformed.
            let block = event
                .get("content_block")
                .ok_or_else(|| TranslatorError::Malformed("missing content_block".into()))?;
            // Filter to tool_use blocks; `text` / `thinking` etc. are skipped.
            if block.get("type").and_then(serde_json::Value::as_str) != Some("tool_use") {
                continue;
            }
            // Lenient on optional fields: the LLM occasionally omits one
            // or both; we'd rather surface an empty call than an error.
            let id = block
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let name_claude = block
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let name = canonical_of_claude(name_claude)
                .unwrap_or(name_claude)
                .to_string();
            let args = block
                .get("input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            calls.push(ToolCall::new(id, name, args));
        }
        Ok(calls)
    }

    fn render_results(&self, _results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        RenderedResults::HandledByBackend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::aliases::ALIASES;
    use roko_core::tool::{ToolCategory, ToolPermission};
    use serde_json::json;

    fn tool(name: &'static str, category: ToolCategory, perm: ToolPermission) -> ToolDef {
        ToolDef::new(name, "test tool", category, perm)
    }

    fn read_tool(name: &'static str) -> ToolDef {
        tool(name, ToolCategory::Read, ToolPermission::read_only())
    }

    fn write_tool(name: &'static str) -> ToolDef {
        tool(name, ToolCategory::Write, ToolPermission::writes())
    }

    // ── outbound: render_tools ─────────────────────────────────────────────

    #[test]
    fn renders_csv_from_two_tools() {
        let tools = vec![read_tool("read_file"), write_tool("edit_file")];
        match ClaudeTranslator.render_tools(&tools) {
            RenderedTools::CliFlag(csv) => assert_eq!(csv, "Read,Edit"),
            other => panic!("expected CliFlag, got {other:?}"),
        }
    }

    #[test]
    fn renders_skips_tools_without_claude_alias() {
        // `ls` is Roko-only — no Claude alias. It must be filtered out.
        let tools = vec![read_tool("ls"), read_tool("read_file")];
        match ClaudeTranslator.render_tools(&tools) {
            RenderedTools::CliFlag(csv) => assert_eq!(csv, "Read"),
            other => panic!("expected CliFlag, got {other:?}"),
        }
    }

    #[test]
    fn renders_empty_tool_list_gives_empty_csv() {
        match ClaudeTranslator.render_tools(&[]) {
            RenderedTools::CliFlag(csv) => assert_eq!(csv, ""),
            other => panic!("expected CliFlag, got {other:?}"),
        }
    }

    #[test]
    fn canonical_to_claude_covers_16_tools() {
        // Every ALIASES entry with Some(claude) must round-trip back via
        // canonical_of_claude to its original canonical name.
        for alias in ALIASES {
            if let Some(claude_name) = alias.claude {
                assert_eq!(
                    canonical_of_claude(claude_name),
                    Some(alias.canonical),
                    "alias round-trip failed for {claude_name}"
                );
            }
        }
        // Spot-check: every alias either has Some(claude) or is known Roko-only.
        let roko_only: &[&str] = &["ls", "apply_patch", "run_tests"];
        for alias in ALIASES {
            if alias.claude.is_none() {
                assert!(
                    roko_only.contains(&alias.canonical),
                    "unexpected Roko-only tool: {}",
                    alias.canonical
                );
            }
        }
    }

    // ── inbound: parse_calls ───────────────────────────────────────────────

    #[test]
    fn parse_single_tool_use_block() {
        let events = vec![json!({
            "type": "content_block_start",
            "index": 0,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_01ABC",
                "name": "Read",
                "input": { "file_path": "/tmp/x.rs" }
            }
        })];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_01ABC");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["file_path"], "/tmp/x.rs");
    }

    #[test]
    fn parse_multiple_tool_uses_in_one_response() {
        let events = vec![
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {
                    "type": "tool_use",
                    "id": "call-1",
                    "name": "Read",
                    "input": { "file_path": "a.rs" }
                }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "…" }
            }),
            json!({
                "type": "content_block_start",
                "index": 1,
                "content_block": {
                    "type": "tool_use",
                    "id": "call-2",
                    "name": "Bash",
                    "input": { "command": "ls -la" }
                }
            }),
        ];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[1].name, "bash");
        assert_eq!(calls[0].id, "call-1");
        assert_eq!(calls[1].id, "call-2");
    }

    #[test]
    fn parse_claude_name_mapped_to_canonical() {
        let events = vec![json!({
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "id": "c1",
                "name": "Read",
                "input": {}
            }
        })];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls[0].name, "read_file");
    }

    #[test]
    fn parse_unmapped_claude_name_falls_through() {
        let events = vec![json!({
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "id": "c1",
                "name": "CustomMcpTool",
                "input": {}
            }
        })];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls[0].name, "CustomMcpTool");
    }

    #[test]
    fn parse_rejects_non_streamjson_response() {
        let response = BackendResponse::Json(json!({ "message": { "content": "hi" } }));
        let err = ClaudeTranslator
            .parse_calls(&response)
            .expect_err("should reject non-stream-json");
        match err {
            TranslatorError::Malformed(msg) => assert!(msg.contains("stream-json")),
            other => panic!("expected Malformed, got {other:?}"),
        }

        let text_response = BackendResponse::Text("plain output".into());
        let err = ClaudeTranslator
            .parse_calls(&text_response)
            .expect_err("should reject text");
        assert!(matches!(err, TranslatorError::Malformed(_)));
    }

    #[test]
    fn parse_missing_content_block_errors() {
        let events = vec![json!({ "type": "content_block_start", "index": 0 })];
        let err = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect_err("missing content_block must error");
        match err {
            TranslatorError::Malformed(msg) => assert!(msg.contains("content_block")),
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn parse_ignores_non_tool_use_blocks() {
        let events = vec![
            json!({
                "type": "content_block_start",
                "content_block": { "type": "text", "text": "thinking out loud" }
            }),
            json!({
                "type": "content_block_start",
                "content_block": { "type": "thinking", "thinking": "hmm" }
            }),
        ];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_defaults_empty_input_to_object() {
        let events = vec![json!({
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "id": "c1",
                "name": "Read"
            }
        })];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, json!({}));
    }

    #[test]
    fn parse_empty_events_returns_empty() {
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(vec![]))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_ignores_unrelated_event_types() {
        // `message_start`, `message_delta`, etc. must be skipped silently
        // even when they include fields we'd otherwise inspect.
        let events = vec![
            json!({ "type": "message_start", "message": { "id": "m1" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "text": "x" } }),
            json!({ "type": "message_stop" }),
        ];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_missing_id_and_name_defaults_to_empty_strings() {
        // Lenient on optional fields: id/name default to "", the call is
        // still produced so the dispatcher can surface the failure downstream.
        let events = vec![json!({
            "type": "content_block_start",
            "content_block": {
                "type": "tool_use",
                "input": { "foo": 1 }
            }
        })];
        let calls = ClaudeTranslator
            .parse_calls(&BackendResponse::StreamJson(events))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "");
        assert_eq!(calls[0].name, "");
        assert_eq!(calls[0].arguments, json!({ "foo": 1 }));
    }

    // ── results / format ──────────────────────────────────────────────────

    #[test]
    fn render_results_is_handled_by_backend() {
        let rendered = ClaudeTranslator.render_results(&[]);
        assert!(matches!(rendered, RenderedResults::HandledByBackend));

        // Even with populated results, still HandledByBackend — Claude owns
        // its own loop.
        let call = ToolCall::new("c1", "read_file", json!({}));
        let result = ToolResult::text("done");
        let rendered = ClaudeTranslator.render_results(&[(call, result)]);
        assert!(matches!(rendered, RenderedResults::HandledByBackend));
    }

    #[test]
    fn format_returns_anthropic_blocks() {
        assert_eq!(ClaudeTranslator.format(), ToolFormat::AnthropicBlocks);
    }
}
