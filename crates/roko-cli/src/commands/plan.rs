//! plan command handlers.
#![allow(unused_imports)]

use crate::*;
use roko_fs::RokoLayout;

pub(crate) async fn cmd_plan(cli: &Cli, cmd: PlanCmd) -> Result<i32> {
    match cmd {
        PlanCmd::List { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let summaries =
                roko_cli::plan::summarize_discovered_plans(&wd).map_err(|e| anyhow!("{e}"))?;
            let executor_state = read_executor_state(&wd);
            let has_run_state = executor_state.is_some();
            let state_entries = executor_state.clone().unwrap_or_default();
            let state_map: std::collections::HashMap<String, (usize, usize)> = state_entries
                .iter()
                .cloned()
                .map(|(id, done, total)| (id, (done, total)))
                .collect();

            let mut summaries = summaries;
            for summary in &mut summaries {
                if let Some((tasks_done, tasks_total)) = state_map.get(&summary.id).copied() {
                    summary.tasks_done = tasks_done;
                    summary.task_count = tasks_total;
                    summary.completed = tasks_total > 0 && tasks_done == tasks_total;
                }
            }

            if cli.json {
                let entries: Vec<serde_json::Value> = summaries
                    .iter()
                    .map(|summary| {
                        serde_json::json!({
                            "id": summary.id.as_str(),
                            "title": summary.title.as_str(),
                            "task_count": summary.task_count,
                            "tasks_done": summary.tasks_done,
                            "tasks_failed": summary.tasks_failed,
                            "completed": summary.completed,
                            "has_run_state": has_run_state,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                if summaries.is_empty() {
                    if has_run_state {
                        println!("no plans found in discovery path");
                    } else {
                        println!("no run state found");
                    }
                } else {
                    println!(
                        "{:<16} {:<40} {:<12} {}",
                        "ID", "TITLE", "PROGRESS", "STATUS"
                    );
                    for summary in &summaries {
                        let status =
                            if summary.task_count > 0 && summary.tasks_done == summary.task_count {
                                "done"
                            } else if summary.tasks_done > 0 {
                                "in-progress"
                            } else {
                                "pending"
                            };
                        println!(
                            "{:<16} {:<40} {:<12} {}",
                            summary.id.as_str(),
                            summary.title.as_str(),
                            format!("{}/{}", summary.tasks_done, summary.task_count),
                            status
                        );
                    }
                    if !has_run_state {
                        println!("(no run state found — counts from tasks.toml files)");
                    }
                }

                for (plan_id, _, _) in &state_entries {
                    if !plan_path_exists(&wd, plan_id) {
                        println!(
                            "warning: state references missing plan: {plan_id} (not found in plans/ or .roko/plans/)"
                        );
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Show { plan_id, workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let Some(plan_info) =
                roko_cli::plan::discover_plan_by_id(&wd, &plan_id).map_err(|e| anyhow!("{e}"))?
            else {
                anyhow::bail!("plan '{plan_id}' not found");
            };
            let summary = roko_cli::plan::summarize_plan_info(&plan_info);
            let tasks_path = roko_cli::plan::tasks_path(&plan_info);
            let stable_id = roko_cli::plan::stable_plan_id(&plan_info);

            if cli.json {
                let payload = json!({
                    "plan_id": stable_id,
                    "base": plan_info.base,
                    "title": summary.title,
                    "plan_path": plan_info.path,
                    "tasks_path": tasks_path,
                    "task_count": summary.task_count,
                    "frontmatter": plan_info.frontmatter,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("plan: {stable_id}");
                println!("base: {}", plan_info.base);
                println!("title: {}", summary.title);
                println!("plan file: {}", plan_info.path.display());
                println!(
                    "tasks file: {}",
                    tasks_path
                        .as_deref()
                        .filter(|path| path.is_file())
                        .map_or_else(|| "(none)".to_string(), |path| path.display().to_string())
                );
                println!("task count: {}", summary.task_count);
                if let Some(frontmatter) = plan_info.frontmatter.as_ref() {
                    if !frontmatter.depends_on.is_empty() {
                        println!("depends_on: {}", frontmatter.depends_on.join(", "));
                    }
                    if !frontmatter.parallel_with.is_empty() {
                        println!("parallel_with: {}", frontmatter.parallel_with.join(", "));
                    }
                    if let Some(priority) = frontmatter.priority {
                        println!("priority: {priority}");
                    }
                    if !frontmatter.tags.is_empty() {
                        println!("tags: {}", frontmatter.tags.join(", "));
                    }
                    if let Some(milestone) = frontmatter.milestone.as_deref() {
                        println!("milestone: {milestone}");
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Create {
            plan_id,
            title,
            description,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let plan = Plan::new(plan_id.clone(), title, description);
            plan.validate()
                .map_err(|errs| anyhow!("plan validation failed: {}", errs.join("; ")))?;

            let plans_dir = roko_cli::plan::plans_dir(&wd);
            std::fs::create_dir_all(&plans_dir).map_err(|e| anyhow!("create plans dir: {e}"))?;
            let plan_dir = plans_dir.join(&plan_id);
            let legacy_plan = plans_dir.join(format!("{plan_id}.md"));
            if plan_dir.exists() || legacy_plan.exists() {
                bail!("plan '{plan_id}' already exists");
            }
            std::fs::create_dir_all(&plan_dir).map_err(|e| anyhow!("create plan dir: {e}"))?;
            let plan_md_path = plan_dir.join("plan.md");
            let tasks_path = plan_dir.join("tasks.toml");

            let yaml_plan_id = serde_json::to_string(&plan.id)?;
            let plan_md = format!(
                "---\nplan: {yaml_plan_id}\n---\n# {}\n\n{}\n",
                plan.title,
                if plan.description.is_empty() {
                    "Describe the plan here.".to_string()
                } else {
                    plan.description.clone()
                }
            );
            let tasks_toml = format!(
                "[meta]\nplan = {:?}\nmax_parallel = 1\n\n# Add [[task]] entries below.\n",
                plan.id
            );
            std::fs::write(&plan_md_path, plan_md)
                .map_err(|e| anyhow!("write {}: {e}", plan_md_path.display()))?;
            std::fs::write(&tasks_path, tasks_toml)
                .map_err(|e| anyhow!("write {}: {e}", tasks_path.display()))?;

            if cli.json {
                let payload = json!({
                    "created": plan_id,
                    "plan_dir": plan_dir,
                    "plan_path": plan_md_path,
                    "tasks_path": tasks_path,
                });
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else if !cli.quiet {
                println!("created plan '{}' at {}", plan_id, plan_dir.display());
            }
            Ok(EXIT_SUCCESS)
        }
        PlanCmd::Validate { dir, strict, json } => {
            let workdir = resolve_workdir(cli);
            let plans_dir = if dir.is_absolute() {
                dir
            } else {
                workdir.join(dir)
            };
            cmd_plan_validate(&plans_dir, &workdir, strict, json || cli.json)
        }
        PlanCmd::Run {
            plans_dir,
            engine,
            workdir,
            resume_plan,
            approval,
            max_retries,
            max_tasks,
            dry_run,
            fresh,
            force_resume,
        } => {
            let t_total = std::time::Instant::now();
            let t_setup = std::time::Instant::now();

            // Resolve workdir FIRST (before using plans_dir)
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let layout = RokoLayout::for_project(&wd);

            // Resolve plans_dir relative to workdir if not absolute
            let resolved_plans_dir = if plans_dir.is_absolute() {
                plans_dir.clone()
            } else {
                wd.join(&plans_dir)
            };

            // ── Mandatory validation: reject malformed plans before execution ──
            // Runs in both normal and `--dry-run` mode.
            if let Some(exit_code) = validate_before_run(&resolved_plans_dir, &wd) {
                return Ok(exit_code);
            }

            // ── Dry-run mode: parse plans + show summary without executing ──
            if dry_run {
                return cmd_plan_dry_run(&resolved_plans_dir, cli).await;
            }

            // ── Graph Engine path (default) ──
            if matches!(engine, PlanEngine::Graph) {
                // Warn about unsupported flags on the Graph Engine path.
                if resume_plan.is_some() {
                    eprintln!("Note: --resume-plan is not yet supported by the Graph Engine; snapshots will be ignored.");
                }

                return cmd_plan_run_engine(&resolved_plans_dir, &wd, cli).await;
            }

            // ── Runner v2 path (legacy, feature-gated) ──
            //
            // When `legacy-runner-v2` is not enabled, PlanEngine only has
            // `Graph` so the match above always returns. The block below
            // is unreachable without the feature.
            #[cfg(not(feature = "legacy-runner-v2"))]
            {
                let _ = (&resume_plan, &approval, &max_retries, &max_tasks, &fresh, &force_resume,
                          &layout, &t_setup, &t_total);
                bail!("Runner v2 is not available; enable the `legacy-runner-v2` feature to use --engine runner-v2");
            }

            #[cfg(feature = "legacy-runner-v2")]
            {
            // Acquire exclusive workspace lock before mutating state.
            let _lock = roko_cli::workspace_lock::acquire_workspace_lock(layout.root())?;

            if fresh {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                let state_dir = layout.state_dir();
                for filename in &["executor.json", "orchestrator.json", "run-state.json"] {
                    let state_path = state_dir.join(filename);
                    if state_path.exists() {
                        let backup_path = state_path.with_extension(format!("json.bak.{ts}"));
                        match std::fs::rename(&state_path, &backup_path) {
                            Ok(()) => {
                                if !cli.quiet {
                                    eprintln!(
                                        "▸ --fresh: archived old state to {}",
                                        backup_path.display()
                                    );
                                }
                            }
                            Err(err) => {
                                eprintln!(
                                    "warning: --fresh: could not archive {}: {err}",
                                    state_path.display()
                                );
                            }
                        }
                    }
                }
            }

            prepare_runtime_hooks(&wd, cli.quiet);
            let config = load_resolved_config(&wd)?.config;

            // Bootstrap: workspace check + unified config load.
            let boot = roko_cli::bootstrap::RokoBootstrap::new(
                &wd,
                roko_cli::bootstrap::BootOpts {
                    require_workspace: true,
                    require_provider: false, // explicit preflight below is more detailed
                    acquire_lock: false,
                },
            )?;
            let early_roko_config = boot.config;

            // Pre-flight: fail fast if the default model's provider is misconfigured.
            {
                let dm = &early_roko_config.agent.default_model;
                if !dm.trim().is_empty() {
                    crate::commands::util::preflight_provider_for_model(&early_roko_config, dm)?;
                }
            }

            // Aggregate provider readiness: warn/abort if no providers are usable.
            crate::commands::util::preflight_providers_aggregate(&early_roko_config)?;

            // Pre-flight: warn if gate tools are missing.
            let missing_gate_tools = crate::commands::util::preflight_gate_deps();
            if !missing_gate_tools.is_empty() {
                eprintln!(
                    "warning: missing gate tools: {}. Some gates may fail.",
                    missing_gate_tools.join(", ")
                );
            }
            let max_concurrent_tasks = if max_tasks > 0 {
                max_tasks
            } else {
                early_roko_config
                    .runner
                    .max_concurrent_tasks
                    .or_else(|| {
                        (config.executor.max_concurrent_tasks
                            != roko_orchestrator::ExecutorConfig::default().max_concurrent_tasks)
                            .then_some(config.executor.max_concurrent_tasks)
                    })
                    .unwrap_or(4)
            }
            .max(1);
            let state_hub = roko_cli::state_hub::shared_state_hub();

            // Runner v2 auto-resumes from .roko/state/executor.json if it exists.
            // Explicit --resume-plan paths are honored by copying to the standard location.
            if !fresh {
                if let Some(ref snap_path) = resume_plan {
                    let snap_path = if snap_path.is_relative() {
                        wd.join(snap_path)
                    } else {
                        snap_path.clone()
                    };
                    let standard = layout.executor_snapshot();
                    if snap_path != standard && snap_path.exists() {
                        if let Some(parent) = standard.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::copy(&snap_path, &standard);
                    }
                }
            }

            // Create the shared metric registry and register standard metrics.
            let metrics = std::sync::Arc::new(roko_core::obs::MetricRegistry::new());
            roko_core::obs::register_standard_metrics(&metrics);

            // ── Runner v2 for all plan run modes ────────────────────
            // Ensure git repo exists — agents need git tools to work.
            if !wd.join(".git").exists() {
                eprintln!("▸ No git repo found — initializing for agent tooling");
                let _ = std::process::Command::new("git")
                    .args(["init"])
                    .current_dir(&wd)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                let _ = std::process::Command::new("git")
                    .args(["add", "-A"])
                    .current_dir(&wd)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
                let _ = std::process::Command::new("git")
                    .args([
                        "commit",
                        "-m",
                        "init (auto-created by roko)",
                        "--allow-empty",
                    ])
                    .current_dir(&wd)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }

            let plans = roko_cli::runner::plan_loader::load_plans(&resolved_plans_dir)?;

            // Scaffold any crates referenced by tasks that don't exist yet.
            // Plans that create new crates need a minimal Cargo.toml + src/lib.rs
            // so the gate pipeline (`cargo check`) can succeed.
            let scaffolded = roko_cli::runner::plan_loader::scaffold_missing_crates(&wd, &plans)?;
            if !scaffolded.is_empty() && !cli.quiet {
                eprintln!(
                    "▸ Scaffolded {} new crate(s): {}",
                    scaffolded.len(),
                    scaffolded.join(", ")
                );
            }

            let roko_config = early_roko_config;

            // Initialize Phase 0 subsystems.
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
            let extension_chain = std::sync::Arc::new(tokio::sync::Mutex::new(
                roko_core::extension::ExtensionChain::new(),
            ));
            let connector_registry =
                std::sync::Arc::new(std::sync::Mutex::new(roko_core::ConnectorRegistry::new()));
            let feed_registry =
                std::sync::Arc::new(std::sync::Mutex::new(roko_core::FeedRegistry::new()));

            // ── Wire dispatch / feedback / projection facades ──────────────
            //
            // The new module families are activated alongside the legacy
            // emit paths: every runner event also lands on the projection
            // broadcast and (when applicable) on the feedback fan-out.
            // Sinks write into `.roko/`, mirroring what the legacy helper
            // path does so resume / dashboard data stays consistent.
            let run_uuid = uuid::Uuid::new_v4().to_string();
            let projection = std::sync::Arc::new(roko_cli::runner::projection::Projection::new(
                run_uuid.clone(),
            ));
            let episodes_path = layout.root_episodes_path();
            let knowledge_path = layout
                .learn_dir()
                .join(roko_neuro::admission::DEFAULT_KNOWLEDGE_CANDIDATES_FILE);
            // Best-effort directory creation — the sinks' own
            // `create_dir_all` will retry on first append.
            let _ = std::fs::create_dir_all(layout.learn_dir());
            let feedback_facade = std::sync::Arc::new(
                roko_cli::runtime_feedback::FeedbackFacade::new()
                    .with_sink(std::sync::Arc::new(
                        roko_cli::runtime_feedback::EpisodeSink::at(&episodes_path),
                    ))
                    .with_sink(std::sync::Arc::new(
                        roko_cli::runtime_feedback::RoutingObservationSink::new(
                            cascade_router.clone(),
                        ),
                    ))
                    .with_sink(std::sync::Arc::new(
                        roko_cli::runtime_feedback::KnowledgeIngestionSink::at(&knowledge_path)
                            .with_ingestor(std::sync::Arc::new(
                                roko_cli::runtime_feedback::NeuroKnowledgeIngestor::new(
                                    roko_neuro::KnowledgeStore::for_workdir(&wd),
                                ),
                            )),
                    )),
            );

            let run_config = roko_cli::runner::RunConfig {
                layout: layout.clone(),
                workdir: wd.clone(),
                plan_dir: resolved_plans_dir.clone(),
                model: roko_config.agent.default_model.clone(),
                cli_model_override: cli.model.clone(),
                timeout_secs: roko_config.timeouts.agent_dispatch_secs,
                plan_timeout_secs: roko_config.timeouts.plan_total_secs,
                max_retries: max_retries.unwrap_or(2),
                max_concurrent_tasks,
                gate_concurrency: max_concurrent_tasks,
                approval,
                dangerously_skip_permissions: true,
                force_resume,
                mcp_config: {
                    // Resolve MCP config: .roko/mcp.json > auto-discovery
                    let mcp = {
                        let roko_local = layout.mcp_config_path();
                        if roko_local.is_file() {
                            Some(roko_local)
                        } else {
                            roko_agent::mcp::find_mcp_config(&wd)
                                .and_then(|r| r.ok())
                                .map(|(p, _)| p)
                        }
                    };
                    if let Some(ref path) = mcp {
                        tracing::info!(path = ?path, "MCP config resolved for plan run");
                    } else {
                        tracing::debug!("no MCP config found for plan run");
                    }
                    mcp
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
                    &wd,
                    config.daimon.strategy_space.clone(),
                )),
                connector_registry: Some(connector_registry),
                feed_registry: Some(feed_registry),
                feedback_facade: Some(feedback_facade),
                projection: Some(projection),
                http_event_sink: None,
                output_sink: if !approval && !cli.quiet && !cli.json {
                    std::sync::Arc::new(
                        roko_cli::runner::output_sink::FormattedStderrSink::new(
                            cli.color.should_color(),
                        ),
                    )
                        as std::sync::Arc<dyn roko_cli::runner::output_sink::RunOutputSink>
                } else {
                    std::sync::Arc::new(roko_cli::runner::output_sink::NoopSink)
                        as std::sync::Arc<dyn roko_cli::runner::output_sink::RunOutputSink>
                },
                warm_cache: true,
                metrics: None,
            };

            // Optionally spawn the approval TUI.
            if approval {
                if !std::io::stdout().is_terminal() {
                    anyhow::bail!("approval mode requires an interactive terminal");
                }

                // Redirect stderr to a log file so the runner's tracing output
                // doesn't corrupt the TUI's raw terminal display.
                let stderr_log_path = layout.runner_stderr_log();
                let _ = std::fs::create_dir_all(stderr_log_path.parent().unwrap_or(&wd));
                #[cfg(unix)]
                if let Ok(log_file) = std::fs::File::create(&stderr_log_path) {
                    use std::os::unix::io::AsRawFd;
                    #[allow(unsafe_code)]
                    unsafe {
                        libc::dup2(log_file.as_raw_fd(), 2);
                    }
                }

                let state_hub_for_tui = state_hub.clone();
                let workdir_for_tui = wd.clone();
                std::thread::Builder::new()
                    .name("roko-plan-approval-tui".to_string())
                    .spawn(move || {
                        let app = App::new_connected_with_page(
                            &workdir_for_tui,
                            None,
                            &state_hub_for_tui,
                        );
                        if let Err(err) = app.run() {
                            tracing::error!(error = %err, "approval TUI exited with error");
                        }
                    })
                    .context("spawn approval TUI thread")?;
            }

            let cancel = tokio_util::sync::CancellationToken::new();
            let cancel_for_signal = cancel.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                cancel_for_signal.cancel();
            });

            let total_tasks: usize = plans.iter().map(|p| p.tasks.tasks.len()).sum();
            let plan_count = plans.len();

            // Print a header line instead of a spinner — real-time streaming
            // output from agent events replaces the old static spinner.
            if !approval && !cli.quiet && !cli.json {
                let plan_names: Vec<&str> = plans.iter().map(|p| p.id.as_str()).collect();
                eprintln!(
                    "\u{25b8} Running plan{} ({} task{}): {}",
                    if plan_count == 1 { "" } else { "s" },
                    total_tasks,
                    if total_tasks == 1 { "" } else { "s" },
                    plan_names.join(", "),
                );
            }

            let setup_ms = t_setup.elapsed().as_millis();
            tracing::info!(
                setup_ms,
                plan_count,
                total_tasks,
                default_model = %roko_config.agent.default_model,
                max_concurrent_tasks,
                max_retries = run_config.max_retries,
                max_gate_rung = run_config.max_gate_rung,
                max_plan_usd = %format!("{:.2}", run_config.max_plan_usd),
                max_turn_usd = %format!("{:.2}", run_config.max_turn_usd),
                clippy_enabled = run_config.clippy_enabled,
                skip_tests = run_config.skip_tests,
                plans_dir = %resolved_plans_dir.display(),
                "plan run: setup complete, entering event loop"
            );
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
                        "plans": v2_report.plans.iter().map(|p| serde_json::json!({
                            "plan_id": p.plan_id,
                            "completed": p.completed,
                            "tasks_completed": p.tasks_completed,
                            "tasks_failed": p.tasks_failed,
                        })).collect::<Vec<_>>(),
                        "task_costs": v2_report.task_costs.iter().map(|tc| serde_json::json!({
                            "plan_id": tc.plan_id,
                            "task_id": tc.task_id,
                            "model": tc.model,
                            "provider": tc.provider,
                            "tokens_in": tc.tokens_in,
                            "tokens_out": tc.tokens_out,
                            "cost_usd": tc.cost_usd,
                            "agent_calls": tc.agent_calls,
                            "outcome": tc.outcome,
                        })).collect::<Vec<_>>(),
                    }))
                    .unwrap_or_default()
                );
            } else if !cli.quiet {
                eprintln!(
                    "\n▸ Plan complete: {}/{} tasks, ${:.2}, {}s",
                    v2_report.tasks_completed,
                    v2_report.total_tasks,
                    v2_report.total_cost_usd,
                    v2_report.duration.as_secs()
                );
                for p in &v2_report.plans {
                    let status = if p.completed { "✓" } else { "✗" };
                    eprintln!(
                        "  {status} {} — {}/{} tasks",
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

            if v2_report.tasks_failed > 0 && !cli.quiet {
                if !v2_report.failure_reasons.is_empty() {
                    eprintln!("\nFailure details:");
                    for (key, reason) in &v2_report.failure_reasons {
                        if reason.contains('\n') {
                            eprintln!("  ✗ {key}:");
                            for line in reason.lines() {
                                eprintln!("    {line}");
                            }
                        } else {
                            eprintln!("  ✗ {key}: {reason}");
                        }
                    }
                    eprintln!("\nhint: check .roko/roko.log for full failure output");
                }
                let state_path = layout.executor_snapshot();
                if state_path.exists() {
                    eprintln!(
                        "hint: if tasks appear stuck or state looks wrong, try: roko plan run {} --fresh",
                        resolved_plans_dir.display()
                    );
                }
            }

            if !cli.quiet && !cli.json {
                let loop_ms = v2_report.duration.as_millis();
                let report_ms = t_total
                    .elapsed()
                    .as_millis()
                    .saturating_sub(setup_ms + loop_ms);
                let total_ms = t_total.elapsed().as_millis();
                eprintln!(
                    "  Timing: setup={setup_ms}ms loop={loop_ms}ms report={report_ms}ms total={total_ms}ms"
                );
            }

            Ok(if v2_report.all_succeeded() {
                EXIT_SUCCESS
            } else {
                EXIT_FAILURE
            })
            } // end #[cfg(feature = "legacy-runner-v2")]
        }
        PlanCmd::Generate { source, from_file } => {
            use roko_cli::agent_config::load_gateway_env;
            use roko_cli::agent_exec::{AgentExecEpisode, AgentExecOpts, run_agent_logged};

            let workdir = std::env::current_dir().context("resolve cwd")?;
            let gw = load_gateway_env(&workdir);

            // Get the source content: either from a file or inline text
            let source_text = if let Some(ref path) = from_file {
                let content = std::fs::read_to_string(path)
                    .with_context(|| format!("read {}", path.display()))?;
                eprintln!("📋 Generating plans from file: {}", path.display());
                content
            } else {
                let text = source.join(" ");
                if text.is_empty() {
                    anyhow::bail!("Provide a prompt or --from-file <path>");
                }
                eprintln!("📋 Generating plans from prompt: {text}");
                text
            };

            let source_type = if from_file.is_some() {
                "file"
            } else {
                "prompt"
            };
            let task_id = from_file
                .as_ref()
                .and_then(|path| path.file_stem())
                .and_then(|stem| stem.to_str())
                .map(|stem| format!("plan:generate:{stem}"))
                .unwrap_or_else(|| "plan:generate:prompt".to_string());
            let system = roko_cli::plan_generate::build_generation_prompt(
                &workdir,
                &source_text,
                source_type,
            );
            let model_key = roko_cli::model_selection::resolve_effective_model_key(
                &workdir,
                cli.model.clone(),
                Some("strategist"),
                "plan generate",
            )?;

            let task_prompt = format!(
                "Read the source below and generate implementation plan directories under .roko/plans/. \
                 Search the codebase first to understand what exists. \
                 Create plan.md and tasks.toml files with tier, model_hint, context (read_files with line ranges), \
                 mcp_servers (per-task MCP server names), and verify steps (executable shell commands). \
                 Use the cheapest model tier for each task.\n\n{source_text}"
            );

            run_agent_logged(
                AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: Some(model_key.as_str()),
                    effort: Some("high"),
                    system_prompt: Some(&system),
                    resume_session: None,
                    env_vars: &gw.vars,
                    role: Some("strategist"),
                    allowed_tools: None,
                },
                AgentExecEpisode {
                    task_kind: "plan-generate",
                    task_id: &task_id,
                },
            )
            .await
        }
        PlanCmd::Regenerate { plan_dir, dry_run } => {
            use roko_cli::agent_config::load_gateway_env;
            use roko_cli::agent_exec::{AgentExecEpisode, AgentExecOpts, run_agent_logged};

            let workdir = std::env::current_dir().context("resolve cwd")?;
            let tasks_path = plan_dir.join("tasks.toml");
            if !tasks_path.exists() {
                anyhow::bail!("No tasks.toml found in {}", plan_dir.display());
            }

            let existing = std::fs::read_to_string(&tasks_path)
                .with_context(|| format!("read {}", tasks_path.display()))?;
            let existing_tasks = roko_cli::task_parser::TasksFile::parse(&tasks_path).ok();
            let source_path = find_plan_source_document(&plan_dir)?;
            let source_content = std::fs::read_to_string(&source_path)
                .with_context(|| format!("read {}", source_path.display()))?;
            let model_key = roko_cli::model_selection::resolve_effective_model_key(
                &workdir,
                cli.model.clone(),
                Some("strategist"),
                "plan regenerate",
            )?;

            // Collect pre-existing validation diagnostics so the agent knows what was wrong.
            let pre_validation_context =
                format_pre_validation_context(&tasks_path, &plan_validate::validate_plans_dir);

            if dry_run {
                let system = roko_cli::plan_generate::build_generation_prompt(
                    &workdir,
                    &source_content,
                    "prd",
                );
                let task_prompt = format!(
                    "Regenerate the plan at {} from the source PRD above. \
                     Rewrite tasks.toml in place with full modern metadata: tier, model_hint, \
                     max_loc, files, allowed_tools, denied_tools, mcp_servers, depends_on, \
                     [task.context], and [[task.verify]]. Preserve the status of any task that \
                     is already marked done in the existing file. Do not create new plan \
                     directories.\n\n## Existing tasks.toml\n\n```toml\n{existing}\n```\
                     {pre_validation_context}",
                    tasks_path.display(),
                    existing = existing,
                );
                eprintln!(
                    "\n[dry-run] Would regenerate {} from {}",
                    tasks_path.display(),
                    source_path.display()
                );
                eprintln!("Prompt length: {} chars", system.len() + task_prompt.len());
                return Ok(EXIT_SUCCESS);
            }

            let gw = load_gateway_env(&workdir);

            let system =
                roko_cli::plan_generate::build_generation_prompt(&workdir, &source_content, "prd");
            let task_prompt = format!(
                "Regenerate the plan at {} from the source PRD above. \
                 Rewrite tasks.toml in place with full modern metadata: tier, model_hint, \
                 max_loc, files, allowed_tools, denied_tools, mcp_servers, depends_on, \
                 [task.context], and [[task.verify]]. Preserve the status of any task that \
                 is already marked done in the existing file. Do not create new plan \
                 directories.\n\n## Existing tasks.toml\n\n```toml\n{existing}\n```\
                 {pre_validation_context}",
                tasks_path.display(),
                existing = existing,
            );
            let plan_name = plan_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unknown");
            let task_id = format!("plan:regenerate:{plan_name}");

            let exit_code = match run_agent_logged(
                AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: Some(model_key.as_str()),
                    effort: Some("high"),
                    system_prompt: Some(&system),
                    resume_session: None,
                    env_vars: &gw.vars,
                    role: Some("strategist"),
                    allowed_tools: None,
                },
                AgentExecEpisode {
                    task_kind: "plan-regenerate",
                    task_id: &task_id,
                },
            )
            .await
            {
                Ok(code) => code,
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            };

            if exit_code != 0 {
                std::fs::write(&tasks_path, &existing)
                    .with_context(|| format!("restore {}", tasks_path.display()))?;
                anyhow::bail!("plan regeneration agent failed with exit code {exit_code}");
            }

            let regenerated = match roko_cli::task_parser::TasksFile::parse(&tasks_path) {
                Ok(tasks) => tasks,
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            };

            let merged =
                preserve_completed_task_status(existing_tasks.as_ref(), regenerated, &plan_dir);
            let rendered =
                toml::to_string_pretty(&merged).context("serialize regenerated tasks.toml")?;
            if let Err(err) = std::fs::write(&tasks_path, rendered) {
                std::fs::write(&tasks_path, &existing)
                    .with_context(|| format!("restore {}", tasks_path.display()))?;
                return Err(err.into());
            }

            match roko_cli::task_parser::TasksFile::validate_modern_fields(&tasks_path) {
                Ok(issues) if !issues.is_empty() => {
                    // Collect post-regeneration diagnostics for richer error output.
                    let post_context = format_pre_validation_context(
                        &tasks_path,
                        &plan_validate::validate_plans_dir,
                    );
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    anyhow::bail!(
                        "regenerated tasks.toml is still missing modern fields after regeneration.\n\
                         Missing fields: {missing}\n\
                         Pre-regeneration issues:{pre}\n\
                         Post-regeneration issues:{post}",
                        missing = issues
                            .into_iter()
                            .map(|issue| format!("{}: {:?}", issue.task_id, issue.missing_fields))
                            .collect::<Vec<_>>()
                            .join("; "),
                        pre = pre_validation_context,
                        post = post_context,
                    );
                }
                Ok(_) => {}
                Err(err) => {
                    std::fs::write(&tasks_path, &existing)
                        .with_context(|| format!("restore {}", tasks_path.display()))?;
                    return Err(err);
                }
            }

            Ok(EXIT_SUCCESS)
        }
    }
}

#[allow(dead_code)]
fn resolve_effective_model_key(
    workdir: &Path,
    cli_model: Option<String>,
    role: Option<&str>,
    context: &str,
) -> Result<String> {
    let config = roko_core::config::loader::load_config_unified(workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let selection = roko_cli::model_selection::resolve_effective_model(
        cli_model,
        None,
        role.map(str::to_string),
        None,
        &config,
        None,
    )
    .map_err(|err| anyhow!("resolve model selection for {context}: {err}"))?;
    eprintln!("[{context}] effective selection: {}", selection.reason);
    Ok(selection.effective_model_key)
}

/// Parse and display a plan directory without executing anything.
pub(crate) async fn cmd_plan_dry_run(plans_dir: &Path, cli: &Cli) -> Result<i32> {
    let plans = roko_orchestrator::discover_plans(plans_dir)
        .map_err(|e| anyhow!("plan discovery failed: {e}"))?;

    if plans.is_empty() {
        if cli.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dry_run": true,
                    "plans": [],
                    "total_plans": 0,
                    "total_tasks": 0,
                }))?
            );
        } else {
            println!("No plans found in {}", plans_dir.display());
        }
        return Ok(EXIT_SUCCESS);
    }

    // For each plan, try to load and count tasks.
    let mut plan_summaries: Vec<serde_json::Value> = Vec::new();
    let mut total_tasks: usize = 0;
    let mut total_estimated_minutes: u32 = 0;

    for plan in &plans {
        // Try loading the tasks.toml adjacent to the plan file.
        let tasks_path = plan
            .path
            .parent()
            .map(|p| p.join("tasks.toml"))
            .filter(|p| p.exists());

        let (task_count, task_details) = if let Some(ref tp) = tasks_path {
            match roko_cli::task_parser::TasksFile::parse(tp) {
                Ok(tf) => {
                    let details: Vec<serde_json::Value> = tf
                        .tasks
                        .iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "title": t.title,
                                "status": t.status,
                                "tier": t.tier,
                                "depends_on": t.depends_on,
                                "files": t.files.len(),
                            })
                        })
                        .collect();
                    (tf.tasks.len(), details)
                }
                Err(_) => (0, vec![]),
            }
        } else {
            // New-layout plans might have tasks.toml at plans_dir/plan_name/tasks.toml
            let dir_tasks = plans_dir.join(&plan.base).join("tasks.toml");
            if dir_tasks.exists() {
                match roko_cli::task_parser::TasksFile::parse(&dir_tasks) {
                    Ok(tf) => {
                        let details: Vec<serde_json::Value> = tf
                            .tasks
                            .iter()
                            .map(|t| {
                                json!({
                                    "id": t.id,
                                    "title": t.title,
                                    "status": t.status,
                                    "tier": t.tier,
                                    "depends_on": t.depends_on,
                                    "files": t.files.len(),
                                })
                            })
                            .collect();
                        (tf.tasks.len(), details)
                    }
                    Err(_) => (0, vec![]),
                }
            } else {
                (0, vec![])
            }
        };

        total_tasks += task_count;
        if let Some(ref fm) = plan.frontmatter {
            if let Some(mins) = fm.estimated_minutes {
                total_estimated_minutes += mins;
            }
        }

        plan_summaries.push(json!({
            "plan": plan.base,
            "num": plan.num,
            "task_count": task_count,
            "estimated_minutes": plan.frontmatter.as_ref().and_then(|f| f.estimated_minutes),
            "parallel_width": plan.frontmatter.as_ref().and_then(|f| f.estimated_parallel_width),
            "priority": plan.frontmatter.as_ref().and_then(|f| f.priority),
            "tags": plan.frontmatter.as_ref().map(|f| &f.tags),
            "tasks": task_details,
        }));
    }

    if cli.json {
        let payload = json!({
            "dry_run": true,
            "plans_dir": plans_dir,
            "total_plans": plans.len(),
            "total_tasks": total_tasks,
            "total_estimated_minutes": total_estimated_minutes,
            "plans": plan_summaries,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!(
            "Dry run: {} plan(s), {} task(s) in {}\n",
            plans.len(),
            total_tasks,
            plans_dir.display()
        );

        for (i, plan) in plans.iter().enumerate() {
            let est = plan
                .frontmatter
                .as_ref()
                .and_then(|f| f.estimated_minutes)
                .map(|m| format!(" (~{m} min)"))
                .unwrap_or_default();
            let priority = plan
                .frontmatter
                .as_ref()
                .and_then(|f| f.priority)
                .map(|p| format!(" [priority={p}]"))
                .unwrap_or_default();
            println!("  {}. {}{}{}", i + 1, plan.base, est, priority);

            // Print task list if available.
            if let Some(tasks) = plan_summaries[i].get("tasks").and_then(|v| v.as_array()) {
                for t in tasks {
                    let tid = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let status = t
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    let tier = t.get("tier").and_then(|v| v.as_str()).unwrap_or("?");
                    let deps = t
                        .get("depends_on")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            let ids: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                            if ids.is_empty() {
                                String::new()
                            } else {
                                format!(" (after {})", ids.join(", "))
                            }
                        })
                        .unwrap_or_default();
                    println!("     {tid}: {title} [{tier}, {status}]{deps}");
                }
            }
        }

        if total_estimated_minutes > 0 {
            println!("\nEstimated total: ~{total_estimated_minutes} min");
        }
        println!("\nNo tasks were executed. Remove --dry-run to run the plan.");
    }

    Ok(EXIT_SUCCESS)
}

