//! Pipeline runner — executes the state machine by performing side effects.
//!
//! This module drives a [`WorkflowRun`] by:
//! 1. Starting the pipeline state machine
//! 2. Performing actions (spawn agents, run gates, commit)
//! 3. Feeding results back as events
//! 4. Emitting ACP session updates (plan entries, tool calls) through the event channel

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use roko_agent::claude_cli_agent::build_settings_json;
use roko_agent::safety::contract::AgentContract;
use roko_agent::safety::{SafetyLayer, SafetyViolation, ViolationSeverity};
use roko_agent::{Agent as RokoAgent, ClaudeCliAgent};
use roko_core::foundation::EventConsumer as CoreEventConsumer;
use roko_core::{
    Body, Context, Engram, Kind, RuntimeEvent as CoreRuntimeEvent, Verify,
    WorkflowOutcome as CoreWorkflowOutcome,
};
use roko_gate::{
    AdaptiveThresholds, ClippyGate, CompileGate, GatePayload, TestGate,
    parse_structured_review_verdict, review_verdict::ReviewVerdictContext,
};
use roko_orchestrator::{ServiceConfig, ServiceFactory};
use roko_runtime::JsonlLogger;
use roko_runtime::effect_driver::RuntimeEvent as RuntimeDriverEvent;
use roko_runtime::event_bus::runtime_event_bus;
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowRunReport};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::bridge_events::CognitiveEvent;
use crate::knowledge::prepend_context;
use crate::pipeline::{PipelineAction, PipelineEvent, PipelinePhase, WorkflowTemplate};
use crate::session::{CancelToken, SharedWorkflowRun};
use crate::types::{
    ContentBlock, FileChangeNotification, FileChangeType, PlanEntry, PlanStatus, Priority,
    StopReason, ToolCallKind, ToolCallStatus, UsageInfo,
};
use crate::workflow::WorkflowRun;

/// Configuration passed from the ACP session to the pipeline runner.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub template: WorkflowTemplate,
    pub max_iterations: u32,
    pub clippy_enabled: bool,
    pub tests_enabled: bool,
    /// Review strictness: "none", "quick", "standard", "thorough".
    pub review_strictness: String,
    /// Resolved model slug used for agent phases.
    pub model_slug: String,
}

const CLAUDE_CLI_BIN: &str = "claude";
const NO_CHANGES_TO_COMMIT_MESSAGE: &str = "(no changes to commit)";

/// Classification of the gate failure cause.
#[derive(Debug)]
enum GateErrorType {
    CompileError {
        file: String,
        line: u32,
        message: String,
    },
    TestFailure {
        test_name: String,
        expected: Option<String>,
        actual: Option<String>,
    },
    ClippyWarning {
        lint: String,
        location: String,
    },
    RuntimePanic {
        message: String,
    },
    Unknown,
}

impl GateErrorType {
    fn description(&self) -> &'static str {
        match self {
            Self::CompileError { .. } => "compile error",
            Self::TestFailure { .. } => "test failure",
            Self::ClippyWarning { .. } => "clippy warning",
            Self::RuntimePanic { .. } => "runtime panic",
            Self::Unknown => "unknown error",
        }
    }
}

/// Forensic analysis of a gate failure.
#[derive(Debug)]
struct GateAutopsy {
    error_type: GateErrorType,
    root_cause: String,
    causal_chain: Vec<String>,
    similar_past: Option<(String, String)>,
    confidence: f64,
}

/// Detect files changed in the most recent commit via `git diff --name-status HEAD~1 HEAD`.
/// Returns an empty vec if git is unavailable or the repo has no prior commits.
async fn detect_file_changes(workdir: &Path) -> Vec<FileChangeNotification> {
    let output = match Command::new("git")
        .args(["diff", "--name-status", "HEAD~1", "HEAD"])
        .current_dir(workdir)
        .output()
        .await
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut changes = Vec::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split('\t').collect();
        let status = match fields.first() {
            Some(status) => status.trim(),
            None => continue,
        };
        let status_code = status.chars().next();
        let path = match status_code {
            Some('R') | Some('C') if fields.len() >= 3 => fields[2].trim().to_string(),
            _ => match fields.get(1) {
                Some(path) => path.trim().to_string(),
                None => continue,
            },
        };

        if path.ends_with(".lock")
            || path.ends_with(".png")
            || path.ends_with(".jpg")
            || path.ends_with(".jpeg")
            || path.ends_with(".gif")
            || path.ends_with(".ico")
        {
            continue;
        }

        let change_type = match status_code {
            Some('A') | Some('C') => FileChangeType::Added,
            Some('M') => FileChangeType::Modified,
            Some('D') => FileChangeType::Deleted,
            Some('R') => FileChangeType::Renamed,
            _ => FileChangeType::Modified,
        };

        changes.push(FileChangeNotification { path, change_type });
    }

    changes
}

/// Split a combined `git diff` output into per-file chunks keyed by `b/` path.
fn split_diff_by_file(combined: &str) -> std::collections::HashMap<String, String> {
    let mut result = std::collections::HashMap::new();
    let mut current_file: Option<String> = None;
    let mut current_chunk = String::new();
    for line in combined.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(file) = current_file.take()
                && !current_chunk.is_empty()
            {
                result.insert(file, std::mem::take(&mut current_chunk));
            }
            if let Some(b_path) = rest.split(" b/").last() {
                current_file = Some(b_path.to_string());
            }
        }
        current_chunk.push_str(line);
        current_chunk.push('\n');
    }
    if let Some(file) = current_file
        && !current_chunk.is_empty()
    {
        result.insert(file, current_chunk);
    }
    result
}

fn classify_gate_error(output: &str) -> GateErrorType {
    if let Some(line) = output.lines().find(|line| line.contains("error[E")) {
        let location_line = output
            .lines()
            .find(|candidate| candidate.contains("-->") && candidate.contains(".rs:"));
        let location = location_line
            .and_then(|line| line.split_once("-->").map(|(_, location)| location.trim()));
        let file = location
            .and_then(|location| location.split_once(':').map(|(file, _)| file.to_string()))
            .unwrap_or_default();
        let line_no: u32 = location
            .and_then(|location| location.split_once(':'))
            .and_then(|(_, rest)| rest.split_once(':'))
            .and_then(|(line, _)| line.parse().ok())
            .unwrap_or(0);
        let message = line
            .split_once("error[E")
            .and_then(|(_, segment)| segment.split_once("]: ").map(|(_, message)| message))
            .unwrap_or(line)
            .chars()
            .take(120)
            .collect();
        return GateErrorType::CompileError {
            file,
            line: line_no,
            message,
        };
    }

    if output.contains("panicked at")
        || output
            .lines()
            .any(|line| line.starts_with("thread '") && line.contains("panicked"))
    {
        let message = output
            .lines()
            .find(|line| line.contains("panicked at") || line.contains("panicked"))
            .map(|line| line.chars().take(120).collect())
            .unwrap_or_default();
        return GateErrorType::RuntimePanic { message };
    }

    if output.contains("test result: FAILED")
        || output
            .lines()
            .any(|line| line.contains("FAILED") && !line.contains("test result:"))
    {
        let test_name = output
            .lines()
            .find(|line| line.contains("FAILED") && !line.contains("test result:"))
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("unknown")
            .trim_end_matches("...")
            .to_string();
        let expected = output
            .lines()
            .find(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("left =")
                    || trimmed.starts_with("left:")
                    || trimmed.starts_with("expected =")
                    || trimmed.starts_with("expected:")
            })
            .map(|line| line.trim().chars().take(80).collect());
        let actual = output
            .lines()
            .find(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("right =")
                    || trimmed.starts_with("right:")
                    || trimmed.starts_with("actual =")
                    || trimmed.starts_with("actual:")
                    || trimmed.starts_with("got =")
                    || trimmed.starts_with("got:")
            })
            .map(|line| line.trim().chars().take(80).collect());
        return GateErrorType::TestFailure {
            test_name,
            expected,
            actual,
        };
    }

    if output.contains("warning:") && output.contains("error: could not compile") {
        let lint = output
            .lines()
            .find(|line| line.trim().starts_with("warning:"))
            .and_then(|line| line.strip_prefix("warning:"))
            .map(|line| line.trim().chars().take(60).collect())
            .unwrap_or_default();
        let location = output
            .lines()
            .find(|line| line.contains("-->") && line.contains(".rs:"))
            .and_then(|line| line.split_once("-->").map(|(_, location)| location.trim()))
            .map(|location| location.chars().take(60).collect())
            .unwrap_or_default();
        return GateErrorType::ClippyWarning { lint, location };
    }

    GateErrorType::Unknown
}

