//! util command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) fn cmd_explain(topic: &str, depth: u8) {
    use roko_cli::explain;
    let depth = depth.clamp(1, 3);
    if topic == "topics" || topic == "list" {
        println!("available topics:");
        for name in explain::topic_names() {
            let entry = explain::find_topic(name).unwrap();
            println!("  {:<12} {}", name, entry.title);
        }
        return;
    }
    match explain::find_topic(topic) {
        Some(entry) => print!("{}", explain::render_topic(entry, depth)),
        None => {
            eprintln!("unknown topic: {topic}");
            eprintln!("available topics: {}", explain::topic_names().join(", "));
            eprintln!("run `roko explain topics` to see all topics with descriptions");
        }
    }
}

#[allow(dead_code)]
pub(crate) fn cmd_repl(cli: &Cli) -> Result<i32> {
    let session_id = cli
        .resume
        .clone()
        .unwrap_or_else(|| format!("repl-{}", std::process::id()));
    let mut repl = ReplMode::new(session_id);

    let _commands = repl
        .run(&mut std::io::stdin().lock(), &mut std::io::stdout().lock())
        .map_err(|e| anyhow!("repl I/O error: {e}"))?;

    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_oneshot(cli: &Cli, prompt: &str) -> Result<i32> {
    let mode = OneshotMode::new(prompt.to_string())
        .with_json(cli.json)
        .with_quiet(cli.quiet);

    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    let mut config = resolve_config(cli)?;
    apply_resume_session_override(&mut config, cli.resume.clone());

    let report = run_once(&workdir, &config, &mode.prepare().prompt, None, None).await?;
    let result = mode.format_result(
        report.overall_success(),
        &format!(
            "episode={} signals={}",
            report.episode_id, report.total_signals
        ),
    );
    if !result.summary.is_empty() {
        println!("{}", result.summary);
    }
    Ok(result.exit_code)
}

pub(crate) async fn cmd_pipe(cli: &Cli) -> Result<i32> {
    let pipe = PipeMode::new().with_json(cli.json).with_quiet(cli.quiet);

    let input = pipe
        .read_input(&mut std::io::stdin().lock())
        .map_err(|e| anyhow!("read stdin: {e}"))?;

    if input.text.is_empty() {
        if !cli.quiet {
            eprintln!("no input received on stdin");
        }
        return Ok(EXIT_SYSTEM_ERROR);
    }

    if input.truncated && !cli.quiet {
        eprintln!(
            "warning: stdin input truncated at {} bytes",
            input.bytes_read
        );
    }

    // Dispatch the piped text as a one-shot prompt.
    cmd_oneshot(cli, &input.text).await
}

pub(crate) async fn cmd_headless(cli: &Cli) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    roko_cli::daemon::daemon_start(&workdir, false, roko_cli::DEFAULT_SERVE_PORT).await?;
    Ok(EXIT_SUCCESS)
}

pub(crate) async fn cmd_init(
    path: Option<PathBuf>,
    cloud: bool,
    profile: Option<String>,
) -> Result<()> {
    let target = path.unwrap_or_else(|| PathBuf::from("."));
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let roko_dir = target.join(".roko");
    tokio::fs::create_dir_all(&roko_dir)
        .await
        .with_context(|| format!("create {}", roko_dir.display()))?;

    // Create all top-level layout directories and VERSION file via RokoLayout.
    // This ensures doctor checks pass and all subsystems have their dirs.
    let layout = RokoLayout::for_project(&target);
    layout
        .ensure_dirs()
        .await
        .with_context(|| "create .roko layout directories")?;

    // Create additional directories used by CLI subsystems but not in
    // RokoLayout::top_level_dirs() (jobs, prd, task-outputs, etc.).
    for extra in &[
        roko_dir.join("jobs"),
        roko_dir.join("prd"),
        roko_dir.join("prd").join("published"),
        roko_dir.join("prd").join("drafts"),
        roko_dir.join("task-outputs"),
        roko_dir.join("research"),
        roko_dir.join("subscriptions"),
        roko_dir.join("templates"),
    ] {
        tokio::fs::create_dir_all(extra)
            .await
            .with_context(|| format!("create {}", extra.display()))?;
    }

    let engrams_path = roko_dir.join("engrams.jsonl");
    if !engrams_path.exists() {
        // Migrate from legacy name if present.
        let legacy = roko_dir.join("signals.jsonl");
        if legacy.exists() {
            tokio::fs::rename(&legacy, &engrams_path)
                .await
                .with_context(|| {
                    format!("migrate {} -> {}", legacy.display(), engrams_path.display())
                })?;
        } else {
            tokio::fs::write(&engrams_path, b"")
                .await
                .with_context(|| format!("create {}", engrams_path.display()))?;
        }
    }

    // Domain detection: use --profile if given, otherwise auto-detect.
    let domain = if let Some(ref p) = profile {
        p.as_str()
    } else {
        crate::commands::prd::detect_project_domain(&target)
    };

    let config_path = target.join("roko.toml");
    if config_path.exists() {
        println!(
            "{} already exists; leaving untouched.",
            config_path.display()
        );
    } else {
        let default = Config::default_toml_template(cloud)?;
        tokio::fs::write(&config_path, default)
            .await
            .with_context(|| format!("write {}", config_path.display()))?;
        println!("wrote {}", config_path.display());
    }

    println!("initialized roko workspace at {}", target.display());
    println!("detected project domain: {domain}");
    println!(
        "suggested gates: {}",
        crate::commands::prd::domain_gate_hint(domain)
    );
    println!(
        "default provider command set to \"claude\". \
         Edit roko.toml [providers.claude_cli] to use a different command."
    );

    // Check for interrupted session from a previous run.
    let snapshot = roko_dir.join("state").join("executor.json");
    if snapshot.is_file() {
        println!();
        println!("interrupted session found: {}", snapshot.display());
        println!(
            "resume with: roko plan run plans/ --resume {}",
            snapshot.display()
        );
    }

    Ok(())
}

