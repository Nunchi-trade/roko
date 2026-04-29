//! config_cmd command handlers.
#![allow(unused_imports)]

use crate::*;
use serde::Serialize;

pub(crate) const fn edit_target(global: bool, project: bool) -> EditTarget {
    if global {
        EditTarget::Global
    } else if project {
        EditTarget::Project
    } else {
        EditTarget::Auto
    }
}

pub(crate) async fn dispatch_config(cli: &Cli, cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Init {
            yes,
            agent,
            model,
            budget,
            role,
            enable_gates,
            path,
            non_interactive,
        } => {
            let mut inputs = WizardInputs {
                agent_command: agent.clone(),
                token_budget: budget,
                model: model.clone(),
                role,
                enable_gates: if enable_gates { Some(true) } else { None },
                yes,
                ..Default::default()
            };
            if let (Some("ollama"), Some(m)) = (agent.as_deref(), model.as_ref()) {
                inputs.agent_args = Some(vec!["run".into(), m.clone()]);
            }
            if non_interactive {
                if inputs.agent_command.is_none() {
                    return Err(anyhow!("--non-interactive requires --agent"));
                }
                inputs.token_budget.get_or_insert(8000);
                inputs
                    .role
                    .get_or_insert_with(|| "You are a Roko agent.".into());
                inputs.enable_gates.get_or_insert(false);
                inputs.yes = true;
                if inputs.agent_args.is_none() {
                    inputs.agent_args = Some(vec![]);
                }
            }
            let _ = run_init_wizard(path, &inputs)?;
            Ok(())
        }
        ConfigCmd::Show { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            config_cmd::cmd_show(&wd)
        }
        ConfigCmd::Path { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            config_cmd::cmd_path(&wd)
        }
        ConfigCmd::Edit {
            global,
            project,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let target = edit_target(global, project);
            config_cmd::cmd_edit(&wd, target)
        }
        ConfigCmd::Set {
            key,
            value,
            global: _,
            project,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let target = if project {
                EditTarget::Project
            } else {
                EditTarget::Global
            };
            config_cmd::cmd_set(&wd, target, &key, &value)
        }
        ConfigCmd::SetSecret { name, value } => config_cmd::cmd_set_secret(&name, &value),
        ConfigCmd::CheckSecrets { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            config_cmd::cmd_check_secrets(&wd)
        }
        ConfigCmd::Validate { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            config_cmd::cmd_validate(&wd).await
        }
        ConfigCmd::Migrate {
            workdir,
            dry_run,
            yes,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            config_cmd::cmd_migrate(&wd, dry_run, yes)
        }
        // ── Providers ───────────────────────────────────────────────
        ConfigCmd::Providers { cmd } => match cmd {
            ConfigProviderCmd::List { workdir } => {
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                cmd_provider_list(&wd).await?;
                Ok(())
            }
            ConfigProviderCmd::Health { workdir } => {
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                cmd_provider_health(&wd)?;
                Ok(())
            }
            ConfigProviderCmd::Test {
                provider,
                all,
                workdir,
            } => {
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                if all {
                    cmd_provider_test_all(&wd, cli.json).await?;
                } else if provider.is_some() || cli.model.is_some() {
                    cmd_provider_test(
                        &wd,
                        provider.as_deref(),
                        cli.model.as_deref(),
                        cli.role.as_deref(),
                        cli.json,
                    )
                    .await?;
                } else {
                    bail!("provide a provider name or use --all");
                }
                Ok(())
            }
        },
        // ── Models ──────────────────────────────────────────────────
        ConfigCmd::Models { cmd } => match cmd {
            ConfigModelCmd::List { workdir } => {
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                cmd_model_list(&wd)?;
                Ok(())
            }
            ConfigModelCmd::Route {
                model,
                explain,
                complexity,
                workdir,
            } => {
                let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
                cmd_model_route(
                    &wd,
                    cli.model.as_deref(),
                    cli.role.as_deref(),
                    &model,
                    explain,
                    complexity.as_deref(),
                )?;
                Ok(())
            }
        },
        // ── Subscriptions ───────────────────────────────────────────
        ConfigCmd::Subscriptions { cmd } => {
            let workdir = resolve_workdir(cli);
            match cmd {
                ConfigSubscriptionCmd::List => {
                    roko_cli::subscriptions::cmd_list(&workdir, cli.json)?
                }
                ConfigSubscriptionCmd::Add { template, trigger } => {
                    roko_cli::subscriptions::cmd_add(&workdir, &template, &trigger)?
                }
                ConfigSubscriptionCmd::Remove { id } => {
                    roko_cli::subscriptions::cmd_remove(&workdir, &id)?
                }
                ConfigSubscriptionCmd::Enable { id } => {
                    roko_cli::subscriptions::cmd_set_enabled(&workdir, &id, true)?
                }
                ConfigSubscriptionCmd::Disable { id } => {
                    roko_cli::subscriptions::cmd_set_enabled(&workdir, &id, false)?
                }
            }
            Ok(())
        }
        // ── Event sources ───────────────────────────────────────────
        ConfigCmd::Events { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::event_sources::cmd_list(&wd, cli.json)?;
            Ok(())
        }
        // ── Experiments (intercepted in dispatch_subcommand) ────────
        ConfigCmd::Experiments { .. } => {
            unreachable!("experiments dispatched in dispatch_subcommand")
        }
        // ── Plugins (intercepted in dispatch_subcommand) ────────────
        ConfigCmd::Plugins { .. } => {
            unreachable!("plugins dispatched in dispatch_subcommand")
        }
        // ── Secrets (intercepted in dispatch_subcommand) ────────────
        ConfigCmd::Secrets { .. } => {
            unreachable!("secrets dispatched in dispatch_subcommand")
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct ProviderHealthSnapshot {
    #[serde(default)]
    pub(crate) providers: HashMap<String, ProviderHealth>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct LatencyStatsSnapshot {
    #[serde(default)]
    pub(crate) entries: Vec<LatencyStatsEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LatencyStatsEntry {
    pub(crate) provider: String,
    pub(crate) stats: LatencyStats,
}

#[derive(Debug, Default)]
pub(crate) struct ProviderLatencySummary {
    pub(crate) recent_latencies: Vec<f64>,
    pub(crate) weighted_latency_ms: f64,
    pub(crate) observations: u64,
}

impl ProviderLatencySummary {
    fn record(&mut self, stats: &LatencyStats) {
        self.recent_latencies
            .extend(stats.recent_latencies.iter().copied());
        self.weighted_latency_ms += stats.total_latency_ema_ms * stats.observations as f64;
        self.observations = self.observations.saturating_add(stats.observations);
    }

    fn p50_ms(&self) -> Option<f64> {
        if !self.recent_latencies.is_empty() {
            let mut latencies = self.recent_latencies.clone();
            latencies.sort_by(|a, b| a.total_cmp(b));
            let idx = ((latencies.len() as f64) * 0.50).floor() as usize;
            let idx = idx.min(latencies.len().saturating_sub(1));
            return latencies.get(idx).copied();
        }

        if self.observations > 0 {
            return Some(self.weighted_latency_ms / self.observations as f64);
        }

        None
    }
}

pub(crate) async fn cmd_provider_list(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let providers = configured_providers(&config);
    if providers.is_empty() {
        println!("no providers configured");
        return Ok(());
    }

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_secs(2))
        .build()
        .context("build provider probe client")?;

    let mut provider_names = providers.keys().cloned().collect::<Vec<_>>();
    provider_names.sort_unstable();

    let mut rows = Vec::with_capacity(provider_names.len());
    for provider_name in provider_names {
        let provider = providers
            .get(&provider_name)
            .expect("provider name collected from provider registry");
        rows.push(inspect_provider(&client, &provider_name, provider).await);
    }

    print!("{}", format_provider_rows(&rows));
    Ok(())
}

pub(crate) fn cmd_provider_health(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let configured = configured_providers(&config);
    let health_path = provider_health_path(workdir);
    let latency_path = latency_stats_path(workdir);
    let provider_health = load_provider_health_snapshot(&health_path)?;
    let latency_stats = load_latency_stats_by_provider(&latency_path)?;

    let mut provider_names = BTreeSet::new();
    provider_names.extend(configured.keys().cloned());
    provider_names.extend(provider_health.keys().cloned());
    provider_names.extend(latency_stats.keys().cloned());

    if provider_names.is_empty() {
        println!("no provider health recorded");
        return Ok(());
    }

    let now_ms = unix_ms_now();
    let health_file_ms = file_modified_ms(&health_path);
    let latency_file_ms = file_modified_ms(&latency_path);
    let rows = provider_names
        .into_iter()
        .map(|provider| {
            build_provider_health_row(
                &provider,
                provider_health.get(&provider),
                latency_stats.get(&provider),
                now_ms,
                health_file_ms,
                latency_file_ms,
            )
        })
        .collect::<Vec<_>>();

    print!("{}", format_provider_health_rows(&rows));
    Ok(())
}

pub(crate) async fn cmd_provider_test(
    workdir: &Path,
    provider_name: Option<&str>,
    cli_model: Option<&str>,
    role_arg: Option<&str>,
    json: bool,
) -> Result<ProviderTestReport> {
    let config = load_roko_config(workdir)?;
    let providers = configured_providers(&config);
    if providers.is_empty() {
        bail!(
            "no providers configured in {}. add a [providers.*] entry or run `roko init`.",
            workdir.display()
        );
    }

    let cli_model = cli_model.map(str::trim).filter(|model| !model.is_empty());
    let role = role_arg.map(str::to_string);

    let (provider_name, provider, model, selection_note) = if let Some(cli_model) = cli_model {
        let selection = crate::model_selection::resolve_effective_model(
            Some(cli_model.to_string()),
            None,
            role.clone(),
            None,
            &config,
        )
        .map_err(|err| anyhow!("resolve provider test selection: {err}"))?;

        if let Some(requested_provider) = provider_name.map(str::trim).filter(|s| !s.is_empty())
            && requested_provider != selection.provider_key
        {
            bail!(
                "--model {cli_model} resolves to provider '{}', but provider '{requested_provider}' was requested",
                selection.provider_key
            );
        }

        let provider_name = selection.provider_key.clone();
        let provider = providers.get(&provider_name).ok_or_else(|| {
            anyhow!("provider '{provider_name}' is not configured")
        })?;
        let model = model_profile_for_effective_selection(&config, &selection);
        let note = format!(
            "Resolved provider test from --model {cli_model}: {} ({})",
            selection.provider_key, selection.reason
        );
        (provider_name, provider, Some(model), Some(note))
    } else {
        let provider_name = provider_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("provide a provider name or use --all"))?
            .to_string();
        let provider = providers.get(&provider_name).ok_or_else(|| {
            anyhow!("provider '{provider_name}' is not configured")
        })?;
        let runtime_selection = crate::model_selection::resolve_effective_model(
            None,
            None,
            role.clone(),
            None,
            &config,
        )
        .ok();
        let model = match provider.kind {
            ProviderKind::OpenAiCompat
            | ProviderKind::AnthropicApi
            | ProviderKind::GeminiApi
            | ProviderKind::PerplexityApi
            | ProviderKind::CerebrasApi => Some(
                if let Some(selection) = runtime_selection.as_ref().filter(|selection| {
                    selection.provider_key == provider_name.as_str()
                }) {
                    model_profile_for_effective_selection(&config, selection)
                } else {
                    select_provider_test_model(&config, &provider_name)
                        .ok_or_else(|| {
                            anyhow!(
                                "provider '{provider_name}' has no configured models; add a [models.*] entry or use --model"
                            )
                        })?
                        .1
                },
            ),
            ProviderKind::ClaudeCli | ProviderKind::CursorAcp => runtime_selection
                .as_ref()
                .filter(|selection| selection.provider_key == provider_name.as_str())
                .map(|selection| model_profile_for_effective_selection(&config, selection)),
        };
        (provider_name, provider, model, None)
    };

    if let Some(note) = selection_note.as_ref() {
        println!("{note}");
        println!();
    }

    let report = match provider.kind {
        ProviderKind::OpenAiCompat => {
            let model = model.as_ref().ok_or_else(|| {
                anyhow!("provider '{provider_name}' requires a model profile for testing")
            })?;
            run_openai_compat_provider_test(&provider_name, provider, model, json).await?
        }
        ProviderKind::AnthropicApi => {
            let model = model.as_ref().ok_or_else(|| {
                anyhow!("provider '{provider_name}' requires a model profile for testing")
            })?;
            run_anthropic_provider_test(&provider_name, provider, model, json).await?
        }
        ProviderKind::ClaudeCli => {
            run_claude_cli_provider_test(&provider_name, provider, model.as_ref(), json).await?
        }
        ProviderKind::GeminiApi => {
            let model = model.as_ref().ok_or_else(|| {
                anyhow!("provider '{provider_name}' requires a model profile for testing")
            })?;
            run_gemini_provider_test(&provider_name, provider, model, json).await?
        }
        ProviderKind::PerplexityApi => {
            let model = model.as_ref().ok_or_else(|| {
                anyhow!("provider '{provider_name}' requires a model profile for testing")
            })?;
            run_openai_compat_provider_test(&provider_name, provider, model, json).await?
        }
        ProviderKind::CursorAcp => {
            run_cursor_provider_test(&provider_name, provider, model.as_ref(), json).await?
        }
        ProviderKind::CerebrasApi => {
            let model = model.as_ref().ok_or_else(|| {
                anyhow!("provider '{provider_name}' requires a model profile for testing")
            })?;
            run_openai_compat_provider_test(&provider_name, provider, model, json).await?
        }
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }

    Ok(report)
}

pub(crate) async fn cmd_provider_test_all(workdir: &Path, json: bool) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let providers = configured_providers(&config);
    if providers.is_empty() {
        bail!(
            "no providers configured in {}. add a [providers.*] entry or run `roko init`.",
            workdir.display()
        );
    }

    let mut results: Vec<ProviderTestAllRow> = Vec::new();
    let mut sorted_names: Vec<_> = providers.keys().cloned().collect();
    sorted_names.sort();

    for name in &sorted_names {
        let provider = &providers[name];
        if let Some(env_name) = provider
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if std::env::var(env_name)
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                results.push(ProviderTestAllRow {
                    provider: name.clone(),
                    kind: provider.kind.to_string(),
                    status: "SKIPPED (no key)".into(),
                    duration_ms: None,
                });
                continue;
            }
        }
        let started = Instant::now();
        match cmd_provider_test(workdir, Some(name.as_str()), None, None, false).await {
            Ok(_) => {
                results.push(ProviderTestAllRow {
                    provider: name.clone(),
                    kind: provider.kind.to_string(),
                    status: "\u{2713} OK".into(),
                    duration_ms: Some(
                        started
                            .elapsed()
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                    ),
                });
            }
            Err(e) => {
                results.push(ProviderTestAllRow {
                    provider: name.clone(),
                    kind: provider.kind.to_string(),
                    status: format!("\u{2717} {e:#}"),
                    duration_ms: Some(
                        started
                            .elapsed()
                            .as_millis()
                            .min(u64::MAX as u128) as u64,
                    ),
                });
            }
        }
    }

    println!();
    println!("Provider Test Summary");
    println!("{}", "\u{2500}".repeat(72));
    println!(
        "{:<16} {:<16} {:<28} {}",
        "Provider", "Kind", "Status", "Latency"
    );
    println!("{}", "\u{2500}".repeat(72));
    for row in &results {
        let latency = row
            .duration_ms
            .map(|ms| format_provider_test_duration(Duration::from_millis(ms)))
            .unwrap_or_else(|| "\u{2014}".to_string());
        println!(
            "{:<16} {:<16} {:<28} {}",
            row.provider, row.kind, row.status, latency
        );
    }
    println!("{}", "\u{2500}".repeat(72));
    if json {
        println!();
        println!("{}", serde_json::to_string_pretty(&results)?);
    }
    Ok(())
}

