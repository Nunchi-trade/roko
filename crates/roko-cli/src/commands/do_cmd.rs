//! `roko do` command — universal entry point with progressive formality.
//!
//! Classifies prompt complexity and routes through the appropriate pipeline:
//! - Trivial/Simple: direct single-agent WorkflowEngine run
//! - Standard: generate plan from prompt, then execute it
//! - Complex: create PRD -> draft PRD -> generate plan -> execute plan

use crate::*;
use roko_core::config::schema::RokoConfig;
use roko_gate::PlanComplexity;
use std::path::{Path, PathBuf};

/// Main entry point for `roko do`.
pub(crate) async fn cmd_do(
    cli: &Cli,
    workdir: Option<PathBuf>,
    prompt_args: Vec<String>,
    plan: bool,
    complexity_override: Option<PlanComplexity>,
    dry_run: bool,
    yes: bool,
    ghost: bool,
    compare: bool,
    continue_work: Option<Option<String>>,
    no_cascade: bool,
    provider: Option<String>,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let prompt = prompt_args.join(" ").trim().to_string();

    if let Some(work_id) = continue_work {
        return cmd_do_continue(&workdir, work_id).await;
    }

    if prompt.is_empty() {
        return cmd_do_resume_hint(&workdir);
    }

    let preview_config = load_resolved_config(&workdir)
        .map(|resolved| resolved.config)
        .unwrap_or_default();
    let scope_config = scope_model_config_from_cli_config(&preview_config);
    let classified_complexity = match complexity_override {
        Some(complexity) => complexity,
        None => roko_cli::scope_resolver::ScopeResolver::resolve(&prompt, &scope_config).await,
    };
    let complexity = if plan {
        promote_to_planned_complexity(classified_complexity)
    } else {
        classified_complexity
    };
    let forced = complexity_override.is_some();
    let dry_preview = dry_run || ghost;

    if dry_preview || compare {
        print_do_preview(
            &prompt,
            complexity,
            forced,
            yes,
            no_cascade,
            &preview_config,
        );
        if compare {
            println!("compare     : cascade-enabled vs --no-cascade");
            println!("execution   : skipped; compare mode is a dry preview in this worktree");
        }
        return Ok(EXIT_SUCCESS);
    }

    // Route based on classified complexity.
    match complexity {
        PlanComplexity::Trivial | PlanComplexity::Simple => {
            run_simple_path(cli, &workdir, &prompt, complexity, no_cascade, provider).await
        }
        PlanComplexity::Standard => {
            run_standard_path(cli, &workdir, &prompt, no_cascade, provider).await
        }
        PlanComplexity::Complex => {
            run_complex_path(cli, &workdir, &prompt, no_cascade, provider).await
        }
    }
}

// ─── Simple path: direct agent run via WorkflowEngine ───────────────

async fn run_simple_path(
    cli: &Cli,
    workdir: &Path,
    prompt: &str,
    complexity: PlanComplexity,
    no_cascade: bool,
    provider: Option<String>,
) -> Result<i32> {
    let workflow_template = workflow_template_for_complexity(complexity);

    eprintln!(
        "\u{25b8} Complexity: {} (auto-detected)",
        complexity_label(complexity)
    );
    eprintln!("\u{25b8} Running single agent...");

    prepare_runtime_hooks(workdir, cli.quiet);
    let mut config = resolve_config_for_workdir(cli, workdir)?;
    apply_resume_session_override(&mut config, cli.resume.clone());

    let enabled_gates = roko_cli::run::workflow_enabled_gate_names(&config.gates);
    let shell_gates = roko_cli::run::workflow_shell_gate_commands(&config.gates);
    let overrides = roko_cli::run::CliOverrides {
        model: cli.model.clone(),
        role: cli.role.clone(),
        provider,
        cascade_enabled: Some(!no_cascade),
    };

    tracing::debug!(
        complexity = complexity_label(complexity),
        workflow_template,
        cascade_enabled = !no_cascade,
        "dispatching roko do (simple) through WorkflowEngine"
    );

    let result = roko_cli::run::run_workflow_engine_report_with_hub(
        prompt,
        workdir,
        workflow_template,
        enabled_gates,
        shell_gates,
        None,
        &overrides,
    )
    .await;

    handle_workflow_result(cli, prompt, workflow_template, result)
}

