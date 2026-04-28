//! The universal loop: prompt → compose → agent → gate → persist → policy.
//!
//! This is the body of `roko run <prompt>`. It reads [`Config`], opens a
//! [`FileSubstrate`] under `.roko/`, seeds prompt sections, composes them
//! into a single Prompt signal, invokes the configured agent backend, runs
//! each configured gate on the working directory, and emits an Episode.

use crate::agent_config::{
    synthesize_claude_cli_config, synthesize_known_protocol_config, synthesize_subprocess_config,
};
use crate::agent_spawn::{SpawnAgentSpec, spawn_agent_scoped};
use crate::clean;
use crate::config::{Config, GateConfig, PromptFile};
use crate::episode::EpisodePolicy;
use crate::output_format;
use crate::prompting::{PromptBuildOptions, build_role_system_prompt_validated};
use crate::state_hub::{StateHub, StateHubSender};
use anyhow::{Context as _, Result, anyhow};
use chrono::Utc;
use roko_agent::provider::is_known_protocol_command;
use roko_agent::translate::{ClaudeTranslator, OllamaTranslator, RenderedTools, Translator};
use roko_agent::{AgentResult, OllamaLlmBackend};
use roko_compose::{Placement, PromptComposer, PromptSection, SectionPriority, TaskContext};
use roko_core::agent::resolve_model;
use roko_core::dashboard_snapshot::DashboardEvent;
use roko_core::foundation::{
    EventConsumer as WorkflowEventConsumer, ShellGateCommand as CoreShellGateCommand,
};
use roko_core::metric::{ConfigHash, TaskMetric};
use roko_core::tool::ExternalAction;
use roko_core::tool::ToolRegistry;
use roko_core::{
    AgentRole, Body, Budget, Compose, Context, Engram, Kind, Provenance, Store, Verdict, Verify,
};
use roko_fs::FileSubstrate;
use roko_gate::{BuildSystem, ClippyGate, CompileGate, GatePayload, ShellGate, TestGate};
use roko_learn::episode_logger::{Episode, GateVerdict, Usage as EpisodeUsage};
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};
use roko_orchestrator::{ServiceConfig, ServiceFactory};
use roko_runtime::effect_driver::EffectServices;
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowOutcome, WorkflowRunConfig};
use roko_std::NoOpScorer;
use roko_std::StaticToolRegistry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Summary of a single `run` invocation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunReport {
    /// Content hash of the episode signal emitted at the end.
    pub episode_id: String,
    /// Content hash of the assembled prompt signal.
    pub prompt_id: String,
    /// Content hash of the agent's output signal.
    pub agent_output_id: String,
    /// Whether the agent invocation succeeded (exit code 0, no timeout).
    pub agent_success: bool,
    /// Per-gate verdicts in declaration order: (gate name, passed).
    pub gate_verdicts: Vec<(String, bool)>,
    /// How many signals are now in the substrate.
    pub total_signals: usize,
    /// Final agent output text, if it was a text payload.
    pub output_text: Option<String>,
}

impl RunReport {
    /// True if the agent succeeded and every configured gate passed.
    #[must_use]
    pub fn overall_success(&self) -> bool {
        self.agent_success && self.gate_verdicts.iter().all(|(_, ok)| *ok)
    }
}

