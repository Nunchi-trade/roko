//! [`CliRuntime`] implementation backed by the real CLI internals.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use async_trait::async_trait;
use roko_core::config::schema::RokoConfig;
use roko_fs::RokoLayout;
use roko_learn::playbook::PlaybookStore;
use roko_neuro::KnowledgeStore;
use roko_serve::bench::{BenchConfigOverrides, BenchStrategy};
use roko_serve::runtime::{
    CliRuntime, DashboardInfo, PlanExecutionResult, PlanGenerationResult, RepoInfo, RunResult,
    RunResultUsage, RuntimeGateResult, SessionStatusInfo,
};

use crate::config::{Config, RepoRegistry};
use crate::prd;
use crate::runner::types::{GateCompletionKind, RunnerEvent};
use crate::state_hub::SharedStateHub;
use crate::status::collect_session_status;
use crate::tui::DashboardScaffold;
use crate::workspace_paths;

/// Concrete runtime that delegates to the real CLI functions.
pub struct RokoCliRuntime {
    config: Config,
    repo_registry: RepoRegistry,
    state_hub: SharedStateHub,
    // Lazily bound to the workspace used by the first bench task that needs it.
    knowledge_store: OnceLock<KnowledgeStore>,
    // Lazily bound to the workspace used by the first bench task that earns a playbook.
    playbook_store: OnceLock<PlaybookStore>,
}

impl RokoCliRuntime {
    #[must_use]
    pub fn new(config: Config, repo_registry: RepoRegistry) -> Self {
        Self::new_with_state_hub(config, repo_registry, SharedStateHub::new_in_process())
    }

    #[must_use]
    pub fn new_with_state_hub(
        config: Config,
        repo_registry: RepoRegistry,
        state_hub: SharedStateHub,
    ) -> Self {
        Self {
            config,
            repo_registry,
            state_hub,
            knowledge_store: OnceLock::new(),
            playbook_store: OnceLock::new(),
        }
    }

    pub fn into_arc(self) -> Arc<dyn CliRuntime> {
        Arc::new(self)
    }
}

#[async_trait]
impl CliRuntime for RokoCliRuntime {
    async fn run_once(&self, workdir: &Path, prompt: &str) -> anyhow::Result<RunResult> {
        let result = dispatch_bench_prompt(workdir, &self.config, prompt, None).await?;
        Ok(RunResult {
            success: true,
            output_text: Some(result.text),
            usage: Some(RunResultUsage {
                input_tokens: result.input_tokens,
                output_tokens: result.output_tokens,
            }),
            gate_results: Vec::new(),
        })
    }

