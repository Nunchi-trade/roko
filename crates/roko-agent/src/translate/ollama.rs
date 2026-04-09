//! Ollama translator (§36.34 + §36.35).
//!
//! Ollama's `/api/chat` endpoint speaks the OpenAI-compatible tool-calling
//! dialect: a `tools: [...]` array goes in the request body, and
//! `message.tool_calls[]` comes back in the response. This translator
//! reshapes the canonical Roko [`ToolDef`] / [`ToolCall`] / [`ToolResult`]
//! types into and out of that wire shape.
//!
//! # M21 (streaming must be disabled when tools are present)
//!
//! Per `MISTAKES-LEARNED.md` M21: when an Ollama request carries a `tools`
//! array, the HTTP backend **must** send `stream: false`. Ollama silently
//! drops tool calls in streaming mode (issues #9632, #12557) — the model
//! decides to call a tool but the streaming response arrives with empty
//! content and `finish_reason: "stop"`, indistinguishable from "no tool
//! was needed". This translator is pure (it does no HTTP), so it can't
//! enforce the policy itself; the enforcement point is the future
//! `OllamaBackend::chat` HTTP adapter. Document the contract here so
//! future callers don't re-introduce the bug.
//!
//! # M22 (constrained decoding on local models)
//!
//! Per M22: local models (especially <32 B) emit malformed JSON at 2–15 %
//! without constrained decoding. The Ollama backend is also responsible
//! for passing each tool's JSON schema as the request-level `format`
//! field, engaging llama.cpp's GBNF/LLGuidance pipeline. That is a
//! request-body concern — this translator only produces the `tools` array
//! (via [`OllamaTranslator::render_tools`]). The backend should copy
//! `t.parameters.as_value()` into the `format` slot when only one tool is
//! on offer, or bundle them via a `oneOf` union otherwise.

use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};

use super::{BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError};

/// Translator for the Ollama `/api/chat` OpenAI-compatible backend.
///
/// Pure, stateless — a zero-sized type. Renders tools as a
/// `tools: [...]` JSON array and parses `tool_calls` from the response
/// body. Roko owns the full multi-turn loop (unlike Claude CLI, which
/// owns its own loop), so [`OllamaTranslator::render_results`] produces a
/// `role: "tool"` message array that the caller splices into the next
/// request's `messages[]`.
///
/// **M21 contract (enforced by caller, not this translator)**: when the
/// HTTP backend sends these tools to Ollama, it must force
/// `stream: false`. See the module docs.
///
/// **M22 contract (enforced by caller, not this translator)**: the HTTP
/// backend should also set the request-level `format` field to the tool's
/// JSON schema for constrained decoding. See the module docs.
#[derive(Debug, Default, Clone, Copy)]
pub struct OllamaTranslator;