/// Write a RunReport to `.roko/shared/{token}.json` and return the token.
pub fn write_shared_run(workdir: &std::path::Path, report: &RunReport) -> anyhow::Result<String> {
    let token = roko_core::generate_share_token();
    let dir = workdir.join(".roko").join("shared");
    std::fs::create_dir_all(&dir)?;
    let transcript = serde_json::json!({
        "id": &token,
        "agent": "unknown",
        "role": "unknown",
        "prompt": &report.prompt_id,
        "success": report.overall_success(),
        "gates": &report.gate_verdicts,
        "output": &report.output_text,
        "cost_usd": null,
        "input_tokens": null,
        "output_tokens": null,
        "model": null,
        "duration_s": null,
        "episode_id": &report.episode_id,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(
        dir.join(format!("{token}.json")),
        serde_json::to_string_pretty(&transcript)?,
    )?;

    output_format::divider();
    output_format::step("Shared", "");
    output_format::bar(&output_format::cyan(&format!(
        "http://localhost:6677/runs/{token}"
    )));
    output_format::note("run with --serve to make the URL accessible");

    Ok(token)
}

/// Summary of running a plan through the WorkflowEngine.
#[derive(Debug)]
pub struct PlanWorkflowReport {
    /// Total tasks attempted.
    pub total: usize,
    /// Tasks that completed successfully.
    pub passed: usize,
    /// Tasks that failed.
    pub failed: usize,
    /// Per-task outcomes: `(task_id, success, message)`.
    pub outcomes: Vec<(String, bool, String)>,
}

/// Bridges WorkflowEngine lifecycle events to the StateHub for TUI/SSE/WS
/// consumption.
struct StateHubBridge {
    sender: StateHubSender,
}

impl WorkflowEventConsumer for StateHubBridge {
    fn consume(&self, event: &roko_core::RuntimeEvent) {
        match event {
            roko_core::RuntimeEvent::WorkflowStarted {
                run_id,
                template,
                prompt,
            } => {
                self.sender.publish(DashboardEvent::PlanStarted {
                    plan_id: run_id.clone(),
                });
                self.sender.publish(DashboardEvent::TaskStarted {
                    plan_id: run_id.clone(),
                    task_id: "workflow".to_string(),
                    title: truncate(prompt, 60).to_string(),
                    phase: format!("starting ({template})"),
                });
            }
            roko_core::RuntimeEvent::PhaseTransition { run_id, from, to } => {
                self.sender.publish(DashboardEvent::PhaseTransition {
                    plan_id: run_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                });
            }
            roko_core::RuntimeEvent::WorkflowCompleted { run_id, outcome } => {
                let success = matches!(outcome, roko_core::WorkflowOutcome::Success { .. });
                self.sender.publish(DashboardEvent::TaskCompleted {
                    plan_id: run_id.clone(),
                    task_id: "workflow".to_string(),
                    outcome: format!("{outcome:?}"),
                });
                self.sender.publish(DashboardEvent::PlanCompleted {
                    plan_id: run_id.clone(),
                    success,
                });
            }
            _ => {}
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> &str {
    text.char_indices()
        .nth(max_chars)
        .map_or(text, |(idx, _)| &text[..idx])
}

/// Format a duration for human display: "3.2s", "1m 42s", "0.8s".
fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let mins = secs as u64 / 60;
        let remaining = secs as u64 % 60;
        format!("{mins}m {remaining}s")
    }
}

fn build_workflow_effect_services(workdir: &std::path::Path) -> anyhow::Result<EffectServices> {
    let config = crate::config::load_layered(workdir)
        .map(|resolved| resolved.config)
        .unwrap_or_default();
    let mut model_config = roko_core::config::load_config(workdir).unwrap_or_default();
    model_config.apply_process_env();
    crate::config::merge_global_providers(&mut model_config);
    model_config.providers.extend(config.providers.clone());
    model_config.models.extend(config.models.clone());
    model_config.agent.command = Some(config.agent.command.clone());
    model_config.agent.args = Some(config.agent.args.clone());
    model_config.agent.timeout_ms = Some(config.agent.timeout_ms);
    model_config.agent.env = Some(config.agent.env.clone());
    model_config.agent.default_effort = config.agent.effort.clone();
    model_config.agent.bare_mode = config.agent.bare_mode;
    model_config.agent.fallback_model = config.agent.fallback_model.clone();
    model_config.agent.tier_models = config.agent.tier_models.clone();
    if let Some(model) = config.agent.model.clone() {
        model_config.agent.default_model = model;
    }

    let services = ServiceFactory::build(ServiceConfig {
        workdir: workdir.to_path_buf(),
        roko_dir: workdir.join(".roko"),
        workspace_config: model_config,
        model_key: config.agent.model.clone(),
        mcp_config: config.agent.mcp_config.clone(),
        feedback_enabled: true,
        affect_enabled: true,
        run_id: Some(format!("cli_workflow_{}", Utc::now().timestamp_millis())),
    })
    .map_err(|error| anyhow!("build workflow services: {error}"))?;

    Ok(services.effect_services())
}

fn workflow_config_for_template(workflow_template: &str) -> WorkflowConfig {
    match workflow_template {
        "express" => WorkflowConfig::express(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::standard(),
    }
}

pub fn workflow_enabled_gate_names(gates: &[GateConfig]) -> Vec<String> {
    gates
        .iter()
        .map(|gate| match gate {
            GateConfig::Compile { .. } => "compile".to_string(),
            GateConfig::Clippy { .. } => "clippy".to_string(),
            GateConfig::Test { .. } => "test".to_string(),
            GateConfig::Shell { .. } => "shell".to_string(),
        })
        .collect()
}

pub fn workflow_shell_gate_commands(gates: &[GateConfig]) -> Vec<CoreShellGateCommand> {
    gates
        .iter()
        .filter_map(|gate| match gate {
            GateConfig::Shell {
                program,
                args,
                timeout_ms,
            } => Some(CoreShellGateCommand {
                program: program.clone(),
                args: args.clone(),
                timeout_ms: *timeout_ms,
            }),
            _ => None,
        })
        .collect()
}

/// Execute a prompt via the new WorkflowEngine (event-driven architecture).
///
/// This is an alternative to the existing orchestrate.rs path. It uses:
/// - PipelineStateV2 for state machine decisions
/// - EffectDriver for side-effect execution
/// - RuntimeEvent bus for observability
///
/// Enable via config or `--engine v2` flag (to be wired).
pub async fn run_with_workflow_engine(
    prompt: &str,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
) -> anyhow::Result<()> {
    run_with_workflow_engine_with_hub(
        prompt,
        workdir,
        workflow_template,
        enabled_gates,
        Vec::new(),
        None,
    )
    .await
}

/// Execute a prompt via the new WorkflowEngine and optionally publish lifecycle
/// events to an existing StateHub.
pub async fn run_with_workflow_engine_with_hub(
    prompt: &str,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
    external_hub: Option<&StateHub>,
) -> anyhow::Result<()> {
    let start_time = std::time::Instant::now();

    use roko_runtime::effect_driver::RuntimeEvent;
    use roko_runtime::jsonl_logger::{EventConsumer as RuntimeEventConsumer, JsonlLogger};

    struct JsonlWorkflowConsumer {
        logger: JsonlLogger,
    }

    impl RuntimeEventConsumer for JsonlWorkflowConsumer {
        fn consume(&self, event: &RuntimeEvent) {
            self.logger.consume(event);
        }
    }

    let services = build_workflow_effect_services(workdir)?;
    let gates_summary = (!enabled_gates.is_empty()).then(|| enabled_gates.join(", "));

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow: workflow_config_for_template(workflow_template),
        enabled_gates,
        shell_gates,
        commit_prefix: Some("feat".to_string()),
    };

    output_format::intro("roko run");
    output_format::step("prompt", &output_format::dim(&truncate(prompt, 60)));
    output_format::step("workflow", workflow_template);
    output_format::step("model", "claude-sonnet-4-20250514");
    output_format::divider();
    output_format::bar("starting workflow...");
    output_format::divider();

    // Run the workflow.
    let mut engine = WorkflowEngine::new(services);
    let roko_dir = workdir.join(".roko");
    let logger = JsonlLogger::from_roko_dir(&roko_dir);
    let consumer = Arc::new(JsonlWorkflowConsumer { logger });
    engine.add_consumer(consumer);

    // Bridge workflow events to the StateHub for TUI/SSE/WS consumers.
    if let Some(hub) = external_hub {
        let bridge = Arc::new(StateHubBridge {
            sender: hub.sender(),
        });
        engine.add_consumer(bridge);
    }

    let result = engine
        .run(config)
        .await
        .map_err(|error| anyhow!("workflow engine failed: {error}"))?;

    match &result.outcome {
        WorkflowOutcome::Success { commit_hash } => {
            let hash_str = commit_hash
                .as_deref()
                .map(|h| format!(" ({})", &h[..7.min(h.len())]))
                .unwrap_or_default();
            output_format::success(&format!(
                "workflow completed ({} iteration{}){hash_str}",
                result.iterations,
                if result.iterations == 1 { "" } else { "s" },
            ));
        }
        WorkflowOutcome::Halted { reason } => {
            output_format::error(&format!("workflow halted: {reason}"));
        }
        WorkflowOutcome::Cancelled => {
            output_format::warning("workflow cancelled");
        }
    }

    // Efficiency summary.
    let elapsed = start_time.elapsed();
    output_format::divider();
    output_format::step("Summary", "");
    output_format::branch(&format!(
        "duration   {}",
        output_format::cyan(&format_duration(elapsed)),
    ));
    output_format::branch(&format!(
        "iterations {}",
        output_format::cyan(&result.iterations.to_string()),
    ));
    if let Some(gates_summary) = gates_summary {
        output_format::branch(&format!(
            "gates      {}",
            output_format::dim(&gates_summary),
        ));
    }
    output_format::end(&output_format::dim(&result.run_id));

    Ok(())
}

/// Execute a plan's tasks via WorkflowEngine (v2 engine path for `roko plan run`).
///
/// Iterates over discovered task prompts and runs each sequentially through
/// the WorkflowEngine. Skips the 21K-line PlanRunner orchestration path.
pub async fn run_plan_with_workflow_engine(
    tasks: &[(String, String)],
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
) -> anyhow::Result<PlanWorkflowReport> {
    let services = build_workflow_effect_services(workdir)?;
    let engine = WorkflowEngine::new(services);
    let workflow = workflow_config_for_template(workflow_template);

    let mut passed = 0;
    let mut failed = 0;
    let mut outcomes = Vec::with_capacity(tasks.len());

    for (task_id, prompt) in tasks {
        let config = WorkflowRunConfig {
            prompt: prompt.clone(),
            workdir: workdir.to_path_buf(),
            workflow: workflow.clone(),
            enabled_gates: enabled_gates.clone(),
            shell_gates: shell_gates.clone(),
            commit_prefix: Some("feat".to_string()),
        };

        match engine.run(config).await {
            Ok(result) => {
                let success = matches!(
                    &result.outcome,
                    roko_runtime::workflow_engine::WorkflowOutcome::Success { .. }
                );
                let message = format!("{:?} in {} iterations", result.outcome, result.iterations);
                println!("[{task_id}] {message}");
                tracing::info!(
                    task_id,
                    outcome = ?result.outcome,
                    iterations = result.iterations,
                    "v2 workflow task complete"
                );
                if success {
                    passed += 1;
                } else {
                    failed += 1;
                }
                outcomes.push((task_id.clone(), success, message));
            }
            Err(error) => {
                let message = error.to_string();
                println!("[{task_id}] failed: {message}");
                tracing::warn!(task_id, error = %message, "v2 workflow task failed");
                failed += 1;
                outcomes.push((task_id.clone(), false, message));
            }
        }
    }

    Ok(PlanWorkflowReport {
        total: tasks.len(),
        passed,
        failed,
        outcomes,
    })
}

/// Discover task (id, prompt) pairs from a plans directory.
///
/// Reads `tasks.toml` files under `plans_dir/*/tasks.toml` and extracts each
/// task's `id` and `prompt` fields. Returns them in dependency order if
/// `depends_on` is present, otherwise in declaration order.
pub fn discover_task_prompts(plans_dir: &std::path::Path) -> anyhow::Result<Vec<(String, String)>> {
    #[derive(serde::Deserialize)]
    struct TasksToml {
        #[serde(default, rename = "task")]
        tasks: Vec<TaskEntry>,
    }

    #[derive(Clone, serde::Deserialize)]
    struct TaskEntry {
        id: String,
        #[serde(default)]
        prompt: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        depends_on: Vec<String>,
    }

    fn task_prompt(task: &TaskEntry) -> String {
        if !task.prompt.trim().is_empty() {
            task.prompt.clone()
        } else if let Some(description) = task
            .description
            .as_ref()
            .filter(|description| !description.trim().is_empty())
        {
            description.clone()
        } else {
            task.id.clone()
        }
    }

    fn dependency_ordered_tasks(tasks: Vec<TaskEntry>) -> Vec<TaskEntry> {
        let mut index_by_id = HashMap::with_capacity(tasks.len());
        for (index, task) in tasks.iter().enumerate() {
            index_by_id.entry(task.id.clone()).or_insert(index);
        }

        let mut emitted = vec![false; tasks.len()];
        let mut ordered = Vec::with_capacity(tasks.len());

        loop {
            let mut progressed = false;
            for (index, task) in tasks.iter().enumerate() {
                if emitted[index] {
                    continue;
                }

                let deps_ready =
                    task.depends_on
                        .iter()
                        .all(|dependency| match index_by_id.get(dependency) {
                            Some(dependency_index) => emitted[*dependency_index],
                            None => true,
                        });
                if deps_ready {
                    emitted[index] = true;
                    ordered.push(task.clone());
                    progressed = true;
                }
            }

            if ordered.len() == tasks.len() {
                break;
            }
            if !progressed {
                for (index, task) in tasks.iter().enumerate() {
                    if !emitted[index] {
                        ordered.push(task.clone());
                    }
                }
                break;
            }
        }

        ordered
    }

    let mut task_files = Vec::new();
    if plans_dir.join("tasks.toml").is_file() {
        task_files.push(plans_dir.join("tasks.toml"));
    } else {
        for entry in
            std::fs::read_dir(plans_dir).with_context(|| format!("read {}", plans_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let tasks_path = path.join("tasks.toml");
            if path.is_dir() && tasks_path.is_file() {
                task_files.push(tasks_path);
            }
        }
        task_files.sort();
    }

    let mut prompts = Vec::new();
    for tasks_path in task_files {
        let content = std::fs::read_to_string(&tasks_path)
            .with_context(|| format!("read {}", tasks_path.display()))?;
        let tasks = toml::from_str::<TasksToml>(&content)
            .with_context(|| format!("parse {}", tasks_path.display()))?;

        prompts.extend(
            dependency_ordered_tasks(tasks.tasks)
                .into_iter()
                .map(|task| (task.id.clone(), task_prompt(&task))),
        );
    }

    Ok(prompts)
}

/// Run the universal loop once for `prompt_text` under `workdir`.
///
/// - Opens (or creates) `workdir/.roko/engrams.jsonl`.
/// - Seeds a role + task `PromptSection`, composes them under the config's budget.
/// - Invokes the configured agent backend.
/// - Runs every gate in the config in declaration order; each gate sees the
///   same `GatePayload` pointing at `workdir`.
/// - Records an Episode signal and persists everything.
///
/// If `external_hub` is provided, events are published to it (e.g. for HTTP
/// observability via `roko run --serve`). Otherwise a local hub is created.
#[allow(clippy::too_many_lines)]
pub async fn run_once(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
    external_hub: Option<&StateHub>,
) -> Result<RunReport> {
    // Future `--engine v2` dispatch should call `run_with_workflow_engine`
    // before entering this existing orchestration path.
    let substrate_dir = workdir.join(".roko");
    let substrate = FileSubstrate::open(substrate_dir)
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;

    let ctx = Context::now();

    // Use the external StateHub if provided, otherwise create a local one.
    let events_path = workdir.join(".roko").join("events.jsonl");
    let local_hub;
    let event_hub: &StateHub = if let Some(hub) = external_hub {
        hub
    } else {
        local_hub = StateHub::with_event_log(64, &events_path);
        &local_hub
    };

    // Seed prompt sections: system role + user prompt + any injected files.
    let mut sections: Vec<Engram> = Vec::with_capacity(2 + config.prompt.files.len());

    let role_sig = PromptSection::new("role", &config.prompt.role)
        .with_priority(SectionPriority::Critical)
        .with_placement(Placement::Start)
        .into_signal()
        .map_err(|e| anyhow!("build role section: {e}"))?;
    sections.push(role_sig);

    // File-injected sections: one per `[[prompt.files]]` entry in roko.toml.
    for file in &config.prompt.files {
        let section = load_file_section(workdir, file)?;
        sections.push(section);
    }

    let task_sig = PromptSection::new("task", prompt_text)
        .with_priority(SectionPriority::Critical)
        .with_placement(Placement::End)
        .into_signal()
        .map_err(|e| anyhow!("build task section: {e}"))?;
    sections.push(task_sig);

    for sig in &sections {
        substrate
            .put(sig.clone())
            .await
            .map_err(|e| anyhow!("persist prompt section: {e}"))?;
    }

    // Compose the prompt under the configured budget.
    let composer = PromptComposer::new();
    let prompt = composer
        .compose(
            &sections,
            &Budget::tokens(config.prompt.token_budget),
            &NoOpScorer,
            &ctx,
        )
        .map_err(|e| anyhow!("compose prompt: {e}"))?;
    substrate
        .put(prompt.clone())
        .await
        .map_err(|e| anyhow!("persist prompt: {e}"))?;

    // Run the configured agent path for this provider/backend mix.
    let (agent_result, external_actions) =
        dispatch_agent(workdir, config, &prompt, prompt_text, &ctx).await?;

    // Optionally post-process the agent output to strip ANSI escapes and
    // reasoning-model thinking traces. The raw body is preserved as an
    // AgentMessage trace so nothing is lost.
    let final_output_sig = if config.agent.clean_output {
        maybe_clean_output(&prompt, &agent_result, &substrate).await?
    } else {
        agent_result.output.clone()
    };
    if config.agent.clean_output {
        // clean path already wrote both signals; skip the normal write
    } else {
        substrate
            .put(agent_result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;
    }
    for trace in &agent_result.trace {
        substrate
            .put(trace.clone())
            .await
            .map_err(|e| anyhow!("persist agent trace: {e}"))?;
    }

    // Emit DashboardEvents for the TUI: plan/task start + agent output.
    let run_plan_id = format!("run-{}", chrono::Utc::now().format("%H%M%S"));
    event_hub.publish(DashboardEvent::PlanStarted {
        plan_id: run_plan_id.clone(),
    });
    event_hub.publish(DashboardEvent::TaskStarted {
        plan_id: run_plan_id.clone(),
        task_id: prompt_text.chars().take(60).collect::<String>(),
        title: String::new(),
        phase: "implementing".into(),
    });
    event_hub.publish(DashboardEvent::AgentSpawned {
        agent_id: config.agent.command.clone(),
        role: config.prompt.role.clone(),
        model: dashboard_agent_model(config),
    });
    if let Ok(text) = final_output_sig.body.as_text() {
        let preview: String = text.chars().take(200).collect();
        event_hub.publish(DashboardEvent::AgentOutput {
            agent_id: config.agent.command.clone(),
            content: preview,
        });
        event_hub.publish(DashboardEvent::TaskOutputAppended {
            task_id: prompt_text.chars().take(60).collect(),
            lines: text.lines().take(20).map(String::from).collect(),
        });
    }

    // Run every configured gate against the working dir.
    let gate_input = build_gate_input(workdir, final_output_sig.id)?;
    substrate
        .put(gate_input.clone())
        .await
        .map_err(|e| anyhow!("persist gate input: {e}"))?;

    let mut verdict_sigs: Vec<Engram> = Vec::new();
    let mut verdict_summary: Vec<(String, bool)> = Vec::new();
    for gate_cfg in &config.gates {
        let verdict = run_gate(gate_cfg, &gate_input, &ctx).await;
        let sig = gate_input
            .derive(
                Kind::GateVerdict,
                Body::from_json(&verdict).map_err(|e| anyhow!("encode verdict: {e}"))?,
            )
            .provenance(Provenance::trusted("cli_gate"))
            .tag("passed", verdict.passed.to_string())
            .tag("gate", &verdict.gate)
            .build();
        substrate
            .put(sig.clone())
            .await
            .map_err(|e| anyhow!("persist verdict: {e}"))?;
        verdict_summary.push((verdict.gate.clone(), verdict.passed));
        verdict_sigs.push(sig);
    }

    // Emit gate result events for the TUI.
    for (gate_name, passed) in &verdict_summary {
        event_hub.publish(DashboardEvent::GateResult {
            plan_id: run_plan_id.clone(),
            task_id: prompt_text.chars().take(60).collect(),
            gate: gate_name.clone(),
            passed: *passed,
        });
    }

    // Emit the wrap-up Episode signal.
    let policy = EpisodePolicy::new();
    let episode = policy.record_run(
        &prompt,
        &final_output_sig,
        agent_result.success,
        &verdict_sigs,
        &ctx,
    );
    substrate
        .put(episode.clone())
        .await
        .map_err(|e| anyhow!("persist episode: {e}"))?;

    if let Err(err) = append_episode_log(
        workdir,
        config,
        &prompt,
        &final_output_sig,
        &verdict_summary,
        &agent_result,
        &external_actions,
    )
    .await
    {
        eprintln!("[run] episode logger failed: {err}");
    }

    // Emit completion, episode, and efficiency events for the TUI.
    let all_passed = verdict_summary.iter().all(|(_, p)| *p);
    event_hub.publish(DashboardEvent::TaskCompleted {
        plan_id: run_plan_id.clone(),
        task_id: prompt_text.chars().take(60).collect(),
        outcome: if agent_result.success && all_passed {
            "success".into()
        } else {
            "failed".into()
        },
    });
    event_hub.publish(DashboardEvent::PlanCompleted {
        plan_id: run_plan_id.clone(),
        success: agent_result.success && all_passed,
    });
    event_hub.publish(DashboardEvent::EpisodeRecorded {
        agent_id: config.agent.command.clone(),
        role: config.prompt.role.clone(),
        episode_id: episode.id.to_hex(),
        passed: agent_result.success && all_passed,
    });
    event_hub.publish(DashboardEvent::EfficiencyEvent {
        plan_id: run_plan_id.clone(),
        task_id: prompt_text.chars().take(60).collect(),
        metric: "input_tokens".into(),
        value: f64::from(agent_result.usage.input_tokens),
    });
    event_hub.publish(DashboardEvent::EfficiencyEvent {
        plan_id: run_plan_id.clone(),
        task_id: prompt_text.chars().take(60).collect(),
        metric: "output_tokens".into(),
        value: f64::from(agent_result.usage.output_tokens),
    });
    event_hub.publish(DashboardEvent::EfficiencyEvent {
        plan_id: run_plan_id,
        task_id: prompt_text.chars().take(60).collect(),
        metric: "cost_usd".into(),
        value: f64::from(agent_result.usage.cost_usd),
    });

    let total_signals = substrate
        .len()
        .await
        .map_err(|e| anyhow!("count signals: {e}"))?;

    Ok(RunReport {
        episode_id: episode.id.to_hex(),
        prompt_id: prompt.id.to_hex(),
        agent_output_id: final_output_sig.id.to_hex(),
        agent_success: agent_result.success,
        gate_verdicts: verdict_summary,
        total_signals,
        output_text: final_output_sig.body.as_text().ok().map(ToOwned::to_owned),
    })
}

fn build_system_prompt(config: &Config, prompt_text: &str, tools_csv: &str) -> String {
    let role = &config.prompt.role;
    let workspace = "Single-shot execution through `roko run`.";
    let context_window_tokens = config.prompt.token_budget.max(4_096);
    parse_agent_role(role).map_or_else(
        || {
            build_role_system_prompt_validated(
                AgentRole::Implementer,
                TaskContext::new(prompt_text)
                    .with_workspace(workspace)
                    .with_domain_notes(format!(
                        "User-configured role text: {role}\n\n{}",
                        gate_policy_context(config)
                    )),
                tools_csv,
                PromptBuildOptions {
                    extra_conventions: Some(format!(
                        "Treat the configured role hint literally: {role}\n\n{}",
                        gate_policy_context(config)
                    )),
                    ..PromptBuildOptions::default()
                },
                context_window_tokens,
                None,
            )
            .unwrap_or_else(|_| fallback_system_prompt(role, prompt_text, tools_csv))
        },
        |agent_role| {
            build_role_system_prompt_validated(
                agent_role,
                TaskContext::new(prompt_text)
                    .with_workspace(workspace)
                    .with_domain_notes(gate_policy_context(config)),
                tools_csv,
                PromptBuildOptions {
                    extra_conventions: Some(gate_policy_context(config)),
                    ..PromptBuildOptions::default()
                },
                context_window_tokens,
                None,
            )
            .unwrap_or_else(|_| fallback_system_prompt(role, prompt_text, tools_csv))
        },
    )
}

fn gate_policy_context(config: &Config) -> String {
    if config.gates.is_empty() {
        return "Verification gates: none configured. Still perform the smallest relevant local check before finishing.".to_string();
    }

    let mut context =
        String::from("Verification gates configured for this run. Optimize for passing them:");
    for gate in &config.gates {
        match gate {
            GateConfig::Shell { program, args, .. } => {
                context.push_str("\n- shell: `");
                context.push_str(program);
                for arg in args {
                    context.push(' ');
                    context.push_str(arg);
                }
                context.push('`');
            }
            GateConfig::Compile { build_system, .. } => {
                context.push_str("\n- compile gate for build system: ");
                context.push_str(build_system);
            }
            GateConfig::Clippy { build_system, .. } => {
                context.push_str("\n- lint gate for build system: ");
                context.push_str(build_system);
            }
            GateConfig::Test { build_system, .. } => {
                context.push_str("\n- test gate for build system: ");
                context.push_str(build_system);
            }
        }
    }
    context
}

fn fallback_system_prompt(role: &str, prompt_text: &str, tools_csv: &str) -> String {
    format!(
        "You are a {role} agent.\n\n## Current Task\n\n{prompt_text}\n\n## Tool Instructions\n\nAvailable tools: {tools_csv}\n\n## Project Conventions\n\nMake minimal, targeted changes and run relevant verification before finishing."
    )
}

fn claude_tool_allowlist(role: &str) -> String {
    let registry = StaticToolRegistry::new();
    let tools: Vec<_> = parse_agent_role(role).map_or_else(
        || registry.all().to_vec(),
        |agent_role| registry.for_role(agent_role).into_iter().cloned().collect(),
    );
    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}

fn parse_agent_role(role: &str) -> Option<AgentRole> {
    let normalized = role.trim().to_ascii_lowercase();
    let normalized = normalized
        .strip_prefix("agentrole::")
        .unwrap_or(&normalized)
        .replace(['_', ' '], "-");
    Some(match normalized.as_str() {
        "conductor" => AgentRole::Conductor,
        "strategist" => AgentRole::Strategist,
        "implementer" | "engineer" | "coder" => AgentRole::Implementer,
        "architect" => AgentRole::Architect,
        "researcher" => AgentRole::Researcher,
        "auditor" => AgentRole::Auditor,
        "quick-reviewer" | "quickreviewer" => AgentRole::QuickReviewer,
        "scribe" => AgentRole::Scribe,
        "critic" => AgentRole::Critic,
        "auto-fixer" | "autofixer" => AgentRole::AutoFixer,
        "refactorer" => AgentRole::Refactorer,
        "pre-planner" | "preplanner" => AgentRole::PrePlanner,
        "doc-verifier" | "docverifier" => AgentRole::DocVerifier,
        "integration-tester" | "integrationtester" => AgentRole::IntegrationTester,
        "merge-resolver" | "mergeresolver" => AgentRole::MergeResolver,
        "terminal-validator" | "terminalvalidator" => AgentRole::TerminalValidator,
        "golem-lifecycle-tester" | "golemlifecycletester" => AgentRole::GolemLifecycleTester,
        "spec-drift-detector" | "specdriftdetector" => AgentRole::SpecDriftDetector,
        "regression-detector" | "regressiondetector" => AgentRole::RegressionDetector,
        "performance-sentinel" | "performancesentinel" => AgentRole::PerformanceSentinel,
        "coverage-tracker" | "coveragetracker" => AgentRole::CoverageTracker,
        "plan-lifecycle-mgr" | "plan-lifecycle-manager" | "planlifecyclemanager" => {
            AgentRole::PlanLifecycleManager
        }
        "cross-system-tester" | "crosssystemtester" => AgentRole::CrossSystemTester,
        "error-diagnoser" | "errordiagnoser" => AgentRole::ErrorDiagnoser,
        "dep-validator" | "dependency-validator" | "dependencyvalidator" => {
            AgentRole::DependencyValidator
        }
        "pattern-extractor" | "patternextractor" => AgentRole::PatternExtractor,
        "snapshot-comparator" | "snapshotcomparator" => AgentRole::SnapshotComparator,
        "full-loop-validator" | "fullloopvalidator" => AgentRole::FullLoopValidator,
        _ => return None,
    })
}

async fn dispatch_agent(
    workdir: &Path,
    config: &Config,
    prompt: &Engram,
    prompt_text: &str,
    ctx: &Context,
) -> Result<(AgentResult, Vec<ExternalAction>)> {
    // TODO(gateway): migrate to ModelCallService.
    let mut routing_config = roko_core::config::load_config(workdir)
        .with_context(|| format!("load routing config from {}", workdir.display()))?;
    routing_config.apply_process_env();
    crate::config::merge_global_providers(&mut routing_config);
    let has_routing = !routing_config.providers.is_empty() || !routing_config.models.is_empty();

    if has_routing {
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let system_prompt = build_system_prompt(config, prompt_text, &tools_csv);
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| routing_config.agent.default_model.clone());
        let resolved = resolve_model(&routing_config, &model);
        let agent = spawn_agent_scoped(
            &routing_config,
            SpawnAgentSpec {
                model: model.clone(),
                command: Some(config.agent.command.clone()),
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: Some(system_prompt),
                cached_content: None,
                tools: Some(tools_csv),
                mcp_config: config.agent.mcp_config.clone(),
                working_dir: Some(workdir.to_path_buf()),
                env: config.agent.env.clone(),
                extra_args: config.agent.args.clone(),
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: role_allows_dangerous_skip_permissions(
                    &config.prompt.role,
                ),
                name: format!("{}:{model}", resolved.provider_kind.label()),
                role: Some(normalized_role_label(&config.prompt.role)),
            },
            format!("create agent for model {model}"),
        )?;
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    } else if config.agent.command == "claude" && has_anthropic_api_key(config) {
        // Anthropic API tool loop — direct HTTP calls with full tool visibility.
        return run_anthropic_api_tool_loop(workdir, config, prompt_text).await;
    } else if config.agent.command == "claude" {
        // Claude CLI keeps its own prompt/tool/settings wiring internally.
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let system_prompt = build_system_prompt(config, prompt_text, &tools_csv);
        let (extra_args, resume_from_args) = split_resume_arg(&config.agent.args);
        let optional_resume = optional_resume_session_id(config, resume_from_args);
        let model = config.agent.model.clone().unwrap_or_else(|| {
            // Prefer the routing config's default_model (from roko.toml or global config)
            // over hardcoded Claude. This ensures ZAI_API_KEY / glm-5.1 setups work.
            if !routing_config.agent.default_model.is_empty() {
                routing_config.agent.default_model.clone()
            } else {
                "claude-sonnet-4-6".to_string()
            }
        });
        let synthesized_config = synthesize_claude_cli_config(&config.agent.command, &model);

        let mut synthetic_extra_args = extra_args;
        if let Some(resume_session) = optional_resume {
            synthetic_extra_args.push("--resume".to_string());
            synthetic_extra_args.push(resume_session);
        }
        if let Some(fallback_model) = &config.agent.fallback_model {
            synthetic_extra_args.push("--fallback-model".to_string());
            synthetic_extra_args.push(fallback_model.clone());
        }

        let agent = spawn_agent_scoped(
            &synthesized_config,
            SpawnAgentSpec {
                model: model.clone(),
                command: Some(config.agent.command.clone()),
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: Some(system_prompt),
                cached_content: None,
                tools: Some(tools_csv),
                mcp_config: config.agent.mcp_config.clone(),
                working_dir: Some(workdir.to_path_buf()),
                env: config.agent.env.clone(),
                extra_args: synthetic_extra_args,
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: role_allows_dangerous_skip_permissions(
                    &config.prompt.role,
                ),
                name: String::new(),
                role: Some(normalized_role_label(&config.prompt.role)),
            },
            format!("create synthesized claude agent for model {model}"),
        )?;
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    } else if config.agent.command == "ollama" {
        Ok(run_ollama_agentic_single(workdir, config, prompt_text).await)
    } else if is_known_protocol_command(&config.agent.command) {
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| config.agent.command.clone());
        let fallback_config = synthesize_known_protocol_config(&config.agent.command, &model);

        let agent = spawn_agent_scoped(
            &fallback_config,
            SpawnAgentSpec {
                model: model.clone(),
                command: Some(config.agent.command.clone()),
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: None,
                cached_content: None,
                tools: None,
                mcp_config: None,
                working_dir: Some(workdir.to_path_buf()),
                env: config.agent.env.clone(),
                extra_args: config.agent.args.clone(),
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: false,
                name: String::new(),
                role: Some(normalized_role_label(&config.prompt.role)),
            },
            format!(
                "create known-protocol subprocess agent for {}",
                config.agent.command
            ),
        )?;
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    } else {
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| config.agent.command.clone());
        let fallback_config = synthesize_subprocess_config(&config.agent.command);
        let agent = spawn_agent_scoped(
            &fallback_config,
            SpawnAgentSpec {
                model: model.clone(),
                command: Some(config.agent.command.clone()),
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: None,
                cached_content: None,
                tools: None,
                mcp_config: None,
                working_dir: Some(workdir.to_path_buf()),
                env: config.agent.env.clone(),
                extra_args: config.agent.args.clone(),
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: false,
                name: String::new(),
                role: Some(normalized_role_label(&config.prompt.role)),
            },
            format!(
                "create generic subprocess agent for {}",
                config.agent.command
            ),
        )?;
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    }
}

/// Ollama agentic path for `roko run`.
async fn run_ollama_agentic_single(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
) -> (AgentResult, Vec<ExternalAction>) {
    use parking_lot::RwLock;
    use roko_agent::dispatcher::ToolDispatcher;
    use roko_agent::tool_loop::{StopReason, ToolLoop};
    use roko_core::tool::{ToolContext, ToolHandler};
    use std::sync::Arc;
    use std::time::Instant;

    let started = Instant::now();
    let model = config
        .agent
        .model
        .clone()
        .unwrap_or_else(|| "llama3.1:8b".to_string());

    let base_url = config
        .agent
        .env
        .iter()
        .find(|(k, _)| k == "OLLAMA_HOST")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "http://localhost:11434".to_string());

    let registry = Arc::new(StaticToolRegistry::new());
    let tools: Vec<roko_core::tool::ToolDef> = registry.all().into_iter().cloned().collect();
    let resolver: Arc<dyn roko_agent::dispatcher::HandlerResolver> =
        Arc::new(|name: &str| -> Option<Arc<dyn ToolHandler>> {
            roko_std::tool::handlers::handler_for(name)
        });
    let dispatcher = Arc::new(ToolDispatcher::new(
        registry as Arc<dyn ToolRegistry>,
        resolver,
    ));
    let translator: Arc<dyn Translator> = Arc::new(OllamaTranslator);
    let backend: Arc<dyn roko_agent::tool_loop::LlmBackend> = Arc::new(
        OllamaLlmBackend::new(&model)
            .with_base_url(base_url)
            .with_timeout_ms(config.agent.timeout_ms),
    );
    let tool_loop = ToolLoop::new(translator, dispatcher, backend);

    let system_prompt = build_system_prompt(config, prompt_text, "");
    let external_actions = Arc::new(RwLock::new(Vec::new()));
    let tool_ctx =
        ToolContext::testing(workdir).with_external_actions(Arc::clone(&external_actions));

    let output = tool_loop
        .run(&system_prompt, prompt_text, &tools, &tool_ctx)
        .await;
    let external_actions = external_actions.read().clone();

    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let agent_name = format!("ollama:{model}");
    let success = matches!(output.stop_reason, StopReason::Stop);
    let body_text = if success {
        output.final_text.clone()
    } else {
        format!(
            "agent stopped: {:?} after {} iterations",
            output.stop_reason, output.iterations
        )
    };

    let sig = Engram::builder(Kind::AgentOutput)
        .body(Body::text(body_text))
        .provenance(Provenance::agent(&agent_name))
        .tag("agent", &agent_name)
        .tag("model", &model)
        .tag("tool_calls", output.tool_calls.len().to_string())
        .tag("iterations", output.iterations.to_string())
        .build();

    let usage = roko_agent::usage::Usage {
        wall_ms,
        ..Default::default()
    };

    if success {
        (AgentResult::ok(sig).with_usage(usage), external_actions)
    } else {
        (AgentResult::fail(sig).with_usage(usage), external_actions)
    }
}

/// Check whether an Anthropic API key is available for the direct-API path.
fn has_anthropic_api_key(config: &Config) -> bool {
    // Check env var first, then config secret store.
    if std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .is_some()
    {
        return true;
    }
    // Check if any env entry in the agent config provides it.
    config
        .agent
        .env
        .iter()
        .any(|(k, v)| k == "ANTHROPIC_API_KEY" && !v.is_empty())
}

/// Resolve the Anthropic API key from env or config.
fn resolve_anthropic_api_key(config: &Config) -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            config
                .agent
                .env
                .iter()
                .find(|(k, _)| k == "ANTHROPIC_API_KEY")
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        })
}