    async fn run_once_with_config(
        &self,
        workdir: &Path,
        prompt: &str,
        overrides: &BenchConfigOverrides,
    ) -> anyhow::Result<RunResult> {
        // Apply model override if provided by cloning the config.
        let mut config = self.config.clone();
        if let Some(ref model) = overrides.model {
            config.agent.model = Some(model.clone());
        }

        let model_override = overrides.model.clone();
        let result = dispatch_bench_prompt(workdir, &config, prompt, model_override.as_deref())
            .await;

        match result {
            Ok(dispatch) => {
                let output_text = Some(dispatch.text);

                // Extract playbook on success (skip for Minimal strategy).
                if !matches!(overrides.strategy, BenchStrategy::Minimal) {
                    match crate::run::extract_bench_playbook(
                        workdir,
                        prompt,
                        output_text.as_deref(),
                    )
                    .await
                    {
                        Ok(Some(playbook)) => {
                            let playbook_store = self.playbook_store(workdir);
                            if let Err(err) = playbook_store.save_or_merge(&playbook).await {
                                tracing::warn!(
                                    error = %err,
                                    playbook_id = %playbook.id,
                                    "failed to save extracted playbook"
                                );
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            tracing::warn!(error = %err, "failed to extract bench playbook");
                        }
                    }
                }

                Ok(RunResult {
                    success: true,
                    output_text,
                    usage: Some(RunResultUsage {
                        input_tokens: dispatch.input_tokens,
                        output_tokens: dispatch.output_tokens,
                    }),
                    gate_results: Vec::new(),
                })
            }
            Err(err) => {
                let error_msg = format!("{err:#}");

                // Record anti-pattern on failure (skip for Minimal strategy).
                if !matches!(overrides.strategy, BenchStrategy::Minimal) {
                    let knowledge_store = self.knowledge_store(workdir);
                    if let Err(record_err) = knowledge_store.record_anti_pattern_from_failure(
                        "bench-dispatch",
                        prompt,
                        "model_call",
                        &error_msg,
                        Some(&error_msg),
                    ) {
                        tracing::warn!(
                            error = %record_err,
                            "failed to save anti-pattern from bench dispatch failure"
                        );
                    }
                }

                Ok(RunResult {
                    success: false,
                    output_text: Some(error_msg),
                    usage: None,
                    gate_results: Vec::new(),
                })
            }
        }
    }

    async fn generate_plan_from_prd(
        &self,
        workdir: &Path,
        slug: &str,
        prd_path: &Path,
    ) -> anyhow::Result<PlanGenerationResult> {
        let plans_root = workspace_paths::plans_dir(workdir);
        let before = snapshot_plan_artifacts(&plans_root);
        let generated_root = prd::generate_plan_from_prd(slug, prd_path, false).await?;
        let after = snapshot_plan_artifacts(&generated_root);

        let mut plan_targets = changed_plan_targets(&generated_root, &before, &after);
        if plan_targets.is_empty() {
            plan_targets.push(generated_root.clone());
        }

        Ok(PlanGenerationResult {
            plans_root: generated_root,
            artifacts: plan_artifact_paths(&plan_targets),
            plan_targets,
        })
    }

    async fn run_plan(
        &self,
        workdir: &Path,
        plan_target: &Path,
    ) -> anyhow::Result<PlanExecutionResult> {
        let workdir = workdir.to_path_buf();
        let plan_target = plan_target.to_path_buf();
        let config = self.config.clone();
        let repo_registry = self.repo_registry.clone();
        let state_hub = self.state_hub.clone();
        tokio::task::spawn_blocking(move || {
            run_plan_on_local_runtime(workdir, plan_target, config, repo_registry, state_hub)
        })
        .await
        .map_err(|err| anyhow::anyhow!("plan execution worker failed: {err}"))?
    }

    fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
        let ss = collect_session_status(&workdir);
        SessionStatusInfo {
            session_id: ss.session_id,
            workdir: ss.workdir,
            daemon_running: ss.daemon_running,
            signal_count: ss.signal_count,
            episode_count: ss.episode_count,
            last_episode_passed: ss.last_episode_passed,
        }
    }

    fn dashboard_scaffold(&self, workdir: &Path) -> DashboardInfo {
        let scaffold = DashboardScaffold::new_in(workdir);
        DashboardInfo {
            rendered: scaffold.render_overview_text(),
        }
    }

    fn resolve_repo_workdir(&self, repo_full_name: &str) -> Option<PathBuf> {
        self.repo_registry
            .find_by_full_name(repo_full_name)
            .map(|entry| entry.root.clone())
    }

    fn repo_roko_config(&self, repo_name: &str) -> Option<RokoConfig> {
        self.repo_registry
            .get(repo_name)
            .and_then(|entry| entry.roko_config.clone())
    }

    fn list_repos(&self) -> Vec<RepoInfo> {
        self.repo_registry
            .repos()
            .iter()
            .map(|entry| RepoInfo {
                name: entry.config.name.clone(),
                path: entry.root.clone(),
                branch: entry.config.branch.clone(),
            })
            .collect()
    }
}

impl RokoCliRuntime {
    fn knowledge_store(&self, workdir: &Path) -> &KnowledgeStore {
        // `run_once_with_config` is the only bench entry point, so this store can
        // be initialized on demand from the workspace passed into that call.
        self.knowledge_store
            .get_or_init(|| KnowledgeStore::for_workdir(workdir))
    }

    fn playbook_store(&self, workdir: &Path) -> &PlaybookStore {
        // `run_once_with_config` is the only bench entry point that creates
        // playbooks, so this store can be initialized on demand from the same
        // workspace.
        self.playbook_store
            .get_or_init(|| PlaybookStore::new(RokoLayout::for_project(workdir).playbooks_dir()))
    }
}