pub(crate) fn cmd_model_list(workdir: &Path) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let models = configured_models(&config);
    if models.is_empty() {
        println!("no models configured");
        return Ok(());
    }

    let mut model_names = models.keys().cloned().collect::<Vec<_>>();
    model_names.sort_unstable();

    let rows = model_names
        .into_iter()
        .map(|model_name| {
            let profile = models
                .get(&model_name)
                .expect("model name collected from model registry");
            build_model_list_row(&model_name, profile)
        })
        .collect::<Vec<_>>();

    print!("{}", format_model_rows(&rows));
    Ok(())
}

fn format_effective_model_selection_summary(
    requested_model: &str,
    cli_model: Option<&str>,
    role: AgentRole,
    selection: &crate::model_selection::EffectiveModelSelection,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Resolved model selection for '{requested_model}':");
    let _ = writeln!(out, "  Task model: {requested_model}");
    let _ = writeln!(out, "  CLI override: {}", cli_model.unwrap_or("—"));
    let _ = writeln!(out, "  Role: {role}");
    let _ = writeln!(
        out,
        "  Requested model: {}",
        selection.requested_model.as_deref().unwrap_or("—")
    );
    let _ = writeln!(out, "  Source: {}", selection.source);
    let _ = writeln!(out, "  Effective model: {}", selection.effective_model_key);
    let _ = writeln!(
        out,
        "  Provider: {} ({})",
        selection.provider_key, selection.provider_kind
    );
    let _ = writeln!(out, "  Backend slug: {}", selection.backend_slug);
    let _ = writeln!(out, "  Reason: {}", selection.reason);
    out
}