pub(crate) async fn cmd_run(
    cli: &Cli,
    workdir: Option<PathBuf>,
    prompt: String,
    serve: bool,
    share: bool,
    engine: crate::EngineVariant,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);
    let mut config = resolve_config_for_workdir(cli, &workdir)?;
    apply_resume_session_override(&mut config, cli.resume.clone());

    // Optionally start the HTTP control plane for external observability.
    let server_guard: Option<(
        std::sync::Arc<roko_serve::state::AppState>,
        tokio::task::JoinHandle<anyhow::Result<()>>,
    )> = if serve || share {
        let repo_registry = RepoRegistry::load(&config, &workdir).unwrap_or_default();
        let runtime =
            roko_cli::serve_runtime::RokoCliRuntime::new(config.clone(), repo_registry).into_arc();
        let (state, handle) =
            roko_serve::start_server_background(workdir.clone(), runtime, None, None).await?;
        if !cli.quiet {
            eprintln!("▸ HTTP server started on :6677");
        }
        Some((state, handle))
    } else {
        None
    };

    // TODO(converge): When --serve is active we should share the server's StateHub
    // so DashboardEvents flow to the HTTP server's SSE/WebSocket/snapshot endpoints.
    // Currently roko_serve::StateHub and roko_cli::state_hub::StateHub are distinct
    // types (same source included via #[path] in both crates). Bridge them once
    // roko-core re-exports StateHub from its crate root.
    let external_hub: Option<&roko_cli::state_hub::StateHub> = None;

    if engine == crate::EngineVariant::V2 {
        // TODO(W03): expose workflow_template via Config.
        let template = "standard";

        // Build enabled gates list and typed shell commands from declared gate configs.
        let enabled_gates = roko_cli::run::workflow_enabled_gate_names(&config.gates);
        let shell_gates = roko_cli::run::workflow_shell_gate_commands(&config.gates);

        let result = roko_cli::run::run_workflow_engine_report_with_hub(
            &prompt,
            &workdir,
            template,
            enabled_gates,
            shell_gates,
            None,
        )
        .await;

        // Shut down the HTTP server if it was started.
        if let Some((state, handle)) = server_guard {
            state.cancel.cancel();
            let _ = handle.await;
        }

        return match result {
            Ok(report) => {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else if !cli.quiet {
                    roko_cli::run::print_workflow_run_report(&prompt, template, &report);
                }

                if share {
                    if let Err(err) = roko_cli::run::write_shared_workflow_run(
                        &workdir,
                        &prompt,
                        &config.agent.command,
                        &config.prompt.role,
                        &report,
                    ) {
                        if !cli.quiet {
                            eprintln!("share failed: {err}");
                        }
                    }
                }

                if report.success {
                    Ok(EXIT_SUCCESS)
                } else {
                    Ok(EXIT_AGENT_FAILURE)
                }
            }
            Err(e) => {
                if !cli.quiet {
                    eprintln!("workflow engine error: {e:#}");
                }
                Ok(EXIT_AGENT_FAILURE)
            }
        };
    }

    // Use inline rendering when stdout is a TTY and we're not in --json or --quiet mode.
    if !cli.json && !cli.quiet && roko_cli::inline::should_use_inline() {
        let start = std::time::Instant::now();
        let report =
            roko_cli::run_inline::run_once_inline(&workdir, &config, &prompt, external_hub).await?;

        // Share the run transcript when --share is active.
        if share {
            let elapsed = start.elapsed().as_secs_f64();
            match roko_cli::share::share_run(
                &workdir,
                &report,
                &prompt,
                &config.agent.command,
                &config.prompt.role,
                elapsed,
            ) {
                Ok(result) => {
                    eprintln!();
                    eprintln!(
                        "  {} share  {}",
                        roko_cli::inline::symbols::PASS,
                        result.url,
                    );
                    if result.backend == "local" {
                        eprintln!(
                            "  {} saved to {}",
                            roko_cli::inline::symbols::INFO,
                            result.local_path,
                        );
                        eprintln!("  (install gh CLI for automatic Gist upload)");
                    }
                    eprintln!();
                }
                Err(err) => {
                    eprintln!("  share failed: {err}");
                }
            }
        }

        // Shut down the HTTP server if it was started.
        if let Some((state, handle)) = server_guard {
            state.cancel.cancel();
            let _ = handle.await;
        }

        return if report.overall_success() {
            Ok(0)
        } else {
            Ok(1)
        };
    }

    // Legacy output path (--json, --quiet, or non-TTY)
    if !cli.quiet {
        println!(
            "running agent `{}` with {} gate(s)",
            config.agent.command,
            config.gates.len()
        );
    }
    let report = run_once(&workdir, &config, &prompt, None, external_hub).await?;

    if cli.json {
        println!(
            r#"{{"success":{},"episode":"{}","prompt":"{}","agent_output":"{}","signals":{}}}"#,
            report.overall_success(),
            report.episode_id,
            report.prompt_id,
            report.agent_output_id,
            report.total_signals,
        );
    } else if !cli.quiet {
        println!("---");
        println!(
            "agent        : {} (success={})",
            config.agent.command, report.agent_success
        );
        println!("prompt_id    : {}", report.prompt_id);
        println!("agent_output : {}", report.agent_output_id);
        if report.gate_verdicts.is_empty() {
            println!("gates        : (none configured)");
        } else {
            println!("gates:");
            for (name, ok) in &report.gate_verdicts {
                let marker = if *ok { "PASS" } else { "FAIL" };
                println!("  [{marker}] {name}");
            }
        }
        println!("episode      : {}", report.episode_id);
        println!("signals      : {}", report.total_signals);
    }

    // Shut down the HTTP server if it was started.
    if let Some((state, handle)) = server_guard {
        state.cancel.cancel();
        let _ = handle.await;
    }

    if report.overall_success() {
        Ok(EXIT_SUCCESS)
    } else {
        Ok(EXIT_AGENT_FAILURE)
    }
}