impl Translator for OllamaTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::OpenAiJson
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let arr: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters.as_value(),
                    }
                })
            })
            .collect();
        RenderedTools::JsonArray(serde_json::Value::Array(arr))
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::Json(json) = response else {
            return Err(TranslatorError::Malformed("expected json".into()));
        };

        let Some(tool_calls) = json
            .pointer("/message/tool_calls")
            .and_then(|v| v.as_array())
        else {
            return Ok(Vec::new());
        };

        let mut out = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            let id = call
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let name = call
                .pointer("/function/name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| TranslatorError::Malformed("missing function.name".into()))?
                .to_string();

            let args = match call.pointer("/function/arguments") {
                // Ollama commonly sends arguments as a JSON-encoded string.
                Some(serde_json::Value::String(s)) => serde_json::from_str::<serde_json::Value>(s)
                    .map_err(|e| TranslatorError::Malformed(format!("bad arguments json: {e}")))?,
                // Some servers pass it through as an inline object.
                Some(other) => other.clone(),
                // Absent → `{}`.
                None => serde_json::json!({}),
            };

            out.push(ToolCall::new(id, name, args));
        }

        Ok(out)
    }

    fn render_assistant_message(&self, response: &BackendResponse) -> Option<serde_json::Value> {
        let BackendResponse::Json(json) = response else {
            return None;
        };
        json.get("message").cloned()
    }

    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        let msgs: Vec<serde_json::Value> = results
            .iter()
            .map(|(call, res)| {
                let content = match res {
                    ToolResult::Ok { content, .. } => content.clone(),
                    ToolResult::Err(e) => format!("error: {e}"),
                };
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": content,
                })
            })
            .collect();
        RenderedResults::JsonMessages(serde_json::Value::Array(msgs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolError, ToolPermission, ToolSchema};

    // ─── helpers ──────────────────────────────────────────────────────────

    fn read_file_def() -> ToolDef {
        ToolDef::new(
            "read_file",
            "Read a UTF-8 file",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"],
        })))
    }

    fn write_file_def() -> ToolDef {
        ToolDef::new(
            "write_file",
            "Write a UTF-8 file",
            ToolCategory::Write,
            ToolPermission::writes(),
        )
    }

    fn bash_def() -> ToolDef {
        ToolDef::new(
            "bash",
            "Run a shell command",
            ToolCategory::Exec,
            ToolPermission::executes(),
        )
    }

    fn ollama_response_with_call(
        id: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> BackendResponse {
        BackendResponse::Json(serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": arguments },
                    }
                ]
            }
        }))
    }

    // ─── format() ─────────────────────────────────────────────────────────

    #[test]
    fn format_returns_openai_json() {
        assert_eq!(OllamaTranslator.format(), ToolFormat::OpenAiJson);
    }

    // ─── render_tools ─────────────────────────────────────────────────────

    #[test]
    fn renders_three_tools_as_openai_array() {
        let tools = [read_file_def(), write_file_def(), bash_def()];
        let RenderedTools::JsonArray(v) = OllamaTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        let arr = v.as_array().expect("expected array at top level");
        assert_eq!(arr.len(), 3);

        // First entry shape.
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "read_file");
        assert_eq!(arr[0]["function"]["description"], "Read a UTF-8 file");
        assert_eq!(
            arr[0]["function"]["parameters"]["type"], "object",
            "parameters must carry the schema object verbatim"
        );
        assert_eq!(
            arr[0]["function"]["parameters"]["properties"]["path"]["type"],
            "string"
        );

        // Second + third entries.
        assert_eq!(arr[1]["function"]["name"], "write_file");
        assert_eq!(arr[2]["function"]["name"], "bash");
        assert_eq!(arr[2]["type"], "function");
    }

    #[test]
    fn renders_empty_tool_list_gives_empty_array() {
        let RenderedTools::JsonArray(v) = OllamaTranslator.render_tools(&[]) else {
            panic!("expected JsonArray");
        };
        assert_eq!(
            v,
            serde_json::Value::Array(Vec::new()),
            "empty tool list must render as []"
        );
    }

    // ─── parse_calls ──────────────────────────────────────────────────────

    #[test]
    fn parse_single_tool_call() {
        let resp = ollama_response_with_call(
            "call_001",
            "read_file",
            serde_json::Value::String(r#"{"path":"src/lib.rs"}"#.to_string()),
        );
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_001");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "src/lib.rs");
    }

    #[test]
    fn parse_no_tool_calls_returns_empty() {
        let resp = BackendResponse::Json(serde_json::json!({
            "message": { "role": "assistant", "content": "hi!" }
        }));
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("parse should succeed on no-tool-call response");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_no_message_field_returns_empty() {
        // Neither `message` nor `tool_calls` present → `Ok(vec![])`.
        let resp = BackendResponse::Json(serde_json::json!({}));
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("parse should succeed on empty json");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_non_json_response_errors() {
        let err = OllamaTranslator
            .parse_calls(&BackendResponse::Text("hello".into()))
            .expect_err("text response should error");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(err.to_string().contains("expected json"));

        let err = OllamaTranslator
            .parse_calls(&BackendResponse::StreamJson(Vec::new()))
            .expect_err("stream-json response should error");
        assert!(matches!(err, TranslatorError::Malformed(_)));
    }

    #[test]
    fn parse_malformed_arguments_json_fails() {
        let resp = ollama_response_with_call(
            "call_bad",
            "read_file",
            serde_json::Value::String("{bad".to_string()),
        );
        let err = OllamaTranslator
            .parse_calls(&resp)
            .expect_err("bad json in arguments should error");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(
            err.to_string().contains("bad arguments json"),
            "error must mention 'bad arguments json': got {err}"
        );
    }

    #[test]
    fn parse_accepts_object_arguments() {
        // Some servers pass arguments through as an inline object instead of
        // a JSON-encoded string.
        let resp =
            ollama_response_with_call("call_obj", "read_file", serde_json::json!({ "path": "x" }));
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("object arguments must be accepted");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["path"], "x");
    }

    #[test]
    fn parse_absent_arguments_defaults_to_empty_object() {
        let resp = BackendResponse::Json(serde_json::json!({
            "message": {
                "tool_calls": [
                    { "id": "c", "type": "function", "function": { "name": "bash" } }
                ]
            }
        }));
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("absent arguments should default to {}");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn parse_missing_function_name_fails() {
        let resp = BackendResponse::Json(serde_json::json!({
            "message": {
                "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "arguments": "{}" } }
                ]
            }
        }));
        let err = OllamaTranslator
            .parse_calls(&resp)
            .expect_err("missing function.name must fail");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(err.to_string().contains("function.name"));
    }

    #[test]
    fn parse_respects_tool_call_id() {
        let resp = ollama_response_with_call(
            "custom-id-abc",
            "bash",
            serde_json::Value::String("{}".to_string()),
        );
        let calls = OllamaTranslator.parse_calls(&resp).expect("parse");
        assert_eq!(calls[0].id, "custom-id-abc");
    }

    #[test]
    fn parse_id_defaults_to_empty_when_absent() {
        let resp = BackendResponse::Json(serde_json::json!({
            "message": {
                "tool_calls": [
                    { "type": "function", "function": { "name": "bash", "arguments": "{}" } }
                ]
            }
        }));
        let calls = OllamaTranslator.parse_calls(&resp).expect("parse");
        assert_eq!(calls[0].id, "");
    }

    #[test]
    fn parse_tool_calls_not_an_array_returns_empty() {
        // If tool_calls exists but isn't an array, act like it's absent.
        let resp = BackendResponse::Json(serde_json::json!({
            "message": { "tool_calls": "unexpected" }
        }));
        let calls = OllamaTranslator.parse_calls(&resp).expect("parse");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_multiple_tool_calls_in_one_response() {
        let resp = BackendResponse::Json(serde_json::json!({
            "message": {
                "tool_calls": [
                    {
                        "id": "c1",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": r#"{"path":"a"}"# }
                    },
                    {
                        "id": "c2",
                        "type": "function",
                        "function": { "name": "bash", "arguments": {"cmd": "ls"} }
                    }
                ]
            }
        }));
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "a");
        assert_eq!(calls[1].id, "c2");
        assert_eq!(calls[1].name, "bash");
        assert_eq!(calls[1].arguments["cmd"], "ls");
    }

    // ─── render_results ───────────────────────────────────────────────────

    #[test]
    fn render_results_uses_role_tool_and_tool_call_id() {
        let call = ToolCall::at("call_xyz", "read_file", serde_json::json!({"path": "x"}), 1);
        let result = ToolResult::text("file contents");
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&[(call, result)])
        else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "tool");
        assert_eq!(arr[0]["tool_call_id"], "call_xyz");
        assert_eq!(arr[0]["content"], "file contents");
    }

    #[test]
    fn render_results_stringifies_err_variants() {
        let call = ToolCall::at("c1", "bash", serde_json::json!({}), 1);
        let result = ToolResult::err(ToolError::Other("boom".into()));
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&[(call, result)])
        else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        let content = arr[0]["content"]
            .as_str()
            .expect("content must be a string");
        assert!(
            content.starts_with("error: "),
            "err variant must be stringified with leading 'error: ': got {content}"
        );
        assert!(content.contains("boom"));
    }

    #[test]
    fn render_results_stringifies_timeout_error() {
        let call = ToolCall::at("c1", "bash", serde_json::json!({}), 1);
        let result = ToolResult::err(ToolError::Timeout { after_ms: 2_000 });
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&[(call, result)])
        else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        let content = arr[0]["content"].as_str().expect("content is string");
        assert!(content.starts_with("error: "));
        assert!(content.contains("2000"));
    }

    #[test]
    fn render_results_ok_copies_content() {
        let call = ToolCall::at("c1", "read_file", serde_json::json!({}), 1);
        let result = ToolResult::text("exact payload");
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&[(call, result)])
        else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        assert_eq!(arr[0]["content"], "exact payload");
    }

    #[test]
    fn render_results_empty_input_gives_empty_array() {
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&[]) else {
            panic!("expected JsonMessages");
        };
        assert_eq!(v, serde_json::Value::Array(Vec::new()));
    }

    #[test]
    fn render_results_preserves_order_and_ids() {
        let pairs = vec![
            (
                ToolCall::at("a", "read_file", serde_json::json!({}), 1),
                ToolResult::text("one"),
            ),
            (
                ToolCall::at("b", "bash", serde_json::json!({}), 2),
                ToolResult::text("two"),
            ),
        ];
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&pairs) else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["tool_call_id"], "a");
        assert_eq!(arr[0]["content"], "one");
        assert_eq!(arr[1]["tool_call_id"], "b");
        assert_eq!(arr[1]["content"], "two");
    }

    // ─── round-trip ───────────────────────────────────────────────────────

    #[test]
    fn round_trip_one_call() {
        // 1. render_tools
        let tools = [read_file_def()];
        let RenderedTools::JsonArray(rendered) = OllamaTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        assert_eq!(rendered[0]["function"]["name"], "read_file");

        // 2. Fake a backend response that picks that tool.
        let resp = ollama_response_with_call(
            "t42",
            "read_file",
            serde_json::Value::String(r#"{"path":"Cargo.toml"}"#.to_string()),
        );

        // 3. parse_calls
        let calls = OllamaTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].id, "t42");
        assert_eq!(calls[0].arguments["path"], "Cargo.toml");

        // 4. render_results (simulate handler success)
        let pairs = vec![(calls[0].clone(), ToolResult::text("contents-here"))];
        let RenderedResults::JsonMessages(v) = OllamaTranslator.render_results(&pairs) else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("expected array");
        assert_eq!(arr[0]["role"], "tool");
        assert_eq!(arr[0]["tool_call_id"], "t42"); // id flowed through
        assert_eq!(arr[0]["content"], "contents-here");
    }
}