fn run_plan_on_local_runtime(
    workdir: PathBuf,
    plan_target: PathBuf,
    config: Config,
    repo_registry: RepoRegistry,
    state_hub: SharedStateHub,
) -> anyhow::Result<PlanExecutionResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, async move {
        let execution_root = prepare_plan_execution_root(&workdir, &plan_target)?;
        ensure_git_repo_for_runner(&workdir);

        let plans = crate::runner::plan_loader::load_plans(&execution_root)?;
        let plan_ids = plans
            .iter()
            .map(|plan| plan.id.clone())
            .collect::<BTreeSet<_>>();
        let roko_config = load_effective_roko_config(&workdir, &repo_registry)?;
        let run_config = build_runner_config(&workdir, &execution_root, &config, roko_config);
        let events_offset = runner_events_offset(&workdir);
        let cancel = tokio_util::sync::CancellationToken::new();

        let report = crate::runner::run(plans, &run_config, &state_hub, cancel).await?;
        let gate_results = collect_runner_gate_results(&workdir, events_offset, &plan_ids)
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "failed to collect runner gate evidence");
                Vec::new()
            });
        let output_text = Some(render_plan_execution_summary(&report, gate_results.len()));

        Ok(PlanExecutionResult {
            success: report.all_succeeded(),
            output_text,
            gate_results,
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlanArtifactSnapshot {
    modified: Option<SystemTime>,
    len: u64,
}

fn snapshot_plan_artifacts(root: &Path) -> BTreeMap<PathBuf, PlanArtifactSnapshot> {
    let mut out = BTreeMap::new();
    collect_plan_artifact_snapshots(root, root, &mut out);
    out
}

fn collect_plan_artifact_snapshots(
    root: &Path,
    dir: &Path,
    out: &mut BTreeMap<PathBuf, PlanArtifactSnapshot>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_plan_artifact_snapshots(root, &path, out);
            continue;
        }
        if !meta.is_file() || !is_plan_artifact_file(&path) {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        out.insert(
            rel,
            PlanArtifactSnapshot {
                modified: meta.modified().ok(),
                len: meta.len(),
            },
        );
    }
}

fn is_plan_artifact_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "tasks.toml" || name == "plan.md" || path.extension().is_some_and(|ext| ext == "md")
}

fn changed_plan_targets(
    root: &Path,
    before: &BTreeMap<PathBuf, PlanArtifactSnapshot>,
    after: &BTreeMap<PathBuf, PlanArtifactSnapshot>,
) -> Vec<PathBuf> {
    let mut targets = BTreeSet::new();
    for (rel, snapshot) in after {
        let changed = before.get(rel).is_none_or(|old| old != snapshot);
        if !changed {
            continue;
        }
        if rel
            .file_name()
            .is_some_and(|name| name == "tasks.toml" || name == "plan.md")
        {
            if let Some(parent) = rel.parent() {
                targets.insert(root.join(parent));
            }
        } else {
            targets.insert(root.join(rel));
        }
    }
    targets.into_iter().collect()
}

fn plan_artifact_paths(targets: &[PathBuf]) -> Vec<PathBuf> {
    let mut artifacts = BTreeSet::new();
    for target in targets {
        if target.extension().is_some_and(|ext| ext == "md") {
            artifacts.insert(target.clone());
            continue;
        }
        artifacts.insert(target.join("plan.md"));
        artifacts.insert(target.join("tasks.toml"));
    }
    artifacts.into_iter().collect()
}

fn prepare_plan_execution_root(workdir: &Path, plan_target: &Path) -> anyhow::Result<PathBuf> {
    let absolute_target = if plan_target.is_absolute() {
        plan_target.to_path_buf()
    } else {
        workdir.join(plan_target)
    };

    if absolute_target.is_dir() && !absolute_target.join("tasks.toml").is_file() {
        return Ok(absolute_target);
    }

    let copy_source = if absolute_target.is_file() {
        let parent = absolute_target.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "plan target has no parent directory: {}",
                absolute_target.display()
            )
        })?;
        if !parent.join("tasks.toml").is_file() {
            anyhow::bail!(
                "runner v2 requires a tasks.toml plan directory; file target is not inside one: {}",
                absolute_target.display()
            );
        }
        parent.to_path_buf()
    } else if absolute_target.is_dir() {
        absolute_target.clone()
    } else {
        anyhow::bail!("plan target does not exist: {}", absolute_target.display());
    };

    let plan_base = copy_source
        .file_stem()
        .or_else(|| copy_source.file_name())
        .and_then(|name| name.to_str())
        .map(sanitize_plan_base)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "job-plan".to_string());
    let run_root = RokoLayout::for_project(workdir)
        .plan_runs_dir()
        .join(format!("{plan_base}-{}", unique_suffix()));
    std::fs::create_dir_all(&run_root)?;

    copy_dir_recursive(&copy_source, &run_root.join(&plan_base))?;

    Ok(run_root)
}