pub(crate) async fn cmd_status(
    cli: &Cli,
    workdir: Option<PathBuf>,
    cfactor: bool,
    surfaces: bool,
) -> Result<()> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    if surfaces {
        let inventory = roko_cli::surface_inventory::full_inventory();
        roko_cli::surface_inventory::print_table(&inventory, cli.json);
        return Ok(());
    }

    if !cli.quiet {
        tracing::info!(
            workdir = %workdir.display(),
            json = cli.json,
            cfactor,
            "collecting status snapshot"
        );
    }
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let ctx = Context::now();

    let all = substrate
        .query(&Query::all(), &ctx)
        .await
        .map_err(|e| anyhow!("query: {e}"))?;

    let cfactor_snapshot = if cfactor {
        Some(
            refresh_cfactor_snapshot(workdir.join(".roko").join("learn"))
                .await
                .map_err(|e| anyhow!("refresh c-factor snapshot: {e}"))?,
        )
    } else {
        None
    };
    let cfactor_history = if cfactor_snapshot.is_some() {
        crate::commands::dashboard::load_cfactor_history(
            workdir.join(".roko").join("learn").join("c-factor.jsonl"),
        )
        .await
    } else {
        Vec::new()
    };
    let cfactor_trend = if cfactor_snapshot.is_some() {
        cfactor_trend_arrow(&cfactor_history, Duration::from_secs(7 * 24 * 60 * 60))
    } else {
        "→"
    };
    let learn_dir = workdir.join(".roko").join("learn");
    let costs_log = CostsLog::at(learn_dir.join("costs.jsonl"));
    let total_cost_usd = costs_log.total_cost().await.ok();
    let today_cost_usd = costs_log
        .daily_cost(1)
        .await
        .ok()
        .and_then(|days| days.last().map(|(_, cost)| *cost));
    let cost_by_model = costs_log.cost_by_model().await.unwrap_or_default();
    let cost_by_plan = costs_log.cost_by_plan().await.unwrap_or_default();

    if cli.json {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for sig in &all {
            *counts.entry(sig.kind.to_string()).or_default() += 1;
        }
        let episode_count = counts.get("episode").copied().unwrap_or(0);

        // Verify verdicts from substrate.
        let verdicts_json = substrate
            .query(&Query::of_kind(Kind::GateVerdict), &ctx)
            .await
            .map_err(|e| anyhow!("query verdicts: {e}"))?;
        let gate_pass = verdicts_json
            .iter()
            .filter(|v| v.tag("passed") == Some("true"))
            .count();
        let gate_fail = verdicts_json
            .iter()
            .filter(|v| v.tag("passed") == Some("false"))
            .count();

        // Running agents from runtime directory.
        let runtime_dir_json = workdir.join(".roko").join("runtime");
        let mut running_agents_json: usize = 0;
        if let Ok(mut entries) = tokio::fs::read_dir(&runtime_dir_json).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".pid") {
                    running_agents_json += 1;
                }
            }
        }

        // Active plans from executor snapshot.
        let executor_path_json = workdir.join(".roko").join("state").join("executor.json");
        let active_plans_json: usize = if executor_path_json.is_file() {
            tokio::fs::read_to_string(&executor_path_json)
                .await
                .ok()
                .and_then(|contents| {
                    serde_json::from_str::<serde_json::Value>(&contents)
                        .ok()?
                        .get("plans")?
                        .as_array()
                        .map(|arr| arr.len())
                })
                .unwrap_or(0)
        } else {
            0
        };

        // Most recent episode.
        let mut episodes_json = substrate
            .query(&Query::of_kind(Kind::Episode), &ctx)
            .await
            .map_err(|e| anyhow!("query episodes: {e}"))?;
        episodes_json.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
        let last_passed = episodes_json
            .first()
            .and_then(|ep| ep.tag("passed").map(|v| v == "true"));

        let status = SessionStatus {
            session_id: cli.resume.clone(),
            workdir: workdir.clone(),
            daemon_running: false,
            signal_count: Some(all.len()),
            episode_count: Some(episode_count),
            last_episode_passed: last_passed,
            cfactor: cfactor_snapshot,
            total_cost_usd,
            today_cost_usd,
            process_session_ledger: None,
            process_sessions: None,
        };

        // Build enriched JSON with gate verdicts, workspace info, and signal counts.
        let counts_json = serde_json::to_string(&counts).unwrap_or_else(|_| "{}".to_string());
        let cost_by_model_json =
            serde_json::to_string(&cost_by_model).unwrap_or_else(|_| "{}".to_string());
        let cost_by_plan_json =
            serde_json::to_string(&cost_by_plan).unwrap_or_else(|_| "{}".to_string());
        let base = status.display_json();
        // Splice additional fields before the closing brace.
        let enriched = format!(
            "{},\"gates\":{{\"pass\":{gate_pass},\"fail\":{gate_fail}}},\"workspace\":{{\"agents\":{running_agents_json},\"plans\":{active_plans_json}}},\"signal_counts\":{counts_json},\"cost_by_model\":{cost_by_model_json},\"cost_by_plan\":{cost_by_plan_json},\"health\":\"ready\"}}",
            &base[..base.len() - 1],
        );
        println!("{enriched}");
        return Ok(());
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for sig in &all {
        *counts.entry(sig.kind.to_string()).or_default() += 1;
    }

    println!("signal counts ({} total):", all.len());
    if counts.is_empty() {
        println!("  (empty)");
    } else {
        for (kind, n) in &counts {
            println!("  {kind:<24} {n}");
        }
    }

    // Running agents from runtime directory.
    let runtime_dir = workdir.join(".roko").join("runtime");
    let mut running_agents: usize = 0;
    if let Ok(mut entries) = tokio::fs::read_dir(&runtime_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".pid") {
                running_agents += 1;
            }
        }
    }

    // Active plans from executor snapshot.
    let executor_path = workdir.join(".roko").join("state").join("executor.json");
    let active_plans: usize = if executor_path.is_file() {
        // Parse minimally: count plans with active=true.
        match tokio::fs::read_to_string(&executor_path).await {
            Ok(contents) => {
                // Quick JSON parse: count occurrences of "active":true or
                // plan entries. For a lightweight check, use serde_json::Value.
                serde_json::from_str::<serde_json::Value>(&contents)
                    .ok()
                    .and_then(|val| val.get("plans")?.as_array().map(|arr| arr.len()))
                    .unwrap_or(0)
            }
            Err(_) => 0,
        }
    } else {
        0
    };

    println!();
    println!(
        "workspace: {} agent pid(s), {} plan(s) in executor snapshot",
        running_agents, active_plans
    );

    let mut episodes = substrate
        .query(&Query::of_kind(Kind::Episode), &ctx)
        .await
        .map_err(|e| anyhow!("query episodes: {e}"))?;
    episodes.sort_by_key(|s| std::cmp::Reverse(s.created_at_ms));
    println!();
    match episodes.first() {
        Some(ep) => {
            println!(
                "most recent episode: {} (passed={})",
                ep.id,
                ep.tag("passed").unwrap_or("?")
            );
            println!(
                "  gates passed={} failed={}",
                ep.tag("gates_passed").unwrap_or("0"),
                ep.tag("gates_failed").unwrap_or("0")
            );
        }
        None => println!("most recent episode: (none)"),
    }

    let verdicts = substrate
        .query(&Query::of_kind(Kind::GateVerdict), &ctx)
        .await
        .map_err(|e| anyhow!("query verdicts: {e}"))?;
    let passed = verdicts
        .iter()
        .filter(|v| v.tag("passed") == Some("true"))
        .count();
    let failed = verdicts
        .iter()
        .filter(|v| v.tag("passed") == Some("false"))
        .count();
    println!("gate verdicts: {passed} pass / {failed} fail");

    // Learning subsystem stats.
    let efficiency_path = learn_dir.join("efficiency.jsonl");
    match read_efficiency_events(&efficiency_path).await {
        Ok(events) if !events.is_empty() => {
            println!();
            println!("efficiency events: {} total", events.len());
            let profiles = compute_role_profiles(&events);
            for p in &profiles {
                println!(
                    "  {:<16} avg_cost=${:.4}  p95_cost=${:.4}  pass_rate={:.0}%  n={}",
                    p.role,
                    p.avg_cost_usd,
                    p.p95_cost_usd,
                    p.pass_rate * 100.0,
                    p.observations,
                );
            }
        }
        _ => {}
    }

    // Experiment store summary.
    let experiments_path = learn_dir.join("experiments.json");
    let exp_store = ExperimentStore::load_or_new(&experiments_path);
    let running = exp_store.running_count();
    let concluded = exp_store.concluded_count();
    if running > 0 || concluded > 0 {
        println!();
        println!("prompt experiments: {running} running, {concluded} concluded");
    }

    // Adaptive threshold summary.
    let thresholds_path = learn_dir.join("gate-thresholds.json");
    let thresholds =
        roko_gate::adaptive_threshold::AdaptiveThresholds::load_or_new(&thresholds_path);
    let rung_count: usize = thresholds.all_rungs().count();
    if rung_count > 0 {
        println!();
        println!("adaptive gate thresholds: {rung_count} rungs tracked");
        for (rung, stats) in thresholds.all_rungs() {
            println!(
                "  rung {rung}: pass_rate={:.0}% retries={} obs={} skip={}",
                stats.ema_pass_rate * 100.0,
                thresholds.suggested_max_retries(*rung),
                stats.total_observations,
                if thresholds.should_skip_rung(*rung) {
                    "yes"
                } else {
                    "no"
                },
            );
        }
    }

    if total_cost_usd.is_some() || !cost_by_model.is_empty() || !cost_by_plan.is_empty() {
        println!();
        println!("Cost Summary:");
        if let Some(total_cost_usd) = total_cost_usd {
            println!("  Total:    ${total_cost_usd:.4}");
        }
        if let Some(today_cost_usd) = today_cost_usd {
            println!("  Today:    ${today_cost_usd:.4}");
        }
        if !cost_by_model.is_empty() {
            println!("  By model: {}", format_cost_breakdown(&cost_by_model, 5));
        }
        if !cost_by_plan.is_empty() {
            println!("  By plan:  {}", format_cost_breakdown(&cost_by_plan, 5));
        }
    }

    // Health probes — quick snapshot of orchestrator readiness.
    let health_probes = roko_core::obs::health::ProbeRegistry::new();
    health_probes.register(std::sync::Arc::new(
        roko_core::obs::health::AlwaysUpProbe::new("orchestrator"),
    ));
    let (readiness_status, degraded_reasons) = health_probes.readiness();
    println!();
    println!("health: {readiness_status}");
    if !degraded_reasons.is_empty() {
        for reason in &degraded_reasons {
            println!("  {} — {}", reason.component, reason.message);
        }
    }

    if let Some(cfactor) = cfactor_snapshot {
        println!();
        println!(
            "c-factor: {:.3} | trend={} | episodes={} | computed={}",
            cfactor.overall, cfactor_trend, cfactor.episode_count, cfactor.computed_at
        );
        println!(
            "  gate={:.3} cost={:.3} speed={:.3} flow={:.3} first_try={:.3} knowledge={:.3} integration={:.3} convergence={:.3} turn={:.3} social={:.3}",
            cfactor.components.gate_pass_rate,
            cfactor.components.cost_efficiency,
            cfactor.components.speed,
            cfactor.components.information_flow_rate,
            cfactor.components.first_try_rate,
            cfactor.components.knowledge_growth,
            cfactor.components.knowledge_integration_rate,
            cfactor.components.convergence_velocity,
            cfactor.components.turn_taking_equality,
            cfactor.components.social_perceptiveness
        );
        if !cfactor.agent_contributions.is_empty() {
            println!(
                "  agent contributions: {}",
                cfactor.top_agent_contribution_lines(3).join(", ")
            );
        }
    }

    Ok(())
}

