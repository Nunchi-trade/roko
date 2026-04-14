//! Coverage tests for canonical streaming chunk variants.

use roko_agent::chat_types::FinishReason;
use roko_agent::{StreamChunk, Usage};

#[test]
fn stream_chunk_types_cover_glm_stream_events() {
    let chunks = vec![
        StreamChunk::ReasoningDelta("Inspecting the tool schema.".into()),
        StreamChunk::ContentDelta("Calling edit_file".into()),
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: Some("call_glm_".into()),
            name_delta: Some("edit_file".into()),
            arguments_delta: "{\"path\":\"note.txt\"".into(),
        },
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: None,
            name_delta: None,
            arguments_delta: ",\"new_string\":\"updated\"}".into(),
        },
        StreamChunk::Usage(Usage {
            input_tokens: 21,
            output_tokens: 9,
            cache_read_tokens: 4,
            ..Usage::default()
        }),
        StreamChunk::Done(FinishReason::ToolCalls),
    ];

    assert!(matches!(chunks[0], StreamChunk::ReasoningDelta(_)));
    assert!(matches!(chunks[1], StreamChunk::ContentDelta(_)));
    assert!(matches!(
        chunks[2],
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: Some(_),
            name_delta: Some(_),
            ..
        }
    ));
    assert!(matches!(
        chunks[3],
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: None,
            name_delta: None,
            ..
        }
    ));
    assert!(matches!(chunks[4], StreamChunk::Usage(_)));
    assert!(matches!(
        chunks[5],
        StreamChunk::Done(FinishReason::ToolCalls)
    ));
}

#[test]
fn stream_chunk_types_cover_kimi_stream_events() {
    let chunks = vec![
        StreamChunk::ReasoningDelta("Need to read the file first.".into()),
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: Some("functions.Read:0".into()),
            name_delta: Some("read_file".into()),
            arguments_delta: "{\"path\":\"note.txt\"}".into(),
        },
        StreamChunk::ContentDelta("I can answer now.".into()),
        StreamChunk::Usage(Usage {
            input_tokens: 17,
            output_tokens: 4,
            ..Usage::default()
        }),
        StreamChunk::Done(FinishReason::Stop),
        StreamChunk::Error("connection dropped".into()),
    ];

    assert!(matches!(chunks[0], StreamChunk::ReasoningDelta(_)));
    assert!(matches!(
        chunks[1],
        StreamChunk::ToolCallDelta {
            index: 0,
            id_delta: Some(_),
            name_delta: Some(_),
            ..
        }
    ));
    assert!(matches!(chunks[2], StreamChunk::ContentDelta(_)));
    assert!(matches!(chunks[3], StreamChunk::Usage(_)));
    assert!(matches!(chunks[4], StreamChunk::Done(FinishReason::Stop)));
    assert!(matches!(chunks[5], StreamChunk::Error(_)));
}