fn ensure_git_repo_for_runner(workdir: &Path) {
    if workdir.join(".git").exists() {
        return;
    }

    tracing::info!(workdir = %workdir.display(), "initializing git repo for runner tooling");
    for args in [
        &["init"][..],
        &["add", "-A"][..],
        &[
            "commit",
            "-m",
            "init (auto-created by roko)",
            "--allow-empty",
        ][..],
    ] {
        let _ = std::process::Command::new("git")
            .args(args)
            .current_dir(workdir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn load_effective_roko_config(
    workdir: &Path,
    repo_registry: &RepoRegistry,
) -> anyhow::Result<RokoConfig> {
    let mut config = if let Some(config) = load_roko_config_file(&workdir.join("roko.toml"))? {
        config
    } else if let Some(config) = repo_roko_config_for_workdir(workdir, repo_registry) {
        config
    } else {
        load_roko_config_file(&RokoLayout::for_project(workdir).roko_toml_path())?
            .unwrap_or_default()
    };

    roko_core::config::loader::merge_global_into(&mut config);
    config.apply_process_env();
    Ok(config)
}

fn load_roko_config_file(path: &Path) -> anyhow::Result<Option<RokoConfig>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}", path.display())));
        }
    };
    let config =
        RokoConfig::from_toml(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(config))
}

fn repo_roko_config_for_workdir(
    workdir: &Path,
    repo_registry: &RepoRegistry,
) -> Option<RokoConfig> {
    let canonical_workdir = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    repo_registry
        .repos()
        .iter()
        .find(|entry| canonical_workdir == entry.root || canonical_workdir.starts_with(&entry.root))
        .and_then(|entry| entry.roko_config.clone())
}

fn build_runner_config(
    workdir: &Path,
    plan_dir: &Path,
    cli_config: &Config,
    roko_config: RokoConfig,
) -> crate::runner::RunConfig {
    let model = non_empty_string(&roko_config.agent.default_model)
        .or_else(|| cli_config.agent.model.clone())
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
    let claude_program = roko_config
        .agent
        .command
        .as_deref()
        .and_then(non_empty_string)
        .or_else(|| non_empty_string(&cli_config.agent.command))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("claude"));
    let max_concurrent_tasks = roko_config
        .runner
        .max_concurrent_tasks
        .or_else(|| {
            (cli_config.executor.max_concurrent_tasks
                != roko_orchestrator::ExecutorConfig::default().max_concurrent_tasks)
                .then_some(cli_config.executor.max_concurrent_tasks)
        })
        .unwrap_or(4)
        .max(1);

    // Initialize Phase 0 subsystems.
    let layout = RokoLayout::for_project(workdir);
    let router_path = layout.cascade_router_path();
    let model_slugs = vec![model.clone(), "claude-haiku-4-5".to_string()];
    let cascade_router = Arc::new(roko_learn::cascade_router::CascadeRouter::load_or_new(
        &router_path,
        model_slugs,
    ));
    let extension_chain = Arc::new(tokio::sync::Mutex::new(
        roko_core::extension::ExtensionChain::new(),
    ));
    let connector_registry = Arc::new(std::sync::Mutex::new(roko_core::ConnectorRegistry::new()));
    let feed_registry = Arc::new(std::sync::Mutex::new(roko_core::FeedRegistry::new()));
    let run_uuid = uuid::Uuid::new_v4().to_string();
    let projection = Arc::new(crate::runner::projection::Projection::new(run_uuid));
    let episodes_path = layout.root_episodes_path();
    let knowledge_path = layout
        .learn_dir()
        .join(roko_neuro::admission::DEFAULT_KNOWLEDGE_CANDIDATES_FILE);
    let _ = std::fs::create_dir_all(layout.learn_dir());
    let feedback_facade = Arc::new(
        crate::runtime_feedback::FeedbackFacade::new()
            .with_sink(Arc::new(crate::runtime_feedback::EpisodeSink::at(
                &episodes_path,
            )))
            .with_sink(Arc::new(
                crate::runtime_feedback::RoutingObservationSink::new(cascade_router.clone()),
            ))
            .with_sink(Arc::new(
                crate::runtime_feedback::KnowledgeIngestionSink::at(&knowledge_path).with_ingestor(
                    Arc::new(crate::runtime_feedback::NeuroKnowledgeIngestor::new(
                        KnowledgeStore::for_workdir(workdir),
                    )),
                ),
            )),
    );

    crate::runner::RunConfig {
        layout,
        workdir: workdir.to_path_buf(),
        plan_dir: plan_dir.to_path_buf(),
        model,
        cli_model_override: None,
        timeout_secs: roko_config.timeouts.agent_dispatch_secs,
        plan_timeout_secs: roko_config.timeouts.plan_total_secs,
        max_retries: cli_config.executor.max_auto_fix_iterations,
        max_concurrent_tasks,
        gate_concurrency: max_concurrent_tasks,
        approval: false,
        dangerously_skip_permissions: roko_config.runner.dangerously_skip_permissions,
        force_resume: false,
        mcp_config: cli_config.agent.mcp_config.clone(),
        resume_session: None,
        max_gate_rung: if roko_config.gates.skip_tests { 1 } else { 2 },
        claude_program,
        max_plan_usd: f64::from(roko_config.budget.max_plan_usd),
        max_turn_usd: f64::from(roko_config.budget.max_turn_usd),
        clippy_enabled: roko_config.gates.clippy_enabled,
        skip_tests: roko_config.gates.skip_tests,
        roko_config: Some(Arc::new(roko_config)),
        extension_chain: Some(extension_chain),
        cascade_router: Some(cascade_router),
        daimon_state: Some(crate::runner::RunConfig::daimon_state_with_strategy(
            workdir,
            cli_config.daimon.strategy_space.clone(),
        )),
        connector_registry: Some(connector_registry),
        feed_registry: Some(feed_registry),
        feedback_facade: Some(feedback_facade),
        projection: Some(projection),
        http_event_sink: None,
        stream_to_stderr: false,
        warm_cache: true,
    }
}