fn model_selection_recommendation(error: &crate::model_selection::Error) -> String {
    match error {
        crate::model_selection::Error::EmptyModel { source } => {
            format!("pass a non-empty model value for {source}")
        }
        crate::model_selection::Error::MissingProvider {
            model,
            provider_key,
            ..
        } => {
            format!("configure provider '{provider_key}' for model '{model}', or choose a model backed by an installed provider")
        }
        crate::model_selection::Error::UnknownModel {
            model,
            provider_kind,
            ..
        } => {
            format!("add a model profile for '{model}' backed by provider kind '{provider_kind}', or use a configured model")
        }
    }
}

pub(crate) fn cmd_model_route(
    workdir: &Path,
    cli_model: Option<&str>,
    role_arg: Option<&str>,
    requested_model: &str,
    explain: bool,
    complexity_arg: Option<&str>,
) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let models = configured_models(&config);
    if models.is_empty() {
        println!("no models configured");
        return Ok(());
    }

    let role = parse_agent_role(role_arg)?;
    let selection = crate::model_selection::resolve_effective_model(
        cli_model.map(str::to_string),
        Some(requested_model.to_string()),
        Some(role.to_string()),
        None,
        &config,
    );

    let mut model_slugs = models
        .values()
        .map(|profile| profile.slug.clone())
        .collect::<Vec<_>>();
    model_slugs.sort();
    model_slugs.dedup();

    let complexity = parse_route_complexity(complexity_arg)?;
    let aliases = model_aliases_by_slug(&models);
    let requested_slug = resolve_requested_model_slug(requested_model, &models)
        .unwrap_or_else(|| requested_model.to_string());
    let context = RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: complexity.band,
        iteration: 1,
        role,
        crate_familiarity: 0.0,
        has_prior_failure: false,
        conductor_load: 0.0,
        active_agents: 0,
        ready_queue_depth: 0,
        max_queue_wait_hours: 0.0,
        daimon_policy: DaimonPolicy::default(),
        thinking_level: None,
        temperament: Some(config.agent.temperament_for_role(role.label())),
        previous_model: Some(requested_slug.clone()),
        plan_context_tokens: None,
        tier_thresholds: None,
    };

    let router = CascadeRouter::load_or_new(&cascade_router_path(workdir), model_slugs.clone());
    let provider_health = load_provider_health_snapshot(&provider_health_path(workdir))?;
    let latency_registry = LatencyRegistry::load_or_new(&latency_stats_path(workdir));
    let model_providers = model_provider_map(&models, &model_slugs);
    let available_candidates = available_model_candidates(
        &model_slugs,
        &model_providers,
        &provider_health,
        unix_ms_now(),
    );
    let route_recommendation = router.explain_route(
        &context,
        (!available_candidates.is_empty()).then_some(available_candidates.as_slice()),
    );
    let confidence = router.confidence_snapshot();
    let cost_table = CostTable::from_config(&models).with_defaults();

    match selection {
        Ok(selection) => {
            if !explain {
                println!(
                    "Resolved '{requested_model}': {} via {} ({}, {})",
                    selection.effective_model_key,
                    selection.provider_key,
                    selection.provider_kind,
                    selection.source
                );
                println!("Reason: {}", selection.reason);
                return Ok(());
            }

            print!(
                "{}",
                format_effective_model_selection_summary(
                    requested_model,
                    cli_model,
                    role,
                    &selection,
                )
            );
            println!();
            print!(
                "{}",
                format_model_route_explanation(
                    requested_model,
                    &requested_slug,
                    &aliases,
                    &route_recommendation,
                    &confidence,
                    &model_providers,
                    &provider_health,
                    &latency_registry,
                    &cost_table,
                )
            );
        }
        Err(err) => {
            println!("Routing failed for '{requested_model}': {err}");
            println!("Recommendation: {}", model_selection_recommendation(&err));
            if !explain {
                let selected_name = display_model_name(&aliases, &route_recommendation.selected_slug);
                let provider = model_providers
                    .get(&route_recommendation.selected_slug)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                println!("Fallback route: {selected_name} via {provider}");
                return Ok(());
            }

            println!();
            print!(
                "{}",
                format_model_route_explanation(
                    requested_model,
                    &requested_slug,
                    &aliases,
                    &route_recommendation,
                    &confidence,
                    &model_providers,
                    &provider_health,
                    &latency_registry,
                    &cost_table,
                )
            );
        }
    }
    Ok(())
}