// ─── Standard path: generate plan from prompt, then execute ─────────

async fn run_standard_path(
    cli: &Cli,
    workdir: &Path,
    prompt: &str,
    no_cascade: bool,
    provider: Option<String>,
) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};

    eprintln!("\u{25b8} Complexity: standard (auto-detected, override with --complexity simple)");
    eprintln!("\u{25b8} Step 1/2: Generating plan...");

    prepare_runtime_hooks(workdir, cli.quiet);

    let gw = load_gateway_env(workdir);
    let model_key = roko_cli::model_selection::resolve_effective_model_key(
        workdir,
        cli.model.clone(),
        Some("strategist"),
        "roko do (standard)",
    )?;

    // Pre-flight: check provider.
    {
        let do_config: RokoConfig = std::fs::read_to_string(workdir.join("roko.toml"))
            .ok()
            .and_then(|s| RokoConfig::from_toml(&s).ok())
            .unwrap_or_default();
        crate::commands::util::preflight_provider_for_model(&do_config, &model_key)?;
        // Aggregate provider readiness: warn/abort if no providers are usable.
        crate::commands::util::preflight_providers_aggregate(&do_config)?;
    }

    // Generate plan from the prompt using the plan generate agent.
    let system = roko_cli::plan_generate::build_generation_prompt(workdir, prompt, "prompt");
    let task_prompt = format!(
        "Read the source below and generate implementation plan directories under .roko/plans/. \
         Search the codebase first to understand what exists. \
         Create plan.md and tasks.toml files with tier, model_hint, context (read_files with line ranges), \
         mcp_servers (per-task MCP server names), and verify steps (executable shell commands). \
         Use the cheapest model tier for each task.\n\n{prompt}"
    );

    let effort = cli.effort.map(|e| e.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let _agent_command = command_from_config(workdir).unwrap_or_else(|| "claude".to_string());

    let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
        prompt: &task_prompt,
        workdir,
        model: Some(model_key.as_str()),
        effort: effort_ref.or(Some("high")),
        system_prompt: Some(&system),
        resume_session,
        env_vars: &gw.vars,
        role: Some("strategist"),
        allowed_tools: None,
    })
    .await?;

    if exit_code != 0 {
        eprintln!("\u{25b8} Plan generation failed (exit {exit_code})");
        if !output.is_empty() && !cli.quiet {
            eprint!("{output}");
        }
        return Ok(EXIT_AGENT_FAILURE);
    }

    // Find the generated plans directory.
    let plans_dir = roko_cli::plan::plans_dir(workdir);
    if !plans_dir.is_dir() {
        eprintln!(
            "\u{25b8} No plans directory found after generation at {}",
            plans_dir.display()
        );
        return Ok(EXIT_AGENT_FAILURE);
    }

    let plans = match roko_cli::runner::plan_loader::load_plans(&plans_dir) {
        Ok(plans) if plans.is_empty() => {
            eprintln!("\u{25b8} Plan generation produced no executable plans");
            return Ok(EXIT_AGENT_FAILURE);
        }
        Ok(plans) => plans,
        Err(err) => {
            eprintln!("\u{25b8} Failed to load generated plans: {err:#}");
            return Ok(EXIT_AGENT_FAILURE);
        }
    };

    let total_tasks: usize = plans.iter().map(|p| p.tasks.tasks.len()).sum();
    eprintln!("\u{25b8} Step 2/2: Executing plan ({total_tasks} tasks)...");

    // Execute the plans through the WorkflowEngine / plan runner.
    run_plan_execution(cli, workdir, &plans_dir, no_cascade, provider).await
}

// ─── Complex path: PRD -> draft -> plan -> execute ──────────────────

