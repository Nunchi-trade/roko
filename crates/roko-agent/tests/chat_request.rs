use roko_agent::{ChatRequest, RequestOptions, ResponseFormat, ToolChoice};
use roko_core::{ChatMessage, ContentBlock, MessageContent, ToolCategory, ToolDef, ToolPermission};
use serde_json::json;

fn read_tool(name: &str) -> ToolDef {
    ToolDef::new(
        name,
        "test tool",
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
}

#[test]
fn chat_request_supports_openai_compat_fields() {
    let request = ChatRequest {
        messages: vec![
            ChatMessage::System {
                content: "System".to_string(),
            },
            ChatMessage::User {
                content: MessageContent::Text("Hello".to_string()),
            },
        ],
        model_slug: "glm-5.1".to_string(),
        tools: vec![read_tool("read_file")],
        tool_choice: ToolChoice::Auto,
        max_tokens: Some(4096),
        temperature: Some(0.2),
        top_p: Some(0.95),
        stop: Some(vec!["</tool>".to_string()]),
        stream: true,
        options: RequestOptions {
            enable_thinking: Some(true),
            preserve_thinking: Some(true),
            enable_tool_streaming: Some(true),
            cache_key: Some("prompt-cache".to_string()),
            response_format: Some(ResponseFormat::JsonObject),
            extra: std::collections::HashMap::from([
                ("thinking".to_string(), json!({ "type": "enabled" })),
                ("tool_stream".to_string(), json!(true)),
            ]),
        },
    };

    assert_eq!(request.messages.len(), 2);
    assert_eq!(request.model_slug, "glm-5.1");
    assert_eq!(request.tools.len(), 1);
    assert!(matches!(request.tool_choice, ToolChoice::Auto));
    assert_eq!(request.max_tokens, Some(4096));
    assert_eq!(request.temperature, Some(0.2));
    assert_eq!(request.top_p, Some(0.95));
    assert_eq!(request.stop.as_deref(), Some(&["</tool>".to_string()][..]));
    assert!(request.stream);
    assert_eq!(request.options.enable_thinking, Some(true));
    assert_eq!(request.options.preserve_thinking, Some(true));
    assert_eq!(request.options.enable_tool_streaming, Some(true));
    assert_eq!(request.options.cache_key.as_deref(), Some("prompt-cache"));
    assert!(matches!(
        request.options.response_format,
        Some(ResponseFormat::JsonObject)
    ));
    assert_eq!(
        request.options.extra.get("thinking"),
        Some(&json!({ "type": "enabled" }))
    );
    assert_eq!(request.options.extra.get("tool_stream"), Some(&json!(true)));
}

#[test]
fn chat_request_supports_anthropic_claude_cli_and_cursor_variants() {
    let anthropic = ChatRequest {
        messages: vec![ChatMessage::User {
            content: MessageContent::Text("Summarize this".to_string()),
        }],
        model_slug: "claude-sonnet-4-5".to_string(),
        tools: vec![read_tool("grep")],
        tool_choice: ToolChoice::Specific {
            name: "grep".to_string(),
        },
        max_tokens: Some(8192),
        temperature: None,
        top_p: None,
        stop: None,
        stream: false,
        options: RequestOptions::default(),
    };

    let claude_cli = ChatRequest {
        messages: vec![ChatMessage::User {
            content: MessageContent::Text("Inspect the repo".to_string()),
        }],
        model_slug: "claude-opus-4-1".to_string(),
        tools: vec![read_tool("glob")],
        tool_choice: ToolChoice::Required,
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop: None,
        stream: true,
        options: RequestOptions {
            response_format: Some(ResponseFormat::Text),
            ..RequestOptions::default()
        },
    };

    let cursor = ChatRequest {
        messages: vec![ChatMessage::User {
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: "Review this code".to_string(),
            }]),
        }],
        model_slug: "kimi-k2.5".to_string(),
        tools: vec![],
        tool_choice: ToolChoice::None,
        max_tokens: Some(2048),
        temperature: Some(0.0),
        top_p: Some(1.0),
        stop: Some(vec!["STOP".to_string()]),
        stream: false,
        options: RequestOptions {
            extra: std::collections::HashMap::from([("partial".to_string(), json!(true))]),
            ..RequestOptions::default()
        },
    };

    assert!(matches!(
        anthropic.tool_choice,
        ToolChoice::Specific { ref name } if name == "grep"
    ));
    assert_eq!(anthropic.max_tokens, Some(8192));

    assert!(matches!(claude_cli.tool_choice, ToolChoice::Required));
    assert!(claude_cli.stream);
    assert!(matches!(
        claude_cli.options.response_format,
        Some(ResponseFormat::Text)
    ));

    assert!(matches!(cursor.tool_choice, ToolChoice::None));
    assert_eq!(cursor.stop.as_deref(), Some(&["STOP".to_string()][..]));
    assert_eq!(cursor.options.extra.get("partial"), Some(&json!(true)));
}