/// Anthropic API tool loop path for `roko run`.
///
/// Uses the Anthropic Messages API directly with the ToolLoop, giving full
/// tool-call visibility, chain tool support, and real-time turn output.
async fn run_anthropic_api_tool_loop(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
) -> Result<(AgentResult, Vec<ExternalAction>)> {
    use parking_lot::RwLock;
    use roko_agent::dispatcher::ToolDispatcher;
    use roko_agent::provider::anthropic_api::tool_loop::create_anthropic_backend_simple;
    use roko_agent::tool_loop::{OnTurnCallback, StopReason, ToolLoop, TurnProgress};
    use roko_core::tool::{ToolContext, ToolHandler};
    use std::sync::Arc;
    use std::time::Instant;

    let started = Instant::now();
    let api_key =
        resolve_anthropic_api_key(config).ok_or_else(|| anyhow!("ANTHROPIC_API_KEY not found"))?;

    let model = config
        .agent
        .model
        .clone()
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());

    // Build backend + translator.
    let (backend, translator) =
        create_anthropic_backend_simple(api_key, &model, config.agent.timeout_ms);

    // Build tool registry + dispatcher with optional chain tools.
    let registry = Arc::new(StaticToolRegistry::new());
    let tools: Vec<roko_core::tool::ToolDef> = registry.all().into_iter().cloned().collect();

    let resolver: Arc<dyn roko_agent::dispatcher::HandlerResolver> =
        match build_chain_resolver(workdir) {
            Some(chain_resolver) => chain_resolver,
            None => Arc::new(|name: &str| -> Option<Arc<dyn ToolHandler>> {
                roko_std::tool::handlers::handler_for(name)
            }),
        };
    let dispatcher = Arc::new(ToolDispatcher::new(
        registry as Arc<dyn ToolRegistry>,
        resolver,
    ));

    // Build tool loop with progress callback.
    let on_turn: OnTurnCallback = Arc::new(|progress: &TurnProgress| {
        if progress.tool_calls.is_empty() {
            return;
        }
        for (i, call) in progress.tool_calls.iter().enumerate() {
            let result_summary = progress
                .tool_results
                .get(i)
                .map(|s| s.as_str())
                .unwrap_or("");
            // Truncate result for display.
            let display_result = if result_summary.len() > 80 {
                format!("{}…", &result_summary[..79])
            } else {
                result_summary.to_string()
            };
            eprintln!(
                "\x1b[2m[roko] tool: {}({})\x1b[0m",
                call.name,
                truncate_json_args(&call.arguments, 60),
            );
            if !display_result.is_empty() {
                eprintln!("\x1b[2m[roko] \u{2192} {display_result}\x1b[0m");
            }
        }
    });

    let tool_loop = ToolLoop::new(translator, dispatcher, backend).with_on_turn(on_turn);

    let system_prompt = build_system_prompt(config, prompt_text, "");
    let external_actions = Arc::new(RwLock::new(Vec::new()));
    let tool_ctx =
        ToolContext::testing(workdir).with_external_actions(Arc::clone(&external_actions));

    eprintln!("\x1b[2m[roko] using Anthropic API ({model})\x1b[0m");
    let output = tool_loop
        .run(&system_prompt, prompt_text, &tools, &tool_ctx)
        .await;
    let external_actions = external_actions.read().clone();

    let wall_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let agent_name = format!("anthropic-api:{model}");
    let success = matches!(output.stop_reason, StopReason::Stop);
    let body_text = if success {
        output.final_text.clone()
    } else {
        format!(
            "agent stopped: {:?} after {} iterations",
            output.stop_reason, output.iterations
        )
    };

    let sig = Engram::builder(Kind::AgentOutput)
        .body(Body::text(body_text))
        .provenance(Provenance::agent(&agent_name))
        .tag("agent", &agent_name)
        .tag("model", &model)
        .tag("tool_calls", output.tool_calls.len().to_string())
        .tag("iterations", output.iterations.to_string())
        .tag("backend", "anthropic-api")
        .build();

    let usage = roko_agent::usage::Usage {
        wall_ms,
        input_tokens: output.total_usage.input_tokens,
        output_tokens: output.total_usage.output_tokens,
        cache_read_tokens: output.total_usage.cache_read_tokens,
        cache_create_tokens: output.total_usage.cache_create_tokens,
        cost_usd: output.total_usage.cost_usd,
        ..Default::default()
    };

    if success {
        Ok((AgentResult::ok(sig).with_usage(usage), external_actions))
    } else {
        Ok((AgentResult::fail(sig).with_usage(usage), external_actions))
    }
}

