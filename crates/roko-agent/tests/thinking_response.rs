use roko_agent::Usage;
use roko_agent::translate::{
    BackendResponse, ChatResponse, FinishReason, OpenAiTranslator, Translator,
    normalize_finish_reason,
};
use serde_json::Value;

fn extract_cache_read_tokens(response: &Value) -> u32 {
    let Some(usage) = response.get("usage") else {
        return 0;
    };

    let cache_read_tokens = usage
        .get("cached_tokens")
        .or_else(|| usage.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);

    u32::try_from(cache_read_tokens).unwrap_or(u32::MAX)
}

fn parse_chat_response(raw: &Value) -> ChatResponse {
    let backend = BackendResponse::Json(raw.clone());
    let content = backend.extract_text();
    let reasoning = backend.extract_reasoning();
    let tool_calls = OpenAiTranslator
        .parse_calls(&backend)
        .expect("tool calls should parse");
    let usage = Usage {
        input_tokens: raw
            .get("usage")
            .and_then(|usage| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(0),
        output_tokens: raw
            .get("usage")
            .and_then(|usage| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
            .and_then(|tokens| u32::try_from(tokens).ok())
            .unwrap_or(0),
        cache_read_tokens: extract_cache_read_tokens(raw),
        ..Default::default()
    };
    let finish_reason = raw
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(normalize_finish_reason)
        .unwrap_or_default();

    ChatResponse {
        content,
        reasoning,
        tool_calls,
        usage,
        finish_reason,
        ..Default::default()
    }
}

#[test]
fn thinking_response_parsing() {
    let raw = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "reasoning_content": "Let me think step by step...",
                "content": "The answer is 42.",
                "tool_calls": [{
                    "id": "call_1",
                    "function": {
                        "name": "Read",
                        "arguments": "{}"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "prompt_tokens_details": {
                "cached_tokens": 80
            }
        }
    });

    let response = parse_chat_response(&raw);

    assert_eq!(
        response.reasoning,
        Some("Let me think step by step...".to_string())
    );
    assert_eq!(response.content, "The answer is 42.");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.usage.cache_read_tokens, 80);
    assert!(matches!(response.finish_reason, FinishReason::ToolCalls));
}