pub(crate) async fn cmd_doctor(
    cli: &Cli,
    workdir: Option<PathBuf>,
    serve_url: Option<String>,
) -> Result<i32> {
    let report = roko_cli::doctor::run_doctor(&roko_cli::doctor::DoctorOptions {
        workdir: workdir.unwrap_or_else(|| resolve_workdir(cli)),
        config_override: cli.config.clone(),
        serve_url,
    })
    .await?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report.render_human());
    }

    Ok(report.exit_code())
}

pub(crate) fn format_cost_breakdown(costs: &HashMap<String, f64>, limit: usize) -> String {
    let mut entries = costs
        .iter()
        .map(|(name, cost)| (name.as_str(), *cost))
        .collect::<Vec<_>>();
    entries.sort_by(|(left_name, left_cost), (right_name, right_cost)| {
        right_cost
            .total_cmp(left_cost)
            .then_with(|| left_name.cmp(right_name))
    });
    entries.truncate(limit);
    if entries.is_empty() {
        return "none".to_string();
    }

    entries
        .into_iter()
        .map(|(name, cost)| format!("{name}=${cost:.4}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) async fn cmd_replay(
    workdir: Option<PathBuf>,
    hash: String,
    forensic: bool,
    as_of: Option<String>,
    format: String,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .map_err(|e| anyhow!("open substrate: {e}"))?;
    let start = ContentHash::from_hex(&hash)
        .ok_or_else(|| anyhow!("invalid hash (expected 64 hex chars): {hash}"))?;

    // Parse --as-of filter: skip signals until this depth/index.
    let skip_until: usize = as_of
        .as_deref()
        .and_then(|s| {
            // Accept "step 5", "step05", "5", "#5"
            let stripped = s.trim_start_matches("step").trim_start_matches('#').trim();
            stripped.parse().ok()
        })
        .unwrap_or(0);

    let is_json = format == "json";

    let mut visited = std::collections::HashSet::new();
    let mut queue = vec![(start, 0usize)];
    let mut printed = 0usize;
    let mut index = 0usize;

    while let Some((id, depth)) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(sig) = substrate.get(&id).await.map_err(|e| anyhow!("get: {e}"))? {
            index += 1;

            // Apply --as-of filter: skip events before the target index.
            if index < skip_until {
                for parent in &sig.lineage {
                    queue.push((*parent, depth + 1));
                }
                continue;
            }

            if is_json {
                // JSON output: one JSON object per line.
                let mut obj = serde_json::Map::new();
                obj.insert("event".into(), serde_json::json!(index));
                obj.insert("hash".into(), serde_json::json!(sig.id.to_string()));
                obj.insert("kind".into(), serde_json::json!(sig.kind.to_string()));
                obj.insert("author".into(), serde_json::json!(sig.provenance.author));
                obj.insert("created_at_ms".into(), serde_json::json!(sig.created_at_ms));
                if !sig.tags.is_empty() {
                    obj.insert("tags".into(), serde_json::json!(sig.tags));
                }
                if let Ok(text) = sig.body.as_text() {
                    let preview: String = text.chars().take(500).collect();
                    obj.insert("body".into(), serde_json::json!(preview));
                }
                println!("{}", serde_json::Value::Object(obj));
            } else if forensic {
                let indent = "  ".repeat(depth);
                println!("{indent}{} {}", sig.kind, sig.id);
                println!("{indent}  event:     {index}");
                println!("{indent}  hash:      {}", sig.id);
                println!("{indent}  author:    {}", sig.provenance.author);
                println!("{indent}  created:   {}", sig.created_at_ms);
                println!(
                    "{indent}  lineage:   [{}]",
                    sig.lineage
                        .iter()
                        .map(|h| h.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                if !sig.tags.is_empty() {
                    println!("{indent}  tags:      {:?}", sig.tags);
                }
                if let Ok(text) = sig.body.as_text() {
                    let body_preview: String = text.chars().take(120).collect();
                    println!("{indent}  body:      {body_preview}");
                }
                println!();
            } else {
                let indent = "  ".repeat(depth);
                println!(
                    "{indent}{} {}  (event={index}, author={})",
                    sig.kind, sig.id, sig.provenance.author
                );
            }
            for parent in &sig.lineage {
                queue.push((*parent, depth + 1));
            }
            printed += 1;
        } else if !is_json {
            let indent = "  ".repeat(depth);
            println!("{indent}<missing {id}>");
        }
    }
    if printed == 0 {
        if !is_json {
            println!("signal {hash} not found in substrate");
        }
        return Ok(EXIT_AGENT_FAILURE);
    }
    Ok(EXIT_SUCCESS)
}

pub(crate) fn cmd_inject(
    cli: &Cli,
    session: String,
    kind_str: &str,
    payload: String,
    workdir: Option<PathBuf>,
) -> Result<i32> {
    let kind = InjectKind::parse(kind_str).map_err(|e| anyhow!("{e}"))?;
    let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
    let request = InjectRequest::new(session, kind, payload, wd);

    request.validate().map_err(|e| anyhow!("{e}"))?;

    if cli.json {
        println!(
            r#"{{"status":"queued","kind":"{}","session":"{}","bytes":{}}}"#,
            request.kind,
            request.session_id,
            request.payload.len(),
        );
    } else if !cli.quiet {
        println!("{}", request.summary());
    }

    Ok(EXIT_SUCCESS)
}

pub(crate) fn cmd_index(cli: &Cli, cmd: IndexCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    match cmd {
        IndexCmd::Build { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let start = Instant::now();
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;
            let elapsed = start.elapsed();
            let stats = idx.stats();
            println!("Index built in {:.2}s", elapsed.as_secs_f64());
            println!("  Files:   {}", stats.indexed_files);
            println!("  Symbols: {}", stats.total_symbols);
            println!("  Edges:   {}", stats.total_edges);
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Rebuild { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            // Remove the existing index database if present.
            let db_path = target.join(".roko").join("index.db");
            if db_path.exists() {
                std::fs::remove_file(&db_path)
                    .with_context(|| format!("remove old index at {}", db_path.display()))?;
                println!("Removed old index: {}", db_path.display());
            }
            // Rebuild from scratch.
            let start = Instant::now();
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("rebuild index for {}", target.display()))?;
            let elapsed = start.elapsed();
            let stats = idx.stats();
            println!("Index rebuilt in {:.2}s", elapsed.as_secs_f64());
            println!("  Files:   {}", stats.indexed_files);
            println!("  Symbols: {}", stats.total_symbols);
            println!("  Edges:   {}", stats.total_edges);
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Search {
            query,
            kind,
            strategy,
            limit,
            path,
        } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;

            let sym_kind = if let Some(ref k) = kind {
                Some(parse_symbol_kind(k)?)
            } else {
                None
            };

            let search_strategy = match strategy.as_str() {
                "keyword" => roko_index::SearchStrategy::Keyword(roko_index::KeywordQuery {
                    text: query.clone(),
                    scope: roko_index::SearchScope::Both,
                    case_sensitive: false,
                    whole_word: false,
                }),
                "structural" => {
                    roko_index::SearchStrategy::Structural(roko_index::StructuralQuery {
                        kind: sym_kind,
                        visibility: None,
                        file_pattern: Some(query.clone()),
                        has_callers: None,
                        min_pagerank: None,
                    })
                }
                "hybrid" => roko_index::SearchStrategy::Hybrid {
                    keyword: Some(roko_index::KeywordQuery {
                        text: query.clone(),
                        scope: roko_index::SearchScope::Both,
                        case_sensitive: false,
                        whole_word: false,
                    }),
                    structural: sym_kind.map(|k| roko_index::StructuralQuery {
                        kind: Some(k),
                        ..Default::default()
                    }),
                    hdc: None,
                },
                other => bail!(
                    "unknown search strategy: {other} (expected keyword, structural, or hybrid)"
                ),
            };

            let results = idx.search(search_strategy, limit);
            if results.is_empty() {
                println!("No results found for \"{query}\"");
            } else {
                println!("{:<50} {:<10} {:<6} {:<8}", "NAME", "KIND", "LINE", "SCORE");
                println!("{}", "-".repeat(76));
                for r in &results {
                    println!(
                        "{:<50} {:<10} {:<6} {:.4}",
                        r.symbol.id.symbol_name,
                        format!("{:?}", r.symbol.id.kind),
                        r.symbol.line,
                        r.score,
                    );
                }
                println!("\n{} result(s)", results.len());
            }
            Ok(EXIT_SUCCESS)
        }
        IndexCmd::Stats { path } => {
            let target = path.unwrap_or_else(|| workdir.clone());
            let idx = roko_index::WorkspaceIndex::load(&target)
                .with_context(|| format!("build index for {}", target.display()))?;
            let stats = idx.stats();

            println!("=== Index Statistics ===\n");
            println!("Files indexed:  {}", stats.indexed_files);
            println!("Total symbols:  {}", stats.total_symbols);
            println!("Total edges:    {}", stats.total_edges);

            println!("\nEdge breakdown:");
            for (kind, count) in &stats.edge_breakdown {
                println!("  {kind}: {count}");
            }

            println!("\nLanguages:");
            for (lang, count) in &stats.languages {
                println!("  {lang}: {count} files");
            }

            if !stats.top_symbols_by_pagerank.is_empty() {
                println!("\nTop-10 symbols by PageRank:");
                println!("{:<50} {:<10} {:<8}", "NAME", "KIND", "SCORE");
                println!("{}", "-".repeat(70));
                for r in &stats.top_symbols_by_pagerank {
                    println!(
                        "{:<50} {:<10} {:.6}",
                        r.symbol.id.symbol_name,
                        format!("{:?}", r.symbol.id.kind),
                        r.score,
                    );
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

pub(crate) fn parse_symbol_kind(s: &str) -> Result<roko_core::language::SymbolKind> {
    use roko_core::language::SymbolKind;
    match s.to_lowercase().as_str() {
        "function" | "fn" => Ok(SymbolKind::Function),
        "struct" => Ok(SymbolKind::Struct),
        "enum" => Ok(SymbolKind::Enum),
        "trait" => Ok(SymbolKind::Trait),
        "const" => Ok(SymbolKind::Const),
        "type" => Ok(SymbolKind::Type),
        "module" | "mod" => Ok(SymbolKind::Module),
        "impl" => Ok(SymbolKind::Impl),
        other => bail!(
            "unknown symbol kind: {other} (expected function, struct, enum, trait, const, type, module, impl)"
        ),
    }
}

pub(crate) fn print_completions(shell: CompletionShell) {
    let words = completion_words();
    let subcommand_map = nested_subcommand_words();
    let dynamic = dynamic_completion_words();
    match shell {
        CompletionShell::Bash => print_bash_completions(&words, &subcommand_map, &dynamic),
        CompletionShell::Zsh => print_zsh_completions(&words, &subcommand_map, &dynamic),
        CompletionShell::Fish => print_fish_completions(&words, &subcommand_map, &dynamic),
    }
}

pub(crate) fn completion_words() -> Vec<String> {
    let mut command = Cli::command();
    command.build();
    let mut words = command
        .get_subcommands()
        .map(|cmd| cmd.get_name().to_string())
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words
}

/// Collect nested subcommand names for each top-level command.
pub(crate) fn nested_subcommand_words() -> Vec<(String, Vec<String>)> {
    let mut command = Cli::command();
    command.build();
    let mut result = Vec::new();
    for sub in command.get_subcommands() {
        let name = sub.get_name().to_string();
        let nested: Vec<String> = sub
            .get_subcommands()
            .map(|s| s.get_name().to_string())
            .collect();
        if !nested.is_empty() {
            result.push((name, nested));
        }
    }
    result
}

/// Scan the filesystem for dynamic completion words (plan names, PRD slugs).
pub(crate) fn dynamic_completion_words() -> Vec<(String, Vec<String>)> {
    let mut result = Vec::new();

    // Scan plans/ directory for plan names.
    if let Ok(entries) = std::fs::read_dir("plans") {
        let plans: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir() || e.path().extension().is_some_and(|x| x == "toml"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        if !plans.is_empty() {
            result.push(("plan".to_string(), plans));
        }
    }

    // Scan .roko/prd/ directory for PRD slugs.
    if let Ok(entries) = std::fs::read_dir(".roko/prd") {
        let prds: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| {
                e.path()
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        if !prds.is_empty() {
            result.push(("prd".to_string(), prds));
        }
    }

    result
}

/// Global flag names for flag completion (UX-1c).
pub(crate) fn completion_flag_words() -> Vec<String> {
    let mut command = Cli::command();
    command.build();
    let mut flags: Vec<String> = command
        .get_arguments()
        .filter_map(|arg| arg.get_long().map(|l| format!("--{l}")))
        .collect();
    flags.sort();
    flags.dedup();
    flags
}

pub(crate) fn print_bash_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let top_words = words.join(" ");
    let flag_words = completion_flag_words().join(" ");
    println!(r#"# roko bash completions (DEPLOY-06: dynamic + nested + flags)"#);
    println!(r#"_roko()"#);
    println!(r#"{{"#);
    println!(r#"    local cur="${{COMP_WORDS[COMP_CWORD]}}""#);
    println!(r#"    local prev="${{COMP_WORDS[COMP_CWORD-1]}}""#);
    println!();
    // Flag completions when current word starts with -.
    println!(r#"    if [[ "$cur" == -* ]]; then"#);
    println!(r#"        COMPREPLY=( $(compgen -W "{flag_words}" -- "$cur") )"#);
    println!(r#"        return 0"#);
    println!(r#"    fi"#);
    println!();
    // Nested subcommand completions.
    println!(r#"    case "$prev" in"#);
    for (parent, children) in subcommands {
        let child_words = children.join(" ");
        println!(r#"        {parent})"#);
        println!(r#"            COMPREPLY=( $(compgen -W "{child_words}" -- "$cur") )"#);
        println!(r#"            return 0"#);
        println!(r#"            ;;"#);
    }
    // Dynamic completions for plan/prd subcommands.
    for (parent, items) in dynamic {
        let item_words = items.join(" ");
        // Add dynamic words to existing subcommand completions.
        println!(r#"        {parent})"#);
        println!(r#"            COMPREPLY=( $(compgen -W "{item_words}" -- "$cur") )"#);
        println!(r#"            return 0"#);
        println!(r#"            ;;"#);
    }
    println!(r#"    esac"#);
    println!();
    // Top-level completions.
    println!(r#"    COMPREPLY=( $(compgen -W "{top_words}" -- "$cur") )"#);
    println!(r#"}}"#);
    println!(r#"complete -F _roko roko"#);
}

pub(crate) fn print_zsh_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let flags = completion_flag_words();
    println!(r#"#compdef roko"#);
    println!(r#"# roko zsh completions (DEPLOY-06: dynamic + nested + flags)"#);
    println!(r#"_roko() {{"#);
    println!(r#"  local -a commands flags"#);
    let top_words = words.join(" ");
    let flag_words = flags.join(" ");
    println!(r#"  commands=({top_words})"#);
    println!(r#"  flags=({flag_words})"#);
    println!();
    // Flag completion at any position when current word starts with -.
    println!(r#"  if [[ "$words[CURRENT]" == -* ]]; then"#);
    println!(r#"    _describe 'roko flag' flags"#);
    println!(r#"    return"#);
    println!(r#"  fi"#);
    println!();
    println!(r#"  if (( CURRENT == 2 )); then"#);
    println!(r#"    _describe 'roko command' commands"#);
    println!(r#"  elif (( CURRENT == 3 )); then"#);
    println!(r#"    case $words[2] in"#);
    for (parent, children) in subcommands {
        let child_words = children.join(" ");
        println!(r#"      {parent})"#);
        println!(r#"        local -a subcmds"#);
        println!(r#"        subcmds=({child_words})"#);
        println!(r#"        _describe '{parent} subcommand' subcmds"#);
        println!(r#"        ;;"#);
    }
    for (parent, items) in dynamic {
        let item_words = items.join(" ");
        println!(r#"      {parent})"#);
        println!(r#"        local -a slugs"#);
        println!(r#"        slugs=({item_words})"#);
        println!(r#"        _describe '{parent} item' slugs"#);
        println!(r#"        ;;"#);
    }
    println!(r#"    esac"#);
    println!(r#"  fi"#);
    println!(r#"}}"#);
    println!(r#"_roko "$@""#);
}

pub(crate) fn print_fish_completions(
    words: &[String],
    subcommands: &[(String, Vec<String>)],
    dynamic: &[(String, Vec<String>)],
) {
    let flags = completion_flag_words();
    println!("# roko fish completions (DEPLOY-06: dynamic + nested + flags)");
    for word in words {
        println!("complete -c roko -f -n '__fish_use_subcommand' -a '{word}'");
    }
    // Global flag completions.
    for flag in &flags {
        let short = flag.trim_start_matches('-');
        println!("complete -c roko -l '{short}'");
    }
    // Nested subcommand completions.
    for (parent, children) in subcommands {
        for child in children {
            println!("complete -c roko -f -n '__fish_seen_subcommand_from {parent}' -a '{child}'");
        }
    }
    // Dynamic completions.
    for (parent, items) in dynamic {
        for item in items {
            println!("complete -c roko -f -n '__fish_seen_subcommand_from {parent}' -a '{item}'");
        }
    }
}

pub(crate) fn resolved_capture_model(agent_command: &str, model: Option<&str>) -> String {
    if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
        return model.to_string();
    }
    if agent_command.eq_ignore_ascii_case("claude") {
        "claude-opus-4-6".to_string()
    } else {
        "unknown-model".to_string()
    }
}

pub(crate) fn capture_provider(agent_command: &str, resolved_model: &str) -> String {
    let command = agent_command.trim();
    let model = resolved_model.to_ascii_lowercase();
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

pub(crate) fn capture_role(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "Researcher"
    } else {
        "Strategist"
    }
}

pub(crate) fn capture_task_category(task_kind: &str) -> &'static str {
    if task_kind.starts_with("research-") {
        "research"
    } else if task_kind.starts_with("prd-plan") {
        "scaffolding"
    } else {
        "docs"
    }
}

pub(crate) fn capture_complexity_band(task_kind: &str) -> &'static str {
    if task_kind == "research-analyze" {
        "standard"
    } else if task_kind.starts_with("research-") {
        "deep"
    } else {
        "standard"
    }
}

pub(crate) fn capture_plan_id(task_id: &str) -> Option<&str> {
    task_id
        .rsplit(':')
        .next()
        .filter(|segment| !segment.is_empty())
}

pub(crate) fn build_capture_episode(
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> (Episode, String) {
    let resolved_model = resolved_capture_model(agent_command, model);
    let provider = capture_provider(agent_command, &resolved_model);
    let role = capture_role(task_kind);
    let task_category = capture_task_category(task_kind);
    let complexity_band = capture_complexity_band(task_kind);
    let mut episode = Episode::new(agent_command.to_string(), task_id.to_string());
    episode.kind = "agent_turn".to_string();
    episode.trigger_kind = task_kind.to_string();
    episode.agent_template = role.to_string();
    episode.episode_id = episode.id.clone();
    episode.model = resolved_model.clone();
    episode.input_signal_hash = ContentHash::of(prompt.as_bytes()).to_hex();
    episode.output_signal_hash = ContentHash::of(output.as_bytes()).to_hex();
    episode.duration_secs = wall_time_ms as f64 / 1000.0;
    episode.usage.wall_ms = wall_time_ms;
    episode.success = success;
    episode.turns = 1;
    if !success {
        episode.failure_reason = Some("agent returned non-zero exit code".to_string());
    }
    episode
        .extra
        .insert("role".to_string(), serde_json::json!(role));
    episode
        .extra
        .insert("command".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("backend".to_string(), serde_json::json!(agent_command));
    episode
        .extra
        .insert("task_kind".to_string(), serde_json::json!(task_kind));
    episode
        .extra
        .insert("task_id".to_string(), serde_json::json!(task_id));
    episode
        .extra
        .insert("model".to_string(), serde_json::json!(resolved_model));
    episode
        .extra
        .insert("provider".to_string(), serde_json::json!(provider.clone()));
    episode.extra.insert(
        "task_category".to_string(),
        serde_json::json!(task_category),
    );
    episode.extra.insert(
        "complexity_band".to_string(),
        serde_json::json!(complexity_band),
    );
    if let Some(plan_id) = capture_plan_id(task_id) {
        episode
            .extra
            .insert("plan_id".to_string(), serde_json::json!(plan_id));
    }
    if let Some(session_id) = resume_session.filter(|value| !value.trim().is_empty()) {
        episode
            .extra
            .insert("session_id".to_string(), serde_json::json!(session_id));
    }
    episode.extra.insert(
        "prompt_chars".to_string(),
        serde_json::json!(prompt.chars().count()),
    );
    episode.extra.insert(
        "output_chars".to_string(),
        serde_json::json!(output.chars().count()),
    );
    episode
        .extra
        .insert("success".to_string(), serde_json::json!(success));
    (episode, provider)
}

pub(crate) async fn persist_capture_episode(
    workdir: &Path,
    agent_command: &str,
    model: Option<&str>,
    task_kind: &str,
    task_id: &str,
    prompt: &str,
    output: &str,
    success: bool,
    wall_time_ms: u64,
    resume_session: Option<&str>,
) -> Result<()> {
    let (episode, provider) = build_capture_episode(
        agent_command,
        model,
        task_kind,
        task_id,
        prompt,
        output,
        success,
        wall_time_ms,
        resume_session,
    );

    let mut runtime = LearningRuntime::open_under(workdir.join(".roko").join("memory"))
        .await
        .map_err(|e| anyhow!("open learning runtime: {e}"))?;
    let distillation_workdir = workdir.to_path_buf();
    runtime.set_episode_completion_hook(move |episode| {
        roko_neuro::spawn_episode_distillation(distillation_workdir.clone(), episode, None);
    });

    let mut completed = CompletedRunInput::from_episode(episode);
    completed.provider = Some(provider);
    runtime
        .record_completed_run(completed)
        .await
        .map_err(|e| anyhow!("record learning feedback: {e}"))?;
    Ok(())
}
