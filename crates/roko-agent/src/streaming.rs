//! Typed streaming events for provider adapters and tool loops.

use crate::chat_types::FinishReason;
use crate::usage::Usage;

/// Incremental stream events normalized across GLM and Kimi responses.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Incremental reasoning or thinking text.
    ReasoningDelta(String),
    /// Incremental assistant-visible content text.
    ContentDelta(String),
    /// Incremental function-call data for one tool call slot.
    ToolCallDelta {
        /// Zero-based tool call index within the current assistant turn.
        index: usize,
        /// Incremental tool call identifier fragment, if the provider streamed one.
        id_delta: Option<String>,
        /// Incremental function name fragment, if the provider streamed one.
        name_delta: Option<String>,
        /// Incremental JSON argument text for the tool call.
        arguments_delta: String,
    },
    /// Token accounting emitted during or after the stream.
    Usage(Usage),
    /// Terminal stream marker with the canonical finish reason.
    Done(FinishReason),
    /// Terminal provider or transport error surfaced as a stream event.
    Error(String),
}
