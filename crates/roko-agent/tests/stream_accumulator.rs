use roko_agent::chat_types::FinishReason;
use roko_agent::{StreamAccumulator, StreamChunk, Usage};

fn chunk_string(input: &str, parts: usize) -> Vec<String> {
    assert!(parts > 0);
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut chunks = Vec::with_capacity(parts);
    let mut start = 0;

    for index in 0..parts {
        let end = ((index + 1) * len) / parts;
        chunks.push(chars[start..end].iter().collect());
        start = end;
    }

    chunks
}

#[test]
fn stream_accumulator_builds_chat_response() {
    let mut accumulator = StreamAccumulator::new();

    for chunk in [
        StreamChunk::ReasoningDelta("Need to inspect the file first. ".into()),
        StreamChunk::ReasoningDelta("Then call the tool.".into()),
        StreamChunk::ContentDelta("Calling ".into()),
        StreamChunk::ContentDelta("read_file".into()),
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: Some("call_1".into()),
            name_delta: Some("read_file".into()),
            arguments_delta: "{\"path\":\"src/main.rs\"".into(),
        },
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: None,
            name_delta: None,
            arguments_delta: ",\"line\":10}".into(),
        },
        StreamChunk::Usage(Usage {
            input_tokens: 42,
            output_tokens: 11,
            cache_read_tokens: 7,
            ..Usage::default()
        }),
        StreamChunk::Done(FinishReason::ToolCalls),
        StreamChunk::Error("socket closed after done".into()),
    ] {
        accumulator.push(chunk);
    }

    let response = accumulator.finalize();

    assert_eq!(
        response.reasoning.as_deref(),
        Some("Need to inspect the file first. Then call the tool.")
    );
    assert_eq!(response.content, "Calling read_file");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].id, "call_1");
    assert_eq!(response.tool_calls[0].name, "read_file");
    assert_eq!(
        response.tool_calls[0].arguments,
        serde_json::json!({ "path": "src/main.rs", "line": 10 })
    );
    assert_eq!(response.usage.input_tokens, 42);
    assert_eq!(response.usage.output_tokens, 11);
    assert_eq!(response.usage.cache_read_tokens, 7);
    assert!(matches!(response.finish_reason, FinishReason::ToolCalls));
    assert!(response.raw_assistant_message.is_some());
}

#[test]
fn stream_accumulator_handles_hundred_chunks() {
    let mut accumulator = StreamAccumulator::new();

    let reasoning = (0..24)
        .map(|index| format!("reason-{index} "))
        .collect::<String>();
    let content = (0..24)
        .map(|index| format!("content-{index} "))
        .collect::<String>();
    let tool_1_args = serde_json::json!({
        "path": "src/main.rs",
        "line": 10,
        "mode": "read"
    })
    .to_string();
    let tool_2_args = serde_json::json!({
        "path": "Cargo.toml",
        "new_string": "workspace = true",
        "replace_all": false
    })
    .to_string();

    for part in chunk_string(&reasoning, 24) {
        accumulator.push(StreamChunk::ReasoningDelta(part));
    }
    for part in chunk_string(&content, 24) {
        accumulator.push(StreamChunk::ContentDelta(part));
    }

    let tool_1_chunks = chunk_string(&tool_1_args, 25);
    for (index, part) in tool_1_chunks.into_iter().enumerate() {
        accumulator.push(StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: (index == 0).then_some("call_1".into()),
            name_delta: (index == 0).then_some("read_file".into()),
            arguments_delta: part,
        });
    }

    let tool_2_chunks = chunk_string(&tool_2_args, 25);
    for (index, part) in tool_2_chunks.into_iter().enumerate() {
        accumulator.push(StreamChunk::ToolCallDelta {
            index: 1,
            id_delta: (index == 0).then_some("call_2".into()),
            name_delta: (index == 0).then_some("edit_file".into()),
            arguments_delta: part,
        });
    }

    accumulator.push(StreamChunk::Usage(Usage {
        input_tokens: 120,
        output_tokens: 80,
        cache_read_tokens: 12,
        ..Usage::default()
    }));
    accumulator.push(StreamChunk::Done(FinishReason::Stop));

    let response = accumulator.finalize();

    assert_eq!(24 + 24 + 25 + 25 + 1 + 1, 100);
    assert_eq!(response.reasoning.as_deref(), Some(reasoning.as_str()));
    assert_eq!(response.content, content);
    assert_eq!(response.tool_calls.len(), 2);
    assert_eq!(
        response.tool_calls[0].arguments,
        serde_json::from_str::<serde_json::Value>(&tool_1_args).expect("tool 1 args json")
    );
    assert_eq!(
        response.tool_calls[1].arguments,
        serde_json::from_str::<serde_json::Value>(&tool_2_args).expect("tool 2 args json")
    );
    assert_eq!(response.usage.input_tokens, 120);
    assert_eq!(response.usage.output_tokens, 80);
    assert_eq!(response.usage.cache_read_tokens, 12);
    assert!(matches!(response.finish_reason, FinishReason::Stop));
}
