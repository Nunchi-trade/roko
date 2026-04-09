//! `ReAct` text-level translator (§36.38).
//!
//! For models **without** native function calling. Embeds tool schemas
//! in the system prompt and parses plain-text `Action:` /
//! `Action Input:` / `Observation:` / `Final Answer:` markers out of
//! completions.
//!
//! # Parsing note
//!
//! The parser picks the **last** `Action:` marker in the response, not
//! the first. Earlier `Action:` strings may appear quoted inside the
//! model's reasoning text (e.g. `"I should call Action: read_file
//! but first..."`); only the trailing one corresponds to the actual
//! action the model is emitting this turn.

use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};

use super::{BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError};

/// Translator for models without native function calling.
///
/// Embeds tool schemas in the system prompt, then parses
/// `Action:` / `Action Input:` / `Observation:` / `Final Answer:`
/// markers out of plain-text completions.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReActTranslator;

impl Translator for ReActTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::ReActText
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let mut block = String::from("You have access to the following tools:\n\n");
        for t in tools {
            let schema = serde_json::to_string_pretty(t.parameters.as_value()).unwrap_or_default();
            block.push_str("### ");
            block.push_str(&t.name);
            block.push('\n');
            block.push_str(&t.description);
            block.push('\n');
            block.push_str("Arguments schema:\n");
            block.push_str("```json\n");
            block.push_str(&schema);
            block.push_str("\n```\n\n");
        }
        block.push_str(
            "To call a tool, emit:\n\
             Action: <tool_name>\n\
             Action Input: {<json args>}\n\n\
             After Observation: is supplied, continue reasoning.\n\
             When done, emit: Final Answer: <text>\n",
        );
        RenderedTools::SystemPromptBlock(block)
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::Text(text) = response else {
            return Err(TranslatorError::Malformed("expected text".into()));
        };

        // Use the LAST "Action:" marker — earlier ones may be quoted in
        // the reasoning text.
        let Some(action_idx) = text.rfind("Action:") else {
            return Ok(Vec::new());
        };
        let after = &text[action_idx + "Action:".len()..];

        // Tool name is the text up to the next newline.
        let Some(newline) = after.find('\n') else {
            return Ok(Vec::new());
        };
        let name = after[..newline].trim().to_string();
        let rest = &after[newline..];

        // Action Input: must follow, otherwise the model hasn't emitted
        // a complete call.
        let Some(input_idx) = rest.find("Action Input:") else {
            return Ok(Vec::new());
        };
        let input_text = &rest[input_idx + "Action Input:".len()..];

        // Args extend to the next blank line ("\n\n") or to end-of-string.
        let input_end = input_text.find("\n\n").unwrap_or(input_text.len());
        let args_str = input_text[..input_end].trim();

        let args: serde_json::Value = serde_json::from_str(args_str)
            .map_err(|e| TranslatorError::Malformed(format!("bad action input json: {e}")))?;

        Ok(vec![ToolCall::new("react-0", name, args)])
    }

    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        let mut block = String::new();
        for (_, res) in results {
            match res {
                ToolResult::Ok { content, .. } => {
                    block.push_str("Observation: ");
                    block.push_str(content);
                    block.push_str("\n\n");
                }
                ToolResult::Err(e) => {
                    block.push_str("Observation: error: ");
                    block.push_str(&e.to_string());
                    block.push_str("\n\n");
                }
            }
        }
        RenderedResults::TextBlock(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolError, ToolPermission, ToolSchema};
    use serde_json::json;

    fn read_tool(name: &'static str) -> ToolDef {
        ToolDef::new(
            name,
            "Read the contents of a UTF-8 file from the worktree.",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
        .with_parameters(ToolSchema::from_value(json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        })))
    }

    fn write_tool(name: &'static str) -> ToolDef {
        ToolDef::new(
            name,
            "Edit a UTF-8 file in the worktree.",
            ToolCategory::Write,
            ToolPermission::writes(),
        )
        .with_parameters(ToolSchema::from_value(json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
            },
            "required": ["path", "content"],
        })))
    }

    // ── format ───────────────────────────────────────────────────────────────

    #[test]
    fn format_returns_react_text() {
        assert_eq!(ReActTranslator.format(), ToolFormat::ReActText);
    }

    // ── render_tools ─────────────────────────────────────────────────────────

    #[test]
    fn renders_tools_in_system_prompt_block() {
        let tools = vec![read_tool("read_file"), write_tool("edit_file")];
        let rendered = ReActTranslator.render_tools(&tools);
        let RenderedTools::SystemPromptBlock(block) = rendered else {
            panic!("expected SystemPromptBlock");
        };
        assert!(
            block.contains("### read_file"),
            "block missing read_file heading: {block}"
        );
        assert!(
            block.contains("### edit_file"),
            "block missing edit_file heading: {block}"
        );
        assert!(
            block.contains("Read the contents of a UTF-8 file from the worktree."),
            "block missing read_file description"
        );
        assert!(
            block.contains("Action:"),
            "block missing Action: instruction"
        );
        assert!(
            block.contains("Final Answer:"),
            "block missing Final Answer: instruction"
        );
        assert!(
            block.contains("```json"),
            "block missing fenced json schema"
        );
    }

    #[test]
    fn renders_empty_tool_list_still_shows_instructions() {
        let rendered = ReActTranslator.render_tools(&[]);
        let RenderedTools::SystemPromptBlock(block) = rendered else {
            panic!("expected SystemPromptBlock");
        };
        assert!(
            block.contains("Action:"),
            "instructions missing from empty block"
        );
        assert!(
            block.contains("Final Answer:"),
            "final answer cue missing from empty block"
        );
        // No tool headings when the catalog is empty.
        assert!(!block.contains("### "));
    }

    // ── parse_calls ──────────────────────────────────────────────────────────

    #[test]
    fn parse_single_action_input_block() {
        let text = "I need to inspect the file.\n\
                    Thought: Call read_file on lib.rs.\n\
                    Action: read_file\n\
                    Action Input: {\"path\": \"src/lib.rs\"}\n\n";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].id, "react-0");
        assert_eq!(calls[0].arguments["path"], "src/lib.rs");
    }

    #[test]
    fn parse_returns_empty_for_final_answer_only() {
        let text = "Thought: I can answer directly.\nFinal Answer: 42\n";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_rejects_malformed_json_input() {
        let text = "Action: read_file\nAction Input: {bad json\n\n";
        let err = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect_err("malformed json should error");
        match err {
            TranslatorError::Malformed(msg) => {
                assert!(
                    msg.contains("bad action input json"),
                    "unexpected msg: {msg}"
                );
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn parse_finds_latest_action_if_multiple() {
        // First Action: appears in reasoning text, second is the real call.
        let text = "Thought: I considered emitting 'Action: glob' but decided against it.\n\
                    After reflection, I'll call a different tool.\n\
                    Action: read_file\n\
                    Action Input: {\"path\": \"README.md\"}\n\n";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "README.md");
    }

    #[test]
    fn parse_respects_blank_line_terminator() {
        // The blank line after the JSON ends the args; any text afterwards
        // (even a stray `}`) must be ignored.
        let text = "Action: edit_file\n\
                    Action Input: {\"path\": \"a.rs\", \"content\": \"x\"}\n\n\
                    Some trailing commentary that must not be parsed }";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "edit_file");
        assert_eq!(calls[0].arguments["path"], "a.rs");
        assert_eq!(calls[0].arguments["content"], "x");
    }

    #[test]
    fn parse_args_without_blank_line_takes_to_end() {
        // No trailing "\n\n" — args run to end-of-string.
        let text = "Action: read_file\nAction Input: {\"path\": \"x.rs\"}";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "x.rs");
    }

    #[test]
    fn parse_non_text_response_errors() {
        let response = BackendResponse::Json(json!({ "message": { "content": "hi" } }));
        let err = ReActTranslator
            .parse_calls(&response)
            .expect_err("json response should be rejected");
        match err {
            TranslatorError::Malformed(msg) => {
                assert!(msg.contains("expected text"), "unexpected msg: {msg}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }

        let stream_response = BackendResponse::StreamJson(vec![]);
        let err = ReActTranslator
            .parse_calls(&stream_response)
            .expect_err("stream response should be rejected");
        assert!(matches!(err, TranslatorError::Malformed(_)));
    }

    #[test]
    fn parse_no_action_input_after_action_returns_empty() {
        // Action: is present but there's no Action Input: line — the model
        // hasn't actually emitted a complete call yet.
        let text = "Action: read_file\n\nI forgot the input line.\n";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_action_without_newline_returns_empty() {
        // Degenerate case: "Action:" at end-of-string with no following
        // newline to terminate the name.
        let text = "I will emit Action:";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(text.into()))
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    // ── render_results ───────────────────────────────────────────────────────

    #[test]
    fn render_results_formats_as_observations() {
        let call = ToolCall::new("react-0", "read_file", json!({"path": "x.rs"}));
        let result = ToolResult::text("line1\nline2");
        let rendered = ReActTranslator.render_results(&[(call, result)]);
        let RenderedResults::TextBlock(block) = rendered else {
            panic!("expected TextBlock");
        };
        assert_eq!(block, "Observation: line1\nline2\n\n");
    }

    #[test]
    fn render_results_formats_err_variants_as_observation_error() {
        let call = ToolCall::new("react-0", "read_file", json!({"path": "x.rs"}));
        let result = ToolResult::err(ToolError::PermissionDenied("needs read".into()));
        let rendered = ReActTranslator.render_results(&[(call, result)]);
        let RenderedResults::TextBlock(block) = rendered else {
            panic!("expected TextBlock");
        };
        assert!(
            block.starts_with("Observation: error: "),
            "block should start with error prefix: {block}"
        );
        assert!(
            block.contains("permission denied"),
            "block should carry the error display: {block}"
        );
        assert!(block.ends_with("\n\n"));
    }

    #[test]
    fn render_results_empty_results_gives_empty_block() {
        let rendered = ReActTranslator.render_results(&[]);
        let RenderedResults::TextBlock(block) = rendered else {
            panic!("expected TextBlock");
        };
        assert!(block.is_empty());
    }

    #[test]
    fn render_results_concatenates_multiple_observations() {
        let call1 = ToolCall::new("react-0", "read_file", json!({}));
        let call2 = ToolCall::new("react-0", "read_file", json!({}));
        let rendered = ReActTranslator.render_results(&[
            (call1, ToolResult::text("first")),
            (call2, ToolResult::text("second")),
        ]);
        let RenderedResults::TextBlock(block) = rendered else {
            panic!("expected TextBlock");
        };
        assert_eq!(block, "Observation: first\n\nObservation: second\n\n");
    }

    // ── round trip ───────────────────────────────────────────────────────────

    #[test]
    fn round_trip_single_tool() {
        // Render tool catalog → simulate a model completion that calls
        // the tool → parse the call → render a fake result.
        let tools = vec![read_tool("read_file")];
        let rendered = ReActTranslator.render_tools(&tools);
        let RenderedTools::SystemPromptBlock(block) = rendered else {
            panic!("expected SystemPromptBlock");
        };
        assert!(block.contains("### read_file"));

        let fake_completion = "Thought: Need to look at the file.\n\
                               Action: read_file\n\
                               Action Input: {\"path\": \"src/lib.rs\"}\n\n";
        let calls = ReActTranslator
            .parse_calls(&BackendResponse::Text(fake_completion.into()))
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");

        let result = ToolResult::text("pub fn main() {}");
        let results = vec![(calls[0].clone(), result)];
        let rendered = ReActTranslator.render_results(&results);
        let RenderedResults::TextBlock(text) = rendered else {
            panic!("expected TextBlock");
        };
        assert_eq!(text, "Observation: pub fn main() {}\n\n");
    }
}
