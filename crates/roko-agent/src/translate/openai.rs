//! `OpenAI` translator (§36.36).
//!
//! Translates the canonical Roko tool catalog to and from the
//! `/v1/chat/completions` wire format.
//!
//! The `OpenAI` API uses the same `tools: [...]` array shape as Ollama, with
//! two important differences on the inbound side:
//!
//! 1. Tool calls live under `choices[0].message.tool_calls` (nested one
//!    deeper than Ollama's `message.tool_calls`).
//! 2. The `arguments` field is **always** a JSON-encoded string, never a
//!    raw object. The translator stringifies on the way out and parses
//!    strings on the way in.
//!
//! Both the outbound tool spec and the tool-result messages re-use the
//! same layout as the Ollama translator; only the inbound JSON pointer
//! and the arguments decoding differ.

use roko_core::tool::{ToolCall, ToolCategory, ToolDef, ToolFormat, ToolResult};

use super::{BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError};

/// Translator for the `OpenAI` `/v1/chat/completions` backend.
///
/// Renders tools as a `tools: [...]` JSON array and parses
/// `choices[0].message.tool_calls` from the response body. Arguments are
/// always JSON-stringified per the `OpenAI` wire contract.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAiTranslator;

impl Translator for OpenAiTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::OpenAiJson
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let arr: Vec<serde_json::Value> = tools
            .iter()
            .map(render_tool)
            .collect();
        RenderedTools::JsonArray(serde_json::Value::Array(arr))
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::Json(json) = response else {
            return Err(TranslatorError::Malformed("expected json".into()));
        };

        let Some(arr) = json
            .pointer("/choices/0/message/tool_calls")
            .and_then(|v| v.as_array())
        else {
            return Ok(Vec::new());
        };

        let mut out = Vec::with_capacity(arr.len());
        for call in arr {
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
            let args_str = call
                .pointer("/function/arguments")
                .and_then(|v| v.as_str())
                .unwrap_or("{}");
            let args: serde_json::Value = serde_json::from_str(args_str)
                .map_err(|e| TranslatorError::Malformed(format!("bad arguments json: {e}")))?;
            out.push(ToolCall::new(id, name, args));
        }
        Ok(out)
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