pub(crate) async fn cmd_plugin(cli: &Cli, cmd: PluginCmd) -> Result<i32> {
    let workdir = match &cmd {
        PluginCmd::List { workdir } => workdir.clone(),
        PluginCmd::Install { workdir, .. } => workdir.clone(),
        PluginCmd::Remove { workdir, .. } => workdir.clone(),
        PluginCmd::Audit { workdir } => workdir.clone(),
    }
    .unwrap_or_else(|| resolve_workdir(cli));

    match cmd {
        PluginCmd::List { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();

            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {} // Directory doesn't exist — fine
                }
            }

            if all_plugins.is_empty() {
                println!("no plugins found");
                println!(
                    "  search paths: {}, {}",
                    plugins_dir.display(),
                    roko_plugins.display()
                );
                println!("  install a plugin with: roko plugin install <path>");
            } else {
                println!("installed plugins ({}):", all_plugins.len());
                for plugin in &all_plugins {
                    let m = &plugin.manifest.plugin;
                    let desc = m.description.as_deref().unwrap_or("no description");
                    println!("  {} v{} — {}", m.name, m.version, desc);
                    if !plugin.manifest.prompts.is_empty() {
                        println!(
                            "    prompts: {}",
                            plugin
                                .manifest
                                .prompts
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    if !plugin.manifest.tools.is_empty() {
                        println!(
                            "    tools: {}",
                            plugin
                                .manifest
                                .tools
                                .iter()
                                .map(|t| t.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                    if !plugin.manifest.profiles.is_empty() {
                        println!(
                            "    profiles: {}",
                            plugin
                                .manifest
                                .profiles
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Install { source, .. } => {
            let source_path = std::path::Path::new(&source);

            // Find the manifest file.
            let manifest_path = if source_path.is_file() {
                source_path.to_path_buf()
            } else if source_path.is_dir() {
                let candidate = source_path.join("plugin.toml");
                if candidate.exists() {
                    candidate
                } else {
                    eprintln!("error: no plugin.toml found in {}", source_path.display());
                    return Ok(EXIT_SYSTEM_ERROR);
                }
            } else {
                eprintln!("error: source path does not exist: {source}");
                return Ok(EXIT_SYSTEM_ERROR);
            };

            // Load and validate the manifest.
            let manifest = match roko_plugin::manifest::load_manifest(&manifest_path) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: failed to load plugin manifest: {e}");
                    return Ok(EXIT_SYSTEM_ERROR);
                }
            };

            // Copy to .roko/plugins/<name>/
            let install_dir = workdir
                .join(".roko")
                .join("plugins")
                .join(&manifest.plugin.name);
            std::fs::create_dir_all(&install_dir)?;

            // Copy the manifest.
            let dest_manifest = install_dir.join("plugin.toml");
            std::fs::copy(&manifest_path, &dest_manifest)?;

            // Copy the containing directory's files if source is a directory.
            if source_path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(source_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path != manifest_path {
                            let dest = install_dir.join(entry.file_name());
                            std::fs::copy(&path, &dest)?;
                        }
                    }
                }
            }

            println!(
                "installed plugin `{}` v{} to {}",
                manifest.plugin.name,
                manifest.plugin.version,
                install_dir.display()
            );
            println!(
                "  {} prompt(s), {} profile(s), {} tool(s), {} trigger(s)",
                manifest.prompts.len(),
                manifest.profiles.len(),
                manifest.tools.len(),
                manifest.triggers.len(),
            );
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Remove { name, .. } => {
            let install_dir = workdir.join(".roko").join("plugins").join(&name);
            if !install_dir.exists() {
                eprintln!("error: plugin `{name}` is not installed");
                eprintln!("  expected at: {}", install_dir.display());
                return Ok(EXIT_SYSTEM_ERROR);
            }
            std::fs::remove_dir_all(&install_dir)?;
            println!("removed plugin `{name}` from {}", install_dir.display());
            Ok(EXIT_SUCCESS)
        }
        PluginCmd::Audit { .. } => {
            let plugins_dir = workdir.join("plugins");
            let roko_plugins = workdir.join(".roko").join("plugins");
            let mut all_plugins = Vec::new();

            for dir in [&plugins_dir, &roko_plugins] {
                match roko_plugin::manifest::discover_plugins(dir) {
                    Ok(found) => all_plugins.extend(found),
                    Err(_) => {}
                }
            }

            if all_plugins.is_empty() {
                println!("no plugins to audit");
            } else {
                println!("plugin audit ({} plugins):", all_plugins.len());
                for plugin in &all_plugins {
                    let m = &plugin.manifest;
                    println!("\n  {} v{}", m.plugin.name, m.plugin.version);
                    println!("    location: {}", plugin.base_dir.display());

                    // Tier capabilities
                    let mut tiers = Vec::new();
                    if !m.prompts.is_empty() {
                        tiers.push(format!("T1:prompts({})", m.prompts.len()));
                    }
                    if !m.profiles.is_empty() {
                        tiers.push(format!("T2:profiles({})", m.profiles.len()));
                    }
                    if !m.tools.is_empty() {
                        tiers.push(format!("T3:tools({})", m.tools.len()));
                    }
                    println!(
                        "    capabilities: {}",
                        if tiers.is_empty() {
                            "none".to_string()
                        } else {
                            tiers.join(", ")
                        }
                    );

                    // Tools with their commands (security audit)
                    for tool in &m.tools {
                        println!(
                            "    tool `{}`: `{}` (timeout: {}ms)",
                            tool.name, tool.command, tool.timeout_ms
                        );
                    }

                    // Triggers
                    for trigger in &m.triggers {
                        match trigger {
                            roko_plugin::manifest::TriggerDef::Cron { expression, .. } => {
                                println!("    trigger: cron({expression})");
                            }
                            roko_plugin::manifest::TriggerDef::FileWatch { paths, .. } => {
                                println!("    trigger: file_watch({})", paths.join(", "));
                            }
                            roko_plugin::manifest::TriggerDef::Webhook { path, .. } => {
                                println!("    trigger: webhook({path})");
                            }
                        }
                    }

                    // Dependencies
                    for dep in &m.dependencies {
                        println!(
                            "    requires: {} {}",
                            dep.name,
                            dep.version.as_deref().unwrap_or("*")
                        );
                    }
                }
            }
            Ok(EXIT_SUCCESS)
        }
    }
}

pub(crate) fn configured_providers(
    config: &RokoConfig,
) -> std::collections::HashMap<String, ProviderConfig> {
    if !config.providers.is_empty() {
        return config.providers.clone();
    }

    if config.agent.command.is_some()
        || config.agent.args.is_some()
        || config.agent.timeout_ms.is_some()
        || config
            .agent
            .env
            .as_ref()
            .is_some_and(|entries| !entries.is_empty())
    {
        return config.effective_providers();
    }

    std::collections::HashMap::new()
}

pub(crate) fn configured_models(
    config: &RokoConfig,
) -> std::collections::HashMap<String, ModelProfile> {
    config.effective_models()
}

pub(crate) fn select_provider_test_model(
    config: &RokoConfig,
    provider_name: &str,
) -> Option<(String, ModelProfile)> {
    let models = configured_models(config);
    let default_model = config.agent.default_model.trim();
    if let Some(profile) = models.get(default_model)
        && profile.provider == provider_name
    {
        return Some((default_model.to_string(), profile.clone()));
    }

    let mut candidates = models
        .into_iter()
        .filter(|(_, profile)| profile.provider == provider_name)
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.into_iter().next()
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderTestReport {
    pub(crate) provider: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    pub(crate) status: String,
    pub(crate) duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    pub(crate) content_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost_usd: Option<f64>,
}

fn model_profile_for_effective_selection(
    config: &RokoConfig,
    selection: &crate::model_selection::EffectiveModelSelection,
) -> ModelProfile {
    roko_core::agent::resolve_model(config, &selection.effective_model_key)
        .profile
        .unwrap_or_else(|| ModelProfile {
            provider: selection.provider_key.clone(),
            slug: selection.backend_slug.clone(),
            ..Default::default()
        })
}

fn build_provider_test_report(
    provider_name: &str,
    kind: ProviderKind,
    model: Option<&ModelProfile>,
    content: Option<String>,
    usage: Option<&roko_core::Usage>,
    cost_usd: Option<f64>,
    duration: Duration,
) -> ProviderTestReport {
    let content_state = if content.as_deref().is_some_and(|text| !text.is_empty()) {
        "content_present"
    } else {
        "content_empty"
    };

    ProviderTestReport {
        provider: provider_name.to_string(),
        kind: kind.to_string(),
        model: model.map(|profile| profile.slug.clone()),
        status: "ok".to_string(),
        duration_ms: duration.as_millis().min(u64::MAX as u128) as u64,
        content: content.filter(|text| !text.is_empty()),
        content_state: content_state.to_string(),
        input_tokens: usage.map(|usage| usage.input_tokens as u64),
        output_tokens: usage.map(|usage| usage.output_tokens as u64),
        cost_usd,
    }
}

pub(crate) async fn inspect_provider(
    client: &reqwest::Client,
    provider_name: &str,
    provider: &ProviderConfig,
) -> ProviderListRow {
    match provider.kind {
        ProviderKind::ClaudeCli => inspect_cli_provider(provider_name, provider),
        _ => inspect_http_provider(client, provider_name, provider).await,
    }
}

pub(crate) fn inspect_cli_provider(
    provider_name: &str,
    provider: &ProviderConfig,
) -> ProviderListRow {
    let command = provider
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty());
    let status = match command {
        Some(command) if command_available(command) => "ok (cli found)".to_string(),
        Some(_) => "warn (cli missing)".to_string(),
        None => "warn (command missing)".to_string(),
    };

    ProviderListRow {
        provider: provider_name.to_string(),
        kind: provider.kind.to_string(),
        base_url: format!("(cli: {})", command.unwrap_or("<missing>")),
        status,
    }
}

pub(crate) async fn inspect_http_provider(
    client: &reqwest::Client,
    provider_name: &str,
    provider: &ProviderConfig,
) -> ProviderListRow {
    let base_url = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|base_url| !base_url.is_empty());
    let mut issues = Vec::new();

    if let Some(env_name) = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|env_name| !env_name.is_empty())
    {
        let has_key = std::env::var(env_name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        if !has_key {
            issues.push("key missing".to_string());
        }
    }

    match base_url {
        Some(base_url) => {
            if let Some(issue) = probe_base_url(client, base_url).await {
                issues.push(issue);
            }
        }
        None => issues.push("base URL missing".to_string()),
    }

    let status = if issues.is_empty() {
        if provider
            .api_key_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|env_name| !env_name.is_empty())
        {
            "ok (key set)".to_string()
        } else {
            "ok (reachable)".to_string()
        }
    } else {
        format!("warn ({})", issues.join(", "))
    };

    ProviderListRow {
        provider: provider_name.to_string(),
        kind: provider.kind.to_string(),
        base_url: base_url.unwrap_or("(missing)").to_string(),
        status,
    }
}

pub(crate) async fn probe_base_url(client: &reqwest::Client, base_url: &str) -> Option<String> {
    match client.head(base_url).send().await {
        Ok(_) => None,
        Err(err) if err.is_builder() => Some("invalid base URL".to_string()),
        Err(_) => Some("unreachable".to_string()),
    }
}

pub(crate) fn command_available(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }

    let command_path = Path::new(command);
    if command_path.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return executable_file(command_path);
    }

    roko_cli::config::command_on_path(command)
}

pub(crate) fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(crate) fn format_provider_rows(rows: &[ProviderListRow]) -> String {
    let mut widths = [
        "Provider".len(),
        "Kind".len(),
        "Base URL".len(),
        "Status".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.provider.len());
        widths[1] = widths[1].max(row.kind.len());
        widths[2] = widths[2].max(row.base_url.len());
        widths[3] = widths[3].max(row.status.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<provider_w$}  {:<kind_w$}  {:<base_w$}  {:<status_w$}",
        "Provider",
        "Kind",
        "Base URL",
        "Status",
        provider_w = widths[0],
        kind_w = widths[1],
        base_w = widths[2],
        status_w = widths[3],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<provider_w$}  {:<kind_w$}  {:<base_w$}  {:<status_w$}",
            row.provider,
            row.kind,
            row.base_url,
            row.status,
            provider_w = widths[0],
            kind_w = widths[1],
            base_w = widths[2],
            status_w = widths[3],
        );
    }

    out
}