async fn run_complex_path(
    cli: &Cli,
    workdir: &Path,
    prompt: &str,
    no_cascade: bool,
    provider: Option<String>,
) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};

    eprintln!("\u{25b8} Complexity: complex (auto-detected, override with --complexity simple)");

    prepare_runtime_hooks(workdir, cli.quiet);

    let gw = load_gateway_env(workdir);
    let effort = cli.effort.map(|e| e.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let agent_command = command_from_config(workdir).unwrap_or_else(|| "claude".to_string());

    // ── Step 1: Create PRD idea ──────────────────────────────────────
    eprintln!("\u{25b8} Step 1/4: Creating PRD...");
    roko_cli::prd::ensure_dirs(workdir)?;
    roko_cli::prd::cmd_idea(workdir, prompt)?;

    // ── Step 2: Draft the PRD ────────────────────────────────────────
    eprintln!("\u{25b8} Step 2/4: Drafting PRD...");
    let slug = roko_cli::prd::slugify(prompt);
    let drafts = roko_cli::workspace_paths::drafts_dir(workdir);
    let draft_path = drafts.join(format!("{slug}.md"));

    let model_key = roko_cli::model_selection::resolve_effective_model_key(
        workdir,
        cli.model.clone(),
        Some("scribe"),
        "roko do (complex) prd draft",
    )?;

    // Pre-flight: check provider.
    {
        let do_config: RokoConfig = std::fs::read_to_string(workdir.join("roko.toml"))
            .ok()
            .and_then(|s| RokoConfig::from_toml(&s).ok())
            .unwrap_or_default();
        crate::commands::util::preflight_provider_for_model(&do_config, &model_key)?;
        // Aggregate provider readiness: warn/abort if no providers are usable.
        crate::commands::util::preflight_providers_aggregate(&do_config)?;
    }

    let title = prompt;
    let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, title);
    let scaffold = format!(
        "{frontmatter}# {title}\n\n\
         ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
         ## Design\n\n## References\n"
    );
    std::fs::write(&draft_path, &scaffold)?;

    let system = roko_cli::prd::prd_agent_prompt(
        workdir,
        &format!(
            "Fill in the draft PRD at {path}. \
             Output the complete PRD markdown (with YAML frontmatter) as your response. \
             Do NOT use file tools \u{2014} they are not available. \
             Do NOT wrap in code fences. \
             Follow the PRD quality standards in your system prompt exactly.",
            path = draft_path.display()
        ),
    );
    let task_prompt = format!(
        "Generate a complete PRD for: {title}. \
         Output the complete PRD markdown with YAML frontmatter. \
         Include specific requirements, machine-verifiable acceptance criteria, \
         and a design section."
    );

    let content_before = std::fs::read(&draft_path).ok();
    let started = Instant::now();
    let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
        prompt: &task_prompt,
        workdir,
        model: Some(model_key.as_str()),
        effort: effort_ref,
        system_prompt: Some(&system),
        resume_session,
        env_vars: &gw.vars,
        role: Some("scribe"),
        allowed_tools: Some("none"),
    })
    .await?;

    // Handle agent output: check if it wrote the file or returned text.
    let content_after = std::fs::read(&draft_path).ok();
    let file_was_modified = match (&content_before, &content_after) {
        (Some(before), Some(after)) => before != after,
        (None, Some(_)) => true,
        _ => false,
    };

    let mut draft_written = false;
    if file_was_modified {
        let content = std::fs::read_to_string(&draft_path).unwrap_or_default();
        if roko_cli::prd::has_substantive_markdown_content(&content) {
            draft_written = true;
        }
    } else if !output.trim().is_empty() {
        let content = roko_cli::prd::materialize_agent_markdown_output(&output, Some(&scaffold))
            .unwrap_or_else(|| scaffold.clone());
        if roko_cli::prd::has_substantive_markdown_content(&content) {
            std::fs::write(&draft_path, content)?;
            draft_written = true;
        }
    }

    let _ = crate::commands::util::persist_capture_episode(
        workdir,
        &agent_command,
        Some(model_key.as_str()),
        "do-prd-draft",
        &format!("do:prd:draft:{slug}"),
        &task_prompt,
        &output,
        draft_written,
        started.elapsed().as_millis() as u64,
        resume_session,
    )
    .await;

    if !draft_written {
        eprintln!(
            "\u{25b8} PRD draft generation failed (exit {exit_code}); \
             falling back to plan-from-prompt path"
        );
        // Fall back to standard path without the PRD step.
        return run_standard_path_inner(cli, workdir, prompt, no_cascade, provider).await;
    }

    // ── Step 3: Generate plan from the PRD ───────────────────────────
    eprintln!("\u{25b8} Step 3/4: Generating plan...");
    let plans_root = match roko_cli::prd::generate_plan_from_prd(&slug, &draft_path, false).await {
        Ok(root) => root,
        Err(err) => {
            eprintln!("\u{25b8} Plan generation from PRD failed: {err:#}");
            eprintln!("\u{25b8} Falling back to plan-from-prompt path");
            return run_standard_path_inner(cli, workdir, prompt, no_cascade, provider).await;
        }
    };

    let plans = match roko_cli::runner::plan_loader::load_plans(&plans_root) {
        Ok(plans) if plans.is_empty() => {
            eprintln!("\u{25b8} Plan generation from PRD produced no executable plans");
            return Ok(EXIT_AGENT_FAILURE);
        }
        Ok(plans) => plans,
        Err(err) => {
            eprintln!("\u{25b8} Failed to load generated plans: {err:#}");
            return Ok(EXIT_AGENT_FAILURE);
        }
    };

    let total_tasks: usize = plans.iter().map(|p| p.tasks.tasks.len()).sum();
    eprintln!("\u{25b8} Step 4/4: Executing plan ({total_tasks} tasks)...");

    // ── Step 4: Execute the plan ─────────────────────────────────────
    run_plan_execution(cli, workdir, &plans_root, no_cascade, provider).await
}