/// Build a chain-aware handler resolver if chain config is present in the workspace.
fn build_chain_resolver(
    workdir: &Path,
) -> Option<std::sync::Arc<dyn roko_agent::dispatcher::HandlerResolver>> {
    use roko_chain::alloy_impl::AlloyChainClient;
    use std::sync::Arc;

    let roko_config = roko_core::config::load_config(workdir).ok()?;
    let rpc_url = roko_config.chain.rpc_url.as_deref()?;

    let client: Arc<dyn roko_chain::ChainClient> = match AlloyChainClient::http(rpc_url) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!(
                "\x1b[33m\u{26a0} chain client init failed: {e}, chain tools unavailable\x1b[0m"
            );
            return None;
        }
    };

    let wallet: Option<Arc<dyn roko_chain::ChainWallet>> = (|| {
        let key = roko_config.chain.wallet_key.as_deref()?;
        let chain_id = roko_config.chain.chain_id.unwrap_or(1);
        match roko_chain::alloy_impl::AlloyChainWallet::from_hex_key(rpc_url, key, chain_id) {
            Ok(w) => Some(Arc::new(w) as Arc<dyn roko_chain::ChainWallet>),
            Err(e) => {
                eprintln!(
                    "\x1b[33m\u{26a0} chain wallet init failed: {e}, write ops unavailable\x1b[0m"
                );
                None
            }
        }
    })();

    let chain_map = crate::chain_registry::chain_handler_map(client, wallet);
    Some(Arc::new(crate::chain_registry::chain_aware_resolver(
        chain_map,
    )))
}