/// Dispatch a bench prompt via the v2 `ModelCallService` path.
///
/// This replaces the legacy `run_once()` which is feature-gated behind
/// `legacy-orchestrate`. The v2 path uses the same ModelCallService that
/// `WorkflowEngine` uses, preserving routing, budget, and feedback behavior.
async fn dispatch_bench_prompt(
    workdir: &Path,
    config: &Config,
    prompt: &str,
    model_override: Option<&str>,
) -> anyhow::Result<BenchDispatchResult> {
    use crate::learning_helpers::{
        capture_runtime_model_slugs, provider_id_for_model, record_persisted_provider_health,
    };
    use roko_agent::model_call_service::ModelCallService;
    use roko_core::agent::resolve_model;
    use roko_core::config::schema::RokoConfig;
    use roko_core::foundation::{
        ChatMessage, FeedbackSink, MessageRole, ModelCallRequest, ModelCaller, caller,
    };
    use roko_learn::cascade_router::CascadeRouter;
    use roko_learn::feedback_service::FeedbackService;

    // Build a RokoConfig from CLI config (same pattern as dispatch_v2.rs).
    let mut model_config = RokoConfig::default();
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
    if let Some(ref model) = model_override.map(ToString::to_string).or_else(|| config.agent.model.clone()) {
        model_config.agent.default_model = model.clone();
    }

    let model_key = model_override
        .map(ToString::to_string)
        .or_else(|| config.agent.model.clone())
        .unwrap_or_else(|| model_config.agent.default_model.clone());
    let model = resolve_model(&model_config, &model_key).slug;

    // Set up cascade router for learning.
    let cascade_path = workdir
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let cascade_model_slugs = capture_runtime_model_slugs(&model_config, &model);
    let cascade_router = (!cascade_model_slugs.is_empty()).then(|| {
        Arc::new(CascadeRouter::load_or_new(
            &cascade_path,
            cascade_model_slugs,
        ))
    });

    // Build feedback sink.
    let feedback_service = FeedbackService::from_roko_dir(&workdir.join(".roko"));
    let feedback_sink: Arc<dyn FeedbackSink> = match &cascade_router {
        Some(router) => Arc::new(feedback_service.with_cascade_router(Arc::clone(router))),
        None => Arc::new(feedback_service),
    };

    // Build and call ModelCallService.
    let cost_table = roko_agent::CostTable::from_config_with_defaults(&model_config.models);
    let mut service = ModelCallService::new(model.clone())
        .with_config(model_config.clone())
        .with_cost_table(cost_table)
        .with_feedback_sink(feedback_sink)
        .with_inference_observer(Arc::new(
            crate::inference_observer::RuntimeEventInferenceObserver::new(),
        ));
    if let Some(ref mcp_path) = config.agent.mcp_config {
        service = service.with_mcp_config(mcp_path.clone());
    }

    let request = ModelCallRequest {
        model: model.clone(),
        system: None,
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: prompt.to_string(),
        }],
        max_tokens: None,
        caller: Some(caller::CLI.to_string()),
        ..Default::default()
    };

    let call_result = service.call(request).await;

    // Persist cascade router observations.
    if let Some(router) = &cascade_router
        && let Err(err) = router.save(&cascade_path)
    {
        tracing::warn!(
            path = %cascade_path.display(),
            error = %err,
            "failed to persist bench cascade observation"
        );
    }

    let response = match call_result {
        Ok(response) => {
            if let Some(provider) = provider_id_for_model(&model_config, &response.model) {
                let _ = record_persisted_provider_health(workdir, &provider, true);
            }
            response
        }
        Err(err) => {
            if let Some(provider) = provider_id_for_model(&model_config, &model) {
                let _ = record_persisted_provider_health(workdir, &provider, false);
            }
            return Err(err).context("ModelCallService bench dispatch failed");
        }
    };

    Ok(BenchDispatchResult {
        text: response.content,
        model: response.model,
        input_tokens: response.usage.input_tokens,
        output_tokens: response.usage.output_tokens,
    })
}