pub(crate) fn build_model_list_row(model_name: &str, profile: &ModelProfile) -> ModelListRow {
    ModelListRow {
        model: model_name.to_string(),
        provider: profile.provider.clone(),
        slug: profile.slug.clone(),
        context: format_context_window(profile.context_window),
        tools: format_bool_capability(profile.supports_tools).to_string(),
        thinking: format_bool_capability(profile.supports_thinking).to_string(),
        vision: format_bool_capability(profile.supports_vision).to_string(),
        cost: format_model_cost(profile),
    }
}

pub(crate) fn format_model_rows(rows: &[ModelListRow]) -> String {
    let mut widths = [
        "Model".len(),
        "Provider".len(),
        "Slug".len(),
        "Context".len(),
        "Tools".len(),
        "Thinking".len(),
        "Vision".len(),
        "Cost (in/out)".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.model.len());
        widths[1] = widths[1].max(row.provider.len());
        widths[2] = widths[2].max(row.slug.len());
        widths[3] = widths[3].max(row.context.len());
        widths[4] = widths[4].max(row.tools.len());
        widths[5] = widths[5].max(row.thinking.len());
        widths[6] = widths[6].max(row.vision.len());
        widths[7] = widths[7].max(row.cost.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<model_w$}  {:<provider_w$}  {:<slug_w$}  {:<context_w$}  {:<tools_w$}  {:<thinking_w$}  {:<vision_w$}  {:<cost_w$}",
        "Model",
        "Provider",
        "Slug",
        "Context",
        "Tools",
        "Thinking",
        "Vision",
        "Cost (in/out)",
        model_w = widths[0],
        provider_w = widths[1],
        slug_w = widths[2],
        context_w = widths[3],
        tools_w = widths[4],
        thinking_w = widths[5],
        vision_w = widths[6],
        cost_w = widths[7],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<model_w$}  {:<provider_w$}  {:<slug_w$}  {:<context_w$}  {:<tools_w$}  {:<thinking_w$}  {:<vision_w$}  {:<cost_w$}",
            row.model,
            row.provider,
            row.slug,
            row.context,
            row.tools,
            row.thinking,
            row.vision,
            row.cost,
            model_w = widths[0],
            provider_w = widths[1],
            slug_w = widths[2],
            context_w = widths[3],
            tools_w = widths[4],
            thinking_w = widths[5],
            vision_w = widths[6],
            cost_w = widths[7],
        );
    }

    out
}

pub(crate) fn format_context_window(tokens: u64) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        let whole_thousands = tokens / 1_000;
        if tokens % 1_000 == 0 {
            format!("{whole_thousands}K")
        } else {
            let value = tokens as f64 / 1_000.0;
            format!("{value:.1}K")
        }
    } else {
        tokens.to_string()
    }
}

pub(crate) fn format_bool_capability(value: bool) -> &'static str {
    if value { "✓" } else { "✗" }
}

pub(crate) fn format_model_cost(profile: &ModelProfile) -> String {
    match (profile.cost_input_per_m, profile.cost_output_per_m) {
        (Some(input), Some(output)) => format!("${input:.2}/${output:.2}"),
        (Some(input), None) => format!("${input:.2}/—"),
        (None, Some(output)) => format!("—/${output:.2}"),
        (None, None) => "—".to_string(),
    }
}

pub(crate) async fn run_openai_compat_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: &ModelProfile,
    _json: bool,
) -> Result<ProviderTestReport> {
    let endpoint = openai_compat_test_endpoint(provider);
    let api_key_env = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|env_name| !env_name.is_empty());
    let api_key = provider
        .resolve_api_key()
        .filter(|value| !value.trim().is_empty());
    let body = json!({
        "model": model.slug,
        "messages": [{
            "role": "user",
            "content": "Say hello"
        }],
        "max_tokens": 10
    });
    let body_text = serde_json::to_string(&body).context("serialize provider test body")?;

    if let (Some(env_name), None) = (api_key_env, api_key.as_ref()) {
        println!("Testing provider '{provider_name}' ({})...", provider.kind);
        println!("  Endpoint: {endpoint}");
        println!("  API Key:  missing ({env_name})");
        bail!("missing API key: env var {env_name} not set");
    }

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!("  Endpoint: {endpoint}");
    match (api_key_env, api_key.as_ref()) {
        (Some(env_name), Some(_)) => println!("  API Key:  set ({env_name})"),
        (None, _) => println!("  API Key:  not required"),
        _ => {}
    }
    println!("  Model:    {}", model.slug);
    println!();
    println!("  Sending: {body_text}");

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_millis(
            provider.timeout_ms.unwrap_or(120_000),
        ))
        .build()
        .context("build provider test client")?;

    let mut request = client
        .post(&endpoint)
        .header("content-type", "application/json");
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key);
    }
    if let Some(extra_headers) = provider.extra_headers.as_ref() {
        let mut entries = extra_headers.iter().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(b.1)));
        for (name, value) in entries {
            request = request.header(name.as_str(), value.as_str());
        }
    }

    let started = Instant::now();
    let response = request
        .json(&body)
        .send()
        .await
        .with_context(|| format!("send provider test request to {endpoint}"))?;
    let elapsed = started.elapsed();
    let status = response.status();
    let status_line = status.to_string();
    let response_text = response
        .text()
        .await
        .context("read provider test response body")?;

    if !status.is_success() {
        println!(
            "  Response: {} ({})",
            status_line,
            format_provider_test_duration(elapsed)
        );
        println!("  Error:    {response_text}");
        bail!("provider '{provider_name}' test failed");
    }

    let response_json: Value = serde_json::from_str(&response_text)
        .with_context(|| format!("parse provider test response from {endpoint}"))?;
    let backend_response = BackendResponse::Json(response_json);
    let content = backend_response.extract_text();
    let usage = backend_response.extract_usage();
    let cost = estimate_provider_test_cost(model, &usage);

    println!(
        "  Response: {} ({})",
        status_line,
        format_provider_test_duration(elapsed)
    );
    if content.is_empty() {
        println!("  Content:  content_empty");
    } else {
        println!(
            "  Content:  {}",
            serde_json::to_string(&content).context("format provider test content")?
        );
    }
    println!(
        "  Tokens:   input={}, output={}",
        usage.input_tokens, usage.output_tokens
    );
    match cost {
        Some(cost) => println!("  Cost:     ${cost:.6}"),
        None => println!("  Cost:     n/a"),
    }
    println!();
    println!("  ✓ Provider '{provider_name}' is working");

    Ok(build_provider_test_report(
        provider_name,
        provider.kind,
        Some(model),
        Some(content),
        Some(&usage),
        cost,
        elapsed,
    ))
}

pub(crate) fn openai_compat_test_endpoint(provider: &ProviderConfig) -> String {
    format!(
        "{}/chat/completions",
        provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1")
            .trim_end_matches('/')
    )
}