/// Truncate JSON arguments for display.
fn truncate_json_args(args: &serde_json::Value, max_len: usize) -> String {
    let s = args.to_string();
    if s.len() > max_len {
        format!("{}…", &s[..max_len.saturating_sub(1)])
    } else {
        s
    }
}

async fn append_episode_log(
    workdir: &Path,
    config: &Config,
    prompt: &Engram,
    final_output: &Engram,
    verdicts: &[(String, bool)],
    agent_result: &AgentResult,
    external_actions: &[ExternalAction],
) -> Result<()> {
    let agent_id = agent_result
        .output
        .tag("agent")
        .map_or_else(|| config.agent.command.clone(), str::to_string);
    let mut episode = Episode::new(agent_id, prompt.id.to_hex());
    episode.input_signal_hash = prompt.id.to_hex();
    episode.output_signal_hash = final_output.id.to_hex();
    episode.gate_verdicts = verdicts
        .iter()
        .map(|(gate, passed)| GateVerdict::new(gate.clone(), *passed))
        .collect();
    episode.success = agent_result.success && verdicts.iter().all(|(_, passed)| *passed);
    if !episode.success {
        episode.failure_reason = Some(if agent_result.success {
            "one or more gates failed".to_string()
        } else {
            "agent failed".to_string()
        });
    }
    episode.usage = EpisodeUsage {
        input_tokens: u64::from(agent_result.usage.input_tokens),
        output_tokens: u64::from(agent_result.usage.output_tokens),
        cache_read_tokens: u64::from(agent_result.usage.cache_read_tokens),
        cache_write_tokens: u64::from(agent_result.usage.cache_create_tokens),
        cost_usd: f64::from(agent_result.usage.cost_usd),
        cost_usd_without_cache: f64::from(agent_result.usage.cost_usd),
        wall_ms: agent_result.usage.wall_ms,
    };
    episode.external_actions = external_actions
        .iter()
        .map(|action| serde_json::to_value(action))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow!("encode external actions: {e}"))?;
    episode.extra.insert(
        "role".to_string(),
        serde_json::json!(normalized_role_label(&config.prompt.role)),
    );
    episode.extra.insert(
        "model".to_string(),
        serde_json::json!(resolved_model(config)),
    );
    episode.extra.insert(
        "provider".to_string(),
        serde_json::json!(infer_provider(config)),
    );
    episode.extra.insert(
        "backend".to_string(),
        serde_json::json!(config.agent.command.clone()),
    );
    episode
        .extra
        .insert("plan_id".to_string(), serde_json::json!("cli-run"));
    episode
        .extra
        .insert("task_id".to_string(), serde_json::json!(prompt.id.to_hex()));
    episode
        .extra
        .insert("iteration".to_string(), serde_json::json!(1_u64));
    episode
        .extra
        .insert("complexity_band".to_string(), serde_json::json!("standard"));
    episode
        .extra
        .insert("task_category".to_string(), serde_json::json!("cli-run"));
    episode
        .extra
        .insert("task_tags".to_string(), serde_json::json!(["cli", "run"]));
    let injected_files = config
        .prompt
        .files
        .iter()
        .map(|spec| spec.path.display().to_string())
        .collect::<Vec<_>>();
    episode.extra.insert(
        "files".to_string(),
        serde_json::json!(injected_files.clone()),
    );
    episode.extra.insert(
        "files_changed".to_string(),
        serde_json::json!(injected_files),
    );
    if let Some(session_id) = optional_resume_session_id(config, None) {
        episode
            .extra
            .insert("session_id".to_string(), serde_json::json!(session_id));
    }

    let learn_root = workdir.join(".roko").join("learn");
    let mut model_keys: Vec<String> = load_roko_config_models(workdir);
    // Ensure the model actually being used is in the cascade router's slug list,
    // even if it comes from the global config rather than the project config.
    let current_model = resolved_model(config);
    if !model_keys.iter().any(|k| k == &current_model) {
        model_keys.push(current_model);
    }
    let mut runtime = if model_keys.is_empty() {
        LearningRuntime::open_under(learn_root)
            .await
            .map_err(|e| anyhow!("open learning runtime: {e}"))?
    } else {
        LearningRuntime::open_under_with_models(learn_root, model_keys)
            .await
            .map_err(|e| anyhow!("open learning runtime: {e}"))?
    };
    let distillation_workdir = workdir.to_path_buf();
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode, None);
    });
    let mut completed = CompletedRunInput::from_episode(episode);
    completed.provider = Some(infer_provider(config));
    completed.task_metric = Some(build_task_metric(config, prompt, verdicts, agent_result));
    runtime
        .record_completed_run(completed)
        .await
        .map_err(|e| anyhow!("record learning feedback: {e}"))?;
    Ok(())
}