/// Result from dispatching a bench prompt via `ModelCallService`.
struct BenchDispatchResult {
    text: String,
    #[allow(dead_code)]
    model: String,
    input_tokens: u64,
    output_tokens: u64,
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn runner_events_offset(workdir: &Path) -> u64 {
    std::fs::metadata(runner_events_path(workdir))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn runner_events_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir).events_jsonl_path()
}

fn collect_runner_gate_results(
    workdir: &Path,
    offset: u64,
    plan_ids: &BTreeSet<String>,
) -> anyhow::Result<Vec<RuntimeGateResult>> {
    let events = read_runner_events_since(&runner_events_path(workdir), offset)?;
    if events.trim().is_empty() {
        return Ok(Vec::new());
    }

    let parsed = events
        .lines()
        .filter_map(|line| {
            serde_json::from_str::<RunnerEvent>(line)
                .map_err(|err| {
                    tracing::debug!(error = %err, "skipping malformed runner event");
                    err
                })
                .ok()
        })
        .collect::<Vec<_>>();
    let run_ids = matching_run_ids(&parsed, plan_ids);
    let mut final_results = BTreeMap::new();

    for event in parsed {
        let RunnerEvent::GateCompleted {
            run_id,
            attempt,
            kind,
            rung,
            passed,
            duration_ms,
            output,
            verdicts,
            ..
        } = event
        else {
            continue;
        };

        if !run_ids.is_empty() && !run_ids.contains(&run_id) {
            continue;
        }
        if !plan_ids.contains(&attempt.plan_id) {
            continue;
        }

        let kind_label = gate_kind_label(kind);
        if verdicts.is_empty() {
            let gate_name = "gate".to_string();
            let key = gate_evidence_key(
                &attempt.plan_id,
                &attempt.task_id,
                kind_label,
                rung,
                &gate_name,
            );
            final_results.insert(
                key,
                RuntimeGateResult {
                    gate: gate_evidence_label(
                        &attempt.plan_id,
                        &attempt.task_id,
                        kind_label,
                        rung,
                        &gate_name,
                    ),
                    passed,
                    detail: gate_detail(
                        kind_label,
                        attempt.attempt,
                        rung,
                        duration_ms,
                        None,
                        None,
                        &output,
                    ),
                },
            );
            continue;
        }

        for verdict in verdicts {
            let key = gate_evidence_key(
                &attempt.plan_id,
                &attempt.task_id,
                kind_label,
                rung,
                &verdict.gate_name,
            );
            final_results.insert(
                key,
                RuntimeGateResult {
                    gate: gate_evidence_label(
                        &attempt.plan_id,
                        &attempt.task_id,
                        kind_label,
                        rung,
                        &verdict.gate_name,
                    ),
                    passed: verdict.passed,
                    detail: gate_detail(
                        kind_label,
                        attempt.attempt,
                        rung,
                        duration_ms,
                        Some(verdict.summary.as_str()),
                        verdict.error_digest.as_deref(),
                        &output,
                    ),
                },
            );
        }
    }

    Ok(final_results.into_values().collect())
}