pub(crate) fn estimate_provider_test_cost(
    model: &ModelProfile,
    usage: &roko_agent::Usage,
) -> Option<f64> {
    let mut cost = 0.0;
    let mut priced = false;

    if let Some(rate) = model.cost_input_per_m {
        cost += f64::from(usage.input_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_output_per_m {
        cost += f64::from(usage.output_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_cache_read_per_m {
        cost += f64::from(usage.cache_read_tokens) * rate / 1_000_000.0;
        priced = true;
    }
    if let Some(rate) = model.cost_cache_write_per_m {
        cost += f64::from(usage.cache_create_tokens) * rate / 1_000_000.0;
        priced = true;
    }

    priced.then_some(cost)
}

pub(crate) fn format_provider_test_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.1}s", duration.as_secs_f64())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

pub(crate) async fn run_anthropic_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: &ModelProfile,
    _json: bool,
) -> Result<ProviderTestReport> {
    let base = provider
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com")
        .trim_end_matches('/');
    let endpoint = format!("{base}/v1/messages");
    let api_key = provider
        .resolve_api_key()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            let env_name = provider.api_key_env.as_deref().unwrap_or("(none)");
            anyhow!("missing API key for provider '{provider_name}' (env: {env_name})")
        })?;

    let body = json!({
        "model": model.slug,
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "Say hello"}]
    });

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!("  Endpoint: {endpoint}");
    println!(
        "  API Key:  set ({})",
        provider.api_key_env.as_deref().unwrap_or("?")
    );
    println!("  Model:    {}", model.slug);
    println!();

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_millis(
            provider.timeout_ms.unwrap_or(120_000),
        ))
        .build()
        .context("build provider test client")?;

    let started = Instant::now();
    let response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .await
        .with_context(|| format!("send provider test request to {endpoint}"))?;
    let elapsed = started.elapsed();
    let status = response.status();
    let response_text = response.text().await.context("read response body")?;

    if !status.is_success() {
        println!(
            "  Response: {} ({})",
            status,
            format_provider_test_duration(elapsed)
        );
        println!("  Error:    {response_text}");
        bail!("provider '{provider_name}' test failed");
    }

    let response_json: Value =
        serde_json::from_str(&response_text).context("parse anthropic response")?;
    let content = response_json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let input_tokens = response_json["usage"]["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = response_json["usage"]["output_tokens"]
        .as_u64()
        .unwrap_or(0);

    println!(
        "  Response: {} ({})",
        status,
        format_provider_test_duration(elapsed)
    );
    if content.is_empty() {
        println!("  Content:  content_empty");
    } else {
        println!("  Content:  {content}");
    }
    println!("  Tokens:   input={input_tokens}, output={output_tokens}");
    println!();
    println!("  \u{2713} Provider '{provider_name}' is working");
    let usage = roko_core::Usage {
        input_tokens: input_tokens as u32,
        output_tokens: output_tokens as u32,
        ..Default::default()
    };
    let cost = estimate_provider_test_cost(model, &usage);
    Ok(build_provider_test_report(
        provider_name,
        provider.kind,
        Some(model),
        Some(content),
        Some(&usage),
        cost,
        elapsed,
    ))
}

pub(crate) async fn run_claude_cli_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: Option<&ModelProfile>,
    _json: bool,
) -> Result<ProviderTestReport> {
    let cmd = provider.command.as_deref().unwrap_or("claude");

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!("  Command: {cmd}");
    if let Some(model) = model {
        println!("  Model:    {}", model.slug);
    }
    println!();

    let started = Instant::now();
    let output = tokio::process::Command::new(cmd)
        .arg("--version")
        .output()
        .await
        .with_context(|| format!("spawn '{cmd} --version'"))?;
    let elapsed = started.elapsed();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!(
            "  Status: exit {} ({})",
            output.status,
            format_provider_test_duration(elapsed)
        );
        println!("  Error:  {stderr}");
        bail!("provider '{provider_name}' test failed");
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!(
        "  Status:  exit 0 ({})",
        format_provider_test_duration(elapsed)
    );
    println!("  Version: {version}");
    println!();
    println!("  \u{2713} Provider '{provider_name}' is working");

    Ok(build_provider_test_report(
        provider_name,
        provider.kind,
        model,
        None,
        None,
        None,
        elapsed,
    ))
}

pub(crate) async fn run_gemini_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: &ModelProfile,
    _json: bool,
) -> Result<ProviderTestReport> {
    let base = provider
        .base_url
        .as_deref()
        .unwrap_or("https://generativelanguage.googleapis.com")
        .trim_end_matches('/');
    let api_key = provider
        .resolve_api_key()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            let env_name = provider.api_key_env.as_deref().unwrap_or("(none)");
            anyhow!("missing API key for provider '{provider_name}' (env: {env_name})")
        })?;
    let endpoint = format!(
        "{base}/v1beta/models/{}:generateContent?key={api_key}",
        model.slug
    );

    let body = json!({
        "contents": [{"parts": [{"text": "Say hello"}]}],
        "generationConfig": {"maxOutputTokens": 10}
    });

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!(
        "  Endpoint: {base}/v1beta/models/{}:generateContent",
        model.slug
    );
    println!(
        "  API Key:  set ({})",
        provider.api_key_env.as_deref().unwrap_or("?")
    );
    println!("  Model:    {}", model.slug);
    println!();

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_millis(
            provider.timeout_ms.unwrap_or(120_000),
        ))
        .build()
        .context("build provider test client")?;

    let started = Instant::now();
    let response = client
        .post(&endpoint)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .context("send provider test request to gemini")?;
    let elapsed = started.elapsed();
    let status = response.status();
    let response_text = response.text().await.context("read response body")?;

    if !status.is_success() {
        println!(
            "  Response: {} ({})",
            status,
            format_provider_test_duration(elapsed)
        );
        println!("  Error:    {response_text}");
        bail!("provider '{provider_name}' test failed");
    }

    let response_json: Value =
        serde_json::from_str(&response_text).context("parse gemini response")?;
    let content = response_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let prompt_tokens = response_json["usageMetadata"]["promptTokenCount"]
        .as_u64()
        .unwrap_or(0);
    let output_tokens = response_json["usageMetadata"]["candidatesTokenCount"]
        .as_u64()
        .unwrap_or(0);

    println!(
        "  Response: {} ({})",
        status,
        format_provider_test_duration(elapsed)
    );
    if content.is_empty() {
        println!("  Content:  content_empty");
    } else {
        println!("  Content:  {content}");
    }
    println!("  Tokens:   input={prompt_tokens}, output={output_tokens}");
    println!();
    println!("  \u{2713} Provider '{provider_name}' is working");
    let usage = roko_core::Usage {
        input_tokens: prompt_tokens as u32,
        output_tokens: output_tokens as u32,
        ..Default::default()
    };
    let cost = estimate_provider_test_cost(model, &usage);
    Ok(build_provider_test_report(
        provider_name,
        provider.kind,
        Some(model),
        Some(content),
        Some(&usage),
        cost,
        elapsed,
    ))
}

pub(crate) async fn run_cursor_provider_test(
    provider_name: &str,
    provider: &ProviderConfig,
    model: Option<&ModelProfile>,
    _json: bool,
) -> Result<ProviderTestReport> {
    let base_url = provider
        .base_url
        .as_deref()
        .unwrap_or("http://localhost:3000");

    println!("Testing provider '{provider_name}' ({})...", provider.kind);
    println!("  Base URL: {base_url}");
    if let Some(model) = model {
        println!("  Model:    {}", model.slug);
    }
    println!();

    let url = reqwest::Url::parse(base_url)
        .with_context(|| format!("parse cursor base_url '{base_url}'"))?;
    let host = url.host_str().unwrap_or("localhost");
    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    let addr = format!("{host}:{port}");

    let started = Instant::now();
    match tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            let elapsed = started.elapsed();
            println!(
                "  TCP:     connected ({})",
                format_provider_test_duration(elapsed)
            );
            println!();
            println!("  \u{2713} Provider '{provider_name}' is reachable");
            Ok(build_provider_test_report(
                provider_name,
                provider.kind,
                model,
                None,
                None,
                None,
                elapsed,
            ))
        }
        Ok(Err(e)) => {
            let elapsed = started.elapsed();
            println!(
                "  TCP:     failed ({}) \u{2014} {e}",
                format_provider_test_duration(elapsed)
            );
            bail!("provider '{provider_name}' test failed: {e}")
        }
        Err(_) => {
            println!("  TCP:     timed out (5s)");
            bail!("provider '{provider_name}' test failed: connection timed out")
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteComplexity {
    pub(crate) band: TaskComplexityBand,
    pub(crate) tier_label: &'static str,
}

pub(crate) fn parse_agent_role(input: Option<&str>) -> Result<AgentRole> {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Ok(AgentRole::Implementer);
    };

    let normalized = normalize_route_token(input);
    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS)
        .find(|role| {
            normalize_route_token(role.label()) == normalized
                || normalize_route_token(role.short()) == normalized
        })
        .ok_or_else(|| anyhow!("unknown role '{input}'"))
}