fn extract_root_cause(output: &str) -> String {
    output
        .lines()
        .find(|line| {
            line.contains("error[E")
                || line.contains("panicked at")
                || (line.starts_with("thread '") && line.contains("panicked"))
                || line.contains("FAILED")
                || line.contains("error:")
        })
        .unwrap_or("could not determine root cause")
        .trim()
        .chars()
        .take(200)
        .collect()
}

async fn analyze_gate_failure(error_output: &str, workdir: &Path, _iteration: u32) -> GateAutopsy {
    use roko_learn::episode_logger::EpisodeLogger;

    let error_type = classify_gate_error(error_output);
    let root_cause = extract_root_cause(error_output);
    let mut causal_chain = Vec::new();

    if let Ok(output) = Command::new("git")
        .args(["diff", "--stat", "HEAD"])
        .current_dir(workdir)
        .output()
        .await
        && output.status.success()
    {
        let stat = String::from_utf8_lossy(&output.stdout);
        if let Some(summary) = stat
            .lines()
            .last()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
        {
            causal_chain.push(format!("Changes: {summary}"));
        }
    }

    match &error_type {
        GateErrorType::CompileError {
            file,
            line,
            message,
        } => {
            if !file.is_empty() {
                causal_chain.push(format!("Compile error at {file}:{line}"));
            }
            causal_chain.push(format!("Message: {message}"));
        }
        GateErrorType::TestFailure {
            test_name,
            expected,
            actual,
        } => {
            causal_chain.push(format!("Test `{test_name}` failed"));
            if let (Some(expected), Some(actual)) = (expected, actual) {
                causal_chain.push(format!("Expected: {expected}"));
                causal_chain.push(format!("Got: {actual}"));
            }
        }
        GateErrorType::ClippyWarning { lint, location } => {
            causal_chain.push(format!("Clippy: {lint}"));
            if !location.is_empty() {
                causal_chain.push(format!("In: {location}"));
            }
        }
        GateErrorType::RuntimePanic { message } => {
            causal_chain.push(format!("Panic: {message}"));
        }
        GateErrorType::Unknown => {
            causal_chain.push("Error type: unclassified".to_string());
        }
    }

    if causal_chain.is_empty() {
        causal_chain.push("No additional causal evidence found".to_string());
    }

    let mut similar_past: Option<(String, String)> = None;
    let episodes_path = workdir.join(".roko").join("episodes.jsonl");

    if let Ok(episodes) = EpisodeLogger::read_all_lossy(&episodes_path).await {
        let error_sig: String = root_cause.chars().take(60).collect();
        for episode in episodes.iter().rev().take(50) {
            if episode.success {
                continue;
            }

            let past_text = episode
                .reflection
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| {
                    episode
                        .failure_reason
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                });

            if let Some(text) = past_text.filter(|text| {
                !error_sig.is_empty()
                    && (text.contains(&error_sig) || similar_strings(text, &error_sig))
            }) {
                let task_id = if episode.task_id.trim().is_empty() {
                    "unknown-task".to_string()
                } else {
                    episode.task_id.clone()
                };
                similar_past = Some((task_id, text.chars().take(120).collect()));
                break;
            }
        }
    }

    let confidence = match (&error_type, &similar_past) {
        (GateErrorType::Unknown, None) => 0.30,
        (GateErrorType::Unknown, Some(_)) => 0.60,
        (_, None) => 0.65,
        (_, Some(_)) => 0.89,
    };

    GateAutopsy {
        error_type,
        root_cause,
        causal_chain,
        similar_past,
        confidence,
    }
}

fn similar_strings(a: &str, b: &str) -> bool {
    use std::collections::HashSet;

    let words_a: HashSet<&str> = a.split_whitespace().collect();
    let words_b: HashSet<&str> = b.split_whitespace().collect();
    let overlap = words_a.intersection(&words_b).count();
    let total = words_a.len().max(words_b.len());
    total > 3 && (overlap as f64 / total as f64) > 0.5
}

/// Execute a prompt via WorkflowEngine, bridging events to ACP protocol.
///
/// This is an alternative to [`run_workflow_pipeline`] that uses the shared
/// WorkflowEngine architecture. Runtime events are bridged to the ACP session
/// via an `EventConsumer` bridge (`RuntimeEvent` -> `CognitiveEvent` -> session updates).
pub async fn run_with_workflow_engine(
    session_id: &str,
    prompt: &str,
    workdir: &Path,
    template: &str,
    provenance_card: Option<String>,
    event_sender: mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<WorkflowRunReport> {
    let runtime_run_id = Arc::new(Mutex::new(None));
    let roko_config = roko_core::config::loader::load_config_with_options(
        workdir,
        &roko_core::config::loader::LoadOptions::acp(),
    )
    .unwrap_or_default();
    let services = ServiceFactory::build(ServiceConfig {
        workdir: workdir.to_path_buf(),
        roko_dir: workdir.join(".roko"),
        workspace_config: roko_config,
        model_key: std::env::var("ROKO_MODEL").ok(),
        mcp_config: None,
        feedback_enabled: true,
        affect_enabled: false,
        cascade_enabled: true,
        run_id: Some(format!("acp_workflow_{session_id}")),
        inference_observer: None,
        metrics: None,
    })
    .map_err(|error| anyhow::anyhow!("build workflow services: {error}"))?
    .effect_services();

    let workflow = match template {
        "express" => WorkflowConfig::express(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::standard(),
    };

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow,
        enabled_gates: vec!["compile".into(), "test".into()],
        shell_gates: Vec::new(),
        commit_prefix: Some("feat".to_string()),
    };

    let mut engine = WorkflowEngine::new(services);
    engine.add_consumer(Arc::new(JsonlLogger::from_roko_dir(&workdir.join(".roko"))));
    engine.add_consumer(Arc::new(AcpWorkflowEventConsumer::new(
        session_id.to_string(),
        Arc::clone(&runtime_run_id),
        event_sender.clone(),
        provenance_card,
    )));

    let bridge_task = spawn_runtime_event_bridge(
        session_id.to_string(),
        Arc::clone(&runtime_run_id),
        event_sender,
    );
    let result = engine
        .run(config)
        .await
        .map_err(|error| anyhow::anyhow!("workflow engine failed: {error}"));
    bridge_task.abort();

    result
}

struct AcpWorkflowEventConsumer {
    run_id: Arc<Mutex<Option<String>>>,
    template: Arc<Mutex<Option<String>>>,
    provenance_card: Arc<Mutex<Option<String>>>,
    sender: mpsc::Sender<CognitiveEvent>,
    /// Total tokens accumulated across all AgentCompleted events in this run.
    accumulated_tokens: Arc<AtomicU64>,
}

impl AcpWorkflowEventConsumer {
    fn new(
        _session_id: String,
        run_id: Arc<Mutex<Option<String>>>,
        sender: mpsc::Sender<CognitiveEvent>,
        provenance_card: Option<String>,
    ) -> Self {
        Self {
            run_id,
            template: Arc::new(Mutex::new(None)),
            provenance_card: Arc::new(Mutex::new(provenance_card)),
            sender,
            accumulated_tokens: Arc::new(AtomicU64::new(0)),
        }
    }

    fn publish(&self, event: CognitiveEvent) {
        let _ = self.sender.try_send(event);
    }

    fn publish_provenance_card(&self, run_id: &str) {
        let Some(card_text) = self
            .provenance_card
            .lock()
            .ok()
            .and_then(|mut current| current.take())
        else {
            return;
        };

        let tool_call_id = format!("decision-provenance-{run_id}");
        self.publish(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: "Decision provenance".into(),
            kind: ToolCallKind::Other,
            locations: None,
        });
        self.publish(CognitiveEvent::ToolCallComplete {
            tool_call_id,
            status: ToolCallStatus::Completed,
            content: vec![text_block(card_text)],
        });
    }
}

