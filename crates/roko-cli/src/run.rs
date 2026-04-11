//! The universal loop: prompt → compose → agent → gate → persist → policy.
//!
//! This is the body of `roko run <prompt>`. It reads [`Config`], opens a
//! [`FileSubstrate`] under `.roko/`, seeds prompt sections, composes them
//! into a single Prompt signal, invokes the configured `ExecAgent`, runs
//! each configured gate on the working directory, and emits an Episode.

use crate::clean;
use crate::config::{Config, GateConfig, PromptFile};
use crate::episode::EpisodePolicy;
use anyhow::{Context as _, Result, anyhow};
use chrono::Utc;
use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_agent::translate::{ClaudeTranslator, OllamaTranslator, RenderedTools, Translator};
use roko_agent::{Agent, AgentResult, ClaudeCliAgent, OllamaLlmBackend};
use roko_compose::{
    Placement, PromptComposer, PromptSection, RoleSystemPromptSpec, SectionPriority, TaskContext,
};
use roko_core::agent::resolve_model;
use roko_core::metric::{ConfigHash, TaskMetric};
use roko_core::tool::ExternalAction;
use roko_core::tool::ToolRegistry;
use roko_core::{
    AgentRole, Body, Budget, Composer, Context, Gate, Kind, Provenance, Signal, Substrate, Verdict,
};
use roko_fs::FileSubstrate;
use roko_gate::{BuildSystem, ClippyGate, CompileGate, GatePayload, ShellGate, TestGate};
use roko_learn::episode_logger::{Episode, GateVerdict, Usage as EpisodeUsage};
use roko_learn::runtime_feedback::{CompletedRunInput, LearningRuntime};
use roko_std::NoOpScorer;
use roko_std::StaticToolRegistry;
use std::path::{Path, PathBuf};

/// Summary of a single `run` invocation.
#[derive(Debug, Clone)]
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
}

impl RunReport {
    /// True if the agent succeeded and every configured gate passed.
    #[must_use]
    pub fn overall_success(&self) -> bool {
        self.agent_success && self.gate_verdicts.iter().all(|(_, ok)| *ok)
    }
}

/// Run the universal loop once for `prompt_text` under `workdir`.
///
/// - Opens (or creates) `workdir/.roko/signals.jsonl`.
/// - Seeds a role + task `PromptSection`, composes them under the config's budget.
/// - Invokes the configured `ExecAgent`.
/// - Runs every gate in the config in declaration order; each gate sees the
///   same `GatePayload` pointing at `workdir`.
/// - Records an Episode signal and persists everything.
#[allow(clippy::too_many_lines)]
pub async fn run_once(workdir: &Path, config: &Config, prompt_text: &str) -> Result<RunReport> {
    let substrate_dir = workdir.join(".roko");
    let substrate = FileSubstrate::open(substrate_dir)
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;

    let ctx = Context::now();

    // Seed prompt sections: system role + user prompt + any injected files.
    let mut sections: Vec<Signal> = Vec::with_capacity(2 + config.prompt.files.len());

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

    // Run the agent.
    // ClaudeCliAgent owns `--append-system-prompt`, `--tools`, and `--settings`
    // internally; ExecAgent stays available for non-Claude backends.
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

    // Run every configured gate against the working dir.
    let gate_input = build_gate_input(workdir, final_output_sig.id)?;
    substrate
        .put(gate_input.clone())
        .await
        .map_err(|e| anyhow!("persist gate input: {e}"))?;

    let mut verdict_sigs: Vec<Signal> = Vec::new();
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
    })
}

