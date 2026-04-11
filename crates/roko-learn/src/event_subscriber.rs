//! Event-driven fan-out from the runtime event bus into learning systems.
//!
//! The current event schema does not carry full turn identity on every event,
//! so this subscriber keeps the latest started turn in memory and uses it to
//! enrich later `TurnCompleted` and `ToolCallExecuted` events.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

use roko_agent::chat_types::FinishReason;

use crate::anomaly::AnomalyDetector;
use crate::cascade_router::CascadeRouter;
use crate::cost_table::CostTable;
use crate::costs_db::{CostsDb, create_cost_record};
use crate::efficiency::{AgentEfficiencyEvent, ToolCallMeta};
use crate::events::AgentEvent;
use crate::latency::LatencyRegistry;
use crate::provider_health::ProviderHealthRegistry;

#[derive(Debug, Clone)]
struct ActiveTurn {
    task_id: String,
    model: String,
    provider: String,
    tool_calls: Vec<ToolCallMeta>,
}

impl ActiveTurn {
    fn from_started(task_id: &str, model: &str, provider: &str) -> Self {
        Self {
            task_id: task_id.to_string(),
            model: model.to_string(),
            provider: provider.to_string(),
            tool_calls: Vec::new(),
        }
    }
}

/// Consume `AgentEvent`s and update the learning subsystems that depend on them.
pub async fn run_learning_subscriber(
    mut rx: broadcast::Receiver<AgentEvent>,
    health: Arc<ProviderHealthRegistry>,
    latency: Arc<LatencyRegistry>,
    router: Arc<CascadeRouter>,
    anomaly: Arc<Mutex<AnomalyDetector>>,
    costs: Arc<CostsDb>,
    efficiency_path: PathBuf,
) {
    let cost_table = CostTable {
        models: HashMap::new(),
    }
    .with_defaults();
    let mut active_turn: Option<ActiveTurn> = None;

    loop {
        let event = match rx.recv().await {
            Ok(event) => event,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "learning subscriber lagged behind event stream");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        };

        match event {
            AgentEvent::TurnStarted {
                task_id,
                model,
                provider,
                ..
            } => {
                active_turn = Some(ActiveTurn::from_started(&task_id, &model, &provider));
            }
            AgentEvent::TurnCompleted {
                turn,
                usage,
                tool_call_count,
                gate_passed,
                finish_reason,
            } => {
                let Some(turn_ctx) = active_turn.take() else {
                    continue;
                };

                let success = gate_passed.unwrap_or(false);
                health.record_success(&turn_ctx.provider);
                let _ = router.record_outcome(&turn_ctx.model, success);

                let cost_record = create_cost_record(
                    Utc::now().to_rfc3339(),
                    &turn_ctx.model,
                    &turn_ctx.provider,
                    "",
                    "",
                    &turn_ctx.task_id,
                    "",
                    &usage,
                    &cost_table,
                    usage.wall_ms,
                    success,
                    "",
                );
                costs.insert(cost_record);

                let tools_used = tool_call_count.min(u32::MAX as usize) as u32;
                let efficiency_event = AgentEfficiencyEvent {
                    agent_id: format!("{}:{turn}", turn_ctx.task_id),
                    role: String::new(),
                    backend: turn_ctx.provider.clone(),
                    model: turn_ctx.model.clone(),
                    plan_id: String::new(),
                    task_id: turn_ctx.task_id,
                    input_tokens: u64::from(usage.input_tokens),
                    output_tokens: u64::from(usage.output_tokens),
                    reasoning_tokens: 0,
                    cache_read_tokens: u64::from(usage.cache_read_tokens),
                    cache_write_tokens: u64::from(usage.cache_create_tokens),
                    cost_usd: f64::from(usage.cost_usd),
                    cost_usd_without_cache: f64::from(usage.cost_usd),
                    prompt_sections: Vec::new(),
                    total_prompt_tokens: u64::from(usage.input_tokens),
                    system_prompt_tokens: 0,
                    tools_available: 0,
                    tools_used,
                    tool_calls: turn_ctx.tool_calls,
                    wall_time_ms: usage.wall_ms,
                    duration_ms: usage.wall_ms,
                    time_to_first_token_ms: 0,
                    was_warm_start: false,
                    iteration: turn,
                    gate_passed: success,
                    outcome: if success {
                        "success".to_string()
                    } else {
                        finish_reason_label(&finish_reason).to_string()
                    },
                    gate_errors: Vec::new(),
                    model_used: turn_ctx.model,
                    frequency: roko_core::OperatingFrequency::Theta,
                    strategy_attempted: String::new(),
                    timestamp: Utc::now().to_rfc3339(),
                };

                if let Err(err) = append_efficiency_event(&efficiency_path, &efficiency_event).await
                {
                    tracing::warn!(
                        path = %efficiency_path.display(),
                        error = %err,
                        "failed to append efficiency event"
                    );
                }
            }
            AgentEvent::ProviderError {
                provider_id,
                error_class,
                ..
            } => {
                health.record_failure(&provider_id, error_class);
            }
            AgentEvent::ToolCallExecuted {
                tool_name,
                duration_ms,
                success,
                result_tokens,
            } => {
                if let Some(turn_ctx) = active_turn.as_mut() {
                    turn_ctx.tool_calls.push(ToolCallMeta {
                        tool_name,
                        duration_ms,
                        result_tokens,
                        succeeded: success,
                        advanced_task: success,
                        was_redundant: false,
                        error_category: (!success).then_some("tool_execution_failed".to_string()),
                    });
                    latency.record(
                        &turn_ctx.model,
                        &turn_ctx.provider,
                        duration_ms as f64,
                        duration_ms as f64,
                        result_tokens,
                    );
                }
            }
            AgentEvent::CostRecorded { cost_usd, .. } => {
                if let Ok(mut detector) = anomaly.lock() {
                    let _ = detector.check_cost(cost_usd);
                }
            }
            AgentEvent::GateResult { .. }
            | AgentEvent::AnomalyDetected { .. }
            | AgentEvent::ExperimentAssigned { .. }
            | AgentEvent::SessionEstablished { .. }
            | AgentEvent::ModelSelected { .. }
            | AgentEvent::StreamChunk { .. } => {}
        }
    }
}