pub(crate) fn parse_route_complexity(input: Option<&str>) -> Result<RouteComplexity> {
    let Some(input) = input.map(str::trim).filter(|input| !input.is_empty()) else {
        return Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "focused",
        });
    };

    match normalize_route_token(input).as_str() {
        "mechanical" | "fast" | "low" => Ok(RouteComplexity {
            band: TaskComplexityBand::Fast,
            tier_label: "mechanical",
        }),
        "focused" | "standard" | "medium" => Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "focused",
        }),
        "integrative" => Ok(RouteComplexity {
            band: TaskComplexityBand::Standard,
            tier_label: "integrative",
        }),
        "architectural" | "complex" | "premium" | "high" => Ok(RouteComplexity {
            band: TaskComplexityBand::Complex,
            tier_label: "architectural",
        }),
        _ => bail!("unknown complexity '{input}'"),
    }
}

pub(crate) fn normalize_route_token(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn resolve_requested_model_slug(
    requested_model: &str,
    models: &HashMap<String, ModelProfile>,
) -> Option<String> {
    if let Some(profile) = models.get(requested_model) {
        return Some(profile.slug.clone());
    }

    let normalized = normalize_route_token(requested_model);
    let mut entries = models.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));

    for (model_key, profile) in entries {
        if normalize_route_token(model_key) == normalized
            || normalize_route_token(&profile.slug) == normalized
        {
            return Some(profile.slug.clone());
        }
    }

    None
}

pub(crate) fn model_aliases_by_slug(
    models: &HashMap<String, ModelProfile>,
) -> HashMap<String, String> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for (model_key, profile) in models {
        grouped
            .entry(profile.slug.clone())
            .or_default()
            .push(model_key.clone());
    }

    let mut aliases = HashMap::new();
    for (slug, mut keys) in grouped {
        keys.sort();
        let alias = if keys.len() == 1 {
            keys[0].clone()
        } else {
            slug.clone()
        };
        aliases.insert(slug, alias);
    }
    aliases
}

pub(crate) fn display_model_name(aliases: &HashMap<String, String>, slug: &str) -> String {
    aliases
        .get(slug)
        .cloned()
        .unwrap_or_else(|| slug.to_string())
}

pub(crate) fn model_provider_map(
    models: &HashMap<String, ModelProfile>,
    model_slugs: &[String],
) -> HashMap<String, String> {
    let mut entries = models.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));

    let mut providers = HashMap::new();
    for slug in model_slugs {
        if let Some((_, profile)) = entries.iter().find(|(_, profile)| profile.slug == *slug) {
            providers.insert(slug.clone(), profile.provider.clone());
        }
    }
    providers
}

pub(crate) fn available_model_candidates(
    model_slugs: &[String],
    model_providers: &HashMap<String, String>,
    provider_health: &HashMap<String, ProviderHealth>,
    now_ms: i64,
) -> Vec<String> {
    model_slugs
        .iter()
        .filter(|slug| {
            model_providers
                .get(slug.as_str())
                .map(|provider| provider_is_available(provider_health.get(provider), now_ms))
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

pub(crate) fn provider_is_available(health: Option<&ProviderHealth>, now_ms: i64) -> bool {
    health
        .map(|snapshot| effective_circuit_state(snapshot, now_ms) != CircuitState::Open)
        .unwrap_or(true)
}

pub(crate) fn format_model_route_explanation(
    requested_model: &str,
    requested_slug: &str,
    aliases: &HashMap<String, String>,
    explanation: &CascadeRouteExplanation,
    confidence: &HashMap<String, (u64, u64)>,
    model_providers: &HashMap<String, String>,
    provider_health: &HashMap<String, ProviderHealth>,
    latency_registry: &LatencyRegistry,
    cost_table: &CostTable,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Routing decision for '{requested_model}':");
    let _ = writeln!(
        out,
        "  Stage: {} ({} observations)",
        format_route_stage(explanation.stage),
        explanation.observations
    );
    if let Some(alpha) = explanation.alpha {
        let _ = writeln!(out, "  Alpha: {alpha:.3} ({})", describe_alpha(alpha));
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  Candidate Scores:");

    let candidate_names = explanation
        .candidates
        .iter()
        .map(|candidate| display_model_name(aliases, &candidate.slug))
        .collect::<Vec<_>>();
    let name_width = candidate_names
        .iter()
        .map(String::len)
        .max()
        .unwrap_or("Model".len())
        .max("Model".len());

    for (candidate, name) in explanation.candidates.iter().zip(candidate_names.iter()) {
        let (trials, successes) = confidence.get(&candidate.slug).copied().unwrap_or((0, 0));
        let pass_rate = if trials > 0 {
            successes as f64 / trials as f64
        } else {
            0.0
        };
        let provider = model_providers.get(&candidate.slug).map(String::as_str);
        let cost = normalized_cost(&candidate.slug, cost_table);
        let latency = provider
            .and_then(|provider| {
                normalized_latency_for_model(&candidate.slug, provider, latency_registry)
            })
            .unwrap_or(0.0);
        let selected_marker = if candidate.selected {
            "  <- selected"
        } else {
            ""
        };

        let _ = writeln!(
            out,
            "    {:<name_width$}  {:>5.3}  (pass: {:>3.0}%, cost: {:.2}, latency: {:.2}){}",
            name,
            candidate.score,
            pass_rate * 100.0,
            cost,
            latency,
            selected_marker,
            name_width = name_width,
        );
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  Provider Health:");
    let mut providers = explanation
        .candidates
        .iter()
        .filter_map(|candidate| model_providers.get(&candidate.slug))
        .cloned()
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        let _ = writeln!(out, "    none");
    } else {
        let now_ms = unix_ms_now();
        for provider in providers {
            let status = format_provider_health_note(provider_health.get(&provider), now_ms);
            let _ = writeln!(out, "    {provider}: {status}");
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "  Cache Affinity:");
    let previous_name = display_model_name(aliases, requested_slug);
    let affinity_note = match explanation.stage {
        roko_learn::cascade_router::CascadeStage::Confidence
            if explanation
                .candidates
                .iter()
                .any(|candidate| candidate.slug == requested_slug) =>
        {
            "(+0.15 bonus applied)"
        }
        roko_learn::cascade_router::CascadeStage::Ucb
            if explanation
                .candidates
                .iter()
                .any(|candidate| candidate.slug == requested_slug) =>
        {
            "(affinity feature active)"
        }
        _ => "(no matching candidate bonus)",
    };
    let _ = writeln!(out, "    Previous model: {previous_name} {affinity_note}");

    let _ = writeln!(out);
    let _ = writeln!(out, "  Pareto Status:");
    let selected_name = display_model_name(aliases, &explanation.selected_slug);
    let pareto_status = if explanation
        .pareto_frontier
        .iter()
        .any(|slug| slug == &explanation.selected_slug)
    {
        "ON frontier (not dominated)"
    } else {
        "OFF frontier (dominated)"
    };
    let _ = writeln!(out, "    {selected_name}: {pareto_status}");

    let selected_provider = model_providers
        .get(&explanation.selected_slug)
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "  Final: {} via {}",
        display_model_name(aliases, &explanation.selected_slug),
        selected_provider
    );
    out
}

pub(crate) fn format_route_stage(stage: roko_learn::cascade_router::CascadeStage) -> &'static str {
    match stage {
        roko_learn::cascade_router::CascadeStage::Static => "Static",
        roko_learn::cascade_router::CascadeStage::Confidence => "Confidence",
        roko_learn::cascade_router::CascadeStage::Ucb => "UCB",
    }
}

pub(crate) fn describe_alpha(alpha: f64) -> &'static str {
    if alpha <= 0.10 {
        "mostly exploitation"
    } else if alpha <= 0.25 {
        "balanced exploration"
    } else {
        "exploration-heavy"
    }
}

pub(crate) fn normalized_latency_for_model(
    model_slug: &str,
    provider: &str,
    latency_registry: &LatencyRegistry,
) -> Option<f64> {
    let stats = latency_registry.get(model_slug, provider)?;
    let sla_ms = default_latency_sla_for_slug(model_slug) as f64;
    (sla_ms > 0.0).then(|| (stats.total_latency_ema_ms / sla_ms).min(1.0))
}

pub(crate) fn default_latency_sla_for_slug(slug: &str) -> u64 {
    if slug.contains("haiku") {
        10_000
    } else if slug.contains("opus") || slug.contains("premium") {
        120_000
    } else {
        30_000
    }
}

pub(crate) fn format_provider_health_note(health: Option<&ProviderHealth>, now_ms: i64) -> String {
    let Some(health) = health else {
        return "CLOSED (healthy)".to_string();
    };

    match effective_circuit_state(health, now_ms) {
        CircuitState::Closed => "CLOSED (healthy)".to_string(),
        CircuitState::HalfOpen => "HALF-OPEN (probe allowed)".to_string(),
        CircuitState::Open => {
            let cooldown = format_cooldown(Some(health), CircuitState::Open, now_ms);
            if cooldown == "—" {
                "OPEN (cooldown active)".to_string()
            } else {
                format!("OPEN ({cooldown})")
            }
        }
    }
}

pub(crate) fn cascade_router_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("cascade-router.json")
}

pub(crate) fn provider_health_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("provider-health.json")
}