fn render_tool(t: &ToolDef) -> serde_json::Value {
    match tool_kind(t) {
        ToolKind::Function => serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters.as_value(),
            }
        }),
        ToolKind::WebSearch => serde_json::json!({
            "type": "web_search",
            "web_search": t.parameters.as_value(),
        }),
        ToolKind::Retrieval => serde_json::json!({
            "type": "retrieval",
            "retrieval": t.parameters.as_value(),
        }),
        ToolKind::McpTool => serde_json::json!({
            "type": "mcp",
            "mcp": t.parameters.as_value(),
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Function,
    WebSearch,
    Retrieval,
    McpTool,
}

fn tool_kind(tool: &ToolDef) -> ToolKind {
    match tool.name.as_str() {
        "web_search" => ToolKind::WebSearch,
        "retrieval" => ToolKind::Retrieval,
        _ if matches!(tool.category, ToolCategory::Mcp) => ToolKind::McpTool,
        _ => ToolKind::Function,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolError, ToolPermission, ToolSchema};

    fn tool(name: &str, desc: &str) -> ToolDef {
        ToolDef::new(name, desc, ToolCategory::Read, ToolPermission::read_only()).with_parameters(
            ToolSchema::from_value(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            })),
        )
    }

    fn ok_text(s: &str) -> ToolResult {
        ToolResult::text(s)
    }

    #[test]
    fn format_returns_openai_json() {
        assert_eq!(OpenAiTranslator.format(), ToolFormat::OpenAiJson);
    }

    #[test]
    fn renders_two_tools_as_openai_array() {
        let tools = [tool("read_file", "Read a file"), tool("grep", "Search")];
        let rendered = OpenAiTranslator.render_tools(&tools);
        let RenderedTools::JsonArray(v) = rendered else {
            panic!("expected JsonArray");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for (i, name) in ["read_file", "grep"].iter().enumerate() {
            assert_eq!(arr[i]["type"], "function");
            assert_eq!(arr[i]["function"]["name"], *name);
            assert!(arr[i]["function"]["description"].is_string());
            assert_eq!(arr[i]["function"]["parameters"]["type"], "object");
        }
    }

    #[test]
    fn renders_empty_tool_list_gives_empty_array() {
        let rendered = OpenAiTranslator.render_tools(&[]);
        let RenderedTools::JsonArray(v) = rendered else {
            panic!("expected JsonArray");
        };
        assert_eq!(v.as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn render_extended_tools_mixed_function_and_web_search() {
        let tools = [
            tool("read_file", "Read a file"),
            ToolDef::new(
                "web_search",
                "Search the web",
                ToolCategory::Network,
                ToolPermission::networked(),
            )
            .with_parameters(ToolSchema::from_value(serde_json::json!({
                "enable": true,
                "search_engine": "bing"
            }))),
        ];

        let rendered = OpenAiTranslator.render_tools(&tools);
        let RenderedTools::JsonArray(v) = rendered else {
            panic!("expected JsonArray");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "read_file");
        assert_eq!(arr[1]["type"], "web_search");
        assert_eq!(arr[1]["web_search"]["enable"], true);
        assert_eq!(arr[1]["web_search"]["search_engine"], "bing");
    }

    #[test]
    fn parse_single_tool_call_with_stringified_args() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"src/lib.rs\"}"
                        }
                    }]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "src/lib.rs");
    }

    #[test]
    fn parse_multiple_tool_calls_in_one_response() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [
                        {
                            "id": "c1",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"path\":\"a.rs\"}"
                            }
                        },
                        {
                            "id": "c2",
                            "function": {
                                "name": "grep",
                                "arguments": "{\"pattern\":\"foo\"}"
                            }
                        }
                    ]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "a.rs");
        assert_eq!(calls[1].id, "c2");
        assert_eq!(calls[1].name, "grep");
        assert_eq!(calls[1].arguments["pattern"], "foo");
    }

    #[test]
    fn parse_no_tool_calls_returns_empty() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": { "content": "just text, no calls" }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_no_tool_calls_returns_empty_when_not_array() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": { "tool_calls": "not-an-array" }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert!(calls.is_empty());
    }

    #[test]
    fn parse_non_json_response_errors() {
        let resp = BackendResponse::Text("hello".into());
        let err = OpenAiTranslator
            .parse_calls(&resp)
            .expect_err("should reject non-json");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(err.to_string().contains("expected json"));

        let stream = BackendResponse::StreamJson(Vec::new());
        let err2 = OpenAiTranslator
            .parse_calls(&stream)
            .expect_err("should reject stream-json");
        assert!(matches!(err2, TranslatorError::Malformed(_)));
    }

    #[test]
    fn parse_missing_function_name_fails() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "x",
                        "function": { "arguments": "{}" }
                    }]
                }
            }]
        }));
        let err = OpenAiTranslator
            .parse_calls(&resp)
            .expect_err("missing name should fail");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(err.to_string().contains("function.name"));
    }

    #[test]
    fn parse_malformed_arguments_json_fails() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "x",
                        "function": {
                            "name": "read_file",
                            "arguments": "{bad"
                        }
                    }]
                }
            }]
        }));
        let err = OpenAiTranslator
            .parse_calls(&resp)
            .expect_err("malformed args should fail");
        assert!(matches!(err, TranslatorError::Malformed(_)));
        assert!(err.to_string().contains("bad arguments json"));
    }

    #[test]
    fn parse_missing_arguments_defaults_to_empty_object() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "x",
                        "function": { "name": "read_file" }
                    }]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("should default arguments to {}");
        assert_eq!(calls.len(), 1);
        assert!(calls[0].arguments.is_object());
        assert_eq!(
            calls[0].arguments.as_object().map(serde_json::Map::len),
            Some(0)
        );
    }

    #[test]
    fn parse_missing_id_defaults_to_empty_string() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "function": {
                            "name": "read_file",
                            "arguments": "{}"
                        }
                    }]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "");
    }

    #[test]
    fn parse_respects_tool_call_id() {
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_abc_123",
                        "function": {
                            "name": "grep",
                            "arguments": "{\"pattern\":\"x\"}"
                        }
                    }]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc_123");
    }

    #[test]
    fn render_results_uses_role_tool_and_tool_call_id() {
        let call = ToolCall::at(
            "call_9",
            "read_file",
            serde_json::json!({"path":"a"}),
            1_700_000_000_000,
        );
        let res = ok_text("file contents");
        let rendered = OpenAiTranslator.render_results(&[(call, res)]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "tool");
        assert_eq!(arr[0]["tool_call_id"], "call_9");
        assert_eq!(arr[0]["content"], "file contents");
    }

    #[test]
    fn render_results_stringifies_err_variants() {
        let call = ToolCall::at("call_err", "bash", serde_json::json!({}), 1_700_000_000_000);
        let res = ToolResult::err(ToolError::Timeout { after_ms: 5_000 });
        let rendered = OpenAiTranslator.render_results(&[(call, res)]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        let content = arr[0]["content"].as_str().expect("content is string");
        assert!(content.starts_with("error: "));
        assert!(content.contains("5000"));
        assert_eq!(arr[0]["tool_call_id"], "call_err");
    }

    #[test]
    fn render_results_handles_empty_input() {
        let rendered = OpenAiTranslator.render_results(&[]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        assert_eq!(v.as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn round_trip_one_call() {
        // 1. Render the tool catalog.
        let tools = [tool("read_file", "Read a UTF-8 file")];
        let RenderedTools::JsonArray(rendered_tools) = OpenAiTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        assert_eq!(rendered_tools.as_array().map(Vec::len), Some(1));

        // 2. Construct a fake backend response that mirrors the catalog.
        let fake = BackendResponse::Json(serde_json::json!({
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "call_rt",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"src/lib.rs\"}"
                        }
                    }]
                }
            }]
        }));

        // 3. Parse back to canonical ToolCall.
        let calls = OpenAiTranslator
            .parse_calls(&fake)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_rt");
        assert_eq!(calls[0].name, "read_file");

        // 4. Render a result message set.
        let results = vec![(calls[0].clone(), ok_text("ok"))];
        let RenderedResults::JsonMessages(msgs) = OpenAiTranslator.render_results(&results) else {
            panic!("expected JsonMessages");
        };
        let arr = msgs.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "tool");
        assert_eq!(arr[0]["tool_call_id"], "call_rt");
        assert_eq!(arr[0]["content"], "ok");
    }

    #[test]
    fn parse_handles_nested_choices_correctly() {
        // Assert the translator walks /choices/0/message/tool_calls and
        // ignores the /message/tool_calls pointer used by Ollama.
        let resp = BackendResponse::Json(serde_json::json!({
            // Ollama-shaped decoy that should NOT be consulted.
            "message": {
                "tool_calls": [{
                    "id": "should_be_ignored",
                    "function": {
                        "name": "grep",
                        "arguments": "{\"pattern\":\"ignored\"}"
                    }
                }]
            },
            // The real OpenAI-shaped payload.
            "choices": [{
                "message": {
                    "tool_calls": [{
                        "id": "real_call",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"x\"}"
                        }
                    }]
                }
            }]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "real_call");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "x");
    }

    #[test]
    fn parse_ignores_second_choice_even_if_present() {
        // Only the first choice should be inspected per the §36.36 spec.
        let resp = BackendResponse::Json(serde_json::json!({
            "choices": [
                {
                    "message": {
                        "tool_calls": [{
                            "id": "first",
                            "function": {
                                "name": "read_file",
                                "arguments": "{}"
                            }
                        }]
                    }
                },
                {
                    "message": {
                        "tool_calls": [{
                            "id": "second",
                            "function": {
                                "name": "grep",
                                "arguments": "{}"
                            }
                        }]
                    }
                }
            ]
        }));
        let calls = OpenAiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "first");
    }
}