/// Run plan validation before `plan run` starts any agents.
///
/// Returns `Some(exit_code)` when validation fails, or `None` when the plan
/// set is valid enough to continue.
fn validate_before_run(plans_dir: &Path, workdir: &Path) -> Option<i32> {
    // If the plans directory doesn't exist yet (e.g. before `prd plan` runs),
    // skip pre-flight validation — the run path will report "No plans found".
    if !plans_dir.exists() {
        return None;
    }

    let config_path = workdir.join("roko.toml");
    let models = if config_path.is_file() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| toml::from_str::<roko_core::config::schema::RokoConfig>(&text).ok())
            .map(|config| crate::commands::config_cmd::configured_models(&config))
    } else {
        None
    };

    let report = match plan_validate::validate_plans_dir_with_workdir(
        plans_dir,
        models.as_ref(),
        Some(workdir),
    ) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: plan validation failed: {error:#}");
            return Some(1);
        }
    };

    // If no tasks.toml files were found, skip validation — the run path will
    // report "No plans found" with better context.
    if report.totals.plans_checked == 0 {
        return None;
    }

    let code = report.exit_code(false);
    if code != 0 {
        eprintln!("{}", plan_validate::render_text(&report));
        eprintln!("error: plan validation failed — fix the errors above before running");
        Some(1)
    } else {
        None
    }
}

