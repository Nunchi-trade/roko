#![allow(missing_docs)]

use roko_agent::translate::openai::parse_glm_metadata;
use roko_agent::translate::{BackendResponse, OpenAiTranslator, Translator};
use serde_json::{Value, json};

fn load_response(fixture: &str) -> BackendResponse {
    let json: Value = serde_json::from_str(fixture).unwrap();
    BackendResponse::Json(json)
}

#[test]
fn glm_tool_call_fixture_parses() {
    let response = load_response(include_str!("fixtures/glm-5.1/tool_call_response.json"));
    let translator = OpenAiTranslator;

    let calls = translator.parse_calls(&response).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "Read");
    assert_eq!(calls[0].arguments, json!({ "path": "/tmp/note.txt" }));
}

#[test]
fn kimi_thinking_fixture_parses() {
    let fixture = include_str!("fixtures/kimi-k2.5/thinking_response.json");
    let json: Value = serde_json::from_str(fixture).unwrap();
    let reasoning = json
        .pointer("/choices/0/message/reasoning_content")
        .and_then(Value::as_str);
    assert!(reasoning.is_some());

    let response = BackendResponse::Json(json);
    let translator = OpenAiTranslator;

    let calls = translator.parse_calls(&response).unwrap();
    assert!(calls.is_empty());
    assert_eq!(
        response.extract_reasoning().as_deref(),
        Some("I should inspect the file before editing it.")
    );
}

#[test]
fn glm_recorded_success_fixtures_parse_without_errors() {
    struct Case {
        fixture: &'static str,
        response_text: &'static str,
        reasoning: Option<&'static str>,
        tool_calls: usize,
    }

    let cases = [
        Case {
            fixture: include_str!("fixtures/glm-5.1/simple_response.json"),
            response_text: "Mock GLM response.",
            reasoning: None,
            tool_calls: 0,
        },
        Case {
            fixture: include_str!("fixtures/glm-5.1/thinking_response.json"),
            response_text: "Choose the cheaper healthy route.",
            reasoning: Some("Need to compare both options before answering."),
            tool_calls: 0,
        },
        Case {
            fixture: include_str!("fixtures/glm-5.1/web_search_response.json"),
            response_text: "Here are the search results.",
            reasoning: None,
            tool_calls: 0,
        },
    ];

    let translator = OpenAiTranslator;

    for case in cases {
        let response = load_response(case.fixture);
        let calls = translator.parse_calls(&response).unwrap();

        assert_eq!(response.extract_text(), case.response_text);
        assert_eq!(response.extract_reasoning().as_deref(), case.reasoning);
        assert_eq!(calls.len(), case.tool_calls);
    }

    let web_search_json: Value =
        serde_json::from_str(include_str!("fixtures/glm-5.1/web_search_response.json")).unwrap();
    let metadata = parse_glm_metadata(&web_search_json);
    assert_eq!(metadata.model_used.as_deref(), Some("glm-5.1"));
    assert_eq!(
        metadata
            .web_search
            .as_ref()
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn kimi_recorded_success_fixtures_parse_without_errors() {
    struct Case {
        fixture: &'static str,
        response_text: &'static str,
        reasoning: Option<&'static str>,
        tool_calls: usize,
    }

    let cases = [
        Case {
            fixture: include_str!("fixtures/kimi-k2.5/simple_response.json"),
            response_text: "Mock Kimi response.",
            reasoning: None,
            tool_calls: 0,
        },
        Case {
            fixture: include_str!("fixtures/kimi-k2.5/tool_call_response.json"),
            response_text: "",
            reasoning: None,
            tool_calls: 1,
        },
        Case {
            fixture: include_str!("fixtures/kimi-k2.5/partial_truncated.json"),
            response_text: "This is the first half of the answer ",
            reasoning: None,
            tool_calls: 0,
        },
        Case {
            fixture: include_str!("fixtures/kimi-k2.5/vision_response.json"),
            response_text: "The image shows a dashboard with a red error banner and three status cards.",
            reasoning: None,
            tool_calls: 0,
        },
    ];

    let translator = OpenAiTranslator;

    for case in cases {
        let response = load_response(case.fixture);
        let calls = translator.parse_calls(&response).unwrap();

        assert_eq!(response.extract_text(), case.response_text);
        assert_eq!(response.extract_reasoning().as_deref(), case.reasoning);
        assert_eq!(calls.len(), case.tool_calls);
    }
}

#[test]
fn openrouter_recorded_success_fixtures_parse_without_errors() {
    struct Case {
        fixture: &'static str,
        response_text: &'static str,
        model_used: &'static str,
    }

    let cases = [
        Case {
            fixture: include_str!("fixtures/openrouter/glm_via_openrouter.json"),
            response_text: "OpenRouter routed to GLM.",
            model_used: "z-ai/glm-5.1",
        },
        Case {
            fixture: include_str!("fixtures/openrouter/fallback_different_model.json"),
            response_text: "OpenRouter fell back to glm-5.",
            model_used: "z-ai/glm-5",
        },
    ];

    let translator = OpenAiTranslator;

    for case in cases {
        let response = load_response(case.fixture);
        let calls = translator.parse_calls(&response).unwrap();
        let json: Value = serde_json::from_str(case.fixture).unwrap();
        let metadata = parse_glm_metadata(&json);

        assert_eq!(response.extract_text(), case.response_text);
        assert!(calls.is_empty());
        assert_eq!(metadata.model_used.as_deref(), Some(case.model_used));
    }
}