fn build_task_metric(
    config: &Config,
    prompt: &Engram,
    verdicts: &[(String, bool)],
    agent_result: &AgentResult,
) -> TaskMetric {
    let config_hash =
        ConfigHash::of(config).unwrap_or_else(|_| ConfigHash::from("unknown-config".to_string()));
    let mut metric = TaskMetric::new(config_hash, "cli-run", prompt.id.to_hex());
    metric.timestamp = Utc::now().to_rfc3339();
    metric.run_id = final_output_run_id(prompt, agent_result);
    metric.iteration = 1;
    metric.role = normalized_role_label(&config.prompt.role);
    metric.backend.clone_from(&config.agent.command);
    metric.model = resolved_model(config);
    metric.complexity_band = "standard".to_string();
    metric.gate = "overall".to_string();
    metric.gate_passed = agent_result.success && verdicts.iter().all(|(_, passed)| *passed);
    metric.wall_time_ms = agent_result.usage.wall_ms;
    metric.input_tokens = u64::from(agent_result.usage.input_tokens);
    metric.output_tokens = u64::from(agent_result.usage.output_tokens);
    metric.cached_tokens = u64::from(agent_result.usage.cache_read_tokens);
    metric.cost_usd = f64::from(agent_result.usage.cost_usd);
    metric.sections_included = u32::try_from(2 + config.prompt.files.len()).unwrap_or(u32::MAX);
    metric.sections_dropped = 0;
    metric.context_tokens = u64::from(agent_result.usage.total_tokens());
    metric.cache_hit_rate = if agent_result.usage.input_tokens == 0 {
        0.0
    } else {
        f64::from(agent_result.usage.cache_read_tokens) / f64::from(agent_result.usage.input_tokens)
    };
    metric
}

