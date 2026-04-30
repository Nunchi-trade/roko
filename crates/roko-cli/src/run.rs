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
use crate::knowledge_helpers::{build_strategy_fragment_context, query_anti_knowledge_patterns};
use crate::learning_helpers::{
    load_or_create_playbook_store, load_or_create_skill_library, playbook_query_context,
    render_prior_experience,
};
use crate::model_selection::{EffectiveModelSelection, resolve_effective_model};
use crate::output_format;
#[cfg(feature = "legacy-orchestrate")]
use crate::prompting::{PromptBuildOptions, build_role_system_prompt_validated};
use crate::state_hub::{StateHub, StateHubSender};
use crate::task_helpers::extract_task_symbols;
use anyhow::{Context as _, Result, anyhow};
use chrono::Utc;
use roko_agent::provider::is_known_protocol_command;
use roko_agent::translate::{ClaudeTranslator, OllamaTranslator, RenderedTools, Translator};
use roko_agent::{AgentResult, OllamaLlmBackend};
#[cfg(feature = "legacy-orchestrate")]
use roko_compose::{Placement, PromptComposer, PromptSection, SectionPriority, TaskContext};
use roko_core::agent::resolve_model;
use roko_core::config::schema::RokoConfig;
use roko_core::dashboard_snapshot::DashboardEvent;
use roko_core::foundation::{
    EventConsumer as WorkflowEventConsumer, ShellGateCommand as CoreShellGateCommand,
};
use roko_core::metric::{ConfigHash, TaskMetric};
use roko_core::tool::ExternalAction;
use roko_core::tool::ToolRegistry;
use roko_core::{
    AgentRole, Body, Budget, Compose, Context, Engram, Kind, Provenance, Store, TaskCategory,
    Verdict, Verify,
};
use roko_fs::FileSubstrate;
use roko_gate::{BuildSystem, ClippyGate, CompileGate, GatePayload, ShellGate, TestGate};
use roko_learn::episode_logger::{Episode, EpisodeLogger, GateVerdict, Usage as EpisodeUsage};
use roko_learn::playbook::Playbook;
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};
use roko_learn::skill_library::{Skill, SkillQuery};
use roko_orchestrator::{ServiceConfig, ServiceFactory};
use roko_runtime::effect_driver::EffectServices;
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowRunReport};
use roko_serve::bench::BenchStrategy;
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
    /// Token usage reported by the agent dispatch, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
}

/// Token usage captured from a single run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunUsage {
    /// Input (prompt) tokens consumed.
    pub input_tokens: u64,
    /// Output (completion) tokens produced.
    pub output_tokens: u64,
}

impl RunReport {
    /// True if the agent succeeded and every configured gate passed.
    #[must_use]
    pub fn overall_success(&self) -> bool {
        self.agent_success && self.gate_verdicts.iter().all(|(_, ok)| *ok)
    }

    /// Return the first gate that failed, if any.
    #[must_use]
    pub(crate) fn first_failed_gate(&self) -> Option<&str> {
        self.gate_verdicts
            .iter()
            .find_map(|(gate, passed)| (!*passed).then_some(gate.as_str()))
    }
}

struct StrategyPromptAugmentation {
    system_prompt: String,
    injected_playbook_ids: Vec<String>,
}

struct ContextEnrichmentOverlay {
    text: String,
    injected_playbook_ids: Vec<String>,
}

struct PlaybookSection {
    text: String,
    injected_playbook_ids: Vec<String>,
}

struct DispatchOutcome {
    agent_result: AgentResult,
    external_actions: Vec<ExternalAction>,
    injected_playbook_ids: Vec<String>,
    model_selection: Option<EffectiveModelSelection>,
}

/// Reject `--share` for engines that do not support it.
pub fn ensure_share_supported(is_legacy_engine: bool, share: bool) -> Result<()> {
    if share && !is_legacy_engine {
        return Err(anyhow!(
            "--share is not yet supported with the v2 engine. \
             Use --engine legacy for share functionality, or omit --share."
        ));
    }

    Ok(())
}

