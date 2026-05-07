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
        ContentBlock::Image { .. } => "[image]".to_string(),
        ContentBlock::Unknown => {
            tracing::debug!("skipping unknown content block type");
            "[unknown]".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_events::CognitiveEvent;
    use crate::types::{
        ContentBlock, McpInitStatus, McpServerStatus, PlanEntry, PlanStatus, Priority, StopReason,
        ToolCallKind, ToolCallStatus, UsageInfo,
    };
    use roko_core::runtime_event::{RuntimeEvent, WorkflowOutcome};

    /// Build a forwarder with a dummy sink for testing map_event.
    fn test_forwarder() -> AcpEventForwarder {
        // HttpEventSink::new spawns a background task, but we never call
        // emit() in mapping tests so the endpoint being unreachable is fine.
        let sink = HttpEventSink::new("http://127.0.0.1:1", None);
        AcpEventForwarder::new(sink, "test-run".into(), "acp:test".into())
    }

    #[tokio::test]
    async fn token_chunk_maps_to_agent_output() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::TokenChunk("hello".into());
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::AgentOutput {
                run_id,
                agent_id,
                chunk,
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(agent_id, "acp:test");
                assert_eq!(chunk, "hello");
            }
            other => panic!("expected AgentOutput, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn thinking_chunk_maps_to_feedback_recorded() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ThinkingChunk("reasoning...".into());
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::FeedbackRecorded {
                run_id,
                kind,
                summary,
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(kind, "acp_thinking");
                assert_eq!(summary, "reasoning...");
            }
            other => panic!("expected FeedbackRecorded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_start_maps_to_gate_started() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ToolCallStart {
            tool_call_id: "tc_1".into(),
            title: "Read file".into(),
            kind: ToolCallKind::Read,
            locations: None,
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::GateStarted {
                run_id,
                gate_name,
                rung,
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(gate_name, "Read file");
                assert_eq!(rung, 0);
            }
            other => panic!("expected GateStarted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_completed_maps_to_gate_passed() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ToolCallComplete {
            tool_call_id: "tc_2".into(),
            status: ToolCallStatus::Completed,
            content: vec![],
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::GatePassed {
                run_id,
                gate_name,
                duration_ms,
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(gate_name, "tc_2");
                assert_eq!(duration_ms, 0);
            }
            other => panic!("expected GatePassed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_failed_maps_to_gate_failed_with_content() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ToolCallComplete {
            tool_call_id: "tc_3".into(),
            status: ToolCallStatus::Failed,
            content: vec![ContentBlock::Text {
                text: "permission denied".into(),
            }],
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::GateFailed {
                run_id,
                gate_name,
                output,
                ..
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(gate_name, "tc_3");
                assert!(
                    output.contains("permission denied"),
                    "output should contain error text: {output}"
                );
            }
            other => panic!("expected GateFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_call_pending_maps_to_none() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ToolCallComplete {
            tool_call_id: "tc_4".into(),
            status: ToolCallStatus::Pending,
            content: vec![],
        };
        assert!(
            fwd.map_event(&event).is_none(),
            "Pending tool calls should not emit a RuntimeEvent"
        );
    }

    #[tokio::test]
    async fn tool_call_in_progress_maps_to_none() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::ToolCallComplete {
            tool_call_id: "tc_5".into(),
            status: ToolCallStatus::InProgress,
            content: vec![],
        };
        assert!(
            fwd.map_event(&event).is_none(),
            "InProgress tool calls should not emit a RuntimeEvent"
        );
    }

    #[tokio::test]
    async fn plan_update_maps_to_feedback_recorded() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::PlanUpdate {
            entries: vec![
                PlanEntry {
                    content: "step one".into(),
                    priority: Priority::High,
                    status: PlanStatus::Completed,
                },
                PlanEntry {
                    content: "step two".into(),
                    priority: Priority::Medium,
                    status: PlanStatus::InProgress,
                },
            ],
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::FeedbackRecorded { kind, summary, .. } => {
                assert_eq!(kind, "acp_plan_update");
                assert!(
                    summary.contains("step one"),
                    "summary should contain entry text"
                );
                assert!(
                    summary.contains("step two"),
                    "summary should contain both entries"
                );
            }
            other => panic!("expected FeedbackRecorded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mcp_status_maps_to_feedback_recorded() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::McpStatus {
            statuses: vec![McpServerStatus {
                name: "code-index".into(),
                status: McpInitStatus::Ready,
                tool_count: 5,
                message: None,
            }],
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::FeedbackRecorded { kind, summary, .. } => {
                assert_eq!(kind, "acp_mcp_status");
                assert!(
                    summary.contains("code-index"),
                    "summary should mention server name: {summary}"
                );
            }
            other => panic!("expected FeedbackRecorded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_end_turn_maps_to_agent_completed() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::Complete {
            stop_reason: StopReason::EndTurn,
            usage: Some(UsageInfo {
                total_tokens: 1500,
                input_tokens: 1000,
                output_tokens: 500,
                thought_tokens: None,
                cached_read_tokens: None,
                cached_write_tokens: None,
            }),
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::AgentCompleted {
                run_id,
                agent_id,
                tokens_used,
                ..
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(agent_id, "acp:test");
                assert_eq!(tokens_used, 1500);
            }
            other => panic!("expected AgentCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_cancelled_maps_to_workflow_cancelled() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::Complete {
            stop_reason: StopReason::Cancelled,
            usage: None,
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::WorkflowCompleted { outcome, .. } => {
                assert!(
                    matches!(outcome, WorkflowOutcome::Cancelled),
                    "expected Cancelled outcome"
                );
            }
            other => panic!("expected WorkflowCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_max_tokens_maps_to_workflow_halted() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::Complete {
            stop_reason: StopReason::MaxTokens,
            usage: None,
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::WorkflowCompleted { outcome, .. } => match outcome {
                WorkflowOutcome::Halted { reason } => {
                    assert!(
                        reason.contains("max tokens"),
                        "reason should mention max tokens: {reason}"
                    );
                }
                other => panic!("expected Halted, got {other:?}"),
            },
            other => panic!("expected WorkflowCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_refusal_maps_to_workflow_halted() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::Complete {
            stop_reason: StopReason::Refusal,
            usage: None,
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::WorkflowCompleted { outcome, .. } => match outcome {
                WorkflowOutcome::Halted { reason } => {
                    assert!(
                        reason.contains("refusal"),
                        "reason should mention refusal: {reason}"
                    );
                }
                other => panic!("expected Halted, got {other:?}"),
            },
            other => panic!("expected WorkflowCompleted, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failure_maps_to_agent_failed() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::Failure {
            message: "connection reset".into(),
        };
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::AgentFailed {
                run_id,
                agent_id,
                error,
            } => {
                assert_eq!(run_id, "test-run");
                assert_eq!(agent_id, "acp:test");
                assert_eq!(error, "connection reset");
            }
            other => panic!("expected AgentFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn max_tokens_event_maps_to_workflow_halted() {
        let fwd = test_forwarder();
        let event = CognitiveEvent::MaxTokens;
        let mapped = fwd.map_event(&event).expect("should map");
        match mapped {
            RuntimeEvent::WorkflowCompleted { outcome, .. } => match outcome {
                WorkflowOutcome::Halted { reason } => {
                    assert!(
                        reason.contains("max tokens"),
                        "reason should mention max tokens: {reason}"
                    );
                }
                other => panic!("expected Halted, got {other:?}"),
            },
            other => panic!("expected WorkflowCompleted, got {other:?}"),
        }
    }

    #[test]
    fn summarize_content_text_block() {
        let blocks = vec![ContentBlock::Text {
            text: "hello world".into(),
        }];
        assert_eq!(summarize_content(&blocks), "hello world");
    }

    #[test]
    fn summarize_content_diff_block() {
        let blocks = vec![ContentBlock::Diff {
            path: "src/main.rs".into(),
            diff: Some("- old\n+ new".into()),
            old_text: None,
            new_text: None,
        }];
        assert_eq!(summarize_content(&blocks), "- old\n+ new");
    }

    #[test]
    fn summarize_content_diff_falls_back_to_path() {
        let blocks = vec![ContentBlock::Diff {
            path: "src/main.rs".into(),
            diff: None,
            old_text: None,
            new_text: None,
        }];
        assert_eq!(summarize_content(&blocks), "diff: src/main.rs");
    }

    #[test]
    fn stop_reason_label_covers_all_variants() {
        assert_eq!(stop_reason_label(&StopReason::EndTurn), "end turn");
        assert_eq!(
            stop_reason_label(&StopReason::MaxTokens),
            "max tokens reached"
        );
        assert_eq!(
            stop_reason_label(&StopReason::MaxTurnRequests),
            "max turn requests reached"
        );
        assert_eq!(stop_reason_label(&StopReason::Refusal), "refusal");
        assert_eq!(stop_reason_label(&StopReason::Cancelled), "cancelled");
    }
}