pub(crate) fn latency_stats_path(workdir: &Path) -> PathBuf {
    RokoLayout::for_project(workdir)
        .learn_dir()
        .join("latency-stats.json")
}

pub(crate) fn load_provider_health_snapshot(
    path: &Path,
) -> Result<HashMap<String, ProviderHealth>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let snapshot: ProviderHealthSnapshot =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(snapshot.providers)
}

pub(crate) fn load_latency_stats_by_provider(
    path: &Path,
) -> Result<HashMap<String, ProviderLatencySummary>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let snapshot: LatencyStatsSnapshot =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;

    let mut providers = HashMap::new();
    for entry in snapshot.entries {
        providers
            .entry(entry.provider)
            .or_insert_with(ProviderLatencySummary::default)
            .record(&entry.stats);
    }

    Ok(providers)
}

pub(crate) fn build_provider_health_row(
    provider: &str,
    health: Option<&ProviderHealth>,
    latency: Option<&ProviderLatencySummary>,
    now_ms: i64,
    health_file_ms: Option<i64>,
    latency_file_ms: Option<i64>,
) -> ProviderHealthRow {
    let state = health
        .map(|snapshot| effective_circuit_state(snapshot, now_ms))
        .unwrap_or(CircuitState::Closed);
    let fails = health
        .map(|snapshot| {
            format!(
                "{}/{}",
                snapshot.consecutive_failures, PROVIDER_FAILURE_THRESHOLD
            )
        })
        .unwrap_or_else(|| format!("0/{PROVIDER_FAILURE_THRESHOLD}"));
    let cooldown = format_cooldown(health, state, now_ms);
    let latency_p50 = latency
        .and_then(ProviderLatencySummary::p50_ms)
        .map(format_latency_p50)
        .unwrap_or_else(|| "—".to_string());
    let error_rate = health
        .filter(|snapshot| snapshot.total_requests > 0)
        .map(|snapshot| {
            format!(
                "{:.1}%",
                (snapshot.total_failures as f64 * 100.0) / snapshot.total_requests as f64
            )
        })
        .unwrap_or_else(|| "—".to_string());

    let mut last_check_ms = health.and_then(|snapshot| snapshot.last_failure_at);
    if health.is_some() {
        last_check_ms = max_timestamp(last_check_ms, health_file_ms);
    }
    if latency.is_some() {
        last_check_ms = max_timestamp(last_check_ms, latency_file_ms);
    }

    ProviderHealthRow {
        provider: provider.to_string(),
        state: format_circuit_state(state).to_string(),
        fails,
        cooldown,
        latency_p50,
        error_rate,
        last_check: last_check_ms
            .map(|timestamp_ms| format_timestamp_age(timestamp_ms, now_ms))
            .unwrap_or_else(|| "—".to_string()),
    }
}

pub(crate) fn effective_circuit_state(health: &ProviderHealth, now_ms: i64) -> CircuitState {
    match health.state {
        CircuitState::Open if health.cooldown_until.is_some_and(|until| now_ms >= until) => {
            CircuitState::HalfOpen
        }
        state => state,
    }
}

pub(crate) fn format_circuit_state(state: CircuitState) -> &'static str {
    match state {
        CircuitState::Closed => "CLOSED",
        CircuitState::Open => "OPEN",
        CircuitState::HalfOpen => "HALF-OPEN",
    }
}

pub(crate) fn format_cooldown(
    health: Option<&ProviderHealth>,
    state: CircuitState,
    now_ms: i64,
) -> String {
    let Some(health) = health else {
        return "—".to_string();
    };

    if state != CircuitState::Open {
        return "—".to_string();
    }

    health
        .cooldown_until
        .map(|until| until.saturating_sub(now_ms))
        .filter(|remaining_ms| *remaining_ms > 0)
        .map(format_remaining_ms)
        .unwrap_or_else(|| "—".to_string())
}

pub(crate) fn format_provider_health_rows(rows: &[ProviderHealthRow]) -> String {
    let mut widths = [
        "Provider".len(),
        "State".len(),
        "Fails".len(),
        "Cooldown".len(),
        "Latency p50".len(),
        "Error Rate".len(),
        "Last Check".len(),
    ];

    for row in rows {
        widths[0] = widths[0].max(row.provider.len());
        widths[1] = widths[1].max(row.state.len());
        widths[2] = widths[2].max(row.fails.len());
        widths[3] = widths[3].max(row.cooldown.len());
        widths[4] = widths[4].max(row.latency_p50.len());
        widths[5] = widths[5].max(row.error_rate.len());
        widths[6] = widths[6].max(row.last_check.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<provider_w$}  {:<state_w$}  {:<fails_w$}  {:<cooldown_w$}  {:<latency_w$}  {:<error_w$}  {:<last_w$}",
        "Provider",
        "State",
        "Fails",
        "Cooldown",
        "Latency p50",
        "Error Rate",
        "Last Check",
        provider_w = widths[0],
        state_w = widths[1],
        fails_w = widths[2],
        cooldown_w = widths[3],
        latency_w = widths[4],
        error_w = widths[5],
        last_w = widths[6],
    );

    for row in rows {
        let _ = writeln!(
            out,
            "{:<provider_w$}  {:<state_w$}  {:<fails_w$}  {:<cooldown_w$}  {:<latency_w$}  {:<error_w$}  {:<last_w$}",
            row.provider,
            row.state,
            row.fails,
            row.cooldown,
            row.latency_p50,
            row.error_rate,
            row.last_check,
            provider_w = widths[0],
            state_w = widths[1],
            fails_w = widths[2],
            cooldown_w = widths[3],
            latency_w = widths[4],
            error_w = widths[5],
            last_w = widths[6],
        );
    }

    out
}

pub(crate) fn format_latency_p50(ms: f64) -> String {
    if ms >= 500.0 {
        format!("{:.1}s", ms / 1000.0)
    } else {
        format!("{ms:.0}ms")
    }
}

pub(crate) fn format_remaining_ms(ms: i64) -> String {
    let secs = (ms.max(0) + 999) / 1000;
    format!("{} left", format_compact_duration(secs))
}

pub(crate) fn format_timestamp_age(timestamp_ms: i64, now_ms: i64) -> String {
    let secs = now_ms.saturating_sub(timestamp_ms).max(0) / 1000;
    format!("{} ago", format_compact_duration(secs))
}

pub(crate) fn format_compact_duration(secs: i64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

pub(crate) fn file_modified_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    system_time_to_ms(modified)
}

pub(crate) fn system_time_to_ms(timestamp: SystemTime) -> Option<i64> {
    timestamp
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

pub(crate) fn unix_ms_now() -> i64 {
    system_time_to_ms(SystemTime::now()).unwrap_or(0)
}

pub(crate) fn max_timestamp(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderListRow {
    pub(crate) provider: String,
    pub(crate) kind: String,
    pub(crate) base_url: String,
    pub(crate) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelListRow {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) slug: String,
    pub(crate) context: String,
    pub(crate) tools: String,
    pub(crate) thinking: String,
    pub(crate) vision: String,
    pub(crate) cost: String,
}

pub(crate) const PROVIDER_FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderTestAllRow {
    pub(crate) provider: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHealthRow {
    pub(crate) provider: String,
    pub(crate) state: String,
    pub(crate) fails: String,
    pub(crate) cooldown: String,
    pub(crate) latency_p50: String,
    pub(crate) error_rate: String,
    pub(crate) last_check: String,
}