fn final_output_run_id(prompt: &Engram, agent_result: &AgentResult) -> String {
    agent_result
        .output
        .tag("run_id")
        .map_or_else(|| prompt.id.to_hex(), str::to_string)
}

fn resolved_model(config: &Config) -> String {
    if let Some(model) = &config.agent.model {
        return model.clone();
    }
    // Check routing config for configured default model before falling back to hardcoded.
    if let Ok(mut rc) = roko_core::config::load_config(std::path::Path::new(".")) {
        rc.apply_process_env();
        crate::config::merge_global_providers(&mut rc);
        if !rc.agent.default_model.is_empty() {
            return rc.agent.default_model;
        }
    }
    if config.agent.command.eq_ignore_ascii_case("claude") {
        "claude-sonnet-4-6".to_string()
    } else {
        "unknown-model".to_string()
    }
}

fn dashboard_agent_model(config: &Config) -> String {
    let model = resolved_model(config);
    if model != "unknown-model" {
        return model;
    }

    let command = config.agent.command.trim();
    if command.is_empty() {
        "unknown-model".to_string()
    } else {
        command.to_string()
    }
}

fn infer_provider(config: &Config) -> String {
    let command = config.agent.command.trim();
    let model = resolved_model(config).to_ascii_lowercase();
    if command.eq_ignore_ascii_case("claude") || model.starts_with("claude") {
        "anthropic".to_string()
    } else if command.eq_ignore_ascii_case("codex")
        || command.eq_ignore_ascii_case("openai")
        || model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("o4")
    {
        "openai".to_string()
    } else if command.eq_ignore_ascii_case("ollama") || model.starts_with("ollama/") {
        "ollama".to_string()
    } else {
        command.to_string()
    }
}

fn normalized_role_label(role: &str) -> String {
    parse_agent_role(role).map_or_else(
        || role.trim().to_string(),
        |parsed| parsed.label().to_string(),
    )
}

fn role_allows_dangerous_skip_permissions(role: &str) -> bool {
    parse_agent_role(role).is_none_or(|parsed| {
        let perms = parsed.tool_permissions();
        perms.write || perms.exec || perms.git || perms.network
    })
}