impl CoreEventConsumer for AcpWorkflowEventConsumer {
    fn consume(&self, event: &CoreRuntimeEvent) {
        match event {
            CoreRuntimeEvent::WorkflowStarted {
                run_id, template, ..
            } => {
                if let Ok(mut current) = self.run_id.lock() {
                    *current = Some(run_id.clone());
                }
                if let Ok(mut current) = self.template.lock() {
                    *current = Some(template.clone());
                }
                self.publish(CognitiveEvent::PlanUpdate {
                    entries: workflow_plan_entries(template, "implementing"),
                });
            }
            CoreRuntimeEvent::PhaseTransition { run_id, to, .. } => {
                if self.accepts_run(run_id) {
                    let template = self
                        .template
                        .lock()
                        .ok()
                        .and_then(|current| current.clone())
                        .unwrap_or_else(|| "standard".to_string());
                    self.publish(CognitiveEvent::PlanUpdate {
                        entries: workflow_plan_entries(&template, to),
                    });
                    if to == "strategizing" {
                        self.publish_provenance_card(run_id);
                    }
                }
            }
            CoreRuntimeEvent::AgentOutput { run_id, chunk, .. } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::TokenChunk(chunk.clone()));
                }
            }
            CoreRuntimeEvent::AgentCompleted {
                run_id,
                tokens_used,
                ..
            } => {
                if self.accepts_run(run_id) {
                    self.accumulated_tokens
                        .fetch_add(*tokens_used, Ordering::Relaxed);
                }
            }
            CoreRuntimeEvent::AgentFailed {
                run_id,
                agent_id,
                error,
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallComplete {
                        tool_call_id: agent_id.clone(),
                        status: ToolCallStatus::Failed,
                        content: vec![text_block(error.clone())],
                    });
                }
            }
            CoreRuntimeEvent::GateStarted {
                run_id, gate_name, ..
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallStart {
                        tool_call_id: gate_call_id(gate_name),
                        title: format!("Gate: {gate_name}"),
                        kind: ToolCallKind::Other,
                        locations: None,
                    });
                }
            }
            CoreRuntimeEvent::GatePassed {
                run_id, gate_name, ..
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallComplete {
                        tool_call_id: gate_call_id(gate_name),
                        status: ToolCallStatus::Completed,
                        content: vec![text_block(format!("{gate_name} passed"))],
                    });
                }
            }
            CoreRuntimeEvent::GateFailed {
                run_id,
                gate_name,
                output,
                ..
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallComplete {
                        tool_call_id: gate_call_id(gate_name),
                        status: ToolCallStatus::Failed,
                        content: vec![text_block(output.clone())],
                    });
                }
            }
            CoreRuntimeEvent::InferenceStarted {
                run_id,
                request_id,
                model,
                agent_id,
                ..
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallStart {
                        tool_call_id: inference_call_id(request_id),
                        title: format!("Inference: {model} ({agent_id})"),
                        kind: ToolCallKind::Other,
                        locations: None,
                    });
                }
            }
            CoreRuntimeEvent::InferenceCompleted {
                run_id,
                request_id,
                model,
                input_tokens,
                output_tokens,
                cost_usd,
                duration_ms,
                ..
            } => {
                if self.accepts_run(run_id) {
                    self.accumulated_tokens
                        .fetch_add(*input_tokens + *output_tokens, Ordering::Relaxed);
                    self.publish(CognitiveEvent::ToolCallComplete {
                        tool_call_id: inference_call_id(request_id),
                        status: ToolCallStatus::Completed,
                        content: vec![text_block(format!(
                            "{model}: {input_tokens} input tokens, {output_tokens} output tokens, ${cost_usd:.4}, {duration_ms}ms"
                        ))],
                    });
                }
            }
            CoreRuntimeEvent::InferenceFailed {
                run_id,
                request_id,
                model,
                error,
                ..
            } => {
                if self.accepts_run(run_id) {
                    self.publish(CognitiveEvent::ToolCallComplete {
                        tool_call_id: inference_call_id(request_id),
                        status: ToolCallStatus::Failed,
                        content: vec![text_block(format!("{model}: {error}"))],
                    });
                }
            }
            CoreRuntimeEvent::AgentTrace {
                run_id, reasoning, ..
            } => {
                if self.accepts_run(run_id) {
                    if let Some(reasoning) = reasoning {
                        if !reasoning.trim().is_empty() {
                            self.publish(CognitiveEvent::ThinkingChunk(reasoning.clone()));
                        }
                    }
                }
            }
            CoreRuntimeEvent::WorkflowCompleted { run_id, outcome } => {
                if self.accepts_run(run_id) {
                    let total_tokens = self.accumulated_tokens.load(Ordering::Relaxed);
                    let usage = if total_tokens > 0 {
                        Some(UsageInfo {
                            total_tokens,
                            // AgentCompleted only carries a combined token count; we surface
                            // the full total as output_tokens so downstream cost calculations
                            // can use it, even though the input/output split is unknown here.
                            input_tokens: 0,
                            output_tokens: total_tokens,
                            thought_tokens: None,
                            cached_read_tokens: None,
                            cached_write_tokens: None,
                        })
                    } else {
                        None
                    };
                    self.publish(CognitiveEvent::Complete {
                        stop_reason: stop_reason_for_core_outcome(outcome),
                        usage,
                    });
                }
            }
            CoreRuntimeEvent::AgentSpawned { .. }
            | CoreRuntimeEvent::TaskFailed { .. }
            | CoreRuntimeEvent::RunStarted { .. }
            | CoreRuntimeEvent::RunCompleted { .. }
            | CoreRuntimeEvent::KnowledgeIngested { .. }
            | CoreRuntimeEvent::KnowledgeConsumed { .. }
            | CoreRuntimeEvent::FeedbackRecorded { .. }
            | CoreRuntimeEvent::StateCheckpointed { .. }
            | CoreRuntimeEvent::InferenceFirstToken { .. }
            | CoreRuntimeEvent::ToolCallStarted { .. }
            | CoreRuntimeEvent::ToolCallCompleted { .. }
            | CoreRuntimeEvent::TaskStarted { .. }
            | CoreRuntimeEvent::TaskCompleted { .. }
            | CoreRuntimeEvent::PipelinePhase { .. } => {}
        }
    }
}

impl AcpWorkflowEventConsumer {
    fn accepts_run(&self, run_id: &str) -> bool {
        self.run_id
            .lock()
            .ok()
            .and_then(|current| current.clone())
            .is_some_and(|current| current == run_id)
    }
}