// ─── Shared: execute a plan directory through the runner v2 ─────────

async fn run_plan_execution(
    cli: &Cli,
    workdir: &Path,
    plans_dir: &Path,
    _no_cascade: bool,
    _provider: Option<String>,
) -> Result<i32> {
    // Load both the CLI Config (for daimon, executor settings) and the
    // unified RokoConfig (for agent/provider/model settings).
    let cli_config = load_resolved_config(workdir)
        .map(|resolved| resolved.config)
        .unwrap_or_default();

    // Load and verify plans exist.
    let plans = roko_cli::runner::plan_loader::load_plans(plans_dir)?;
    if plans.is_empty() {
        eprintln!("\u{25b8} No plans found at {}", plans_dir.display());
        return Ok(EXIT_AGENT_FAILURE);
    }

    // Scaffold missing crates if needed.
    let scaffolded = roko_cli::runner::plan_loader::scaffold_missing_crates(workdir, &plans)?;
    if !scaffolded.is_empty() && !cli.quiet {
        eprintln!(
            "\u{25b8} Scaffolded {} new crate(s): {}",
            scaffolded.len(),
            scaffolded.join(", ")
        );
    }

    // Build run config from the workspace config.
    let roko_config: RokoConfig =
        roko_core::config::loader::load_config_unified(workdir).unwrap_or_default();

    let layout = roko_fs::RokoLayout::for_project(workdir);
    let state_hub = roko_cli::state_hub::shared_state_hub();

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

    // Initialize cascade router.
    let router_path = layout.cascade_router_path();
    let mut model_slugs = roko_config
        .effective_models()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    model_slugs.sort();
    model_slugs.dedup();
    if model_slugs.is_empty() && !roko_config.agent.default_model.trim().is_empty() {
        model_slugs.push(roko_config.agent.default_model.clone());
    }
    let cascade_router = std::sync::Arc::new(
        roko_learn::cascade_router::CascadeRouter::load_or_new(&router_path, model_slugs),
    );

    // Feedback sinks.
    let episodes_path = layout.root_episodes_path();
    let knowledge_path = layout
        .learn_dir()
        .join(roko_neuro::admission::DEFAULT_KNOWLEDGE_CANDIDATES_FILE);
    let _ = std::fs::create_dir_all(layout.learn_dir());
    let feedback_facade = std::sync::Arc::new(
        roko_cli::runtime_feedback::FeedbackFacade::new()
            .with_sink(std::sync::Arc::new(
                roko_cli::runtime_feedback::EpisodeSink::at(&episodes_path),
            ))
            .with_sink(std::sync::Arc::new(
                roko_cli::runtime_feedback::RoutingObservationSink::new(cascade_router.clone()),
            ))
            .with_sink(std::sync::Arc::new(
                roko_cli::runtime_feedback::KnowledgeIngestionSink::at(&knowledge_path)
                    .with_ingestor(std::sync::Arc::new(
                        roko_cli::runtime_feedback::NeuroKnowledgeIngestor::new(
                            roko_neuro::KnowledgeStore::for_workdir(workdir),
                        ),
                    )),
            )),
    );

    let run_uuid = uuid::Uuid::new_v4().to_string();
    let projection = std::sync::Arc::new(roko_cli::runner::projection::Projection::new(run_uuid));
    let extension_chain = std::sync::Arc::new(tokio::sync::Mutex::new(
        roko_core::extension::ExtensionChain::new(),
    ));
    let connector_registry =
        std::sync::Arc::new(std::sync::Mutex::new(roko_core::ConnectorRegistry::new()));
    let feed_registry = std::sync::Arc::new(std::sync::Mutex::new(roko_core::FeedRegistry::new()));

    let run_config = roko_cli::runner::RunConfig {
        layout: layout.clone(),
        workdir: workdir.to_path_buf(),
        plan_dir: plans_dir.to_path_buf(),
        model: roko_config.agent.default_model.clone(),
        cli_model_override: cli.model.clone(),
        timeout_secs: roko_config.timeouts.agent_dispatch_secs,
        plan_timeout_secs: roko_config.timeouts.plan_total_secs,
        max_retries: 2,
        max_concurrent_tasks,
        gate_concurrency: max_concurrent_tasks,
        approval: false,
        dangerously_skip_permissions: true,
        force_resume: false,
        mcp_config: {
            let roko_local = layout.mcp_config_path();
            if roko_local.is_file() {
                Some(roko_local)
            } else {
                roko_agent::mcp::find_mcp_config(workdir)
                    .and_then(|r| r.ok())
                    .map(|(p, _)| p)
            }
        },
        resume_session: cli.resume.clone(),
        max_gate_rung: if roko_config.gates.skip_tests {
            u32::from(roko_config.gates.clippy_enabled)
        } else {
            2
        },
        claude_program: roko_config
            .agent
            .command
            .clone()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("claude")),
        max_plan_usd: f64::from(roko_config.budget.max_plan_usd),
        max_turn_usd: f64::from(roko_config.budget.max_turn_usd),
        clippy_enabled: roko_config.gates.clippy_enabled,
        skip_tests: roko_config.gates.skip_tests,
        roko_config: Some(std::sync::Arc::new(roko_config.clone())),
        extension_chain: Some(extension_chain),
        cascade_router: Some(cascade_router),
        daimon_state: Some(roko_cli::runner::RunConfig::daimon_state_with_strategy(
            workdir,
            cli_config.daimon.strategy_space.clone(),
        )),
        connector_registry: Some(connector_registry),
        feed_registry: Some(feed_registry),
        feedback_facade: Some(feedback_facade),
        projection: Some(projection),
        http_event_sink: None,
        output_sink: if !cli.quiet && !cli.json {
            std::sync::Arc::new(roko_cli::runner::output_sink::StderrSink::new())
                as std::sync::Arc<dyn roko_cli::runner::output_sink::RunOutputSink>
        } else {
            std::sync::Arc::new(roko_cli::runner::output_sink::NoopSink)
                as std::sync::Arc<dyn roko_cli::runner::output_sink::RunOutputSink>
        },
        warm_cache: true,
    };

    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel_for_signal.cancel();
    });

    let v2_report =
        roko_cli::runner::event_loop::run(plans, &run_config, &state_hub, cancel).await?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "succeeded": v2_report.all_succeeded(),
                "total_tasks": v2_report.total_tasks,
                "tasks_completed": v2_report.tasks_completed,
                "tasks_failed": v2_report.tasks_failed,
                "total_cost_usd": v2_report.total_cost_usd,
                "total_agent_calls": v2_report.total_agent_calls,
                "duration_secs": v2_report.duration.as_secs(),
            }))
            .unwrap_or_default()
        );
    } else if !cli.quiet {
        eprintln!(
            "\n\u{25b8} Plan complete: {}/{} tasks, ${:.2}, {}s",
            v2_report.tasks_completed,
            v2_report.total_tasks,
            v2_report.total_cost_usd,
            v2_report.duration.as_secs()
        );
        for p in &v2_report.plans {
            let status = if p.completed { "done" } else { "failed" };
            eprintln!(
                "  {status} {} -- {}/{} tasks",
                p.plan_id, p.tasks_completed, p.tasks_total,
            );
        }
        // Per-task cost breakdown.
        if !v2_report.task_costs.is_empty() {
            eprintln!("\n  Task costs:");
            eprintln!(
                "  {:.<24} {:>8} {:>8} {:>9} {:>6} {:>6}",
                "task", "tok_in", "tok_out", "cost", "calls", "result"
            );
            for tc in &v2_report.task_costs {
                eprintln!(
                    "  {:.<24} {:>8} {:>8} ${:>7.4} {:>6} {:>6}",
                    tc.task_id,
                    tc.tokens_in,
                    tc.tokens_out,
                    tc.cost_usd,
                    tc.agent_calls,
                    tc.outcome,
                );
            }
        }
    }

    if v2_report.tasks_failed > 0 && !cli.quiet && !v2_report.failure_reasons.is_empty() {
        eprintln!("\nFailure details:");
        for (key, reason) in &v2_report.failure_reasons {
            if reason.contains('\n') {
                eprintln!("  {key}:");
                for line in reason.lines() {
                    eprintln!("    {line}");
                }
            } else {
                eprintln!("  {key}: {reason}");
            }
        }
    }

    Ok(if v2_report.all_succeeded() {
        EXIT_SUCCESS
    } else {
        EXIT_AGENT_FAILURE
    })
}