fn finish_reason_label(reason: &FinishReason) -> &str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Error(_) => "error",
    }
}

async fn append_efficiency_event(path: &Path, event: &AgentEfficiencyEvent) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut line = serde_json::to_string(event)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(line.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::run_learning_subscriber;
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;
    use tokio::sync::broadcast;

    use crate::anomaly::AnomalyDetector;
    use crate::cascade_router::CascadeRouter;
    use crate::costs_db::CostsDb;
    use crate::events::AgentEvent;
    use crate::latency::LatencyRegistry;
    use crate::provider_health::{ErrorClass, ProviderHealthRegistry};
    use crate::runtime_feedback::read_efficiency_events;
    use roko_agent::Usage;
    use roko_agent::chat_types::FinishReason;

    #[tokio::test]
    async fn event_subscriber_turn_completed_updates_router_costs_and_efficiency() {
        let (tx, rx) = broadcast::channel(16);
        let tempdir = TempDir::new().expect("tempdir");
        let efficiency_path = tempdir.path().join("efficiency.jsonl");

        let health = Arc::new(ProviderHealthRegistry::new());
        let latency = Arc::new(LatencyRegistry::new());
        let router = Arc::new(CascadeRouter::new(vec!["glm-5.1".to_string()]));
        let anomaly = Arc::new(Mutex::new(AnomalyDetector::new(1_700_000_000_000)));
        let costs = Arc::new(CostsDb::new());

        let handle = tokio::spawn(run_learning_subscriber(
            rx,
            Arc::clone(&health),
            Arc::clone(&latency),
            Arc::clone(&router),
            Arc::clone(&anomaly),
            Arc::clone(&costs),
            efficiency_path.clone(),
        ));

        tx.send(AgentEvent::TurnStarted {
            task_id: "task-2k22".into(),
            model: "glm-5.1".into(),
            provider: "zai".into(),
            timestamp_ms: 1_700_000_000_000,
        })
        .expect("turn started");
        tx.send(AgentEvent::ToolCallExecuted {
            tool_name: "Read".into(),
            duration_ms: 33,
            success: true,
            result_tokens: 128,
        })
        .expect("tool call");
        tx.send(AgentEvent::TurnCompleted {
            turn: 2,
            usage: Usage {
                input_tokens: 120,
                output_tokens: 45,
                cache_read_tokens: 10,
                cache_create_tokens: 2,
                cost_usd: 0.12,
                wall_ms: 850,
            },
            tool_call_count: 1,
            gate_passed: Some(true),
            finish_reason: FinishReason::Stop,
        })
        .expect("turn completed");

        drop(tx);
        handle.await.expect("subscriber task");

        let snapshot = router.confidence_snapshot();
        assert_eq!(snapshot.get("glm-5.1"), Some(&(1, 1)));

        let records = costs.all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "glm-5.1");
        assert_eq!(records[0].provider, "zai");
        assert_eq!(records[0].task_id, "task-2k22");
        assert!((records[0].cost_usd - 0.12).abs() < 1e-6);

        let events = read_efficiency_events(&efficiency_path)
            .await
            .expect("read efficiency events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].model, "glm-5.1");
        assert_eq!(events[0].task_id, "task-2k22");
        assert_eq!(events[0].tools_used, 1);
        assert_eq!(events[0].tool_calls.len(), 1);
        assert!(events[0].gate_passed);
    }

    #[tokio::test]
    async fn event_subscriber_routes_provider_errors_and_tool_latency() {
        let (tx, rx) = broadcast::channel(16);
        let tempdir = TempDir::new().expect("tempdir");
        let efficiency_path = tempdir.path().join("efficiency.jsonl");

        let health = Arc::new(ProviderHealthRegistry::new());
        let latency = Arc::new(LatencyRegistry::new());
        let router = Arc::new(CascadeRouter::new(vec!["glm-5.1".to_string()]));
        let anomaly = Arc::new(Mutex::new(AnomalyDetector::new(1_700_000_000_000)));
        let costs = Arc::new(CostsDb::new());

        let handle = tokio::spawn(run_learning_subscriber(
            rx,
            Arc::clone(&health),
            Arc::clone(&latency),
            router,
            anomaly,
            costs,
            efficiency_path,
        ));

        tx.send(AgentEvent::TurnStarted {
            task_id: "task-latency".into(),
            model: "glm-5.1".into(),
            provider: "zai".into(),
            timestamp_ms: 1_700_000_000_000,
        })
        .expect("turn started");
        tx.send(AgentEvent::ToolCallExecuted {
            tool_name: "Read".into(),
            duration_ms: 50,
            success: true,
            result_tokens: 64,
        })
        .expect("tool call");
        tx.send(AgentEvent::ProviderError {
            provider_id: "zai".into(),
            error_class: ErrorClass::RateLimit,
            status: 429,
        })
        .expect("provider error");
        tx.send(AgentEvent::ProviderError {
            provider_id: "zai".into(),
            error_class: ErrorClass::RateLimit,
            status: 429,
        })
        .expect("provider error");
        tx.send(AgentEvent::ProviderError {
            provider_id: "zai".into(),
            error_class: ErrorClass::RateLimit,
            status: 429,
        })
        .expect("provider error");

        drop(tx);
        handle.await.expect("subscriber task");

        let stats = latency.get("glm-5.1", "zai").expect("latency stats");
        assert_eq!(stats.observations, 1);
        assert_eq!(stats.recent_latencies, vec![50.0]);

        assert!(!health.is_available("zai"));
    }
}
