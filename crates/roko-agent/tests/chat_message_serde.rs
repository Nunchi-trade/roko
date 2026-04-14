//! Serde coverage for the canonical `roko_core` chat message types.

use roko_core::{
    ChatMessage, ContentBlock, ImageUrl, MessageContent, ToolCallFunction, ToolCallMessage,
};
use serde_json::json;

#[test]
fn chat_message_serde_system_and_tool_round_trip() {
    let system = ChatMessage::System {
        content: "You are a helpful system.".to_string(),
    };
    let tool = ChatMessage::Tool {
        tool_call_id: "call_123".to_string(),
        content: "{\"ok\":true}".to_string(),
    };

    let system_json = serde_json::to_value(&system).expect("serialize system message");
    let tool_json = serde_json::to_value(&tool).expect("serialize tool message");

    assert_eq!(
        system_json,
        json!({
            "role": "system",
            "content": "You are a helpful system."
        })
    );
    assert_eq!(
        tool_json,
        json!({
            "role": "tool",
            "tool_call_id": "call_123",
            "content": "{\"ok\":true}"
        })
    );

    let parsed_system: ChatMessage =
        serde_json::from_value(system_json).expect("deserialize system message");
    let parsed_tool: ChatMessage =
        serde_json::from_value(tool_json).expect("deserialize tool message");

    assert!(matches!(parsed_system, ChatMessage::System { .. }));
    assert!(matches!(parsed_tool, ChatMessage::Tool { .. }));
}

#[test]
fn chat_message_serde_assistant_with_tool_calls() {
    let message = ChatMessage::Assistant {
        content: Some("Let me call a tool.".to_string()),
        reasoning_content: Some("Need filesystem context first.".to_string()),
        tool_calls: Some(vec![ToolCallMessage {
            id: "call_abc".to_string(),
            r#type: "function".to_string(),
            function: ToolCallFunction {
                name: "read_file".to_string(),
                arguments: "{\"path\":\"/tmp/demo.txt\"}".to_string(),
            },
        }]),
        partial: true,
    };

    let value = serde_json::to_value(&message).expect("serialize assistant message");
    assert_eq!(
        value,
        json!({
            "role": "assistant",
            "content": "Let me call a tool.",
            "reasoning_content": "Need filesystem context first.",
            "tool_calls": [{
                "id": "call_abc",
                "type": "function",
                "function": {
                    "name": "read_file",
                    "arguments": "{\"path\":\"/tmp/demo.txt\"}"
                }
            }],
            "partial": true
        })
    );

    let parsed: ChatMessage = serde_json::from_value(value).expect("deserialize assistant message");

    match parsed {
        ChatMessage::Assistant {
            content,
            reasoning_content,
            tool_calls,
            partial,
        } => {
            assert_eq!(content.as_deref(), Some("Let me call a tool."));
            assert_eq!(
                reasoning_content.as_deref(),
                Some("Need filesystem context first.")
            );
            assert!(partial);
            let calls = tool_calls.expect("assistant tool calls");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_abc");
            assert_eq!(calls[0].r#type, "function");
            assert_eq!(calls[0].function.name, "read_file");
            assert_eq!(calls[0].function.arguments, "{\"path\":\"/tmp/demo.txt\"}");
        }
        other => panic!("expected assistant message, got {other:?}"),
    }
}

#[test]
fn chat_message_serde_user_blocks_support_image_content() {
    let message = ChatMessage::User {
        content: MessageContent::Blocks(vec![
            ContentBlock::Text {
                text: "What is in this image?".to_string(),
            },
            ContentBlock::ImageUrl {
                image_url: ImageUrl {
                    url: "data:image/png;base64,AAAA".to_string(),
                },
            },
        ]),
    };

    let value = serde_json::to_value(&message).expect("serialize user blocks");
    assert_eq!(
        value,
        json!({
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "What is in this image?"
                },
                {
                    "type": "image_url",
                    "image_url": {
                        "url": "data:image/png;base64,AAAA"
                    }
                }
            ]
        })
    );

    let parsed: ChatMessage = serde_json::from_value(value).expect("deserialize user blocks");

    match parsed {
        ChatMessage::User {
            content: MessageContent::Blocks(blocks),
        } => {
            assert_eq!(blocks.len(), 2);
            assert!(matches!(blocks[0], ContentBlock::Text { .. }));
            assert!(matches!(blocks[1], ContentBlock::ImageUrl { .. }));
        }
        other => panic!("expected user blocks message, got {other:?}"),
    }
}