/// Inner standard path used as fallback when the complex path's PRD step fails.
async fn run_standard_path_inner(
    cli: &Cli,
    workdir: &Path,
    prompt: &str,
    no_cascade: bool,
    provider: Option<String>,
) -> Result<i32> {
    use roko_cli::agent_config::load_gateway_env;
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};

    let gw = load_gateway_env(workdir);
    let model_key = roko_cli::model_selection::resolve_effective_model_key(
        workdir,
        cli.model.clone(),
        Some("strategist"),
        "roko do (fallback plan)",
    )?;

    let system = roko_cli::plan_generate::build_generation_prompt(workdir, prompt, "prompt");
    let task_prompt = format!(
        "Read the source below and generate implementation plan directories under .roko/plans/. \
         Search the codebase first to understand what exists. \
         Create plan.md and tasks.toml files with tier, model_hint, context (read_files with line ranges), \
         mcp_servers (per-task MCP server names), and verify steps (executable shell commands). \
         Use the cheapest model tier for each task.\n\n{prompt}"
    );

    let effort = cli.effort.map(|e| e.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();

    let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
        prompt: &task_prompt,
        workdir,
        model: Some(model_key.as_str()),
        effort: effort_ref.or(Some("high")),
        system_prompt: Some(&system),
        resume_session,
        env_vars: &gw.vars,
        role: Some("strategist"),
        allowed_tools: None,
    })
    .await?;

    if exit_code != 0 {
        eprintln!("\u{25b8} Fallback plan generation failed (exit {exit_code})");
        if !output.is_empty() && !cli.quiet {
            eprint!("{output}");
        }
        return Ok(EXIT_AGENT_FAILURE);
    }

    let plans_dir = roko_cli::plan::plans_dir(workdir);
    if !plans_dir.is_dir() {
        eprintln!("\u{25b8} No plans directory found after generation");
        return Ok(EXIT_AGENT_FAILURE);
    }

    run_plan_execution(cli, workdir, &plans_dir, no_cascade, provider).await
}

