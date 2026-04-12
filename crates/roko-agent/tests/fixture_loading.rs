use std::fs;
use std::path::{Path, PathBuf};

use roko_agent::provider::{OpenAiCompatAdapter, ProviderAdapter, ProviderError};
use roko_agent::translate::openai::parse_glm_metadata;
use roko_agent::translate::{
    BackendResponse, FinishReason, OpenAiTranslator, Translator, normalize_finish_reason,
};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn load_fixture(relative_path: &str) -> Value {
    let path = fixtures_dir().join(relative_path);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read fixture {}: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse fixture {}: {err}", path.display()))
}

fn finish_reason(raw: &Value) -> FinishReason {
    raw.pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .map(normalize_finish_reason)
        .unwrap_or_default()
}

#[test]
fn fixture_loading_reads_every_recorded_response_body() {
    let expected = [
        "glm-5.1/simple_response.json",
        "glm-5.1/tool_call_response.json",
        "glm-5.1/thinking_response.json",
        "glm-5.1/web_search_response.json",
        "glm-5.1/error_rate_limit.json",
        "kimi-k2.5/simple_response.json",
        "kimi-k2.5/tool_call_response.json",
        "kimi-k2.5/thinking_response.json",
        "kimi-k2.5/partial_truncated.json",
        "kimi-k2.5/vision_response.json",
        "openrouter/glm_via_openrouter.json",
        "openrouter/fallback_different_model.json",
        "common/429_rate_limit.json",
        "common/401_auth_failure.json",
        "common/500_server_error.json",
    ];

    for relative_path in expected {
        let fixture = load_fixture(relative_path);
        assert!(
            fixture.is_object(),
            "fixture {relative_path} should decode as a JSON object"
        );
    }
}

#[test]
fn fixture_loading_parses_recorded_provider_success_shapes() {
    let glm_simple = load_fixture("glm-5.1/simple_response.json");
    let glm_simple_backend = BackendResponse::Json(glm_simple.clone());
    assert_eq!(glm_simple_backend.extract_text(), "Mock GLM response.");
    assert_eq!(glm_simple_backend.extract_usage().cache_read_tokens, 3);
    assert!(matches!(finish_reason(&glm_simple), FinishReason::Stop));

    let glm_tool = load_fixture("glm-5.1/tool_call_response.json");
    let glm_tool_calls = OpenAiTranslator
        .parse_calls(&BackendResponse::Json(glm_tool.clone()))
        .expect("GLM tool calls should parse");
    assert_eq!(glm_tool_calls.len(), 1);
    assert_eq!(glm_tool_calls[0].id, "call-glm-read-1");
    assert_eq!(glm_tool_calls[0].name, "Read");
    assert!(matches!(finish_reason(&glm_tool), FinishReason::ToolCalls));

    let glm_thinking = load_fixture("glm-5.1/thinking_response.json");
    let glm_thinking_backend = BackendResponse::Json(glm_thinking.clone());
    assert_eq!(
        glm_thinking_backend.extract_reasoning().as_deref(),
        Some("Need to compare both options before answering.")
    );
    assert_eq!(
        glm_thinking_backend.extract_text(),
        "Choose the cheaper healthy route."
    );

    let glm_web_search = load_fixture("glm-5.1/web_search_response.json");
    let glm_metadata = parse_glm_metadata(&glm_web_search);
    assert_eq!(glm_metadata.model_used.as_deref(), Some("glm-5.1"));
    assert_eq!(
        glm_metadata
            .web_search
            .as_ref()
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );

    let kimi_simple = load_fixture("kimi-k2.5/simple_response.json");
    let kimi_simple_backend = BackendResponse::Json(kimi_simple.clone());
    assert_eq!(kimi_simple_backend.extract_text(), "Mock Kimi response.");
    assert_eq!(kimi_simple_backend.extract_usage().cache_read_tokens, 2);

    let kimi_tool = load_fixture("kimi-k2.5/tool_call_response.json");
    let kimi_tool_calls = OpenAiTranslator
        .parse_calls(&BackendResponse::Json(kimi_tool.clone()))
        .expect("Kimi tool calls should parse");
    assert_eq!(kimi_tool_calls.len(), 1);
    assert_eq!(kimi_tool_calls[0].id, "functions.Read:0");
    assert_eq!(kimi_tool_calls[0].name, "Read");

    let kimi_thinking = load_fixture("kimi-k2.5/thinking_response.json");
    let kimi_thinking_backend = BackendResponse::Json(kimi_thinking);
    assert_eq!(
        kimi_thinking_backend.extract_reasoning().as_deref(),
        Some("I should inspect the file before editing it.")
    );

    let kimi_partial = load_fixture("kimi-k2.5/partial_truncated.json");
    let kimi_partial_backend = BackendResponse::Json(kimi_partial.clone());
    assert_eq!(
        kimi_partial_backend.extract_text(),
        "This is the first half of the answer "
    );
    assert!(matches!(finish_reason(&kimi_partial), FinishReason::Length));

    let kimi_vision = load_fixture("kimi-k2.5/vision_response.json");
    let kimi_vision_backend = BackendResponse::Json(kimi_vision);
    assert!(
        kimi_vision_backend
            .extract_text()
            .contains("dashboard with a red error banner")
    );

    let openrouter_glm = load_fixture("openrouter/glm_via_openrouter.json");
    let openrouter_glm_metadata = parse_glm_metadata(&openrouter_glm);
    assert_eq!(
        openrouter_glm_metadata.model_used.as_deref(),
        Some("z-ai/glm-5.1")
    );

    let openrouter_fallback = load_fixture("openrouter/fallback_different_model.json");
    let openrouter_fallback_metadata = parse_glm_metadata(&openrouter_fallback);
    assert_eq!(
        openrouter_fallback_metadata.model_used.as_deref(),
        Some("z-ai/glm-5")
    );
}

#[test]
fn fixture_loading_classifies_recorded_error_bodies() {
    let adapter = OpenAiCompatAdapter;

    let common_rate_limit = load_fixture("common/429_rate_limit.json");
    match adapter.classify_error(429, &common_rate_limit) {
        ProviderError::RateLimit {
            retry_after_ms: Some(ms),
        } => assert_eq!(ms, 12_000),
        other => panic!("unexpected common rate-limit classification: {other:?}"),
    }

    let common_auth = load_fixture("common/401_auth_failure.json");
    assert!(matches!(
        adapter.classify_error(401, &common_auth),
        ProviderError::AuthFailure
    ));

    let common_server = load_fixture("common/500_server_error.json");
    assert!(matches!(
        adapter.classify_error(500, &common_server),
        ProviderError::ServerError(500)
    ));

    let glm_rate_limit = load_fixture("glm-5.1/error_rate_limit.json");
    match adapter.classify_error(429, &glm_rate_limit) {
        ProviderError::RateLimit {
            retry_after_ms: Some(ms),
        } => assert_eq!(ms, 5_000),
        other => panic!("unexpected GLM rate-limit classification: {other:?}"),
    }
}