fn spawn_runtime_event_bridge(
    session_id: String,
    run_id: Arc<Mutex<Option<String>>>,
    sender: mpsc::Sender<CognitiveEvent>,
) -> tokio::task::JoinHandle<()> {
    let mut receiver = runtime_event_bus::<RuntimeDriverEvent>().subscribe();

    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(envelope) => {
                    let event = envelope.payload;
                    if matches!(
                        event,
                        RuntimeDriverEvent::WorkflowStarted { .. }
                            | RuntimeDriverEvent::PhaseTransition { .. }
                            | RuntimeDriverEvent::WorkflowCompleted { .. }
                            | RuntimeDriverEvent::GateStarted { .. }
                            | RuntimeDriverEvent::FeedbackRecorded { .. }
                            | RuntimeDriverEvent::StateCheckpointed { .. }
                    ) {
                        continue;
                    }
                    let Some(active_run_id) = run_id.lock().ok().and_then(|guard| guard.clone())
                    else {
                        continue;
                    };
                    if driver_event_run_id(&event) != active_run_id {
                        continue;
                    }

                    let core_event = core_runtime_event_from_driver(event);
                    let consumer = AcpWorkflowEventConsumer::new(
                        session_id.clone(),
                        Arc::new(Mutex::new(Some(active_run_id))),
                        sender.clone(),
                        None,
                    );
                    CoreEventConsumer::consume(&consumer, &core_event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn workflow_plan_entries(template: &str, phase: &str) -> Vec<PlanEntry> {
    let has_strategy = template == "full";
    let has_review = template != "express";
    let mut entries = Vec::new();

    if has_strategy {
        entries.push(PlanEntry {
            content: "Strategy brief".to_string(),
            priority: Priority::High,
            status: plan_status(phase, &["strategizing"], &[]),
        });
    }

    entries.push(PlanEntry {
        content: "Implementation".to_string(),
        priority: Priority::High,
        status: plan_status(
            phase,
            &["implementing", "auto_fixing"],
            if has_strategy { &["strategizing"] } else { &[] },
        ),
    });

    entries.push(PlanEntry {
        content: "Run gates".to_string(),
        priority: Priority::Medium,
        status: plan_status(
            phase,
            &["gating"],
            &["pending", "strategizing", "implementing", "auto_fixing"],
        ),
    });

    if has_review {
        entries.push(PlanEntry {
            content: "Code review".to_string(),
            priority: Priority::Medium,
            status: plan_status(
                phase,
                &["reviewing"],
                &[
                    "pending",
                    "strategizing",
                    "implementing",
                    "auto_fixing",
                    "gating",
                ],
            ),
        });
    }

    entries.push(PlanEntry {
        content: "Commit changes".to_string(),
        priority: Priority::Low,
        status: plan_status(
            phase,
            &["committing"],
            &[
                "pending",
                "strategizing",
                "implementing",
                "auto_fixing",
                "gating",
                "reviewing",
            ],
        ),
    });

    entries
}

fn plan_status(phase: &str, active: &[&str], pending: &[&str]) -> PlanStatus {
    if active.contains(&phase) {
        PlanStatus::InProgress
    } else if pending.contains(&phase) {
        PlanStatus::Pending
    } else {
        PlanStatus::Completed
    }
}

fn gate_call_id(gate_name: &str) -> String {
    format!("gate-{gate_name}")
}

fn inference_call_id(request_id: &str) -> String {
    format!("inference-{request_id}")
}

fn text_block(text: String) -> ContentBlock {
    ContentBlock::Text { text }
}

fn stop_reason_for_core_outcome(outcome: &CoreWorkflowOutcome) -> StopReason {
    match outcome {
        CoreWorkflowOutcome::Cancelled => StopReason::Cancelled,
        CoreWorkflowOutcome::Success { .. } | CoreWorkflowOutcome::Halted { .. } => {
            StopReason::EndTurn
        }
    }
}

fn driver_event_run_id(event: &RuntimeDriverEvent) -> &str {
    event.run_id()
}

fn core_runtime_event_from_driver(event: RuntimeDriverEvent) -> CoreRuntimeEvent {
    event
}

/// Build a restrictive `SafetyLayer` for an ACP session mode.
fn safety_layer_for_mode(mode: &str) -> SafetyLayer {
    let mode = mode.trim();
    if mode.is_empty() {
        return SafetyLayer::with_defaults().with_contract(AgentContract::restricted("default"));
    }
    SafetyLayer::with_defaults().with_role(mode)
}

/// Build a restrictive `SafetyLayer` for a pipeline phase role.
fn safety_layer_for_pipeline_role(role: &str) -> SafetyLayer {
    safety_layer_for_mode(role)
}

fn log_safety_violations(role: &str, violations: &[SafetyViolation]) {
    for violation in violations {
        match violation.severity {
            ViolationSeverity::Block => {
                error!(
                    role,
                    violation = ?violation.violation_type,
                    message = %violation.message,
                    "ACP pipeline safety violation (BLOCK)"
                );
            }
            ViolationSeverity::Warn => {
                warn!(
                    role,
                    violation = ?violation.violation_type,
                    message = %violation.message,
                    "ACP pipeline safety violation (warn)"
                );
            }
        }
    }
}

/// Run a workflow pipeline, emitting ACP events as it progresses.
///
/// This is the main entry point called from `bridge_events.rs` when
/// the session workflow config is not "none".
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow_pipeline(
    session_id: &str,
    prompt: &str,
    knowledge_context: String,
    mut provenance_card: Option<String>,
    workdir: &Path,
    config: PipelineConfig,
    cancel_token: CancelToken,
    event_sender: mpsc::Sender<CognitiveEvent>,
    shared_run: SharedWorkflowRun,
) -> anyhow::Result<()> {
    let mut run = WorkflowRun::new(
        config.template.clone(),
        prompt.to_owned(),
        config.max_iterations,
    );

    info!(
        run_id = %run.run_id,
        template = run.template_name(),
        "starting workflow pipeline"
    );

    // Publish initial state and emit plan update.
    sync_shared_run(&shared_run, &run).await;
    emit_plan_update(&run, &event_sender).await;

    // Start the state machine.
    let mut action = run.pipeline.step(PipelineEvent::Start);

    loop {
        if cancel_token.is_cancelled() {
            action = run.pipeline.step(PipelineEvent::UserCancel);
        }

        debug!(
            run_id = %run.run_id,
            phase = ?run.pipeline.phase,
            action = ?action,
            "pipeline step"
        );

        // Emit plan update, shared state, and inline phase badge after each transition.
        // Phase narratives are emitted by the matched arm before the next loop badge.
        sync_shared_run(&shared_run, &run).await;
        emit_plan_update(&run, &event_sender).await;
        if let Some(badge) = phase_badge(&run.pipeline.phase, run.pipeline.iteration) {
            let _ = event_sender.send(CognitiveEvent::TokenChunk(badge)).await;
        }

        match action {
            PipelineAction::SpawnStrategist { ref prompt } => {
                run.agents_spawned += 1;
                if let Some(card_text) = provenance_card.take() {
                    let tool_call_id = format!("decision-provenance-{}", run.run_id);
                    let _ = event_sender
                        .send(CognitiveEvent::ToolCallStart {
                            tool_call_id: tool_call_id.clone(),
                            title: "Decision provenance".into(),
                            kind: ToolCallKind::Other,
                            locations: None,
                        })
                        .await;
                    let _ = event_sender
                        .send(CognitiveEvent::ToolCallComplete {
                            tool_call_id,
                            status: ToolCallStatus::Completed,
                            content: vec![text_block(card_text)],
                        })
                        .await;
                }
                let full_prompt = prepend_context(prompt, &knowledge_context);
                let result = run_agent_phase(
                    session_id,
                    "Strategist",
                    &full_prompt,
                    workdir,
                    &config.model_slug,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => {
                        let narrative = narrate_strategy(&output);
                        emit_narrative(&narrative, &event_sender).await;
                        run.pipeline
                            .step(PipelineEvent::StrategyComplete { brief: output })
                    }
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::SpawnImplementer {
                ref prompt,
                ref context,
            } => {
                run.agents_spawned += 1;
                let full_prompt = if context.is_empty() {
                    prompt.clone()
                } else {
                    format!("{context}\n\n{prompt}")
                };
                let full_prompt = prepend_context(&full_prompt, &knowledge_context);
                let result = run_agent_phase(
                    session_id,
                    "Implementer",
                    &full_prompt,
                    workdir,
                    &config.model_slug,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => run.pipeline.step(PipelineEvent::AgentCompleted {
                        output,
                        files_changed: 0,
                    }),
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::SpawnAutoFixer { ref error_output } => {
                run.agents_spawned += 1;
                let fix_prompt = prepend_context(
                    &format!("Fix the following errors. Make minimal changes:\n\n{error_output}"),
                    &knowledge_context,
                );
                let result = run_agent_phase(
                    session_id,
                    "AutoFixer",
                    &fix_prompt,
                    workdir,
                    &config.model_slug,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match result {
                    Ok(output) => run.pipeline.step(PipelineEvent::AgentCompleted {
                        output,
                        files_changed: 0,
                    }),
                    Err(e) => run.pipeline.step(PipelineEvent::AgentFailed {
                        error: e.to_string(),
                    }),
                };
            }

            PipelineAction::RunGates => {
                let gate_result = run_gates(
                    session_id,
                    workdir,
                    config.clippy_enabled,
                    config.tests_enabled,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match gate_result {
                    Ok(()) => {
                        emit_narrative(narrate_gates_passed(), &event_sender).await;
                        run.pipeline.step(PipelineEvent::GatesPassed)
                    }
                    Err(e) => {
                        let error_str = e.to_string();
                        let attempt = run.pipeline.iteration + 1;
                        let narrative = narrate_gate_failure(&error_str, attempt);
                        emit_narrative(&narrative, &event_sender).await;
                        let autopsy =
                            analyze_gate_failure(&error_str, workdir, run.pipeline.iteration).await;
                        let GateAutopsy {
                            error_type,
                            root_cause,
                            causal_chain,
                            similar_past,
                            confidence,
                        } = autopsy;

                        let autopsy_id = format!("gate-autopsy-{}", run.pipeline.iteration);
                        let _ = event_sender
                            .send(CognitiveEvent::ToolCallStart {
                                tool_call_id: autopsy_id.clone(),
                                title: format!("Gate autopsy: {}", error_type.description()),
                                kind: ToolCallKind::Other,
                                locations: None,
                            })
                            .await;

                        let mut card_text =
                            format!("**Root cause:** {root_cause}\n\n**Causal chain:**\n");
                        for (index, step) in causal_chain.iter().enumerate() {
                            card_text.push_str(&format!("  {}. {}\n", index + 1, step));
                        }
                        if let Some((task_id, resolution)) = &similar_past {
                            card_text.push_str(&format!(
                                "\n**Similar past failure:** episode `{task_id}`\n  Reflection/resolution: {resolution}\n"
                            ));
                        }
                        card_text.push_str(&format!(
                            "\n**Confidence:** {:.0}% ({})",
                            confidence * 100.0,
                            error_type.description()
                        ));

                        let _ = event_sender
                            .send(CognitiveEvent::ToolCallComplete {
                                tool_call_id: autopsy_id,
                                status: ToolCallStatus::Completed,
                                content: vec![ContentBlock::Text { text: card_text }],
                            })
                            .await;

                        let enhanced_error = if let Some((task_id, resolution)) = &similar_past {
                            format!(
                                "Fix the following {} error.\nRoot cause: {root_cause}\nSimilar past episode: {task_id}\nSimilar past resolution: {resolution}\n\nError output:\n{error_str}",
                                error_type.description()
                            )
                        } else {
                            format!(
                                "Fix the following {} error.\nRoot cause: {root_cause}\n\nError output:\n{error_str}",
                                error_type.description()
                            )
                        };
                        run.pipeline.step(PipelineEvent::GateFailed {
                            gate: "gate".into(),
                            output: enhanced_error,
                        })
                    }
                };
            }

            PipelineAction::SpawnReviewer { .. } => {
                // If review_strictness is "none", skip review entirely.
                if config.review_strictness == "none" {
                    action = run.pipeline.step(PipelineEvent::ReviewApproved {
                        summary: "Review skipped (strictness=none)".into(),
                    });
                } else if config.review_strictness == "thorough" {
                    // Multi-role review: architect + auditor, both must approve.
                    action = run_multi_role_review(
                        session_id,
                        &mut run,
                        workdir,
                        &config,
                        &knowledge_context,
                        &cancel_token,
                        &event_sender,
                    )
                    .await;
                } else {
                    // Single reviewer (quick/standard).
                    action = run_single_review(
                        session_id,
                        &mut run,
                        workdir,
                        &config,
                        &knowledge_context,
                        &cancel_token,
                        &event_sender,
                    )
                    .await;
                }
            }

            PipelineAction::Commit => {
                let commit_result = run_commit(
                    session_id,
                    workdir,
                    &run.pipeline.original_prompt,
                    &cancel_token,
                    &event_sender,
                )
                .await;
                action = match commit_result {
                    Ok(msg) => {
                        let real_count = if msg != NO_CHANGES_TO_COMMIT_MESSAGE {
                            let changes = detect_file_changes(workdir).await;

                            // Batch: single async git diff for all files.
                            let all_diffs = Command::new("git")
                                .args(["diff", "HEAD~1"])
                                .current_dir(workdir)
                                .output()
                                .await
                                .ok()
                                .and_then(|o| {
                                    o.status
                                        .success()
                                        .then(|| String::from_utf8(o.stdout).ok())
                                        .flatten()
                                })
                                .unwrap_or_default();
                            let file_diffs = split_diff_by_file(&all_diffs);

                            for (index, change) in changes.iter().enumerate() {
                                let tool_call_id = format!("file-change-{}", index + 1);
                                let (title_prefix, kind) = match change.change_type {
                                    FileChangeType::Added => ("+", ToolCallKind::Create),
                                    FileChangeType::Modified => ("~", ToolCallKind::Edit),
                                    FileChangeType::Deleted => ("-", ToolCallKind::Delete),
                                    FileChangeType::Renamed => (">", ToolCallKind::Edit),
                                };
                                let _ = event_sender
                                    .send(CognitiveEvent::ToolCallStart {
                                        tool_call_id: tool_call_id.clone(),
                                        title: format!("{title_prefix} {}", change.path),
                                        kind,
                                        locations: Some(vec![crate::types::ToolCallLocation {
                                            uri: format!(
                                                "file://{}/{}",
                                                workdir.display(),
                                                change.path
                                            ),
                                            range: None,
                                        }]),
                                    })
                                    .await;
                                // Look up diff from batched result instead of per-file subprocess.
                                let diff_output = file_diffs
                                    .get(&change.path)
                                    .cloned()
                                    .filter(|s| !s.is_empty());

                                let content = vec![ContentBlock::Diff {
                                    path: change.path.clone(),
                                    old_text: None,
                                    new_text: None,
                                    diff: diff_output,
                                }];
                                let _ = event_sender
                                    .send(CognitiveEvent::ToolCallComplete {
                                        tool_call_id,
                                        status: ToolCallStatus::Completed,
                                        content,
                                    })
                                    .await;
                            }

                            changes.len() as u32
                        } else {
                            0
                        };

                        run.pipeline.files_changed = real_count;
                        let narrative = narrate_commit(&msg);
                        emit_narrative(&narrative, &event_sender).await;
                        run.pipeline
                            .step(PipelineEvent::CommitDone { message: msg })
                    }
                    Err(e) => {
                        error!(error = %e, "commit failed");
                        // Still mark as done — user can commit manually.
                        run.pipeline.step(PipelineEvent::CommitDone {
                            message: format!("(commit failed: {e})"),
                        })
                    }
                };
            }

            PipelineAction::Done => {
                run.mark_complete();
                sync_shared_run(&shared_run, &run).await;
                emit_plan_update(&run, &event_sender).await;

                // Emit final summary message.
                let elapsed = run.elapsed().num_seconds();
                let summary = format!(
                    "\n\n---\nWorkflow complete ({} pipeline).\n\
                     Duration: {}s | Agents: {} | Iterations: {}/{}",
                    run.template_name(),
                    elapsed,
                    run.agents_spawned,
                    run.pipeline.iteration,
                    run.pipeline.max_iterations,
                );
                let _ = event_sender.send(CognitiveEvent::TokenChunk(summary)).await;
                let _ = event_sender
                    .send(CognitiveEvent::Complete {
                        stop_reason: StopReason::EndTurn,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }

            PipelineAction::Halt { reason } => {
                run.mark_complete();
                sync_shared_run(&shared_run, &run).await;
                emit_plan_update(&run, &event_sender).await;

                let msg = format!("\n\n---\nWorkflow halted: {reason}");
                let _ = event_sender.send(CognitiveEvent::TokenChunk(msg)).await;
                let _ = event_sender
                    .send(CognitiveEvent::Complete {
                        stop_reason: StopReason::EndTurn,
                        usage: None,
                    })
                    .await;
                return Ok(());
            }
        }
    }
}

/// Sync the shared workflow run handle so slash commands can read live state.
async fn sync_shared_run(shared: &SharedWorkflowRun, run: &WorkflowRun) {
    let mut guard = shared.lock().await;
    *guard = Some(run.clone());
}

/// Emit a plan update reflecting the current pipeline state.
async fn emit_plan_update(run: &WorkflowRun, sender: &mpsc::Sender<CognitiveEvent>) {
    let entries = build_plan_entries(run);
    if !entries.is_empty() {
        let _ = sender.send(CognitiveEvent::PlanUpdate { entries }).await;
    }
}

/// Emit a best-effort narrative token between phases.
async fn emit_narrative(text: &str, sender: &mpsc::Sender<CognitiveEvent>) {
    if !text.is_empty() {
        let _ = sender
            .send(CognitiveEvent::TokenChunk(text.to_string()))
            .await;
    }
}

/// Generate the inline badge that marks the current pipeline phase.
fn phase_badge(phase: &PipelinePhase, iteration: u32) -> Option<String> {
    let (icon, name) = match phase {
        PipelinePhase::Strategizing => ("🧭", "Strategizing"),
        PipelinePhase::Implementing => ("🛠️", "Implementing"),
        PipelinePhase::AutoFixing => ("🔧", "Auto-fixing"),
        PipelinePhase::Gating => ("🧪", "Running gates"),
        PipelinePhase::Reviewing => ("🔍", "Reviewing"),
        PipelinePhase::Committing => ("📝", "Committing"),
        PipelinePhase::Complete => ("✅", "Complete"),
        PipelinePhase::Halted { .. } => ("⛔", "Halted"),
        PipelinePhase::Cancelled => ("⏹️", "Cancelled"),
        PipelinePhase::Pending => return None,
    };

    let name = if iteration > 1
        && matches!(
            phase,
            PipelinePhase::Implementing | PipelinePhase::AutoFixing | PipelinePhase::Gating
        ) {
        format!("{name} (iter {iteration})")
    } else {
        name.to_string()
    };

    Some(format!("\n\n{icon} **{name}**\n\n"))
}

/// Narrative after strategy agent completes.
fn narrate_strategy(brief: &str) -> String {
    let first_line = brief
        .lines()
        .find(|line| !line.trim().is_empty() && line.len() > 10)
        .unwrap_or(brief);
    let truncated: String = first_line.chars().take(120).collect();
    let body = if first_line.chars().count() > 120 {
        format!("{truncated}...")
    } else {
        truncated
    };
    format!("Approach: {body}\n\n")
}

/// Narrative after all gates pass.
fn narrate_gates_passed() -> &'static str {
    "All gates passed.\n\n"
}

/// Narrative after gate failure.
fn narrate_gate_failure(error_output: &str, iteration: u32) -> String {
    let test_failures = error_output
        .lines()
        .filter(|line| line.contains("FAILED") || line.contains("test result: FAILED"))
        .count();
    let error_count = error_output
        .lines()
        .filter(|line| {
            line.contains("error[E") || (line.starts_with("error:") && line.contains(':'))
        })
        .count();

    if test_failures > 0 {
        format!(
            "{} test{} fail. Moving to auto-fixer (attempt {iteration}).\n\n",
            test_failures,
            if test_failures == 1 { "" } else { "s" }
        )
    } else if error_count > 0 {
        format!(
            "{} compile error{}. Moving to auto-fixer (attempt {iteration}).\n\n",
            error_count,
            if error_count == 1 { "" } else { "s" }
        )
    } else {
        format!("Gate failed. Moving to auto-fixer (attempt {iteration}).\n\n")
    }
}

/// Narrative after commit.
fn narrate_commit(message: &str) -> String {
    if message.is_empty() || message.starts_with("(commit failed:") {
        String::new()
    } else {
        let short = message.lines().next().unwrap_or(message);
        let short: String = short.chars().take(72).collect();
        format!("Committed: {short}\n\n")
    }
}

/// Build plan entries from the current run state.
fn build_plan_entries(run: &WorkflowRun) -> Vec<PlanEntry> {
    let phase = &run.pipeline.phase;
    let template = &run.pipeline.template;

    let mut entries = Vec::new();

    // Strategy phase (full only).
    if template.has_strategy() {
        let status = match phase {
            PipelinePhase::Strategizing => PlanStatus::InProgress,
            PipelinePhase::Pending => PlanStatus::Pending,
            _ => PlanStatus::Completed,
        };
        entries.push(PlanEntry {
            content: "Strategy brief".into(),
            priority: Priority::High,
            status,
        });
    }

    // Implementation phase.
    let impl_status = match phase {
        PipelinePhase::Implementing | PipelinePhase::AutoFixing => PlanStatus::InProgress,
        PipelinePhase::Pending | PipelinePhase::Strategizing => PlanStatus::Pending,
        _ => PlanStatus::Completed,
    };
    let impl_label = if run.pipeline.iteration > 1 {
        format!(
            "Implementation (attempt {}/{})",
            run.pipeline.iteration, run.pipeline.max_iterations
        )
    } else {
        "Implementation".into()
    };
    entries.push(PlanEntry {
        content: impl_label,
        priority: Priority::High,
        status: impl_status,
    });

    // Gates phase.
    let gate_status = match phase {
        PipelinePhase::Gating => PlanStatus::InProgress,
        PipelinePhase::Pending
        | PipelinePhase::Strategizing
        | PipelinePhase::Implementing
        | PipelinePhase::AutoFixing => PlanStatus::Pending,
        _ => PlanStatus::Completed,
    };
    entries.push(PlanEntry {
        content: "Run gates (compile + test)".into(),
        priority: Priority::Medium,
        status: gate_status,
    });

    // Review phase (standard, full only).
    if template.has_review() {
        let review_status = match phase {
            PipelinePhase::Reviewing => PlanStatus::InProgress,
            PipelinePhase::Committing | PipelinePhase::Complete => PlanStatus::Completed,
            _ => PlanStatus::Pending,
        };
        entries.push(PlanEntry {
            content: "Code review".into(),
            priority: Priority::Medium,
            status: review_status,
        });
    }

    // Commit phase.
    let commit_status = match phase {
        PipelinePhase::Committing => PlanStatus::InProgress,
        PipelinePhase::Complete => PlanStatus::Completed,
        _ => PlanStatus::Pending,
    };
    entries.push(PlanEntry {
        content: "Commit changes".into(),
        priority: Priority::Low,
        status: commit_status,
    });

    entries
}

/// JSON schema hint appended to review prompts for structured output.
const REVIEW_JSON_SCHEMA: &str = r#"
Respond with a JSON object (no markdown fences needed):
{
  "status": "passed" | "failed" | "needs_human",
  "confidence": 0.0-1.0,
  "blocking_findings": ["list of blocking issues"],
  "non_blocking_findings": ["list of advisory issues"],
  "required_next_action": "none" | "needs_human_review" | "needs_rework",
  "evidence_refs": []
}"#;

/// Build a review prompt appropriate for the configured strictness level.
fn build_review_prompt(strictness: &str, original_prompt: &str, knowledge_context: &str) -> String {
    let base = match strictness {
        "quick" => format!(
            "Quickly review the recent changes. Only flag blocking issues (bugs, security).\n\n\
             Original request: {original_prompt}"
        ),
        "thorough" => format!(
            "Perform a thorough review of the recent changes. Check:\n\
             1. Correctness and edge cases\n\
             2. Security vulnerabilities\n\
             3. Architecture and design patterns\n\
             4. Documentation completeness\n\
             5. Test coverage\n\n\
             Original request: {original_prompt}"
        ),
        _ => format!(
            "Review the recent changes in this workspace. Focus on correctness, security, \
             and code quality.\n\n\
             Original request: {original_prompt}"
        ),
    };
    prepend_context(&format!("{base}\n{REVIEW_JSON_SCHEMA}"), knowledge_context)
}

/// Parse a reviewer's output into a pipeline event (approved or revise).
fn parse_review_output(
    output: &str,
    run: &WorkflowRun,
    session_id: &str,
    role_id: &str,
) -> (bool, Vec<String>) {
    let ctx = ReviewVerdictContext {
        verdict_id: format!("acp-{}-{role_id}", run.run_id),
        batch_id: session_id.to_string(),
        task_id: run.run_id.clone(),
        reviewer_role_id: role_id.to_string(),
        raw_output_ref: String::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let parsed = parse_structured_review_verdict(output, ctx);
    if parsed.passed() {
        (true, Vec::new())
    } else {
        let findings = if !parsed.evidence.blocking_findings.is_empty() {
            parsed.evidence.blocking_findings.clone()
        } else {
            let lines: Vec<String> = output
                .lines()
                .filter(|l| l.starts_with("- ") || l.starts_with("* "))
                .map(|l| {
                    l.trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .to_owned()
                })
                .collect();
            if lines.is_empty() {
                vec![output.to_string()]
            } else {
                lines
            }
        };
        (false, findings)
    }
}

/// Run a single-reviewer review (quick/standard strictness).
async fn run_single_review(
    session_id: &str,
    run: &mut WorkflowRun,
    workdir: &Path,
    config: &PipelineConfig,
    knowledge_context: &str,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> PipelineAction {
    run.agents_spawned += 1;
    let review_prompt = build_review_prompt(
        &config.review_strictness,
        &run.pipeline.original_prompt,
        knowledge_context,
    );
    let review_result = run_agent_phase(
        session_id,
        "Reviewer",
        &review_prompt,
        workdir,
        &config.model_slug,
        cancel_token,
        event_sender,
    )
    .await;
    match review_result {
        Ok(output) => {
            let (approved, findings) =
                parse_review_output(&output, run, session_id, &config.review_strictness);
            if approved {
                run.pipeline
                    .step(PipelineEvent::ReviewApproved { summary: output })
            } else {
                run.pipeline.step(PipelineEvent::ReviewRevise { findings })
            }
        }
        Err(e) => {
            warn!(error = %e, "reviewer failed, treating as approved");
            run.pipeline.step(PipelineEvent::ReviewApproved {
                summary: "Review skipped (agent error)".into(),
            })
        }
    }
}

/// Run a multi-role review for "thorough" mode.
///
/// Two reviewers run sequentially: an architect (design/patterns) and an
/// auditor (security/correctness). Both must approve for the review to pass.
/// If either revises, all findings are merged.
async fn run_multi_role_review(
    session_id: &str,
    run: &mut WorkflowRun,
    workdir: &Path,
    config: &PipelineConfig,
    knowledge_context: &str,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> PipelineAction {
    let original_prompt = &run.pipeline.original_prompt;

    let architect_prompt = format!(
        "You are the **Architect Reviewer**. Focus on:\n\
         1. Architecture and design pattern adherence\n\
         2. API contract correctness\n\
         3. Dependency layering violations\n\
         4. Code organization and modularity\n\n\
        Original request: {original_prompt}\n{REVIEW_JSON_SCHEMA}"
    );
    let architect_prompt = prepend_context(&architect_prompt, knowledge_context);

    let auditor_prompt = format!(
        "You are the **Security & Correctness Auditor**. Focus on:\n\
         1. Security vulnerabilities (injection, auth bypass, data leaks)\n\
         2. Edge cases and error handling\n\
         3. Resource leaks (files, connections, memory)\n\
         4. Test coverage gaps\n\n\
        Original request: {original_prompt}\n{REVIEW_JSON_SCHEMA}"
    );
    let auditor_prompt = prepend_context(&auditor_prompt, knowledge_context);

    let mut all_findings: Vec<String> = Vec::new();
    let mut all_approved = true;

    // Architect review.
    run.agents_spawned += 1;
    let arch_result = run_agent_phase(
        session_id,
        "Architect",
        &architect_prompt,
        workdir,
        &config.model_slug,
        cancel_token,
        event_sender,
    )
    .await;
    match arch_result {
        Ok(output) => {
            let (approved, findings) = parse_review_output(&output, run, session_id, "architect");
            if !approved {
                all_approved = false;
                all_findings.extend(findings.into_iter().map(|f| format!("[architect] {f}")));
            }
        }
        Err(e) => {
            warn!(error = %e, "architect reviewer failed, continuing");
        }
    }

    // Auditor review.
    run.agents_spawned += 1;
    let audit_result = run_agent_phase(
        session_id,
        "Auditor",
        &auditor_prompt,
        workdir,
        &config.model_slug,
        cancel_token,
        event_sender,
    )
    .await;
    match audit_result {
        Ok(output) => {
            let (approved, findings) = parse_review_output(&output, run, session_id, "auditor");
            if !approved {
                all_approved = false;
                all_findings.extend(findings.into_iter().map(|f| format!("[auditor] {f}")));
            }
        }
        Err(e) => {
            warn!(error = %e, "auditor reviewer failed, continuing");
        }
    }

    if all_approved {
        run.pipeline.step(PipelineEvent::ReviewApproved {
            summary: "Both architect and auditor approved".into(),
        })
    } else {
        run.pipeline.step(PipelineEvent::ReviewRevise {
            findings: all_findings,
        })
    }
}

/// Run a single agent phase using ClaudeCliAgent and stream output.
async fn run_agent_phase(
    session_id: &str,
    role: &str,
    prompt: &str,
    workdir: &Path,
    model_slug: &str,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<String> {
    // Emit tool call start.
    let tool_call_id = format!("phase-{}-{}", role.to_lowercase(), uuid::Uuid::new_v4());
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: format!("{role}: working..."),
            kind: ToolCallKind::Other,
            locations: None,
        })
        .await;

    // Run through the shared Claude CLI agent adapter.
    let output = run_claude_cli_via_agent(prompt, workdir, model_slug, cancel_token).await;

    match &output {
        Ok(text) => {
            let safety = safety_layer_for_pipeline_role(role);
            let violations: Vec<SafetyViolation> =
                safety.post_dispatch_check(session_id, "pipeline-phase", role, text, &[]);
            log_safety_violations(role, &violations);

            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Completed,
                    content: vec![ContentBlock::Text {
                        text: format!("[{role}] Done ({} chars)", text.len()),
                    }],
                })
                .await;
        }
        Err(e) => {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Failed,
                    content: vec![ContentBlock::Text {
                        text: format!("[{role}] Failed: {e}"),
                    }],
                })
                .await;
        }
    }

    output
}

/// Build an Engram signal with a GatePayload body pointing at `workdir`.
fn build_gate_signal(workdir: &Path) -> Engram {
    let payload = GatePayload::in_dir(workdir);
    let body = Body::from_json(&payload).unwrap_or_else(|_| Body::empty());
    Engram::builder(Kind::Task).body(body).build()
}

/// Path to adaptive gate thresholds relative to workdir.
const THRESHOLDS_PATH: &str = ".roko/learn/gate-thresholds.json";

/// Run a gate pipeline using roko-gate's proper Verify trait.
///
/// Runs CompileGate, optionally TestGate, optionally ClippyGate.
/// Each gate gets its own tool_call event in Zed. Results update
/// adaptive thresholds for future skip/retry decisions.
async fn run_gates(
    _session_id: &str,
    workdir: &Path,
    clippy_enabled: bool,
    tests_enabled: bool,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<()> {
    let signal = build_gate_signal(workdir);
    let ctx = Context::at(chrono::Utc::now().timestamp_millis());

    // Load adaptive thresholds (creates new if missing).
    let thresholds_path = workdir.join(THRESHOLDS_PATH);
    let mut thresholds = AdaptiveThresholds::load_or_new(&thresholds_path);

    // Compile gate (rung 0).
    let compile_result = run_verify_gate(
        "compile",
        &CompileGate::cargo(),
        &signal,
        &ctx,
        cancel_token,
        event_sender,
    )
    .await;
    thresholds.observe(0, compile_result.is_ok());
    compile_result?;

    // Test gate (rung 2).
    if tests_enabled {
        if thresholds.should_skip_rung(2) {
            debug!("skipping test gate (adaptive: {} consecutive passes)", 20);
        } else {
            let test_result = run_verify_gate(
                "test",
                &TestGate::cargo(),
                &signal,
                &ctx,
                cancel_token,
                event_sender,
            )
            .await;
            thresholds.observe(2, test_result.is_ok());
            if let Err(e) = test_result {
                save_thresholds(&thresholds, &thresholds_path);
                return Err(e);
            }
        }
    }

    // Clippy gate (rung 1).
    if clippy_enabled {
        if thresholds.should_skip_rung(1) {
            debug!("skipping clippy gate (adaptive: {} consecutive passes)", 20);
        } else {
            let clippy_result = run_verify_gate(
                "clippy",
                &ClippyGate::cargo(),
                &signal,
                &ctx,
                cancel_token,
                event_sender,
            )
            .await;
            thresholds.observe(1, clippy_result.is_ok());
            if let Err(e) = clippy_result {
                save_thresholds(&thresholds, &thresholds_path);
                return Err(e);
            }
        }
    }

    // Persist updated thresholds.
    save_thresholds(&thresholds, &thresholds_path);

    Ok(())
}

/// Persist adaptive thresholds, logging on error.
fn save_thresholds(thresholds: &AdaptiveThresholds, path: &Path) {
    if let Err(e) = thresholds.save(path) {
        warn!(error = %e, "failed to save adaptive gate thresholds");
    }
}

/// Run a single roko-gate `Verify` impl and emit ACP tool_call events.
async fn run_verify_gate(
    gate_name: &str,
    gate: &dyn Verify,
    signal: &Engram,
    ctx: &Context,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<()> {
    let tool_call_id = format!("gate-{gate_name}");

    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: format!("Gate: {gate_name}"),
            kind: ToolCallKind::Terminal,
            locations: None,
        })
        .await;

    if cancel_token.is_cancelled() {
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: "Cancelled".into(),
                }],
            })
            .await;
        return Err(anyhow::anyhow!("cancelled"));
    }

    let verdict = gate.verify(signal, ctx).await;

    if verdict.passed {
        let detail_summary = verdict
            .detail
            .as_deref()
            .map(|d| {
                // Show first line of detail for context.
                d.lines().next().unwrap_or("")
            })
            .unwrap_or("");
        let test_info = verdict
            .test_count
            .map(|tc| format!(" ({} passed, {} failed)", tc.passed, tc.failed))
            .unwrap_or_default();
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Completed,
                content: vec![ContentBlock::Text {
                    text: format!(
                        "\u{2713} {gate_name} passed ({}ms){test_info}",
                        verdict.duration_ms,
                    ),
                }],
            })
            .await;
        if !detail_summary.is_empty() {
            debug!(gate = gate_name, detail = detail_summary, "gate detail");
        }
        Ok(())
    } else {
        let error_text = if verdict.reason.is_empty() {
            verdict
                .detail
                .as_deref()
                .unwrap_or("unknown error")
                .to_string()
        } else {
            verdict.reason.clone()
        };
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: format!(
                        "\u{2717} {gate_name} failed ({}ms):\n{error_text}",
                        verdict.duration_ms,
                    ),
                }],
            })
            .await;
        Err(anyhow::anyhow!("{gate_name} failed:\n{error_text}"))
    }
}