// ─── Continue / resume helpers ──────────────────────────────────────

async fn cmd_do_continue(workdir: &Path, work_id: Option<String>) -> Result<i32> {
    let snapshot = match work_id {
        Some(id) => match roko_core::Workspace::open(workdir) {
            Ok(workspace) => workspace.state_dir().join(format!("{id}.json")),
            Err(_) => workdir
                .join(".roko")
                .join("state")
                .join(format!("{id}.json")),
        },
        None => executor_snapshot_path(workdir),
    };

    if snapshot.exists() {
        eprintln!(
            "found resumable snapshot at {}; use `roko resume` until first-class work items land",
            snapshot.display()
        );
        Ok(EXIT_SUCCESS)
    } else {
        eprintln!("no resumable work found at {}", snapshot.display());
        Ok(EXIT_AGENT_FAILURE)
    }
}

fn cmd_do_resume_hint(workdir: &Path) -> Result<i32> {
    let snapshot = executor_snapshot_path(workdir);
    if snapshot.exists() {
        eprintln!("interrupted work found at {}", snapshot.display());
        eprintln!("resume with: roko do --continue");
        Ok(EXIT_AGENT_FAILURE)
    } else {
        eprintln!("no prompt supplied");
        eprintln!("usage: roko do \"fix the bug\"");
        Ok(EXIT_AGENT_FAILURE)
    }
}