pub(crate) fn cmd_plan_validate(
    dir: &Path,
    workdir: &Path,
    strict: bool,
    json_output: bool,
) -> Result<i32> {
    let config_path = workdir.join("roko.toml");
    let models = if config_path.is_file() {
        let config_text = std::fs::read_to_string(&config_path)
            .with_context(|| format!("read {}", config_path.display()))?;
        let config: RokoConfig = toml::from_str(&config_text)
            .map_err(|error| anyhow!(error))
            .with_context(|| format!("parse {}", config_path.display()))?;
        Some(crate::commands::config_cmd::configured_models(&config))
    } else {
        None
    };

    let report =
        plan_validate::validate_plans_dir_with_workdir(dir, models.as_ref(), Some(workdir))?;
    if json_output {
        println!("{}", plan_validate::render_json(&report)?);
    } else {
        println!("{}", plan_validate::render_text(&report));
    }
    Ok(report.exit_code(strict))
}

pub(crate) fn find_plan_source_document(plan_dir: &Path) -> Result<PathBuf> {
    for candidate in ["source-prd.md", "prd-extract.md", "plan.md"] {
        let path = plan_dir.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }

    anyhow::bail!(
        "no source PRD found in {} (looked for source-prd.md, prd-extract.md, and plan.md)",
        plan_dir.display()
    )
}

