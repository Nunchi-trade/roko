//! Forward ACP cognitive events into the canonical runtime event pipeline.

use roko_core::runtime_event::{RuntimeEvent, WorkflowOutcome};
use roko_runtime::HttpEventSink;

use crate::{
    bridge_events::CognitiveEvent,
    types::{ContentBlock, ResourceRef, StopReason, ToolCallStatus, UsageInfo},
};

/// Thin ACP adapter over the shared runtime HTTP event sink.
#[derive(Clone)]
pub struct AcpEventForwarder {
    sink: HttpEventSink,
    run_id: String,
    agent_id: String,
}

impl AcpEventForwarder {
    /// Builds a forwarder from `ROKO_SERVE_URL`, returning `None` when forwarding is disabled.
    pub fn from_env(session_id: impl Into<String>) -> Option<Self> {
        let session_id = session_id.into();
        let sink = HttpEventSink::from_env()?;
        Some(Self::new(
            sink,
            session_id.clone(),
            format!("acp:{session_id}"),
        ))
    }

    pub fn new(sink: HttpEventSink, run_id: String, agent_id: String) -> Self {
        Self {
            sink,
            run_id,
            agent_id,
        }
    }

    /// Maps a cognitive event to the closest existing `RuntimeEvent` and forwards it.
    pub fn forward(&self, event: &CognitiveEvent) {
        if let Some(runtime_event) = self.map_event(event) {
            self.sink.emit(runtime_event);
        }
    }

    fn map_event(&self, event: &CognitiveEvent) -> Option<RuntimeEvent> {
        match event {
            CognitiveEvent::TokenChunk(chunk) => Some(RuntimeEvent::AgentOutput {
                run_id: self.run_id.clone(),
                agent_id: self.agent_id.clone(),
                chunk: chunk.clone(),
            }),
            CognitiveEvent::ThinkingChunk(chunk) => Some(RuntimeEvent::FeedbackRecorded {
                run_id: self.run_id.clone(),
                kind: "acp_thinking".to_owned(),
                summary: chunk.clone(),
            }),
            CognitiveEvent::ToolCallStart { title, .. } => Some(RuntimeEvent::GateStarted {
                run_id: self.run_id.clone(),
                gate_name: title.clone(),
                rung: 0,
            }),
            CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status,
                content,
            } => match status {
                ToolCallStatus::Completed => Some(RuntimeEvent::GatePassed {
                    run_id: self.run_id.clone(),
                    gate_name: tool_call_id.clone(),
                    duration_ms: 0,
                }),
                ToolCallStatus::Failed => Some(RuntimeEvent::GateFailed {
                    run_id: self.run_id.clone(),
                    gate_name: tool_call_id.clone(),
                    output: summarize_content(content),
                    duration_ms: 0,
                }),
                ToolCallStatus::Pending | ToolCallStatus::InProgress => None,
            },
            CognitiveEvent::PlanUpdate { entries } => {
                let summary = entries
                    .iter()
                    .map(|entry| format!("{:?}: {}", entry.status, entry.content))
                    .collect::<Vec<_>>()
                    .join("; ");
                Some(RuntimeEvent::FeedbackRecorded {
                    run_id: self.run_id.clone(),
                    kind: "acp_plan_update".to_owned(),
                    summary,
                })
            }
            CognitiveEvent::McpStatus { statuses } => Some(RuntimeEvent::FeedbackRecorded {
                run_id: self.run_id.clone(),
                kind: "acp_mcp_status".to_owned(),
                summary: serde_json::to_string(statuses)
                    .unwrap_or_else(|_| format!("{} MCP server status updates", statuses.len())),
            }),
            CognitiveEvent::Complete { stop_reason, usage } => {
                Some(self.completion_event(stop_reason, usage.as_ref()))
            }
            CognitiveEvent::Failure { message } => Some(RuntimeEvent::AgentFailed {
                run_id: self.run_id.clone(),
                agent_id: self.agent_id.clone(),
                error: message.clone(),
            }),
            CognitiveEvent::MaxTokens => Some(RuntimeEvent::WorkflowCompleted {
                run_id: self.run_id.clone(),
                outcome: WorkflowOutcome::Halted {
                    reason: "max tokens reached".to_owned(),
                },
            }),
        }
    }

    fn completion_event(
        &self,
        stop_reason: &StopReason,
        usage: Option<&UsageInfo>,
    ) -> RuntimeEvent {
        match stop_reason {
            StopReason::EndTurn => RuntimeEvent::AgentCompleted {
                run_id: self.run_id.clone(),
                agent_id: self.agent_id.clone(),
                output: String::new(),
                tokens_used: usage.map_or(0, |usage| usage.total_tokens),
                cost_usd: 0.0,
            },
            StopReason::Cancelled => RuntimeEvent::WorkflowCompleted {
                run_id: self.run_id.clone(),
                outcome: WorkflowOutcome::Cancelled,
            },
            StopReason::MaxTokens | StopReason::MaxTurnRequests | StopReason::Refusal => {
                RuntimeEvent::WorkflowCompleted {
                    run_id: self.run_id.clone(),
                    outcome: WorkflowOutcome::Halted {
                        reason: stop_reason_label(stop_reason).to_owned(),
                    },
                }
            }
        }
    }
}

fn stop_reason_label(stop_reason: &StopReason) -> &'static str {
    match stop_reason {
        StopReason::EndTurn => "end turn",
        StopReason::MaxTokens => "max tokens reached",
        StopReason::MaxTurnRequests => "max turn requests reached",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
    }
}

fn summarize_content(content: &[ContentBlock]) -> String {
    content
        .iter()
        .map(summarize_content_block)
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_content_block(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text { text } => text.clone(),
        ContentBlock::Resource {
            resource: ResourceRef::File { uri },
        } => format!("file: {uri}"),
        ContentBlock::Diff {
            path,
            diff,
            new_text,
            old_text,
        } => diff
            .as_ref()
            .or(new_text.as_ref())
            .or(old_text.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("diff: {path}")),
    }
}