fn executor_snapshot_path(workdir: &Path) -> PathBuf {
    roko_core::Workspace::open(workdir)
        .map(|workspace| workspace.executor_snapshot_path())
        .unwrap_or_else(|_| workdir.join(".roko").join("state").join("executor.json"))
}

// ─── Preview / formatting helpers ───────────────────────────────────

fn print_do_preview(
    prompt: &str,
    complexity: PlanComplexity,
    forced: bool,
    yes: bool,
    no_cascade: bool,
    config: &Config,
) {
    let gate_count = roko_cli::run::workflow_enabled_gate_names(&config.gates).len();
    let pipeline = pipeline_description(complexity);

    println!("roko do");
    println!("prompt      : {}", truncate_for_preview(prompt, 80));
    println!(
        "complexity  : {} ({})",
        complexity_label(complexity),
        if forced {
            "forced"
        } else {
            "auto-detected, override with --complexity simple"
        }
    );
    println!("pipeline    : {pipeline}");
    println!("cost        : {}", estimated_cost_range(complexity));
    println!("gates       : {gate_count}");
    println!("approval    : {}", if yes { "auto" } else { "workflow" });
    println!(
        "cascade     : {}",
        if no_cascade { "disabled" } else { "enabled" }
    );
    println!("execution   : skipped");
}

fn pipeline_description(complexity: PlanComplexity) -> &'static str {
    match complexity {
        PlanComplexity::Trivial => "single agent (direct)",
        PlanComplexity::Simple => "single agent (focused)",
        PlanComplexity::Standard => "generate plan -> execute",
        PlanComplexity::Complex => "PRD -> draft -> plan -> execute",
    }
}

fn handle_workflow_result(
    cli: &Cli,
    prompt: &str,
    workflow_template: &str,
    result: anyhow::Result<roko_runtime::workflow_engine::WorkflowRunReport>,
) -> Result<i32> {
    match result {
        Ok(report) => {
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if !cli.quiet {
                roko_cli::run::print_workflow_run_report(prompt, workflow_template, &report);
            }

            if report.success {
                Ok(EXIT_SUCCESS)
            } else {
                Ok(EXIT_AGENT_FAILURE)
            }
        }
        Err(error) => {
            if !cli.quiet {
                eprintln!("workflow engine error: {error:#}");
            }
            Ok(EXIT_AGENT_FAILURE)
        }
    }
}

fn promote_to_planned_complexity(complexity: PlanComplexity) -> PlanComplexity {
    match complexity {
        PlanComplexity::Trivial | PlanComplexity::Simple => PlanComplexity::Standard,
        PlanComplexity::Standard | PlanComplexity::Complex => complexity,
    }
}

fn workflow_template_for_complexity(complexity: PlanComplexity) -> &'static str {
    match complexity {
        PlanComplexity::Trivial => "mechanical",
        PlanComplexity::Simple => "focused",
        PlanComplexity::Standard => "integrative",
        PlanComplexity::Complex => "architectural",
    }
}

fn complexity_label(complexity: PlanComplexity) -> &'static str {
    match complexity {
        PlanComplexity::Trivial => "trivial",
        PlanComplexity::Simple => "simple",
        PlanComplexity::Standard => "standard",
        PlanComplexity::Complex => "complex",
    }
}

fn estimated_cost_range(complexity: PlanComplexity) -> &'static str {
    match complexity {
        PlanComplexity::Trivial => "<$0.01",
        PlanComplexity::Simple => "$0.01-$0.05",
        PlanComplexity::Standard => "$0.05-$0.25",
        PlanComplexity::Complex => "$0.25+",
    }
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut truncated = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push_str("...");
    truncated
}