pub(crate) fn normalize_task_title(title: &str) -> String {
    title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub(crate) fn preserve_completed_task_status(
    old_tasks: Option<&roko_cli::task_parser::TasksFile>,
    mut regenerated: roko_cli::task_parser::TasksFile,
    plan_dir: &Path,
) -> roko_cli::task_parser::TasksFile {
    if let Some(old_tasks) = old_tasks {
        let completed: Vec<&roko_cli::task_parser::TaskDef> = old_tasks
            .tasks
            .iter()
            .filter(|task| task.status.eq_ignore_ascii_case("done"))
            .collect();

        for task in &mut regenerated.tasks {
            let normalized = normalize_task_title(&task.title);
            if completed.iter().any(|old| {
                old.id == task.id
                    || normalize_task_title(&old.title) == normalized
                    || normalize_task_title(&old.title).contains(&normalized)
                    || normalized.contains(&normalize_task_title(&old.title))
            }) {
                task.status = "done".to_string();
            }
        }

        regenerated.meta.iteration = old_tasks.meta.iteration.saturating_add(1);
        if regenerated.meta.plan.trim().is_empty() {
            regenerated.meta.plan = old_tasks.meta.plan.clone();
        }
    }

    if regenerated.meta.plan.trim().is_empty() {
        regenerated.meta.plan = plan_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown-plan".to_string());
    }

    regenerated.meta.total = regenerated.tasks.len() as u32;
    regenerated.meta.done = regenerated
        .tasks
        .iter()
        .filter(|task| task.status.eq_ignore_ascii_case("done"))
        .count() as u32;
    regenerated.meta.status =
        if regenerated.meta.total > 0 && regenerated.meta.done == regenerated.meta.total {
            "complete".to_string()
        } else {
            "ready".to_string()
        };

    regenerated
}

pub(crate) fn read_executor_state(
    workdir: &std::path::Path,
) -> Option<Vec<(String, usize, usize)>> {
    let executor_path = RokoLayout::for_project(workdir).executor_snapshot();
    if !executor_path.is_file() {
        return None;
    }

    let contents = std::fs::read_to_string(&executor_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&contents).ok()?;

    if let Some(plans) = value.get("plans").and_then(serde_json::Value::as_array) {
        let mut entries = Vec::with_capacity(plans.len());
        for plan in plans {
            let id = json_str_field(plan, &["plan_id", "id"]).unwrap_or("unknown");
            let tasks_done =
                json_usize_field(plan, &["tasks_completed", "completed_tasks"]).unwrap_or(0);
            let tasks_total =
                json_usize_field(plan, &["tasks_total", "total_tasks", "task_count"]).unwrap_or(0);
            entries.push((id.to_string(), tasks_done, tasks_total));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        return Some(entries);
    }

    if let Some(plan_states) = value
        .get("plan_states")
        .and_then(serde_json::Value::as_object)
    {
        let completed_counts = read_run_state_completed_counts(workdir);
        let discovered_totals = discovered_plan_totals(workdir);
        let mut entries = Vec::with_capacity(plan_states.len());

        for (plan_id, plan_state) in plan_states {
            let tasks_total = discovered_totals.get(plan_id).copied().unwrap_or_else(|| {
                json_usize_field(plan_state, &["tasks_total", "total_tasks", "task_count"])
                    .unwrap_or(0)
            });
            let mut tasks_done = completed_counts.get(plan_id).copied().unwrap_or(0);
            if tasks_done == 0
                && tasks_total > 0
                && json_str_field(
                    plan_state
                        .get("current_phase")
                        .unwrap_or(&serde_json::Value::Null),
                    &["kind"],
                )
                .is_some_and(|kind| {
                    kind.eq_ignore_ascii_case("complete") || kind.eq_ignore_ascii_case("completed")
                })
            {
                tasks_done = tasks_total;
            }
            entries.push((plan_id.clone(), tasks_done, tasks_total));
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        return Some(entries);
    }

    if let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) {
        let mut progress: std::collections::BTreeMap<String, (usize, usize)> =
            std::collections::BTreeMap::new();
        for task in tasks {
            let Some(plan_id) = json_str_field(task, &["plan", "plan_id"]) else {
                continue;
            };
            let entry = progress.entry(plan_id.to_string()).or_insert((0, 0));
            entry.0 += 1;

            let status = task
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(
                status.as_str(),
                "done" | "complete" | "completed" | "passed" | "skipped"
            ) {
                entry.1 += 1;
            }
        }

        return Some(
            progress
                .into_iter()
                .map(|(plan_id, (tasks_total, tasks_done))| (plan_id, tasks_done, tasks_total))
                .collect(),
        );
    }

    Some(Vec::new())
}

fn discovered_plan_totals(workdir: &std::path::Path) -> std::collections::HashMap<String, usize> {
    roko_cli::plan::summarize_discovered_plans(workdir)
        .ok()
        .map(|summaries| {
            summaries
                .into_iter()
                .map(|summary| (summary.id, summary.task_count))
                .collect()
        })
        .unwrap_or_default()
}

fn read_run_state_completed_counts(
    workdir: &std::path::Path,
) -> std::collections::HashMap<String, usize> {
    let run_state_path = RokoLayout::for_project(workdir).run_state_path();
    let Ok(contents) = std::fs::read_to_string(&run_state_path) else {
        return std::collections::HashMap::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return std::collections::HashMap::new();
    };
    let Some(completed_tasks) = value
        .get("completed_tasks")
        .and_then(serde_json::Value::as_object)
    else {
        return std::collections::HashMap::new();
    };

    completed_tasks
        .iter()
        .map(|(plan_id, tasks)| {
            (
                plan_id.clone(),
                tasks.as_array().map_or(0, std::vec::Vec::len),
            )
        })
        .collect()
}

fn json_str_field<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
}

fn json_usize_field(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
        .map(|count| count as usize)
}

pub(crate) fn plan_path_exists(workdir: &std::path::Path, plan_id: &str) -> bool {
    let plan_dir = workdir.join("plans").join(plan_id);
    let roko_plan_dir = RokoLayout::for_project(workdir).plan_dir(plan_id);
    plan_dir.exists() || roko_plan_dir.exists()
}

/// Format validation diagnostics for a tasks.toml file into a string context
/// block suitable for embedding in agent prompts and error messages.
fn format_pre_validation_context(
    tasks_path: &std::path::Path,
    validate_fn: &dyn Fn(
        &std::path::Path,
        Option<&indexmap::IndexMap<String, roko_core::config::ModelProfile>>,
    ) -> anyhow::Result<crate::plan_validate::ValidationReport>,
) -> String {
    let parent = tasks_path.parent().unwrap_or(tasks_path);
    match validate_fn(parent, None) {
        Ok(report) => {
            let issues: Vec<String> = report
                .plans
                .iter()
                .flat_map(|p| {
                    p.diagnostics
                        .iter()
                        .map(move |d| format!("  - [{}] {}", p.plan_id, d.message))
                })
                .collect();
            if issues.is_empty() {
                String::new()
            } else {
                format!("\n\n## Validation issues\n\n{}", issues.join("\n"))
            }
        }
        Err(_) => String::new(),
    }
}

/// Execute plans via the Graph Engine path.
///
/// Loads plans using the Runner v2 plan_loader, converts each to a Graph
/// via `roko_graph::convert::plan_to_graph`, and runs them through the
/// GraphEngine with the default cell registry.
async fn cmd_plan_run_engine(
    plans_dir: &std::path::Path,
    _workdir: &std::path::Path,
    cli: &Cli,
) -> Result<i32> {
    use roko_graph::convert::{PlanTaskInfo, plan_to_graph};
    use roko_graph::engine::GraphEngine;
    use roko_graph::cell::CellContext;

    let plans = roko_cli::runner::plan_loader::load_plans(plans_dir)?;

    let total_tasks: usize = plans.iter().map(|p| p.tasks.tasks.len()).sum();
    let plan_count = plans.len();

    if !cli.quiet && !cli.json {
        let plan_names: Vec<&str> = plans.iter().map(|p| p.id.as_str()).collect();
        eprintln!(
            "\u{25b8} Running plan{} via Graph Engine ({} task{}): {}",
            if plan_count == 1 { "" } else { "s" },
            total_tasks,
            if total_tasks == 1 { "" } else { "s" },
            plan_names.join(", "),
        );
    }

    let mut all_succeeded = true;
    let mut total_output_count = 0usize;

    for plan in &plans {
        if !cli.quiet && !cli.json {
            eprintln!(
                "  Running plan '{}' via Graph Engine ({} tasks)...",
                plan.id,
                plan.tasks.tasks.len()
            );
        }

        // Convert Runner v2 tasks into PlanTaskInfo for the converter.
        let tasks: Vec<(String, PlanTaskInfo)> = plan
            .tasks
            .tasks
            .iter()
            .map(|t| {
                let info = PlanTaskInfo {
                    title: t.title.clone(),
                    description: t.description.clone(),
                    role: t.role.clone(),
                    tier: t.tier.clone(),
                    model_hint: t.model_hint.clone(),
                    files: t.files.clone(),
                    depends_on: t.depends_on.clone(),
                    depends_on_plan: t.depends_on_plan.clone(),
                    timeout_secs: t.timeout_secs,
                    max_retries: t.max_retries,
                    domain: t.domain.as_ref().map(|d| format!("{d:?}")),
                    sequence: t.sequence,
                    full_config_json: serde_json::to_value(t).unwrap_or_default(),
                };
                (t.id.clone(), info)
            })
            .collect();

        let max_parallel = plan.tasks.meta.max_parallel;
        let plan_dir_str = plan.dir.display().to_string();

        let graph = match plan_to_graph(&plan.id, &plan_dir_str, &tasks, max_parallel) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("  error: failed to convert plan '{}' to graph: {e}", plan.id);
                all_succeeded = false;
                continue;
            }
        };

        let registry = roko_graph::default_registry();
        let engine = GraphEngine::new(graph, registry);
        let ctx = CellContext::new();

        // Validate before running.
        let issues = engine.validate();
        if !issues.is_empty() {
            eprintln!("  validation errors for plan '{}':", plan.id);
            for issue in &issues {
                eprintln!("    - {issue}");
            }
            all_succeeded = false;
            continue;
        }

        match engine.execute(&ctx).await {
            Ok(output) => {
                let output_count = output
                    .node_results
                    .iter()
                    .map(|r| r.output_count)
                    .sum::<usize>();
                total_output_count += output_count;

                if !cli.quiet && !cli.json {
                    eprintln!(
                        "  Plan '{}' completed: {} nodes, {} output signals, {}",
                        plan.id,
                        output.node_results.len(),
                        output_count,
                        if output.success { "SUCCESS" } else { "FAILED" },
                    );
                }
                if !output.success {
                    all_succeeded = false;
                }
            }
            Err(e) => {
                eprintln!("  error: plan '{}' execution failed: {e}", plan.id);
                all_succeeded = false;
            }
        }
    }

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "engine": "graph",
                "succeeded": all_succeeded,
                "plan_count": plan_count,
                "total_tasks": total_tasks,
                "total_outputs": total_output_count,
            }))
            .unwrap_or_default()
        );
    } else if !cli.quiet {
        eprintln!(
            "\n\u{25b8} Graph Engine complete: {} plan{}, {} tasks, {} output signals",
            plan_count,
            if plan_count == 1 { "" } else { "s" },
            total_tasks,
            total_output_count,
        );
    }

    Ok(if all_succeeded {
        EXIT_SUCCESS
    } else {
        EXIT_FAILURE
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn read_executor_state_returns_none_without_snapshot() {
        let dir = tempdir().expect("tempdir");
        assert!(read_executor_state(dir.path()).is_none());
    }

    #[test]
    fn read_executor_state_parses_plans_array() {
        let dir = tempdir().expect("tempdir");
        let state_dir = RokoLayout::for_project(dir.path()).state_dir();
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(
            state_dir.join("executor.json"),
            r#"{"plans":[{"plan_id":"plan-a","tasks_completed":1,"tasks_total":3}]}"#,
        )
        .expect("write executor state");

        let state = read_executor_state(dir.path()).expect("state");
        assert_eq!(state, vec![("plan-a".to_string(), 1, 3)]);
    }
}