fn read_runner_events_since(path: &Path, offset: u64) -> anyhow::Result<String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };
    let len = file
        .metadata()
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    file.seek(SeekFrom::Start(offset.min(len)))
        .with_context(|| format!("seek {}", path.display()))?;

    let mut events = String::new();
    file.read_to_string(&mut events)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(events)
}

fn matching_run_ids(events: &[RunnerEvent], plan_ids: &BTreeSet<String>) -> BTreeSet<String> {
    events
        .iter()
        .filter_map(|event| {
            let RunnerEvent::RunStarted {
                run_id,
                plan_ids: event_plan_ids,
                ..
            } = event
            else {
                return None;
            };
            let event_plan_ids = event_plan_ids.iter().cloned().collect::<BTreeSet<_>>();
            (event_plan_ids.len() == plan_ids.len()
                && event_plan_ids
                    .iter()
                    .all(|plan_id| plan_ids.contains(plan_id)))
            .then(|| run_id.clone())
        })
        .collect()
}

fn gate_kind_label(kind: GateCompletionKind) -> &'static str {
    match kind {
        GateCompletionKind::Gate => "gate",
        GateCompletionKind::PlanVerify => "plan_verify",
        GateCompletionKind::Merge => "merge",
    }
}

fn gate_evidence_key(
    plan_id: &str,
    task_id: &str,
    kind: &str,
    rung: u32,
    gate_name: &str,
) -> (String, String, String, u32, String) {
    (
        plan_id.to_string(),
        task_id.to_string(),
        kind.to_string(),
        rung,
        gate_name.to_string(),
    )
}

fn gate_evidence_label(
    plan_id: &str,
    task_id: &str,
    kind: &str,
    rung: u32,
    gate_name: &str,
) -> String {
    format!("{plan_id}:{task_id}:{kind}:{rung}:{gate_name}")
}

fn gate_detail(
    kind: &str,
    attempt: u32,
    rung: u32,
    duration_ms: u64,
    summary: Option<&str>,
    error_digest: Option<&str>,
    output: &str,
) -> String {
    let evidence = summary
        .and_then(non_empty_string)
        .or_else(|| error_digest.and_then(non_empty_string))
        .or_else(|| first_non_empty_line(output))
        .unwrap_or_else(|| "gate completed".to_string());
    format!("{kind} attempt {attempt}, rung {rung}, {duration_ms}ms: {evidence}")
}

fn first_non_empty_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let meta = entry.metadata()?;
        if meta.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if meta.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn sanitize_plan_base(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase();
    if out
        .chars()
        .next()
        .is_none_or(|ch| !ch.is_ascii_alphanumeric())
    {
        out.insert_str(0, "job-");
    }
    out
}

fn unique_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("{}-{millis}", std::process::id())
}

fn render_plan_execution_summary(report: &crate::runner::RunReport, gate_count: usize) -> String {
    let mut lines = vec![format!(
        "plan execution {}: {}/{} tasks, {} failed, {} agent calls, ${:.2}, {}s, {} gate results",
        if report.all_succeeded() {
            "succeeded"
        } else {
            "failed"
        },
        report.tasks_completed,
        report.total_tasks,
        report.tasks_failed,
        report.total_agent_calls,
        report.total_cost_usd,
        report.duration.as_secs(),
        gate_count
    )];
    for plan in &report.plans {
        lines.push(format!(
            "{}: {} ({}/{} tasks, {} failed)",
            plan.plan_id,
            if plan.completed {
                "completed"
            } else {
                "incomplete"
            },
            plan.tasks_completed,
            plan.tasks_total,
            plan.tasks_failed
        ));
    }
    lines.join("\n")
}