fn optional_resume_session_id(config: &Config, resume_from_args: Option<String>) -> Option<String> {
    resume_from_args.or_else(|| {
        config
            .agent
            .env
            .iter()
            .find_map(|(k, v)| is_resume_env_key(k).then_some(v.trim()))
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn is_resume_env_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("ROKO_RESUME")
        || key.eq_ignore_ascii_case("ROKO_SESSION_ID")
        || key.eq_ignore_ascii_case("CLAUDE_RESUME")
        || key.eq_ignore_ascii_case("CLAUDE_SESSION_ID")
}

fn split_resume_arg(args: &[String]) -> (Vec<String>, Option<String>) {
    let mut cleaned = Vec::with_capacity(args.len());
    let mut resume = None;
    let mut idx = 0;
    while let Some(arg) = args.get(idx) {
        if let Some(value) = arg.strip_prefix("--resume=") {
            if resume.is_none() {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    resume = Some(trimmed.to_string());
                }
            }
            idx += 1;
            continue;
        }
        if arg == "--resume" {
            if resume.is_none()
                && let Some(value) = args
                    .get(idx + 1)
                    .map(|v| v.trim())
                    .filter(|v| !v.is_empty() && !v.starts_with('-'))
            {
                resume = Some(value.to_string());
                idx += 2;
                continue;
            }
            idx += 1;
            continue;
        }
        cleaned.push(arg.clone());
        idx += 1;
    }
    (cleaned, resume)
}

fn load_file_section(workdir: &Path, spec: &PromptFile) -> Result<Engram> {
    let full_path = if spec.path.is_absolute() {
        spec.path.clone()
    } else {
        workdir.join(&spec.path)
    };
    let contents = std::fs::read_to_string(&full_path)
        .with_context(|| format!("read prompt file {}", full_path.display()))?;
    let name = spec
        .name
        .clone()
        .unwrap_or_else(|| spec.path.display().to_string());
    let priority = match spec.priority.as_deref() {
        Some("low") => SectionPriority::Low,
        Some("high") => SectionPriority::High,
        Some("critical") => SectionPriority::Critical,
        _ => SectionPriority::Normal,
    };
    let labeled = format!("File `{}`:\n\n{}", spec.path.display(), contents);
    let mut section = PromptSection::new(&name, labeled)
        .with_priority(priority)
        .with_placement(Placement::Middle);
    if let Some(cap) = spec.hard_cap {
        section = section.with_hard_cap(cap);
    }
    section
        .into_signal()
        .map_err(|e| anyhow!("build file section for {}: {e}", spec.path.display()))
}

/// Post-process the agent output to strip ANSI escapes + thinking traces.
/// Persists both the raw output (as an `AgentMessage` trace) and the cleaned
/// version (as the canonical `AgentOutput`). Returns the cleaned signal.
async fn maybe_clean_output(
    prompt: &Engram,
    agent_result: &AgentResult,
    substrate: &FileSubstrate,
) -> Result<Engram> {
    let raw = agent_result.output.body.as_text().unwrap_or("").to_string();
    let cleaned = clean::clean(&raw);
    if cleaned == raw.trim() {
        // No-op cleaning — just persist the original and move on.
        substrate
            .put(agent_result.output.clone())
            .await
            .map_err(|e| anyhow!("persist agent output: {e}"))?;
        return Ok(agent_result.output.clone());
    }

    // Persist the raw version as a trace signal so nothing is lost.
    let raw_trace = agent_result
        .output
        .derive(Kind::AgentMessage, Body::text(&raw))
        .provenance(Provenance::agent("exec:raw"))
        .tag("stream", "raw_stdout")
        .build();
    substrate
        .put(raw_trace)
        .await
        .map_err(|e| anyhow!("persist raw agent output trace: {e}"))?;

    // Build a fresh AgentOutput signal whose body is the cleaned text. The
    // new signal chains to the prompt (not the raw output) so lineage stays
    // linear: prompt → cleaned_output → gate_input → verdict → episode.
    let clean_sig = prompt
        .derive(Kind::AgentOutput, Body::text(&cleaned))
        .provenance(
            agent_result
                .output
                .tag("agent")
                .map_or_else(|| Provenance::agent("exec"), Provenance::agent),
        )
        .tag("cleaned", "true")
        .tag("agent", agent_result.output.tag("agent").unwrap_or("exec"))
        .build();
    substrate
        .put(clean_sig.clone())
        .await
        .map_err(|e| anyhow!("persist cleaned agent output: {e}"))?;
    Ok(clean_sig)
}

fn build_gate_input(workdir: &Path, parent_id: roko_core::ContentHash) -> Result<Engram> {
    let working_dir: PathBuf = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize workdir {}", workdir.display()))?;
    let payload = GatePayload::in_dir(working_dir).with_label("roko-cli");
    let body = Body::from_json(&payload).map_err(|e| anyhow!("encode gate payload: {e}"))?;
    Ok(Engram::builder(Kind::Task)
        .body(body)
        .provenance(Provenance::trusted("cli_run"))
        .lineage([parent_id])
        .build())
}

async fn run_gate(cfg: &GateConfig, input: &Engram, ctx: &Context) -> Verdict {
    match cfg {
        GateConfig::Shell {
            program,
            args,
            timeout_ms,
        } => {
            ShellGate::new(program, args.clone())
                .with_timeout_ms(*timeout_ms)
                .verify(input, ctx)
                .await
        }
        GateConfig::Compile {
            build_system,
            timeout_ms,
        } => match parse_build_system(build_system) {
            Ok(bs) => {
                CompileGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await
            }
            Err(e) => Verdict::fail("compile", e),
        },
        GateConfig::Clippy {
            build_system,
            timeout_ms,
        } => match parse_build_system(build_system) {
            Ok(bs) => {
                ClippyGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await
            }
            Err(e) => Verdict::fail("clippy", e),
        },
        GateConfig::Test {
            build_system,
            timeout_ms,
        } => match parse_build_system(build_system) {
            Ok(bs) => {
                TestGate::new(bs)
                    .with_timeout_ms(*timeout_ms)
                    .verify(input, ctx)
                    .await
            }
            Err(e) => Verdict::fail("test", e),
        },
    }
}

fn parse_build_system(s: &str) -> Result<BuildSystem, String> {
    match s.to_ascii_lowercase().as_str() {
        "cargo" => Ok(BuildSystem::Cargo),
        "npm" => Ok(BuildSystem::Npm),
        "go" => Ok(BuildSystem::Go),
        "python" | "py" => Ok(BuildSystem::Python),
        "forge" => Ok(BuildSystem::Forge),
        "make" => Ok(BuildSystem::Make),
        other => Err(format!("unknown build_system: {other}")),
    }
}

/// Extract model keys from the project's `roko.toml` for cascade router
/// initialization. Returns an empty vec if the config is missing or has
/// no models (which falls back to the hardcoded defaults).
fn load_roko_config_models(workdir: &Path) -> Vec<String> {
    let path = workdir.join("roko.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let config = match roko_core::config::RokoConfig::from_toml(&text) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    config.effective_models().keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_agent_role_accepts_known_labels_and_aliases() {
        assert_eq!(
            parse_agent_role("implementer"),
            Some(AgentRole::Implementer)
        );
        assert_eq!(
            parse_agent_role("quick-reviewer"),
            Some(AgentRole::QuickReviewer)
        );
        assert_eq!(parse_agent_role("engineer"), Some(AgentRole::Implementer));
        assert_eq!(parse_agent_role("unknown-role"), None);
    }

    #[test]
    fn parse_build_system_accepts_known_names() {
        assert!(matches!(
            parse_build_system("cargo"),
            Ok(BuildSystem::Cargo)
        ));
        assert!(matches!(parse_build_system("NPM"), Ok(BuildSystem::Npm)));
        assert!(matches!(parse_build_system("py"), Ok(BuildSystem::Python)));
        assert!(parse_build_system("bazel").is_err());
    }

    #[test]
    fn run_report_overall_success_requires_all_gates() {
        let r = RunReport {
            episode_id: "a".into(),
            prompt_id: "b".into(),
            agent_output_id: "c".into(),
            agent_success: true,
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), true)],
            total_signals: 5,
            output_text: Some("done".into()),
        };
        assert!(r.overall_success());

        let r = RunReport {
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), false)],
            ..r
        };
        assert!(!r.overall_success());
    }

    #[test]
    fn write_shared_run_creates_file() {
        let tmp = std::env::temp_dir().join("roko-test-share");
        let _ = std::fs::remove_dir_all(&tmp);
        let report = RunReport {
            episode_id: "ep-1".into(),
            prompt_id: "hi".into(),
            agent_output_id: "out-1".into(),
            agent_success: true,
            gate_verdicts: vec![],
            total_signals: 3,
            output_text: Some("done".into()),
        };
        let token = write_shared_run(&tmp, &report).unwrap();
        assert!(
            tmp.join(".roko/shared")
                .join(format!("{token}.json"))
                .exists()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn run_system_prompt_contains_composed_layers_and_gates() {
        let mut config = Config::default();
        config.prompt.role = "implementer".into();
        config.prompt.token_budget = 8_000;
        config.gates = vec![GateConfig::Compile {
            build_system: "cargo".into(),
            timeout_ms: 60_000,
        }];

        let prompt = build_system_prompt(
            &config,
            "Implement bounded prompt assembly.",
            "Read,Edit,Bash",
        );

        assert!(prompt.contains("## Project Conventions"));
        assert!(prompt.contains("## Tool Instructions"));
        assert!(prompt.contains("## Domain Context"));
        assert!(prompt.contains("## Current Task"));
        assert!(prompt.contains("Implement bounded prompt assembly."));
        assert!(prompt.contains("Verification gates configured for this run"));
        assert!(prompt.contains("compile gate for build system: cargo"));
        assert!(prompt.contains("Read,Edit,Bash"));
    }

    #[test]
    fn role_permissions_drive_skip_permissions_flag() {
        assert!(role_allows_dangerous_skip_permissions("implementer"));
        assert!(role_allows_dangerous_skip_permissions("researcher"));
        assert!(!role_allows_dangerous_skip_permissions("architect"));
        assert!(!role_allows_dangerous_skip_permissions("auditor"));
        assert!(role_allows_dangerous_skip_permissions("custom-role"));
    }

    #[test]
    fn split_resume_arg_extracts_and_strips_resume_flags() {
        let args = vec![
            "--foo".to_string(),
            "--resume".to_string(),
            "sess-1".to_string(),
            "--bar".to_string(),
            "--resume=sess-2".to_string(),
        ];
        let (cleaned, resume) = split_resume_arg(&args);
        assert_eq!(resume.as_deref(), Some("sess-1"));
        assert_eq!(cleaned, vec!["--foo", "--bar"]);
    }

    #[test]
    fn optional_resume_prefers_args_then_env() {
        let mut cfg = Config::default();
        cfg.agent
            .env
            .push(("ROKO_SESSION_ID".to_string(), "env-sess".to_string()));
        assert_eq!(
            optional_resume_session_id(&cfg, Some("arg-sess".to_string())).as_deref(),
            Some("arg-sess")
        );
        assert_eq!(
            optional_resume_session_id(&cfg, None).as_deref(),
            Some("env-sess")
        );
    }

    #[test]
    fn dashboard_agent_model_is_never_empty_for_run_events() {
        let mut cfg = Config::default();
        cfg.agent.command = "codex".to_string();
        cfg.agent.model = None;

        assert_eq!(dashboard_agent_model(&cfg), "codex");

        cfg.agent.model = Some("gpt-5.4".to_string());
        assert_eq!(dashboard_agent_model(&cfg), "gpt-5.4");
    }

    #[test]
    fn engine_flag_express_selects_express_config() {
        let workflow = match "express" {
            "express" => WorkflowConfig::express(),
            "full" => WorkflowConfig::full(),
            _ => WorkflowConfig::standard(),
        };

        assert!(!workflow.has_strategy);
        assert!(!workflow.has_review);
        assert_eq!(workflow.max_iterations, 1);
    }

    #[test]
    fn engine_flag_full_selects_full_config() {
        let workflow = match "full" {
            "express" => WorkflowConfig::express(),
            "full" => WorkflowConfig::full(),
            _ => WorkflowConfig::standard(),
        };

        assert!(workflow.has_strategy);
        assert!(workflow.has_review);
        assert_eq!(workflow.max_iterations, 3);
    }

    #[test]
    fn engine_flag_legacy_and_unknown_select_standard_config() {
        for workflow_template in ["legacy", "v2", "standard", "unknown"] {
            let workflow = match workflow_template {
                "express" => WorkflowConfig::express(),
                "full" => WorkflowConfig::full(),
                _ => WorkflowConfig::standard(),
            };

            assert!(
                !workflow.has_strategy,
                "{workflow_template} should not enable strategy"
            );
            assert!(
                workflow.has_review,
                "{workflow_template} should enable review"
            );
            assert_eq!(workflow.max_iterations, 2);
        }
    }

    #[tokio::test]
    async fn dispatch_agent_uses_exec_agent_for_plain_commands_without_routing() {
        if std::env::var("ANTHROPIC_API_KEY").is_err() {
            eprintln!("skipping: ANTHROPIC_API_KEY not set");
            return;
        }
        let tempdir = TempDir::new().expect("tempdir");
        let config = Config::default();
        let prompt = Engram::builder(Kind::Prompt)
            .body(Body::text("plain-exec-ok"))
            .build();

        let (result, external_actions) = dispatch_agent(
            tempdir.path(),
            &config,
            &prompt,
            "plain-exec-ok",
            &Context::now(),
        )
        .await
        .expect("dispatch succeeds");

        assert!(result.success);
        assert_eq!(result.output.body.as_text().unwrap_or(""), "plain-exec-ok");
        assert!(external_actions.is_empty());
    }
}