fn build_system_prompt(role: &str, prompt_text: &str, tools_csv: &str) -> String {
    let workspace = "Single-shot execution through `roko run`.";
    parse_agent_role(role).map_or_else(
        || {
            let task_context = TaskContext::new(prompt_text)
                .with_workspace(workspace)
                .with_domain_notes(format!("User-configured role text: {role}"));
            RoleSystemPromptSpec::new(AgentRole::Implementer, task_context, tools_csv)
                .with_extra_conventions(format!("Treat the configured role hint literally: {role}"))
                .build()
        },
        |agent_role| {
            let task_context = TaskContext::new(prompt_text).with_workspace(workspace);
            RoleSystemPromptSpec::new(agent_role, task_context, tools_csv).build()
        },
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
    prompt: &Signal,
    prompt_text: &str,
    ctx: &Context,
) -> Result<(AgentResult, Vec<ExternalAction>)> {
    let mut routing_config = roko_core::config::load_config(workdir)
        .with_context(|| format!("load routing config from {}", workdir.display()))?;
    routing_config.apply_process_env();
    let has_routing = !routing_config.providers.is_empty() || !routing_config.models.is_empty();

    if has_routing {
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let system_prompt = build_system_prompt(&config.prompt.role, prompt_text, &tools_csv);
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| routing_config.agent.default_model.clone());
        let resolved = resolve_model(&routing_config, &model);
        let agent = create_agent_for_model(
            &routing_config,
            &model,
            AgentOptions {
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: Some(system_prompt),
                tools: Some(tools_csv),
                mcp_config: config.agent.mcp_config.clone(),
                env: config.agent.env.clone(),
                extra_args: config.agent.args.clone(),
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: role_allows_dangerous_skip_permissions(
                    &config.prompt.role,
                ),
                name: format!("{}:{model}", resolved.provider_kind.label()),
            },
        )
        .with_context(|| format!("create agent for model {model}"))?;
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    } else if config.agent.command == "claude" {
        // ClaudeCliAgent owns `--append-system-prompt`, `--tools`, and `--settings`
        // internally; ExecAgent stays available for non-Claude backends.
        let tools_csv = claude_tool_allowlist(&config.prompt.role);
        let system_prompt = build_system_prompt(&config.prompt.role, prompt_text, &tools_csv);
        let (extra_args, resume_from_args) = split_resume_arg(&config.agent.args);
        let optional_resume = optional_resume_session_id(config, resume_from_args);
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| "claude-opus-4-6".to_string());
        let mut agent = ClaudeCliAgent::new(&config.agent.command, workdir, model.clone())
            .with_timeout_ms(config.agent.timeout_ms)
            .with_bare_mode(config.agent.bare_mode)
            .with_effort(config.agent.effort.clone())
            .with_system_prompt(system_prompt)
            .with_tools(tools_csv)
            .with_settings_json(roko_agent::claude_cli_agent::build_settings_json())
            .with_extra_args(extra_args)
            .with_dangerously_skip_permissions(role_allows_dangerous_skip_permissions(
                &config.prompt.role,
            ))
            .with_optional_resume(optional_resume);
        if let Some(fallback_model) = &config.agent.fallback_model {
            agent = agent.with_fallback_model(fallback_model.clone());
        }
        for (k, v) in &config.agent.env {
            agent = agent.with_env_var(k, v);
        }
        Ok((agent.run(prompt, ctx).await, Vec::new()))
    } else if config.agent.command == "ollama" {
        Ok(run_ollama_agentic_single(workdir, config, prompt_text).await)
    } else {
        let model = config
            .agent
            .model
            .clone()
            .unwrap_or_else(|| routing_config.agent.default_model.clone());
        let resolved = resolve_model(&routing_config, &model);
        let agent = create_agent_for_model(
            &routing_config,
            &model,
            AgentOptions {
                timeout_ms: Some(config.agent.timeout_ms),
                system_prompt: None,
                tools: None,
                mcp_config: config.agent.mcp_config.clone(),
                env: config.agent.env.clone(),
                extra_args: config.agent.args.clone(),
                effort: Some(config.agent.effort.clone()),
                bare_mode: config.agent.bare_mode,
                dangerously_skip_permissions: role_allows_dangerous_skip_permissions(
                    &config.prompt.role,
                ),
                name: format!("{}:{model}", resolved.provider_kind.label()),
            },
        )
        .with_context(|| format!("create agent for model {model}"))?;
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

    let system_prompt = build_system_prompt(&config.prompt.role, prompt_text, "");
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

    let sig = Signal::builder(Kind::AgentOutput)
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

async fn append_episode_log(
    workdir: &Path,
    config: &Config,
    prompt: &Signal,
    final_output: &Signal,
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

    let mut runtime = LearningRuntime::open_under(workdir.join(".roko").join("memory"))
        .await
        .map_err(|e| anyhow!("open learning runtime: {e}"))?;
    let distillation_workdir = workdir.to_path_buf();
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode);
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
    prompt: &Signal,
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

fn final_output_run_id(prompt: &Signal, agent_result: &AgentResult) -> String {
    agent_result
        .output
        .tag("run_id")
        .map_or_else(|| prompt.id.to_hex(), str::to_string)
}

fn resolved_model(config: &Config) -> String {
    if let Some(model) = &config.agent.model {
        return model.clone();
    }
    if config.agent.command.eq_ignore_ascii_case("claude") {
        "claude-opus-4-6".to_string()
    } else {
        "unknown-model".to_string()
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

fn load_file_section(workdir: &Path, spec: &PromptFile) -> Result<Signal> {
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
    prompt: &Signal,
    agent_result: &AgentResult,
    substrate: &FileSubstrate,
) -> Result<Signal> {
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

fn build_gate_input(workdir: &Path, parent_id: roko_core::ContentHash) -> Result<Signal> {
    let working_dir: PathBuf = workdir
        .canonicalize()
        .with_context(|| format!("canonicalize workdir {}", workdir.display()))?;
    let payload = GatePayload::in_dir(working_dir).with_label("roko-cli");
    let body = Body::from_json(&payload).map_err(|e| anyhow!("encode gate payload: {e}"))?;
    Ok(Signal::builder(Kind::Task)
        .body(body)
        .provenance(Provenance::trusted("cli_run"))
        .lineage([parent_id])
        .build())
}

async fn run_gate(cfg: &GateConfig, input: &Signal, ctx: &Context) -> Verdict {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert!(r.overall_success());

        let r = RunReport {
            gate_verdicts: vec![("g1".into(), true), ("g2".into(), false)],
            ..r
        };
        assert!(!r.overall_success());
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
}