fn scope_model_config_from_cli_config(config: &Config) -> RokoConfig {
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
    if let Some(model) = config.agent.model.clone() {
        model_config.agent.default_model = model;
    }
    model_config
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── complexity_label ───────────────────────────────────────────

    #[test]
    fn complexity_label_trivial() {
        assert_eq!(complexity_label(PlanComplexity::Trivial), "trivial");
    }

    #[test]
    fn complexity_label_simple() {
        assert_eq!(complexity_label(PlanComplexity::Simple), "simple");
    }

    #[test]
    fn complexity_label_standard() {
        assert_eq!(complexity_label(PlanComplexity::Standard), "standard");
    }

    #[test]
    fn complexity_label_complex() {
        assert_eq!(complexity_label(PlanComplexity::Complex), "complex");
    }

    // ── workflow_template_for_complexity ────────────────────────────

    #[test]
    fn workflow_template_trivial() {
        assert_eq!(
            workflow_template_for_complexity(PlanComplexity::Trivial),
            "mechanical"
        );
    }

    #[test]
    fn workflow_template_simple() {
        assert_eq!(
            workflow_template_for_complexity(PlanComplexity::Simple),
            "focused"
        );
    }

    #[test]
    fn workflow_template_standard() {
        assert_eq!(
            workflow_template_for_complexity(PlanComplexity::Standard),
            "integrative"
        );
    }

    #[test]
    fn workflow_template_complex() {
        assert_eq!(
            workflow_template_for_complexity(PlanComplexity::Complex),
            "architectural"
        );
    }

    // ── pipeline_description ───────────────────────────────────────

    #[test]
    fn pipeline_trivial_is_direct() {
        assert_eq!(
            pipeline_description(PlanComplexity::Trivial),
            "single agent (direct)"
        );
    }

    #[test]
    fn pipeline_simple_is_focused() {
        assert_eq!(
            pipeline_description(PlanComplexity::Simple),
            "single agent (focused)"
        );
    }

    #[test]
    fn pipeline_standard_is_plan_execute() {
        assert_eq!(
            pipeline_description(PlanComplexity::Standard),
            "generate plan -> execute"
        );
    }

    #[test]
    fn pipeline_complex_is_full_prd() {
        assert_eq!(
            pipeline_description(PlanComplexity::Complex),
            "PRD -> draft -> plan -> execute"
        );
    }

    // ── estimated_cost_range ───────────────────────────────────────

    #[test]
    fn cost_trivial() {
        assert_eq!(estimated_cost_range(PlanComplexity::Trivial), "<$0.01");
    }

    #[test]
    fn cost_simple() {
        assert_eq!(estimated_cost_range(PlanComplexity::Simple), "$0.01-$0.05");
    }

    #[test]
    fn cost_standard() {
        assert_eq!(
            estimated_cost_range(PlanComplexity::Standard),
            "$0.05-$0.25"
        );
    }

    #[test]
    fn cost_complex() {
        assert_eq!(estimated_cost_range(PlanComplexity::Complex), "$0.25+");
    }

    // ── promote_to_planned_complexity ──────────────────────────────

    #[test]
    fn promote_trivial_to_standard() {
        assert_eq!(
            promote_to_planned_complexity(PlanComplexity::Trivial),
            PlanComplexity::Standard
        );
    }

    #[test]
    fn promote_simple_to_standard() {
        assert_eq!(
            promote_to_planned_complexity(PlanComplexity::Simple),
            PlanComplexity::Standard
        );
    }

    #[test]
    fn promote_standard_stays_standard() {
        assert_eq!(
            promote_to_planned_complexity(PlanComplexity::Standard),
            PlanComplexity::Standard
        );
    }

    #[test]
    fn promote_complex_stays_complex() {
        assert_eq!(
            promote_to_planned_complexity(PlanComplexity::Complex),
            PlanComplexity::Complex
        );
    }

    // ── truncate_for_preview ───────────────────────────────────────

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_for_preview("hello", 80), "hello");
    }

    #[test]
    fn truncate_exact_boundary() {
        assert_eq!(truncate_for_preview("abcde", 5), "abcde");
    }

    #[test]
    fn truncate_over_limit_adds_ellipsis() {
        let result = truncate_for_preview("abcdefghij", 5);
        assert!(result.ends_with("..."));
        // Should be 4 original chars + "..."
        assert_eq!(result, "abcd...");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_for_preview("", 10), "");
    }

    #[test]
    fn truncate_unicode() {
        // Unicode chars count by char, not byte.
        let emoji = "\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}"; // 5 emoji
        assert_eq!(truncate_for_preview(emoji, 5), emoji);
    }
}
