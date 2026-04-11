use roko_agent::chat_types::{ChatResponse, FinishReason, ResponseMetadata};
use roko_agent::{ChatRequest, RequestOptions};
use roko_core::{Body, ChatMessage, Kind, MessageContent, Signal};

#[test]
fn signal_chat_roundtrip() {
    let input = Signal::builder(Kind::Prompt)
        .body(Body::text("Preserve this content"))
        .build();

    let request = ChatRequest::from_signal(
        &input,
        "glm-5.1",
        Some("System instructions"),
        Vec::new(),
        RequestOptions::default(),
    );

    assert_eq!(request.model_slug, "glm-5.1");
    assert_eq!(request.messages.len(), 2);
    assert!(matches!(
        request.messages.first(),
        Some(ChatMessage::System { content }) if content == "System instructions"
    ));
    assert!(matches!(
        request.messages.get(1),
        Some(ChatMessage::User {
            content: MessageContent::Text(content),
        }) if content == "Preserve this content"
    ));

    let response = ChatResponse {
        content: "Preserve this content".to_string(),
        finish_reason: FinishReason::Stop,
        metadata: ResponseMetadata {
            model_used: Some(request.model_slug.clone()),
            ..Default::default()
        },
        ..Default::default()
    };

    let output = response.to_signal();

    assert_eq!(output.kind, Kind::AgentOutput);
    assert_eq!(
        output.body.as_text().expect("agent output text body"),
        "Preserve this content"
    );
    assert_eq!(output.tag("model"), Some("glm-5.1"));
    assert_eq!(output.tag("finish_reason"), Some("Stop"));
}
