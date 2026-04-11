use roko_agent::chat_types::ChatResponse;
use roko_core::tool::ToolCall;
use roko_core::ChatMessage;

#[test]
fn chat_response_to_message_preserves_tool_calls() {
    let response = ChatResponse {
        content: "Done".to_string(),
        reasoning: Some("Need to inspect the file first.".to_string()),
        tool_calls: vec![ToolCall::at(
            "call_1",
            "read_file",
            serde_json::json!({ "path": "src/main.rs", "line": 10 }),
            1_700_000_000_000,
        )],
        ..Default::default()
    };

    let message = response.as_assistant_message();

    match message {
        ChatMessage::Assistant {
            content,
            reasoning_content,
            tool_calls,
            partial,
        } => {
            assert_eq!(content.as_deref(), Some("Done"));
            assert_eq!(
                reasoning_content.as_deref(),
                Some("Need to inspect the file first.")
            );
            assert!(!partial);

            let tool_calls = tool_calls.expect("assistant tool calls");
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].id, "call_1");
            assert_eq!(tool_calls[0].r#type, "function");
            assert_eq!(tool_calls[0].function.name, "read_file");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&tool_calls[0].function.arguments)
                    .expect("tool arguments json"),
                serde_json::json!({ "path": "src/main.rs", "line": 10 })
            );
        }
        other => panic!("expected assistant message, got {other:?}"),
    }
}