/// Write a RunReport to `.roko/shared/{token}.json` and return the token.
#[cfg(feature = "legacy-orchestrate")]
pub fn write_shared_run(workdir: &std::path::Path, report: &RunReport) -> anyhow::Result<String> {
    let token = roko_core::generate_share_token();
    let transcript = roko_serve::routes::shared_runs::RunTranscript {
        id: token.clone(),
        agent: "unknown".to_string(),
        role: "unknown".to_string(),
        prompt: report.prompt_id.clone(),
        success: report.overall_success(),
        gates: report.gate_verdicts.clone(),
        output: report.output_text.clone(),
        cost_usd: None,
        input_tokens: report.usage.map(|usage| usage.input_tokens),
        output_tokens: report.usage.map(|usage| usage.output_tokens),
        model: None,
        duration_s: None,
        episode_id: Some(report.episode_id.clone()),
        transcript: Vec::new(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    write_shared_transcript(workdir, &transcript)
}

pub fn write_shared_workflow_run(
    workdir: &std::path::Path,
    prompt: &str,
    agent: &str,
    role: &str,
    report: &WorkflowRunReport,
) -> anyhow::Result<String> {
    let token = roko_core::generate_share_token();
    let (report_agent, report_role) = workflow_report_agent_role(report);
    let transcript = roko_serve::routes::shared_runs::RunTranscript {
        id: token.clone(),
        agent: non_empty(agent)
            .map(ToOwned::to_owned)
            .or(report_agent)
            .unwrap_or_else(|| "workflow".to_string()),
        role: non_empty(role)
            .map(ToOwned::to_owned)
            .or(report_role)
            .unwrap_or_else(|| "workflow".to_string()),
        prompt: prompt.to_string(),
        success: report.success,
        gates: report
            .gates
            .iter()
            .map(|gate| (gate.name.clone(), gate.passed))
            .collect(),
        output: non_empty(&report.output).map(ToOwned::to_owned),
        cost_usd: report.cost,
        // GAP: WorkflowRunReport exposes only a combined `token_usage: u64` total; the
        // workflow engine does not track input vs. output token counts separately. To
        // populate these fields the engine would need to accumulate per-turn TokenUsage
        // breakdowns and surface them on WorkflowRunReport.
        input_tokens: None,
        output_tokens: None,
        model: non_empty(&report.model).map(ToOwned::to_owned),
        duration_s: Some(report.duration_secs),
        episode_id: Some(report.run_id.clone()),
        transcript: report.events.clone(),
        timestamp: report
            .events
            .first()
            .map(|event| event.ts.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
    };
    write_shared_transcript(workdir, &transcript)
}

fn write_shared_transcript(
    workdir: &std::path::Path,
    transcript: &roko_serve::routes::shared_runs::RunTranscript,
) -> anyhow::Result<String> {
    let token = transcript.id.clone();
    let dir = workdir.join(".roko").join("shared");
    std::fs::create_dir_all(&dir)?;
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
    pub task_reports: Vec<PlanTaskWorkflowReport>,
    pub task_errors: Vec<PlanTaskWorkflowError>,
}

#[derive(Debug, Clone)]
pub struct PlanWorkflowTask {
    pub plan_id: String,
    pub task: crate::task_parser::TaskDef,
}

#[derive(Debug, Clone)]
pub struct PlanTaskWorkflowReport {
    pub plan_id: String,
    pub task_id: String,
    pub report: WorkflowRunReport,
}

#[derive(Debug, Clone)]
pub struct PlanTaskWorkflowError {
    pub plan_id: String,
    pub task_id: String,
    pub error: String,
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

fn non_empty(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn workflow_report_agent_role(report: &WorkflowRunReport) -> (Option<String>, Option<String>) {
    let mut first = None;
    for envelope in &report.events {
        if let roko_core::RuntimeEvent::AgentSpawned { agent_id, role, .. } = &envelope.payload {
            let values = (Some(agent_id.clone()), Some(role.clone()));
            if role == "implementer" {
                return values;
            }
            first.get_or_insert(values);
        }
    }
    first.unwrap_or((None, None))
}

pub fn workflow_report_outcome(
    report: &WorkflowRunReport,
) -> Option<roko_core::WorkflowOutcome> {
    report
        .events
        .iter()
        .rev()
        .find_map(|envelope| match &envelope.payload {
            roko_core::RuntimeEvent::WorkflowCompleted { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
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

// `cmd_run` only passes the resolved workspace config into the v2 path, so we
// re-read the process args/env here to preserve the caller's `--model`/`--role`
// inputs without introducing a second selection chain.
fn workflow_cli_overrides() -> (Option<String>, Option<String>) {
    let mut model = None;
    let mut role = None;
    let mut parsing_flags = true;
    let mut args = std::env::args_os().skip(1).peekable();

    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy();
        if parsing_flags && arg == "--" {
            parsing_flags = false;
            continue;
        }
        if !parsing_flags {
            continue;
        }
        if let Some(value) = arg.strip_prefix("--model=") {
            model = Some(value.to_string());
            continue;
        }
        if let Some(value) = arg.strip_prefix("--role=") {
            role = Some(value.to_string());
            continue;
        }
        if arg == "--model" {
            if let Some(value) = args.peek() {
                let value = value.to_string_lossy().into_owned();
                if !value.starts_with('-') {
                    model = Some(value);
                    let _ = args.next();
                }
            }
            continue;
        }
        if arg == "--role" {
            if let Some(value) = args.peek() {
                let value = value.to_string_lossy().into_owned();
                if !value.starts_with('-') {
                    role = Some(value);
                    let _ = args.next();
                }
            }
            continue;
        }
    }

    if model.is_none() {
        model = std::env::var("ROKO_MODEL")
            .ok()
            .filter(|value| !value.is_empty());
    }
    if role.is_none() {
        role = std::env::var("ROKO_ROLE")
            .ok()
            .filter(|value| !value.is_empty());
    }

    (model, role)
}

fn resolve_workflow_model_selection(
    workdir: &std::path::Path,
) -> anyhow::Result<(Config, RokoConfig, EffectiveModelSelection)> {
    let mut config = crate::config::load_layered(workdir)
        .map(|resolved| resolved.config)
        .unwrap_or_default();
    let (cli_model_override, cli_role_override) = workflow_cli_overrides();
    if let Some(model) = cli_model_override.clone() {
        config.agent.model = Some(model);
    }
    if let Some(role) = cli_role_override.clone() {
        config.prompt.role = role;
    }

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

    let role = non_empty(&config.prompt.role).map(str::to_owned);
    let selection = resolve_effective_model(cli_model_override, None, role, None, &model_config)
        .map_err(|error| anyhow!("resolve workflow model selection: {error}"))?;

    Ok((config, model_config, selection))
}

fn build_workflow_effect_services(
    workdir: &std::path::Path,
    config: &Config,
    mut model_config: RokoConfig,
    selection: &EffectiveModelSelection,
) -> anyhow::Result<EffectServices> {
    model_config.agent.default_model = selection.effective_model_key.clone();

    let services = ServiceFactory::build(ServiceConfig {
        workdir: workdir.to_path_buf(),
        roko_dir: workdir.join(".roko"),
        workspace_config: model_config,
        model_key: Some(selection.effective_model_key.clone()),
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

/// Convert a `PipelineBandConfig` from `roko.toml` into a `WorkflowConfig` for the V2 engine.
fn workflow_config_from_band(band: &roko_core::config::PipelineBandConfig) -> WorkflowConfig {
    WorkflowConfig {
        has_strategy: band.strategist,
        has_review: band.reviewers,
        max_iterations: band.max_iterations,
        // When reviewers are disabled, one autofix attempt is enough.
        // When reviewers are enabled, allow two rounds.
        max_autofix_attempts: if band.reviewers { 2 } else { 1 },
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
) -> anyhow::Result<WorkflowRunReport> {
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
) -> anyhow::Result<WorkflowRunReport> {
    let (config, model_config, selection) = resolve_workflow_model_selection(workdir)?;
    selection.print_stderr();

    let pipeline_config = model_config.pipeline.clone();
    let services = build_workflow_effect_services(workdir, &config, model_config, &selection)?;

    // Use the pipeline bands declared in roko.toml for the interactive run path.
    let workflow = match workflow_template {
        "express" | "mechanical" => workflow_config_from_band(&pipeline_config.mechanical),
        "focused" => workflow_config_from_band(&pipeline_config.focused),
        "integrative" => workflow_config_from_band(&pipeline_config.integrative),
        "full" | "architectural" => workflow_config_from_band(&pipeline_config.architectural),
        "standard" => workflow_config_from_band(&pipeline_config.mechanical),
        _ => workflow_config_for_template(workflow_template),
    };
    let workflow_label = match workflow_template {
        "express" | "mechanical" | "standard" => "mechanical",
        "focused" => "focused",
        "integrative" => "integrative",
        "full" | "architectural" => "architectural",
        _ => workflow_template,
    };

    let report = run_workflow_engine_with_services(
        prompt,
        workdir,
        workflow,
        enabled_gates,
        shell_gates,
        external_hub,
        services,
        selection.provider_key,
    )
    .await?;
    print_workflow_run_report(prompt, workflow_label, &report);
    Ok(report)
}

pub async fn run_workflow_engine_report_with_hub(
    prompt: &str,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
    external_hub: Option<&StateHub>,
) -> anyhow::Result<WorkflowRunReport> {
    let (config, model_config, selection) = resolve_workflow_model_selection(workdir)?;
    selection.print_stderr();
    let services = build_workflow_effect_services(workdir, &config, model_config, &selection)?;

    run_workflow_engine_with_services(
        prompt,
        workdir,
        workflow_config_for_template(workflow_template),
        enabled_gates,
        shell_gates,
        external_hub,
        services,
        selection.provider_key,
    )
    .await
}

async fn run_workflow_engine_with_services(
    prompt: &str,
    workdir: &std::path::Path,
    workflow: WorkflowConfig,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
    external_hub: Option<&StateHub>,
    services: EffectServices,
    provider_key: String,
) -> anyhow::Result<WorkflowRunReport> {
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

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow,
        enabled_gates,
        shell_gates,
        commit_prefix: Some("feat".to_string()),
    };

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

    let mut result = engine
        .run(config)
        .await
        .map_err(|error| anyhow!("workflow engine failed: {error}"))?;

    result.provider = Some(provider_key);

    Ok(result)
}

pub fn print_workflow_run_report(
    prompt: &str,
    workflow_template: &str,
    report: &WorkflowRunReport,
) {
    output_format::intro("roko run");
    output_format::step("prompt", &output_format::dim(&truncate(prompt, 60)));
    output_format::step("workflow", workflow_template);
    output_format::step("model", &report.model);
    output_format::divider();

    if report.success {
        output_format::success(&format!(
            "workflow completed ({} agent turn{})",
            report.agent_turns,
            if report.agent_turns == 1 { "" } else { "s" },
        ));
    } else {
        output_format::error("workflow failed");
    }

    if !report.output.trim().is_empty() {
        output_format::bar(&truncate(&report.output, 200));
    }

    output_format::divider();
    output_format::step("Summary", "");
    output_format::branch(&format!(
        "duration   {}",
        output_format::cyan(&format_duration(std::time::Duration::from_secs_f64(
            report.duration_secs,
        ))),
    ));
    output_format::branch(&format!(
        "tokens     {}",
        output_format::cyan(&report.token_usage.to_string()),
    ));
    if let Some(cost) = report.cost {
        output_format::branch(&format!(
            "cost       {}",
            output_format::cyan(&format!("{cost:.4}"))
        ));
    }
    if report.gates.is_empty() {
        output_format::branch("gates      (none configured)");
    } else {
        for gate in &report.gates {
            let marker = if gate.passed { "PASS" } else { "FAIL" };
            output_format::branch(&format!("gate       [{marker}] {}", gate.name));
        }
    }
    output_format::end(&output_format::dim(&report.run_id));
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
    // Convert (task_id, prompt) pairs into (plan_id, task_id, prompt) triples.
    // Derive plan_id from the task_id prefix (e.g. "my-plan:task-1" → "my-plan");
    // falls back to the full task_id when no colon separator is present.
    let triples: Vec<(String, String, String)> = tasks
        .iter()
        .map(|(task_id, prompt)| {
            let plan_id = task_id.split(':').next().unwrap_or(task_id).to_string();
            (plan_id, task_id.clone(), prompt.clone())
        })
        .collect();
    run_plan_prompts_core(
        triples,
        workdir,
        workflow_template,
        enabled_gates,
        shell_gates,
    )
    .await
}

pub async fn run_plan_tasks_with_workflow_engine(
    tasks: &[PlanWorkflowTask],
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
) -> anyhow::Result<PlanWorkflowReport> {
    // Build prompts up front so the shared core loop only handles (plan_id, task_id, prompt).
    let triples: Vec<(String, String, String)> = tasks
        .iter()
        .map(|pt| {
            let prompt = pt.task.build_prompt(&pt.plan_id, workdir);
            (pt.plan_id.clone(), pt.task.id.clone(), prompt)
        })
        .collect();
    run_plan_prompts_core(
        triples,
        workdir,
        workflow_template,
        enabled_gates,
        shell_gates,
    )
    .await
}

/// Shared accumulator loop for both plan-running entry points.
///
/// Runs each `(plan_id, task_id, prompt)` triple sequentially through the
/// WorkflowEngine and collects per-task outcomes into a [`PlanWorkflowReport`].
async fn run_plan_prompts_core(
    triples: Vec<(String, String, String)>,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
) -> anyhow::Result<PlanWorkflowReport> {
    let total = triples.len();
    let mut passed = 0;
    let mut failed = 0;
    let mut outcomes = Vec::with_capacity(total);
    let mut task_reports = Vec::new();
    let mut task_errors = Vec::new();

    for (plan_id, task_id, prompt) in triples {
        match execute_plan_prompt_with_workflow_engine(
            &prompt,
            workdir,
            workflow_template,
            enabled_gates.clone(),
            shell_gates.clone(),
        )
        .await
        {
            Ok(result) => {
                let success = result.success;
                let message = format!(
                    "{} in {} agent turn{}",
                    if success { "success" } else { "failed" },
                    result.agent_turns,
                    if result.agent_turns == 1 { "" } else { "s" },
                );
                println!("[{plan_id}:{task_id}] {message}");
                tracing::info!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    success = result.success,
                    agent_turns = result.agent_turns,
                    "v2 workflow task complete"
                );
                if success {
                    passed += 1;
                } else {
                    failed += 1;
                }
                outcomes.push((task_id.clone(), success, message));
                task_reports.push(PlanTaskWorkflowReport {
                    plan_id,
                    task_id,
                    report: result,
                });
            }
            Err(error) => {
                let message = error.to_string();
                println!("[{plan_id}:{task_id}] failed: {message}");
                tracing::warn!(
                    plan_id = %plan_id,
                    task_id = %task_id,
                    error = %message,
                    "v2 workflow task failed"
                );
                failed += 1;
                outcomes.push((task_id.clone(), false, message.clone()));
                task_errors.push(PlanTaskWorkflowError {
                    plan_id,
                    task_id,
                    error: message,
                });
            }
        }
    }

    Ok(PlanWorkflowReport {
        total,
        passed,
        failed,
        outcomes,
        task_reports,
        task_errors,
    })
}

pub async fn execute_plan_task_with_workflow_engine(
    plan_id: &str,
    task: &crate::task_parser::TaskDef,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
) -> anyhow::Result<WorkflowRunReport> {
    let prompt = task.build_prompt(plan_id, workdir);
    execute_plan_prompt_with_workflow_engine(
        &prompt,
        workdir,
        workflow_template,
        enabled_gates,
        shell_gates,
    )
    .await
}

async fn execute_plan_prompt_with_workflow_engine(
    prompt: &str,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
    shell_gates: Vec<CoreShellGateCommand>,
) -> anyhow::Result<WorkflowRunReport> {
    run_workflow_engine_report_with_hub(
        prompt,
        workdir,
        workflow_template,
        enabled_gates,
        shell_gates,
        None,
    )
    .await
}

pub fn discover_plan_workflow_tasks(
    plans_dir: &std::path::Path,
) -> anyhow::Result<Vec<PlanWorkflowTask>> {
    let mut tasks = Vec::new();
    for tasks_path in discover_task_files(plans_dir)? {
        let tasks_file = crate::task_parser::TasksFile::parse(&tasks_path)?;
        let plan_id = tasks_file.meta.plan.clone();
        tasks.extend(
            dependency_ordered_task_defs(tasks_file.tasks)
                .into_iter()
                .map(|task| PlanWorkflowTask {
                    plan_id: plan_id.clone(),
                    task,
                }),
        );
    }

    Ok(tasks)
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

    let mut prompts = Vec::new();
    for tasks_path in discover_task_files(plans_dir)? {
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

fn discover_task_files(plans_dir: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
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

    Ok(task_files)
}

fn dependency_ordered_task_defs(
    tasks: Vec<crate::task_parser::TaskDef>,
) -> Vec<crate::task_parser::TaskDef> {
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
#[cfg(feature = "legacy-orchestrate")]
#[allow(clippy::too_many_lines)]
pub async fn run_once(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
    strategy: Option<BenchStrategy>,
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

    // Batch-write the prompt sections and composed prompt in one I/O pass.
    let mut prompt_signals = sections;
    prompt_signals.push(prompt.clone());
    substrate
        .put_batch(prompt_signals)
        .await
        .map_err(|e| anyhow!("persist prompt signals: {e}"))?;

    // Run the configured agent path for this provider/backend mix.
    let DispatchOutcome {
        agent_result,
        external_actions,
        injected_playbook_ids,
        model_selection,
    } = dispatch_agent(workdir, config, &prompt, prompt_text, &ctx, strategy).await?;

    // Optionally post-process the agent output to strip ANSI escapes and
    // reasoning-model thinking traces. The raw body is preserved as an
    // AgentMessage trace so nothing is lost.
    let final_output_sig = if config.agent.clean_output {
        maybe_clean_output(&prompt, &agent_result, &substrate).await?
    } else {
        agent_result.output.clone()
    };
    if config.agent.clean_output {
        // The clean path already wrote the canonical output; batch any traces.
        if !agent_result.trace.is_empty() {
            substrate
                .put_batch(agent_result.trace.clone())
                .await
                .map_err(|e| anyhow!("persist agent traces: {e}"))?;
        }
    } else {
        let mut batch = vec![agent_result.output.clone()];
        batch.extend(agent_result.trace.iter().cloned());
        substrate
            .put_batch(batch)
            .await
            .map_err(|e| anyhow!("persist agent output + traces: {e}"))?;
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
        verdict_summary.push((verdict.gate.clone(), verdict.passed));
        verdict_sigs.push(sig);
    }
    substrate
        .put_batch(verdict_sigs.clone())
        .await
        .map_err(|e| anyhow!("persist verdicts: {e}"))?;

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
        model_selection.as_ref(),
    )
    .await
    {
        eprintln!("[run] episode logger failed: {err}");
    }

    // Emit completion, episode, and efficiency events for the TUI.
    let all_passed = verdict_summary.iter().all(|(_, p)| *p);
    let task_passed = agent_result.success && all_passed;
    event_hub.publish(DashboardEvent::TaskCompleted {
        plan_id: run_plan_id.clone(),
        task_id: prompt_text.chars().take(60).collect(),
        outcome: if task_passed {
            "success".into()
        } else {
            "failed".into()
        },
    });
    event_hub.publish(DashboardEvent::PlanCompleted {
        plan_id: run_plan_id.clone(),
        success: task_passed,
    });
    event_hub.publish(DashboardEvent::EpisodeRecorded {
        agent_id: config.agent.command.clone(),
        role: config.prompt.role.clone(),
        episode_id: episode.id.to_hex(),
        passed: task_passed,
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

    record_injected_playbook_outcomes(workdir, strategy, &injected_playbook_ids, task_passed).await;

    let total_signals = substrate
        .len()
        .await
        .map_err(|e| anyhow!("count signals: {e}"))?;
    let usage = Some(RunUsage {
        input_tokens: u64::from(agent_result.usage.input_tokens),
        output_tokens: u64::from(agent_result.usage.output_tokens),
    });

    Ok(RunReport {
        episode_id: episode.id.to_hex(),
        prompt_id: prompt.id.to_hex(),
        agent_output_id: final_output_sig.id.to_hex(),
        agent_success: agent_result.success,
        gate_verdicts: verdict_summary,
        total_signals,
        output_text: final_output_sig.body.as_text().ok().map(ToOwned::to_owned),
        usage,
    })
}

#[cfg(not(feature = "legacy-orchestrate"))]
pub async fn run_once(
    _workdir: &Path,
    _config: &Config,
    _prompt_text: &str,
    strategy: Option<BenchStrategy>,
    _external_hub: Option<&StateHub>,
) -> Result<RunReport> {
    let _ = strategy;
    anyhow::bail!(
        "legacy run_once is disabled; use the WorkflowEngine v2 path or enable legacy-orchestrate"
    )
}

#[cfg(feature = "legacy-orchestrate")]
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

#[cfg(feature = "legacy-orchestrate")]
async fn augment_system_prompt_for_strategy(
    base_system_prompt: String,
    workdir: &Path,
    role: &str,
    prompt_text: &str,
    current_model: &str,
    strategy: Option<BenchStrategy>,
) -> StrategyPromptAugmentation {
    if skip_bench_enrichment(strategy) {
        return StrategyPromptAugmentation {
            system_prompt: base_system_prompt,
            injected_playbook_ids: Vec::new(),
        };
    }

    let overlay = match strategy {
        Some(BenchStrategy::ContextEnriched) => {
            // Context-enriched is the first step above minimal: add learned
            // playbooks + skills, but skip neuro knowledge and retry guidance.
            build_context_enrichment_overlay(workdir, role, prompt_text).await
        }
        _ => {
            // Default path keeps the full enrichment stack.
            let mut overlay = build_context_enrichment_overlay(workdir, role, prompt_text).await;
            let neuro_overlay =
                build_neuro_augmented_overlay(workdir, role, prompt_text, current_model).await;
            if !neuro_overlay.is_empty() {
                if !overlay.text.is_empty() {
                    overlay.text.push_str("\n\n");
                }
                overlay.text.push_str(&neuro_overlay);
            }
            overlay
        }
    };

    if overlay.text.trim().is_empty() {
        StrategyPromptAugmentation {
            system_prompt: base_system_prompt,
            injected_playbook_ids: overlay.injected_playbook_ids,
        }
    } else {
        let mut prompt = base_system_prompt;
        prompt.push_str("\n\n");
        prompt.push_str(&overlay.text);
        StrategyPromptAugmentation {
            system_prompt: prompt,
            injected_playbook_ids: overlay.injected_playbook_ids,
        }
    }
}

#[cfg(feature = "legacy-orchestrate")]
fn skip_bench_enrichment(strategy: Option<BenchStrategy>) -> bool {
    matches!(strategy, Some(BenchStrategy::Minimal))
}

#[cfg(feature = "legacy-orchestrate")]
fn bench_strategy_uses_playbooks(strategy: Option<BenchStrategy>) -> bool {
    matches!(strategy, Some(BenchStrategy::ContextEnriched))
        || matches!(strategy, Some(BenchStrategy::NeuroAugmented))
        || matches!(strategy, Some(BenchStrategy::FullCascade))
}

#[cfg(feature = "legacy-orchestrate")]
async fn build_context_enrichment_overlay(
    workdir: &Path,
    role: &str,
    prompt_text: &str,
) -> ContextEnrichmentOverlay {
    let (playbooks, skills) = tokio::join!(
        build_relevant_playbooks_section(workdir, role, "cli-run", prompt_text),
        build_relevant_skills_section(workdir, prompt_text),
    );

    let PlaybookSection {
        text: playbook_text,
        injected_playbook_ids,
    } = playbooks;

    let mut sections = Vec::new();
    if !playbook_text.is_empty() {
        sections.push(playbook_text);
    }
    if !skills.is_empty() {
        sections.push(skills);
    }

    if sections.is_empty() {
        ContextEnrichmentOverlay {
            text: String::new(),
            injected_playbook_ids: Vec::new(),
        }
    } else {
        ContextEnrichmentOverlay {
            text: format!("## Relevant Techniques\n\n{}", sections.join("\n\n")),
            injected_playbook_ids,
        }
    }
}

#[cfg(feature = "legacy-orchestrate")]
async fn build_relevant_playbooks_section(
    workdir: &Path,
    role: &str,
    task: &str,
    task_text: &str,
) -> PlaybookSection {
    let parsed_role = parse_agent_role(role).unwrap_or(AgentRole::Implementer);
    let store = match load_or_create_playbook_store(
        &workdir.join(".roko").join("learn").join("playbooks"),
    )
    .await
    {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(error = %err, "failed to load playbook store for strategy enrichment");
            return PlaybookSection {
                text: String::new(),
                injected_playbook_ids: Vec::new(),
            };
        }
    };

    let query = playbook_query_context(parsed_role, task, task_text, None);
    let playbooks = match store.query(&query).await {
        Ok(playbooks) => playbooks,
        Err(err) => {
            tracing::warn!(error = %err, "failed to query playbooks for strategy enrichment");
            return PlaybookSection {
                text: String::new(),
                injected_playbook_ids: Vec::new(),
            };
        }
    };

    if playbooks.is_empty() {
        PlaybookSection {
            text: String::new(),
            injected_playbook_ids: Vec::new(),
        }
    } else {
        let text = render_relevant_playbooks(&playbooks);
        let injected_playbook_ids = playbooks.into_iter().map(|playbook| playbook.id).collect();
        PlaybookSection {
            text,
            injected_playbook_ids,
        }
    }
}

#[cfg(feature = "legacy-orchestrate")]
async fn record_injected_playbook_outcomes(
    workdir: &Path,
    strategy: Option<BenchStrategy>,
    injected_playbook_ids: &[String],
    task_passed: bool,
) {
    if injected_playbook_ids.is_empty() || !bench_strategy_uses_playbooks(strategy) {
        return;
    }

    let playbook_root = workdir.join(".roko").join("learn").join("playbooks");
    let store = match load_or_create_playbook_store(&playbook_root).await {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(
                error = %err,
                path = %playbook_root.display(),
                "failed to load playbook store for outcome recording"
            );
            return;
        }
    };

    for pb_id in injected_playbook_ids {
        match store.record_outcome(pb_id, task_passed).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    playbook_id = %pb_id,
                    "playbook not found while recording bench outcome"
                );
            }
            Err(err) => {
                tracing::warn!(
                    playbook_id = %pb_id,
                    error = %err,
                    "failed to record playbook outcome"
                );
            }
        }
    }
}

#[cfg(feature = "legacy-orchestrate")]
async fn build_relevant_skills_section(workdir: &Path, prompt_text: &str) -> String {
    let store = match load_or_create_skill_library(
        &workdir.join(".roko").join("learn").join("skills.json"),
    )
    .await
    {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(error = %err, "failed to load skill library for strategy enrichment");
            return String::new();
        }
    };

    let mut tags = extract_task_symbols(prompt_text);
    tags.sort();
    tags.dedup();
    let query = SkillQuery {
        tags,
        category: Some(TaskCategory::Implementation.label().to_string()),
        files_hint: Vec::new(),
    };

    let skills = store
        .select(&query, 5)
        .into_iter()
        .filter(|skill| skill.score >= 0.5)
        .filter(|skill| !skill.tags.iter().any(|tag| tag == "outcome:failure"))
        .collect::<Vec<Skill>>();

    if skills.is_empty() {
        String::new()
    } else {
        render_prior_experience(&skills)
    }
}

#[cfg(feature = "legacy-orchestrate")]
async fn build_neuro_augmented_overlay(
    workdir: &Path,
    role: &str,
    prompt_text: &str,
    current_model: &str,
) -> String {
    let parsed_role = parse_agent_role(role).unwrap_or(AgentRole::Implementer);
    let knowledge_store = roko_neuro::KnowledgeStore::for_workdir(workdir);
    let mut sections = Vec::new();

    if let Some(strategy_fragments) = build_strategy_fragment_context(
        &knowledge_store,
        parsed_role,
        None,
        prompt_text,
        current_model,
    ) {
        sections.push(strategy_fragments);
    }

    let anti_patterns = query_anti_knowledge_patterns(&knowledge_store, prompt_text, 5);
    if !anti_patterns.is_empty() {
        sections.push(render_common_mistakes_section(&anti_patterns));
    }

    sections.join("\n\n")
}

#[cfg(feature = "legacy-orchestrate")]
fn render_relevant_playbooks(playbooks: &[Playbook]) -> String {
    use std::fmt::Write as _;

    let mut body = String::from("## Playbook Techniques\n\nReusable proven procedures:\n");
    for playbook in playbooks {
        let _ = writeln!(
            body,
            "- {}: {} (successes {}, failures {})",
            playbook.id, playbook.goal, playbook.success_count, playbook.failure_count
        );
        for step in playbook.steps.iter().take(5) {
            let expected = if step.expected_signals.is_empty() {
                "task-local verification".to_string()
            } else {
                step.expected_signals.join(", ")
            };
            let _ = writeln!(
                body,
                "  - {} via {}; expect {}",
                step.description, step.action_kind, expected
            );
        }
    }
    body
}

#[cfg(feature = "legacy-orchestrate")]
fn render_common_mistakes_section(patterns: &[String]) -> String {
    use std::fmt::Write as _;

    let mut body =
        String::from("## Common Mistakes to Avoid\n\nKnown anti-patterns and failure modes:\n");
    for pattern in patterns {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        let _ = writeln!(body, "- {pattern}");
    }
    body
}

#[cfg(feature = "legacy-orchestrate")]
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

#[cfg(feature = "legacy-orchestrate")]
fn fallback_system_prompt(role: &str, prompt_text: &str, tools_csv: &str) -> String {
    format!(
        "You are a {role} agent.\n\n## Current Task\n\n{prompt_text}\n\n## Tool Instructions\n\nAvailable tools: {tools_csv}\n\n## Project Conventions\n\nMake minimal, targeted changes and run relevant verification before finishing."
    )
}

#[cfg(feature = "legacy-orchestrate")]
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

#[cfg(feature = "legacy-orchestrate")]
async fn dispatch_agent(
    workdir: &Path,
    config: &Config,
    prompt: &Engram,
    prompt_text: &str,
    ctx: &Context,
    strategy: Option<BenchStrategy>,
) -> Result<DispatchOutcome> {
    // TODO(gateway): migrate to ModelCallService.
    let mut routing_config = roko_core::config::load_config(workdir)
        .with_context(|| format!("load routing config from {}", workdir.display()))?;
    routing_config.apply_process_env();
    crate::config::merge_global_providers(&mut routing_config);
    let has_routing = !routing_config.providers.is_empty() || !routing_config.models.is_empty();
    let use_provider_routing = has_routing && config.agent.command == "claude";
    let (cli_model_override, _) = workflow_cli_overrides();
    let resolved_cli_model = if let Some(requested_model) = cli_model_override.clone() {
        Some(
            resolve_effective_model(
                Some(requested_model),
                None,
                Some(config.prompt.role.clone()),
                None,
                &routing_config,
            )
            .map_err(|error| anyhow!("resolve legacy run model selection: {error}"))?,
        )
    } else {
        None
    };
    let selected_model_override = resolved_cli_model
        .as_ref()
        .map(|selection| selection.backend_slug.clone());
    if let Some(selection) = resolved_cli_model.as_ref() {
        tracing::info!(
            model = %selection.effective_model_key,
            backend_slug = %selection.backend_slug,
            provider = %selection.provider_key,
            source = %selection.source,
            "[run] resolved model selection"
        );
    }

    if use_provider_routing {
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let model = selected_model_override
            .clone()
            .or_else(|| config.agent.model.clone())
            .unwrap_or_else(|| routing_config.agent.default_model.clone());
        let StrategyPromptAugmentation {
            system_prompt,
            injected_playbook_ids,
        } = augment_system_prompt_for_strategy(
            build_system_prompt(config, prompt_text, &tools_csv),
            workdir,
            &config.prompt.role,
            prompt_text,
            &model,
            strategy,
        )
        .await;
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
        Ok(DispatchOutcome {
            agent_result: agent.run(prompt, ctx).await,
            external_actions: Vec::new(),
            injected_playbook_ids,
            model_selection: resolved_cli_model.clone(),
        })
    } else if config.agent.command == "claude" && has_anthropic_api_key(config) {
        // Anthropic API tool loop — direct HTTP calls with full tool visibility.
        return run_anthropic_api_tool_loop(
            workdir,
            config,
            selected_model_override.clone(),
            prompt_text,
            strategy,
        )
        .await;
    } else if config.agent.command == "claude" {
        // Claude CLI keeps its own prompt/tool/settings wiring internally.
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let (extra_args, resume_from_args) = split_resume_arg(&config.agent.args);
        let optional_resume = optional_resume_session_id(config, resume_from_args);
        let model = selected_model_override
            .clone()
            .or_else(|| config.agent.model.clone())
            .unwrap_or_else(|| {
                // Prefer the routing config's default_model (from roko.toml or global config)
                // over hardcoded Claude. This ensures ZAI_API_KEY / glm-5.1 setups work.
                if !routing_config.agent.default_model.is_empty() {
                    routing_config.agent.default_model.clone()
                } else {
                    "claude-sonnet-4-6".to_string()
                }
            });
        let StrategyPromptAugmentation {
            system_prompt,
            injected_playbook_ids,
        } = augment_system_prompt_for_strategy(
            build_system_prompt(config, prompt_text, &tools_csv),
            workdir,
            &config.prompt.role,
            prompt_text,
            &model,
            strategy,
        )
        .await;
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
        Ok(DispatchOutcome {
            agent_result: agent.run(prompt, ctx).await,
            external_actions: Vec::new(),
            injected_playbook_ids,
            model_selection: resolved_cli_model.clone(),
        })
    } else if config.agent.command == "ollama" {
        Ok(run_ollama_agentic_single(
            workdir,
            config,
            selected_model_override.clone(),
            prompt_text,
            strategy,
        )
        .await)
    } else if is_known_protocol_command(&config.agent.command) {
        let model = selected_model_override
            .clone()
            .or_else(|| {
                config
                    .agent
                    .model
                    .clone()
                    .and_then(|model| (!model.is_empty()).then_some(model))
            })
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
        Ok(DispatchOutcome {
            agent_result: agent.run(prompt, ctx).await,
            external_actions: Vec::new(),
            injected_playbook_ids: Vec::new(),
            model_selection: resolved_cli_model.clone(),
        })
    } else {
        let model = selected_model_override
            .clone()
            .or_else(|| {
                config
                    .agent
                    .model
                    .clone()
                    .and_then(|model| (!model.is_empty()).then_some(model))
            })
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
        Ok(DispatchOutcome {
            agent_result: agent.run(prompt, ctx).await,
            external_actions: Vec::new(),
            injected_playbook_ids: Vec::new(),
            model_selection: resolved_cli_model.clone(),
        })
    }
}

/// Ollama agentic path for `roko run`.
#[cfg(feature = "legacy-orchestrate")]
async fn run_ollama_agentic_single(
    workdir: &Path,
    config: &Config,
    model_override: Option<String>,
    prompt_text: &str,
    strategy: Option<BenchStrategy>,
    model_selection: Option<EffectiveModelSelection>,
) -> DispatchOutcome {
    use parking_lot::RwLock;
    use roko_agent::dispatcher::ToolDispatcher;
    use roko_agent::tool_loop::{StopReason, ToolLoop};
    use roko_core::tool::{ToolContext, ToolHandler};
    use std::sync::Arc;
    use std::time::Instant;

    let started = Instant::now();
    let model = model_override
        .or_else(|| config.agent.model.clone())
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

    let StrategyPromptAugmentation {
        system_prompt,
        injected_playbook_ids,
    } = augment_system_prompt_for_strategy(
        build_system_prompt(config, prompt_text, ""),
        workdir,
        &config.prompt.role,
        prompt_text,
        &model,
        strategy,
    )
    .await;
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
        DispatchOutcome {
            agent_result: AgentResult::ok(sig).with_usage(usage),
            external_actions,
            injected_playbook_ids,
            model_selection,
        }
    } else {
        DispatchOutcome {
            agent_result: AgentResult::fail(sig).with_usage(usage),
            external_actions,
            injected_playbook_ids,
            model_selection,
        }
    }
}

/// Check whether an Anthropic API key is available for the direct-API path.
#[cfg(feature = "legacy-orchestrate")]
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
#[cfg(feature = "legacy-orchestrate")]
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
#[cfg(feature = "legacy-orchestrate")]
async fn run_anthropic_api_tool_loop(
    workdir: &Path,
    config: &Config,
    model_override: Option<String>,
    prompt_text: &str,
    strategy: Option<BenchStrategy>,
    model_selection: Option<EffectiveModelSelection>,
) -> Result<DispatchOutcome> {
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

    let model = model_override
        .or_else(|| config.agent.model.clone())
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

    let StrategyPromptAugmentation {
        system_prompt,
        injected_playbook_ids,
    } = augment_system_prompt_for_strategy(
        build_system_prompt(config, prompt_text, ""),
        workdir,
        &config.prompt.role,
        prompt_text,
        &model,
        strategy,
    )
    .await;
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
        Ok(DispatchOutcome {
            agent_result: AgentResult::ok(sig).with_usage(usage),
            external_actions,
            injected_playbook_ids,
            model_selection,
        })
    } else {
        Ok(DispatchOutcome {
            agent_result: AgentResult::fail(sig).with_usage(usage),
            external_actions,
            injected_playbook_ids,
            model_selection,
        })
    }
}

/// Build a chain-aware handler resolver if chain config is present in the workspace.
#[cfg(feature = "legacy-orchestrate")]
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
#[cfg(feature = "legacy-orchestrate")]
fn truncate_json_args(args: &serde_json::Value, max_len: usize) -> String {
    let s = args.to_string();
    if s.len() > max_len {
        format!("{}…", &s[..max_len.saturating_sub(1)])
    } else {
        s
    }
}

/// Extract a playbook for a successful bench run, using structured output
/// when available and otherwise falling back to the latest episode log entry.
pub(crate) async fn extract_bench_playbook(
    workdir: &Path,
    prompt: &str,
    output_text: Option<&str>,
) -> Result<Option<Playbook>> {
    if let Some(playbook) = extract_playbook_from_output_text(prompt, output_text) {
        return Ok(Some(playbook));
    }

    let Some(episode) = latest_learning_episode(workdir).await? else {
        return Ok(None);
    };
    let tool_calls = roko_learn::playbook::extract_tool_calls_from_episode(&episode);
    if tool_calls.is_empty() {
        return Ok(None);
    }

    let task_id = non_empty(&episode.task_id).unwrap_or("bench-episode");
    Ok(roko_learn::playbook::extract_playbook_from_episode(
        task_id,
        prompt,
        &tool_calls,
    ))
}

fn extract_playbook_from_output_text(prompt: &str, output_text: Option<&str>) -> Option<Playbook> {
    let tool_calls = extract_tool_calls_from_output_text(output_text)?;
    roko_learn::playbook::extract_playbook_from_episode("bench-output", prompt, &tool_calls)
}

fn extract_tool_calls_from_output_text(output_text: Option<&str>) -> Option<Vec<(String, String)>> {
    let text = non_empty(output_text?)?;
    let value = serde_json::from_str::<serde_json::Value>(text).ok()?;
    if !value.is_array() && !value.is_object() {
        return None;
    }

    let mut episode = Episode::new("bench-output", "bench-output");
    episode.extra.insert("tool_calls".to_string(), value);
    let tool_calls = roko_learn::playbook::extract_tool_calls_from_episode(&episode);
    (!tool_calls.is_empty()).then_some(tool_calls)
}

async fn latest_learning_episode(workdir: &Path) -> Result<Option<Episode>> {
    let mut last_error: Option<anyhow::Error> = None;
    for path in learning_episode_paths(workdir) {
        match EpisodeLogger::read_all_lossy(&path).await {
            Ok(episodes) => {
                if let Some(episode) = episodes.last().cloned() {
                    return Ok(Some(episode));
                }
            }
            Err(err) => {
                last_error = Some(anyhow!("read {}: {err}", path.display()));
            }
        }
    }

    if let Some(err) = last_error {
        Err(err)
    } else {
        Ok(None)
    }
}

fn learning_episode_paths(workdir: &Path) -> Vec<PathBuf> {
    let roko = workdir.join(".roko");
    // Prefer the learn-root log because the legacy `run_once` path appends
    // there today; keep the root/memory locations as compatibility fallbacks.
    vec![
        roko.join("learn").join("episodes.jsonl"),
        roko.join("episodes.jsonl"),
        roko.join("memory").join("episodes.jsonl"),
    ]
}

#[cfg(feature = "legacy-orchestrate")]
async fn append_episode_log(
    workdir: &Path,
    config: &Config,
    prompt: &Engram,
    final_output: &Engram,
    verdicts: &[(String, bool)],
    agent_result: &AgentResult,
    external_actions: &[ExternalAction],
    model_selection: Option<&EffectiveModelSelection>,
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
    let resolved_model_value = model_selection
        .map(|selection| selection.effective_model_key.clone())
        .unwrap_or_else(|| resolved_model(config));
    episode.extra.insert(
        "model".to_string(),
        serde_json::json!(resolved_model_value.as_str()),
    );
    episode.extra.insert(
        "resolved_model".to_string(),
        serde_json::json!(resolved_model_value.as_str()),
    );
    if let Some(selection) = model_selection {
        episode.extra.insert(
            "selection_source".to_string(),
            serde_json::json!(selection.source.to_string()),
        );
    }
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

#[cfg(feature = "legacy-orchestrate")]
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

#[cfg(feature = "legacy-orchestrate")]
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
    // Check routing config for configured default model before returning an empty model for
    // non-Claude commands.
    if let Ok(mut rc) = roko_core::config::load_config(std::path::Path::new(".")) {
        rc.apply_process_env();
        crate::config::merge_global_providers(&mut rc);
        if !rc.agent.default_model.is_empty() {
            return rc.agent.default_model;
        }
    }
    if config.agent.command.trim().eq_ignore_ascii_case("claude") {
        "claude-sonnet-4-6".to_string()
    } else {
        String::new()
    }
}

fn dashboard_agent_model(config: &Config) -> String {
    let model = resolved_model(config);
    if !model.is_empty() {
        return model;
    }

    let command = config.agent.command.trim();
    command.to_string()
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

#[cfg(feature = "legacy-orchestrate")]
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
#[cfg(feature = "legacy-orchestrate")]
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
            .put_batch(vec![agent_result.output.clone()])
            .await
            .map_err(|e| anyhow!("persist agent output batch: {e}"))?;
        return Ok(agent_result.output.clone());
    }

    // Persist the raw version as a trace signal so nothing is lost.
    let raw_trace = agent_result
        .output
        .derive(Kind::AgentMessage, Body::text(&raw))
        .provenance(Provenance::agent("exec:raw"))
        .tag("stream", "raw_stdout")
        .build();

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
        .put_batch(vec![raw_trace, clean_sig.clone()])
        .await
        .map_err(|e| anyhow!("persist cleaned agent output batch: {e}"))?;
    Ok(clean_sig)
}

#[cfg(feature = "legacy-orchestrate")]
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

#[cfg(feature = "legacy-orchestrate")]
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
/// no models.
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
    use roko_core::foundation::{
        FeedbackEvent, FeedbackSink, GateConfig as WorkflowGateConfig, GateReport, GateRunner,
        GateVerdict, ModelCallRequest, ModelCallResponse, ModelCaller, PromptAssembler, PromptSpec,
        TokenUsage,
    };
    use tempfile::TempDir;
    use tokio::sync::Mutex as TokioMutex;

    struct ShareMockModelCaller;

    #[async_trait::async_trait]
    impl ModelCaller for ShareMockModelCaller {
        async fn call(&self, req: ModelCallRequest) -> roko_core::Result<ModelCallResponse> {
            assert_eq!(req.model, "share-mock-model");
            let role = req.role.as_deref().unwrap_or("unknown");
            let content = format!("mock response from {role}");
            Ok(ModelCallResponse {
                content,
                model: req.model,
                usage: TokenUsage {
                    input_tokens: 11,
                    output_tokens: 7,
                    total_tokens: 18,
                    cost_usd: 0.001,
                },
                stop_reason: Some("stop".to_string()),
                request_id: Some("share-mock-request".to_string()),
            })
        }
    }

    struct ShareMockPromptAssembler {
        assembled: TokioMutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl PromptAssembler for ShareMockPromptAssembler {
        async fn assemble(&self, spec: PromptSpec) -> roko_core::Result<String> {
            let role = spec.role.unwrap_or_else(|| "unknown".to_string());
            let task = spec.task.unwrap_or_else(|| "missing task".to_string());
            let prompt = format!("assembled prompt for {role}: {task}");
            self.assembled.lock().await.push(prompt.clone());
            Ok(prompt)
        }

        fn last_prompt_section_ids(&self) -> Vec<String> {
            vec!["share_test_section".to_string()]
        }

        fn last_knowledge_ids(&self) -> Vec<String> {
            vec!["share_test_knowledge".to_string()]
        }
    }

    struct ShareMockFeedbackSink {
        events: TokioMutex<Vec<FeedbackEvent>>,
        flushes: TokioMutex<u32>,
    }

    #[async_trait::async_trait]
    impl FeedbackSink for ShareMockFeedbackSink {
        async fn record(&self, event: FeedbackEvent) -> roko_core::Result<()> {
            self.events.lock().await.push(event);
            Ok(())
        }

        async fn flush(&self) -> roko_core::Result<()> {
            *self.flushes.lock().await += 1;
            Ok(())
        }
    }

    struct ShareMockGateRunner;

    #[async_trait::async_trait]
    impl GateRunner for ShareMockGateRunner {
        async fn run_gates(&self, config: WorkflowGateConfig) -> roko_core::Result<GateReport> {
            if config.enabled_gates.is_empty() {
                return Err(roko_core::RokoError::invalid(
                    "share test expected at least one configured gate",
                ));
            }

            Ok(GateReport {
                verdicts: config
                    .enabled_gates
                    .into_iter()
                    .map(|gate_name| GateVerdict {
                        gate_name,
                        passed: true,
                        skipped: false,
                        skip_reason: None,
                        output: "mock gate passed".to_string(),
                        duration_ms: 5,
                    })
                    .collect(),
            })
        }
    }

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
            usage: None,
        };
        assert!(r.overall_success());

        let r = RunReport {
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), false)],
            ..r
        };
        assert!(!r.overall_success());
    }

    #[test]
    fn run_report_first_failed_gate_returns_first_failure() {
        let r = RunReport {
            episode_id: "a".into(),
            prompt_id: "b".into(),
            agent_output_id: "c".into(),
            agent_success: true,
            gate_verdicts: vec![
                ("compile".into(), true),
                ("clippy".into(), false),
                ("test".into(), false),
            ],
            total_signals: 5,
            output_text: Some("done".into()),
            usage: None,
        };

        assert_eq!(r.first_failed_gate(), Some("clippy"));
    }

    #[cfg(feature = "legacy-orchestrate")]
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
            usage: None,
        };
        let token = write_shared_run(&tmp, &report).unwrap();
        assert!(
            tmp.join(".roko/shared")
                .join(format!("{token}.json"))
                .exists()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_v2_share_produces_real_transcript() {
        let tempdir = TempDir::new().expect("tempdir");
        init_git_workdir(tempdir.path());
        std::fs::write(
            tempdir.path().join("change.txt"),
            "share transcript change\n",
        )
        .expect("write test change");

        let prompt = "produce a share transcript with real data";
        let role = "implementer";
        let agent = "share-mock-provider";
        let prompt_assembler = Arc::new(ShareMockPromptAssembler {
            assembled: TokioMutex::new(Vec::new()),
        });
        let services = EffectServices {
            default_model: "share-mock-model".to_string(),
            model_caller: Arc::new(ShareMockModelCaller),
            prompt_assembler: prompt_assembler.clone(),
            feedback_sink: Arc::new(ShareMockFeedbackSink {
                events: TokioMutex::new(Vec::new()),
                flushes: TokioMutex::new(0),
            }),
            gate_runner: Arc::new(ShareMockGateRunner),
            affect_policy: None,
        };
        let engine = WorkflowEngine::new(services);
        let report = engine
            .run(WorkflowRunConfig {
                prompt: prompt.to_string(),
                workdir: tempdir.path().to_path_buf(),
                workflow: WorkflowConfig::express(),
                enabled_gates: vec!["compile".to_string()],
                shell_gates: Vec::new(),
                commit_prefix: Some("test".to_string()),
            })
            .await
            .expect("workflow run succeeds");

        let token = write_shared_workflow_run(tempdir.path(), prompt, agent, role, &report)
            .expect("shared transcript is written");
        let path = tempdir
            .path()
            .join(".roko")
            .join("shared")
            .join(format!("{token}.json"));
        let transcript: roko_serve::routes::shared_runs::RunTranscript =
            serde_json::from_str(&std::fs::read_to_string(path).expect("read transcript"))
                .expect("parse transcript");
        let assembled_prompts = prompt_assembler.assembled.lock().await;

        assert!(!transcript.agent.trim().is_empty());
        assert_ne!(transcript.agent, "unknown");
        assert_eq!(transcript.agent, agent);
        assert!(!transcript.role.trim().is_empty());
        assert_eq!(transcript.role, role);
        assert_eq!(
            assembled_prompts.first().map(String::as_str),
            Some("assembled prompt for implementer: produce a share transcript with real data")
        );
        assert_eq!(transcript.prompt, prompt);
        assert_eq!(transcript.model.as_deref(), Some("share-mock-model"));
        assert_eq!(
            transcript.output.as_deref(),
            Some("mock response from implementer")
        );
        assert!(transcript.success);
        assert_eq!(transcript.gates, vec![("compile".to_string(), true)]);
        assert_eq!(transcript.cost_usd, Some(0.001));
        assert_eq!(
            transcript.episode_id.as_deref(),
            Some(report.run_id.as_str())
        );
        assert!(report.events.iter().any(|event| matches!(
            event.payload,
            roko_core::RuntimeEvent::AgentSpawned { ref agent_id, ref role, ref model, .. }
                if !agent_id.trim().is_empty()
                    && role == "implementer"
                    && model == "share-mock-model"
        )));
    }

    #[test]
    fn share_requires_legacy_engine() {
        assert!(ensure_share_supported(true, true).is_ok());
        assert!(ensure_share_supported(true, false).is_ok());

        let err = ensure_share_supported(false, true).expect_err("v2 share should error");
        assert!(
            err.to_string()
                .contains("--share is not yet supported with the v2 engine")
        );
    }

    #[cfg(feature = "legacy-orchestrate")]
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

        assert!(!dashboard_agent_model(&cfg).trim().is_empty());

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

    #[test]
    fn workflow_config_from_band_maps_pipeline_fields() {
        let mechanical = roko_core::config::PipelineBandConfig {
            strategist: false,
            reviewers: false,
            reviewer_mode: roko_core::config::PipelineReviewerMode::Quick,
            max_iterations: 1,
        };
        let workflow = workflow_config_from_band(&mechanical);
        assert!(!workflow.has_strategy);
        assert!(!workflow.has_review);
        assert_eq!(workflow.max_iterations, 1);
        assert_eq!(workflow.max_autofix_attempts, 1);

        let architectural = roko_core::config::PipelineBandConfig {
            strategist: true,
            reviewers: true,
            reviewer_mode: roko_core::config::PipelineReviewerMode::Full,
            max_iterations: 3,
        };
        let workflow = workflow_config_from_band(&architectural);
        assert!(workflow.has_strategy);
        assert!(workflow.has_review);
        assert_eq!(workflow.max_iterations, 3);
        assert_eq!(workflow.max_autofix_attempts, 2);
    }

    #[test]
    fn workflow_report_outcome_reads_terminal_event() {
        let report = WorkflowRunReport {
            run_id: "run-1".to_string(),
            success: false,
            model: "test-model".to_string(),
            provider: None,
            prompt_summary: "prompt".to_string(),
            output: "output".to_string(),
            agent_turns: 0,
            token_usage: 0,
            cost: None,
            duration_secs: 0.0,
            gates: Vec::new(),
            events: vec![roko_core::runtime_event::RuntimeEventEnvelope::new(
                "run-1",
                1,
                "workflow_engine",
                roko_core::RuntimeEvent::WorkflowCompleted {
                    run_id: "run-1".to_string(),
                    outcome: roko_core::WorkflowOutcome::Halted {
                        reason: "missing API key".to_string(),
                    },
                },
            )],
            checkpoint_path: None,
        };

        assert!(matches!(
            workflow_report_outcome(&report),
            Some(roko_core::WorkflowOutcome::Halted { ref reason })
                if reason == "missing API key"
        ));
    }

    #[cfg(feature = "legacy-orchestrate")]
    #[tokio::test]
    async fn minimal_strategy_leaves_system_prompt_unmodified() {
        let tempdir = TempDir::new().expect("tempdir");
        let mut config = Config::default();
        config.prompt.role = "implementer".to_string();

        let base_prompt = build_system_prompt(&config, "Implement the feature.", "Read,Edit");
        let augmented = augment_system_prompt_for_strategy(
            base_prompt.clone(),
            tempdir.path(),
            &config.prompt.role,
            "Implement the feature.",
            "mock-model",
            Some(BenchStrategy::Minimal),
        )
        .await;

        assert_eq!(augmented.system_prompt, base_prompt);
        assert!(augmented.injected_playbook_ids.is_empty());
        assert!(skip_bench_enrichment(Some(BenchStrategy::Minimal)));
        assert!(!skip_bench_enrichment(None));
        assert!(!skip_bench_enrichment(Some(BenchStrategy::ContextEnriched)));
    }

    #[cfg(feature = "legacy-orchestrate")]
    #[tokio::test]
    async fn record_injected_playbook_outcomes_updates_bench_playbooks_only() {
        let tempdir = TempDir::new().expect("tempdir");
        let playbook_root = tempdir.path().join(".roko").join("learn").join("playbooks");
        let store = load_or_create_playbook_store(&playbook_root)
            .await
            .expect("playbook store");
        let playbook = Playbook::new("pb-1", "Audit dependencies");
        store.save(&playbook).await.expect("save playbook");
        let injected_ids = vec!["pb-1".to_string()];

        record_injected_playbook_outcomes(
            tempdir.path(),
            Some(BenchStrategy::ContextEnriched),
            &injected_ids,
            true,
        )
        .await;
        let loaded = store.load("pb-1").await.expect("load").expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 0);

        record_injected_playbook_outcomes(
            tempdir.path(),
            Some(BenchStrategy::NeuroAugmented),
            &injected_ids,
            false,
        )
        .await;
        let loaded = store.load("pb-1").await.expect("load").expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 1);

        record_injected_playbook_outcomes(
            tempdir.path(),
            Some(BenchStrategy::Minimal),
            &injected_ids,
            false,
        )
        .await;
        let loaded = store.load("pb-1").await.expect("load").expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 1);

        record_injected_playbook_outcomes(tempdir.path(), None, &injected_ids, false).await;
        let loaded = store.load("pb-1").await.expect("load").expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 1);

        let empty_ids: Vec<String> = Vec::new();
        record_injected_playbook_outcomes(
            tempdir.path(),
            Some(BenchStrategy::FullCascade),
            &empty_ids,
            false,
        )
        .await;
        let loaded = store.load("pb-1").await.expect("load").expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 1);
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

        let result = dispatch_agent(
            tempdir.path(),
            &config,
            &prompt,
            "plain-exec-ok",
            &Context::now(),
            None,
        )
        .await
        .expect("dispatch succeeds");

        assert!(result.agent_result.success);
        assert_eq!(
            result.agent_result.output.body.as_text().unwrap_or(""),
            "plain-exec-ok"
        );
        assert!(result.external_actions.is_empty());
        assert!(result.injected_playbook_ids.is_empty());
    }

    fn init_git_workdir(workdir: &std::path::Path) {
        run_git(workdir, &["init"]);
        run_git(workdir, &["config", "user.email", "test@example.com"]);
        run_git(workdir, &["config", "user.name", "Roko Test"]);
    }

    fn run_git(workdir: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(workdir)
            .output()
            .expect("run git command");

        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
