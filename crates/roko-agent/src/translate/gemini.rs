//! Gemini-native tool translator.
//!
//! Gemini's `generateContent` API uses a distinct native tool format:
//! request tools live under `functionDeclarations`, tool calls arrive as
//! `functionCall` parts inside `candidates[0].content.parts`, and tool
//! results are sent back as `functionResponse` parts on a follow-up
//! `user` content item.

use crate::gemini::types::FunctionDeclaration;
use roko_core::tool::{ToolCall, ToolDef, ToolFormat, ToolResult};
use serde_json::{Value, json};

use super::{BackendResponse, RenderedResults, RenderedTools, Translator, TranslatorError};

/// Translator for Gemini native `generateContent` tool calling.
#[derive(Debug, Default, Clone, Copy)]
pub struct GeminiTranslator;

impl Translator for GeminiTranslator {
    fn format(&self) -> ToolFormat {
        ToolFormat::Custom("gemini_native".to_string())
    }

    fn render_tools(&self, tools: &[ToolDef]) -> RenderedTools {
        let declarations: Vec<FunctionDeclaration> = tools
            .iter()
            .map(|tool| FunctionDeclaration {
                name: tool.name.clone(),
                description: tool.description.clone(),
                parameters: tool.parameters.as_value().clone(),
            })
            .collect();

        RenderedTools::JsonArray(json!([{
            "functionDeclarations": declarations
        }]))
    }

    fn parse_calls(&self, response: &BackendResponse) -> Result<Vec<ToolCall>, TranslatorError> {
        let BackendResponse::Json(json) = response else {
            return Err(TranslatorError::Malformed("expected json".into()));
        };

        if let Some(function_calls) = json.get("functionCalls").and_then(Value::as_array) {
            return parse_function_calls(function_calls);
        }

        let Some(parts) = json
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
        else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        for part in parts {
            let Some(function_call) = part.get("functionCall") else {
                continue;
            };

            let name = function_call
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| TranslatorError::Malformed("missing functionCall.name".into()))?
                .to_string();
            let args = function_call
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let id = function_call
                .get("id")
                .and_then(Value::as_str)
                .map_or_else(|| format!("call_{}", out.len()), ToString::to_string);
            out.push(ToolCall::new(id, name, args));
        }

        Ok(out)
    }

    fn render_results(&self, results: &[(ToolCall, ToolResult)]) -> RenderedResults {
        let messages: Vec<Value> = results
            .iter()
            .map(|(call, result)| {
                json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": call.name,
                            "response": render_response_payload(result),
                            "id": (!call.id.is_empty()).then_some(call.id.as_str()),
                        }
                    }]
                })
            })
            .collect();
        RenderedResults::JsonMessages(Value::Array(messages))
    }

    fn render_assistant_message(&self, response: &BackendResponse) -> Option<Value> {
        let BackendResponse::Json(json) = response else {
            return None;
        };
        json.pointer("/candidates/0/content").cloned()
    }
}

fn parse_function_calls(function_calls: &[Value]) -> Result<Vec<ToolCall>, TranslatorError> {
    let mut out = Vec::with_capacity(function_calls.len());
    for function_call in function_calls {
        let name = function_call
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| TranslatorError::Malformed("missing functionCall.name".into()))?
            .to_string();
        let args = function_call
            .get("args")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let id = function_call
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| format!("call_{}", out.len()), ToString::to_string);
        out.push(ToolCall::new(id, name, args));
    }
    Ok(out)
}

