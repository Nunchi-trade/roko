//! Provider-neutral runtime events emitted by agent dispatch backends.
//!
//! The runner historically consumed Claude-CLI specific event shapes parsed
//! out of `--output-format stream-json`. This module defines an
//! [`AgentRuntimeEvent`] that all provider adapters can emit so the runner
//! never has to import provider-specific types.
//!
//! ## Layering
//!
//! - Provider adapters that already speak streaming (e.g. the Claude CLI
//!   `stream-json` parser in [`crate::provider::claude_cli::stream`]) translate
//!   their wire format into [`AgentRuntimeEvent`]s.
//! - One-shot providers that return a single
//!   [`AgentResult`](crate::AgentResult) emit a synthetic sequence of
//!   `Started -> MessageDelta -> TokenUsage -> TurnCompleted -> Exited`.
//! - The runner only ever sees [`AgentRuntimeEvent`] so swapping providers
//!   never requires runner changes.
//!
//! The variants intentionally mirror the legacy runner `AgentEvent` shape so
//! the relocation is mechanical for callers; future variants may be added
//! (cost deltas, retry hints, structured tool diffs, ...) without breaking
//! existing consumers.

use serde::{Deserialize, Serialize};

/// A provider-neutral event emitted while a dispatched agent is running.
///
/// Every dispatch path is expected to:
/// - emit exactly one [`AgentRuntimeEvent::Started`] before any output,
/// - emit any number of `SystemInit` / `MessageDelta` / `ToolCall` /
///   `ToolOutput` / `TokenUsage` events while the agent is running,
/// - emit a [`AgentRuntimeEvent::TurnCompleted`] when an assistant turn
///   finishes (carries the authoritative cost/turn count),
/// - emit a [`AgentRuntimeEvent::Exited`] (or [`AgentRuntimeEvent::Error`]
///   followed by `Exited`) when the underlying executor stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AgentRuntimeEvent {
    /// The runtime has launched an agent and bound it to a transport.
    Started {
        /// Stable identifier the dispatcher assigned to this run.
        agent_id: String,
        /// Provider label as reported by the dispatcher (e.g. `"claude_cli"`,
        /// `"anthropic_api"`, `"openai_compat"`). Provider-neutral consumers
        /// use this only for telemetry and display.
        provider: String,
        /// Model slug the dispatcher actually selected.
        model: String,
        /// OS pid when the transport is a subprocess; `None` for in-process
        /// providers (HTTP API, mock, etc.).
        pid: Option<u32>,
    },
    /// Provider-side initialization handshake. Some providers (notably the
    /// Claude CLI) emit a `system` event mid-stream announcing the resolved
    /// session id and effective model. Other providers may never produce one.
    SystemInit { session_id: String, model: String },
    /// Incremental assistant-visible text from the current turn.
    MessageDelta { text: String },
    /// The agent has invoked a tool; `id` is the provider-assigned tool call
    /// identifier echoed back in `ToolOutput`.
    ToolCall { id: String, name: String },
    /// The result of a tool invocation. `output` is the captured stdout/stderr
    /// (or structured tool result) the provider sent back, possibly truncated.
    ToolOutput { id: String, output: String },
    /// Token accounting delta. Cache token fields default to `0` for providers
    /// that do not surface caching information.
    TokenUsage {
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    },
    /// One assistant turn (or, for one-shot providers, the entire response)
    /// has finished. `total_cost_usd` is the authoritative cumulative cost,
    /// not a delta; `is_error` flags providers that completed with a soft
    /// error envelope rather than a transport failure.
    TurnCompleted {
        session_id: Option<String>,
        total_cost_usd: Option<f64>,
        num_turns: Option<u32>,
        is_error: bool,
    },
    /// A provider-level error string. Multiple `Error` events may precede a
    /// terminal `Exited`.
    Error { message: String },
    /// The runtime has fully terminated. Once `Exited` is observed no further
    /// events for this dispatch should arrive.
    Exited { exit_code: Option<i32> },
}

impl AgentRuntimeEvent {
    /// Return a stable, normalized event-type tag suitable for logs and
    /// projections. The values match the historical runner schema so existing
    /// telemetry consumers keep working unchanged.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::Started { .. } => "agent.started",
            Self::SystemInit { .. } => "agent.system_init",
            Self::MessageDelta { .. } => "agent.message_delta",
            Self::ToolCall { .. } => "agent.tool_call",
            Self::ToolOutput { .. } => "agent.tool_output",
            Self::TokenUsage { .. } => "agent.token_usage",
            Self::TurnCompleted { .. } => "agent.turn_completed",
            Self::Error { .. } => "agent.error",
            Self::Exited { .. } => "agent.exited",
        }
    }

    /// Whether this event is a terminal lifecycle signal (`Exited`).
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Exited { .. })
    }
}

/// Trait implemented by provider-specific stream adapters. Adapters consume
/// their wire protocol (CLI stdout, SSE, websocket, ...) and yield the
/// canonical [`AgentRuntimeEvent`] sequence the runner expects.
///
/// Implementations should be `Send` so they can be driven from a tokio task,
/// and should return `None` once the underlying transport has been fully
/// drained (after emitting the terminal [`AgentRuntimeEvent::Exited`]).
#[async_trait::async_trait]
pub trait AgentEventStream: Send {
    /// Pull the next event from the underlying transport. Returns `None` when
    /// the stream has been fully drained.
    async fn next_event(&mut self) -> Option<AgentRuntimeEvent>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_tags_are_stable() {
        let cases: [(AgentRuntimeEvent, &str); 9] = [
            (
                AgentRuntimeEvent::Started {
                    agent_id: "a".into(),
                    provider: "claude_cli".into(),
                    model: "claude-sonnet-4-6".into(),
                    pid: Some(1),
                },
                "agent.started",
            ),
            (
                AgentRuntimeEvent::SystemInit {
                    session_id: "s".into(),
                    model: "m".into(),
                },
                "agent.system_init",
            ),
            (
                AgentRuntimeEvent::MessageDelta { text: "hi".into() },
                "agent.message_delta",
            ),
            (
                AgentRuntimeEvent::ToolCall {
                    id: "t".into(),
                    name: "Bash".into(),
                },
                "agent.tool_call",
            ),
            (
                AgentRuntimeEvent::ToolOutput {
                    id: "t".into(),
                    output: "ok".into(),
                },
                "agent.tool_output",
            ),
            (
                AgentRuntimeEvent::TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
                "agent.token_usage",
            ),
            (
                AgentRuntimeEvent::TurnCompleted {
                    session_id: None,
                    total_cost_usd: None,
                    num_turns: None,
                    is_error: false,
                },
                "agent.turn_completed",
            ),
            (
                AgentRuntimeEvent::Error {
                    message: "oops".into(),
                },
                "agent.error",
            ),
            (
                AgentRuntimeEvent::Exited { exit_code: Some(0) },
                "agent.exited",
            ),
        ];
        for (event, tag) in cases {
            assert_eq!(event.event_type(), tag);
        }
    }

    #[test]
    fn only_exited_is_terminal() {
        assert!(AgentRuntimeEvent::Exited { exit_code: None }.is_terminal());
        assert!(
            !AgentRuntimeEvent::TurnCompleted {
                session_id: None,
                total_cost_usd: None,
                num_turns: None,
                is_error: false,
            }
            .is_terminal()
        );
    }
}