/// Run Claude CLI through the shared agent adapter and capture output.
async fn run_claude_cli_via_agent(
    prompt: &str,
    workdir: &Path,
    model_slug: &str,
    cancel_token: &CancelToken,
) -> anyhow::Result<String> {
    if cancel_token.is_cancelled() {
        return Err(anyhow::anyhow!("cancelled"));
    }

    let agent = ClaudeCliAgent::new(CLAUDE_CLI_BIN, workdir, model_slug)
        .with_settings_json(build_settings_json());
    let input = Engram::builder(Kind::Task).body(Body::text(prompt)).build();
    let ctx = Context::now();

    let result = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            return Err(anyhow::anyhow!("cancelled"));
        }
        result = agent.run(&input, &ctx) => result,
    };

    let text = result.output.body.as_text().unwrap_or("").to_string();
    if result.success {
        Ok(text)
    } else {
        let error_text = if text.is_empty() {
            "agent failed".to_string()
        } else {
            text
        };
        Err(anyhow::anyhow!("agent failed: {error_text}"))
    }
}

/// Create a commit for the workflow output.
async fn run_commit(
    _session_id: &str,
    workdir: &Path,
    original_prompt: &str,
    cancel_token: &CancelToken,
    event_sender: &mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<String> {
    if cancel_token.is_cancelled() {
        return Err(anyhow::anyhow!("cancelled"));
    }

    let tool_call_id = "commit".to_owned();
    let _ = event_sender
        .send(CognitiveEvent::ToolCallStart {
            tool_call_id: tool_call_id.clone(),
            title: "Creating commit".into(),
            kind: ToolCallKind::Terminal,
            locations: None,
        })
        .await;

    // Stage all changes.
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(workdir)
        .output()
        .await?;

    if !add_output.status.success() {
        let err = String::from_utf8_lossy(&add_output.stderr).to_string();
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Failed,
                content: vec![ContentBlock::Text {
                    text: format!("git add failed: {err}"),
                }],
            })
            .await;
        return Err(anyhow::anyhow!("git add failed: {err}"));
    }

    // Generate commit message from the prompt (truncated).
    let msg = if original_prompt.len() > 72 {
        format!("feat: {}", &original_prompt[..69])
    } else {
        format!("feat: {original_prompt}")
    };

    let commit_output = Command::new("git")
        .args(["commit", "-m", &msg])
        .current_dir(workdir)
        .output()
        .await?;

    if commit_output.status.success() {
        let _ = event_sender
            .send(CognitiveEvent::ToolCallComplete {
                tool_call_id,
                status: ToolCallStatus::Completed,
                content: vec![ContentBlock::Text {
                    text: format!("\u{2713} Committed: {msg}"),
                }],
            })
            .await;
        Ok(msg)
    } else {
        let stderr = String::from_utf8_lossy(&commit_output.stderr).to_string();
        // It's okay if there's nothing to commit.
        if stderr.contains("nothing to commit") {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Completed,
                    content: vec![ContentBlock::Text {
                        text: "No changes to commit".into(),
                    }],
                })
                .await;
            Ok(NO_CHANGES_TO_COMMIT_MESSAGE.into())
        } else {
            let _ = event_sender
                .send(CognitiveEvent::ToolCallComplete {
                    tool_call_id,
                    status: ToolCallStatus::Failed,
                    content: vec![ContentBlock::Text {
                        text: format!("git commit failed: {stderr}"),
                    }],
                })
                .await;
            Err(anyhow::anyhow!("git commit failed: {stderr}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::session::AcpSession;
    use roko_compose::SystemPromptBuilder;

    #[test]
    fn system_prompt_builder_is_available_for_acp_workflows() {
        let tmp = tempfile::tempdir().expect("create tmpdir");
        std::fs::write(tmp.path().join("CLAUDE.md"), "Use snake_case.").expect("write claude");

        let prompt = SystemPromptBuilder::new("You are an expert code implementer.")
            .with_conventions(
                AcpSession::load_conventions(tmp.path()).expect("load cached conventions"),
            )
            .with_domain("Working directory: /tmp/workspace")
            .with_task("Review the ACP workflow dispatch path")
            .build();

        assert!(prompt.contains("Use snake_case."));
        assert!(prompt.contains("Working directory: /tmp/workspace"));
        assert!(prompt.contains("Review the ACP workflow dispatch path"));
    }
}