fn render_response_payload(result: &ToolResult) -> Value {
    match result {
        ToolResult::Ok {
            content,
            is_structured,
            ..
        } if *is_structured => serde_json::from_str(content).unwrap_or_else(|_| {
            json!({
                "result": content,
            })
        }),
        ToolResult::Ok { content, .. } => json!({
            "result": content,
        }),
        ToolResult::Err(error) => json!({
            "error": error.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolError, ToolPermission, ToolSchema};

    fn tool(name: &str, desc: &str) -> ToolDef {
        ToolDef::new(name, desc, ToolCategory::Read, ToolPermission::read_only()).with_parameters(
            ToolSchema::from_value(json!({
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
    fn gemini_translator_format_returns_gemini_native() {
        assert_eq!(
            GeminiTranslator.format(),
            ToolFormat::Custom("gemini_native".to_string())
        );
    }

    #[test]
    fn gemini_translator_render_tools_uses_function_declarations() {
        let tools = [tool("read_file", "Read a UTF-8 file")];
        let RenderedTools::JsonArray(v) = GeminiTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["functionDeclarations"][0]["name"], "read_file");
        assert_eq!(
            arr[0]["functionDeclarations"][0]["parameters"]["required"],
            json!(["path"])
        );
    }

    #[test]
    fn gemini_translator_parse_calls_reads_function_call_parts_and_preserves_id() {
        let resp = BackendResponse::Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        { "text": "Let me inspect that." },
                        {
                            "functionCall": {
                                "name": "read_file",
                                "args": { "path": "src/lib.rs" },
                                "id": "gemini-call-7"
                            }
                        }
                    ]
                }
            }]
        }));

        let calls = GeminiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "gemini-call-7");
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments, json!({ "path": "src/lib.rs" }));
    }

    #[test]
    fn gemini_translator_parse_calls_falls_back_to_generated_ids_when_missing() {
        let resp = BackendResponse::Json(json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {
                            "functionCall": {
                                "name": "read_file",
                                "args": { "path": "a.rs" }
                            }
                        },
                        {
                            "functionCall": {
                                "name": "read_file",
                                "args": { "path": "b.rs" }
                            }
                        }
                    ]
                }
            }]
        }));

        let calls = GeminiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[1].id, "call_1");
    }

    #[test]
    fn gemini_translator_parse_calls_handles_sdk_style_function_calls_array() {
        let resp = BackendResponse::Json(json!({
            "functionCalls": [
                {
                    "name": "read_file",
                    "args": { "path": "Cargo.toml" },
                    "id": "sdk-call"
                }
            ]
        }));

        let calls = GeminiTranslator
            .parse_calls(&resp)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "sdk-call");
        assert_eq!(calls[0].arguments["path"], "Cargo.toml");
    }

    #[test]
    fn gemini_translator_parse_calls_errors_when_function_call_name_missing() {
        let resp = BackendResponse::Json(json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "args": { "path": "src/lib.rs" }
                        }
                    }]
                }
            }]
        }));

        let err = GeminiTranslator
            .parse_calls(&resp)
            .expect_err("parse should fail");
        assert!(matches!(err, TranslatorError::Malformed(_)));
    }

    #[test]
    fn gemini_translator_render_results_emits_user_function_response_messages() {
        let call = ToolCall::at(
            "call_9",
            "read_file",
            json!({"path":"a"}),
            1_700_000_000_000,
        );
        let rendered = GeminiTranslator.render_results(&[(call, ok_text("file contents"))]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "user");
        assert_eq!(arr[0]["parts"][0]["functionResponse"]["name"], "read_file");
        assert_eq!(arr[0]["parts"][0]["functionResponse"]["id"], "call_9");
        assert_eq!(
            arr[0]["parts"][0]["functionResponse"]["response"],
            json!({ "result": "file contents" })
        );
    }

    #[test]
    fn gemini_translator_render_results_wraps_errors_for_model_consumption() {
        let call = ToolCall::at("call_err", "bash", json!({}), 1_700_000_000_000);
        let rendered = GeminiTranslator.render_results(&[(
            call,
            ToolResult::err(ToolError::Timeout { after_ms: 5_000 }),
        )]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        let response = &v.as_array().expect("array")[0]["parts"][0]["functionResponse"]["response"];
        let error = response["error"].as_str().expect("error string");
        assert!(error.contains("5000"));
    }

    #[test]
    fn gemini_translator_render_results_preserves_structured_payloads() {
        let call = ToolCall::at("call_struct", "read_file", json!({}), 1_700_000_000_000);
        let rendered = GeminiTranslator.render_results(&[(
            call,
            ToolResult::structured(r#"{"path":"src/lib.rs","size":123}"#),
        )]);
        let RenderedResults::JsonMessages(v) = rendered else {
            panic!("expected JsonMessages");
        };
        assert_eq!(
            v.as_array().expect("array")[0]["parts"][0]["functionResponse"]["response"],
            json!({"path":"src/lib.rs","size":123})
        );
    }

    #[test]
    fn gemini_translator_render_assistant_message_returns_candidate_content() {
        let resp = BackendResponse::Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "src/lib.rs" },
                            "id": "call-1"
                        }
                    }]
                }
            }]
        }));

        let assistant = GeminiTranslator
            .render_assistant_message(&resp)
            .expect("assistant message");
        assert_eq!(assistant["role"], "model");
        assert_eq!(assistant["parts"][0]["functionCall"]["id"], "call-1");
    }

    #[test]
    fn gemini_translator_round_trip_one_call() {
        let tools = [tool("read_file", "Read a UTF-8 file")];
        let RenderedTools::JsonArray(rendered_tools) = GeminiTranslator.render_tools(&tools) else {
            panic!("expected JsonArray");
        };
        assert_eq!(
            rendered_tools[0]["functionDeclarations"][0]["name"],
            "read_file"
        );

        let fake = BackendResponse::Json(json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "read_file",
                            "args": { "path": "src/lib.rs" },
                            "id": "call_rt"
                        }
                    }]
                }
            }]
        }));

        let calls = GeminiTranslator
            .parse_calls(&fake)
            .expect("parse should succeed");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_rt");
        assert_eq!(calls[0].name, "read_file");

        let results = vec![(calls[0].clone(), ok_text("ok"))];
        let RenderedResults::JsonMessages(msgs) = GeminiTranslator.render_results(&results) else {
            panic!("expected JsonMessages");
        };
        let arr = msgs.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["role"], "user");
        assert_eq!(arr[0]["parts"][0]["functionResponse"]["id"], "call_rt");
        assert_eq!(
            arr[0]["parts"][0]["functionResponse"]["response"],
            json!({ "result": "ok" })
        );
    }
}
