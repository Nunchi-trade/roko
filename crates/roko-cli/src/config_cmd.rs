//! `roko config` subcommand group — setup wizard, provenance viewer, editor.
//!
//! These commands operate on the global (`~/.roko/config.toml`) and/or
//! project (`./roko.toml`) config files. The `init` wizard is the primary
//! onboarding path: it detects installed LLM CLIs and writes a working
//! global config with one interactive pass.

use crate::config::{
    AgentLayer, ConfigLayer, ConfigPaths, DetectedCli, ExecutorLayer, GateConfig, PromptLayer,
    ResolvedConfig, ServeAuthLayer, ServeLayer, Source, ToolsLayer, apply_layer_value, detect_clis,
    global_config_path, load_layered, resolve_paths,
};
use anyhow::{Context as _, Result, anyhow};
use roko_core::agent::ProviderKind;
use roko_core::config::schema::{CURRENT_SCHEMA_VERSION, ModelProfile, ProviderConfig, RokoConfig};
use roko_core::tool::{ToolFormat, profile_for_model};
use roko_orchestrator::ExecutorConfig;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VALIDATION_REACHABILITY_TIMEOUT_SECS: u64 = 2;

/// Non-interactive inputs for `config init` (used by CI / tests).
#[derive(Clone, Debug, Default)]
pub struct WizardInputs {
    /// Pre-selected agent command (skip CLI picker).
    pub agent_command: Option<String>,
    /// Extra args for the agent (replaces the detected defaults).
    pub agent_args: Option<Vec<String>>,
    /// Preferred model slug, if the backend supports one.
    pub model: Option<String>,
    /// Token budget for prompt composition.
    pub token_budget: Option<usize>,
    /// System role text.
    pub role: Option<String>,
    /// Whether to enable cargo compile+clippy gates by default.
    pub enable_gates: Option<bool>,
    /// Skip the final confirmation and write without asking.
    pub yes: bool,
}

/// Run the config-init wizard interactively and write a global config file.
///
/// `target` overrides the global path (for tests). `inputs` pre-fills
/// answers; any field left `None` triggers an interactive prompt.
pub fn run_init_wizard(target: Option<PathBuf>, inputs: &WizardInputs) -> Result<PathBuf> {
    let path = target.unwrap_or_else(global_config_path);
    println!("\nRoko config wizard");
    println!("==================");
    println!("Writing global config to: {}\n", path.display());

    // 1. Agent backend.
    let detected = detect_clis();
    let agent_command = resolve_agent_command(inputs.agent_command.clone(), &detected)?;
    let suggested_args = detected
        .iter()
        .find(|d| d.command == agent_command)
        .map(|d| d.default_args.clone())
        .unwrap_or_default();
    let agent_args = resolve_agent_args(inputs.agent_args.clone(), &agent_command, suggested_args)?;

    // 2. Token budget.
    let token_budget = match inputs.token_budget {
        Some(b) => b,
        None => prompt_usize("Token budget for prompt composition", 8000)?,
    };

    // 3. Role text.
    let role = match &inputs.role {
        Some(r) => r.clone(),
        None => prompt_string(
            "System role / persona",
            "You are a Roko agent — concise, precise, and correct.",
        )?,
    };

    // 4. Default gates.
    let enable_gates = match inputs.enable_gates {
        Some(v) => v,
        None => prompt_bool("Enable default cargo gates (compile + clippy)?", false)?,
    };
    let gates = if enable_gates {
        Some(vec![
            GateConfig::Compile {
                build_system: "cargo".into(),
                timeout_ms: 600_000,
            },
            GateConfig::Clippy {
                build_system: "cargo".into(),
                timeout_ms: 600_000,
            },
        ])
    } else {
        None
    };

    let layer = ConfigLayer {
        agent: Some(AgentLayer {
            command: Some(agent_command),
            args: Some(agent_args),
            model: inputs.model.clone(),
            effort: None,
            bare_mode: None,
            fallback_model: None,
            timeout_ms: None,
            env: None,
            clean_output: None,
            mcp_config: None,
        }),
        auto_plan: None,
        dreams: None,
        daimon: None,
        tools: Some(ToolsLayer {
            prefer_mcp: Some(false),
            global_denied: Some(Vec::new()),
            mcp_timeout_secs: Some(30),
        }),
        prompt: Some(PromptLayer {
            token_budget: Some(token_budget),
            role: Some(role),
            files: None,
        }),
        repos: None,
        gates,
        executor: Some(default_executor_layer()),
        runtime: None,
        providers: None,
        models: None,
        serve: Some(ServeLayer {
            port: None,
            terminal_enabled: None,
            auto_orchestrate: None,
            auth: Some(ServeAuthLayer {
                enabled: Some(false),
                api_key: Some(String::new()),
            }),
            deploy: None,
        }),
    };
    let rendered = toml::to_string_pretty(&layer).context("serialize config")?;

    println!("\n--- generated config ---");
    println!("{rendered}");
    println!("--- end config ---\n");

    if !inputs.yes && path.exists() {
        let diff_ok = prompt_bool(
            &format!("{} already exists. Overwrite?", path.display()),
            false,
        )?;
        if !diff_ok {
            return Err(anyhow!("cancelled"));
        }
    } else if !inputs.yes {
        let confirm = prompt_bool(&format!("Write to {}?", path.display()), true)?;
        if !confirm {
            return Err(anyhow!("cancelled"));
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, rendered).with_context(|| format!("write {}", path.display()))?;
    println!("wrote {}", path.display());

    // Also create ~/.roko/.env with a commented template if it doesn't exist.
    if let Some(parent) = path.parent() {
        let env_path = parent.join(".env");
        if !env_path.exists() {
            let template = "\
# Roko environment — secrets and API keys.
# This file is loaded at startup by load_startup_env_files().
#
# ANTHROPIC_API_KEY=sk-ant-...
# GITHUB_TOKEN=ghp_...
# SLACK_BOT_TOKEN=xoxb-...
";
            std::fs::write(&env_path, template)
                .with_context(|| format!("write {}", env_path.display()))?;
            println!("wrote {}", env_path.display());
        }
    }

    Ok(path)
}

fn default_executor_layer() -> ExecutorLayer {
    let defaults = ExecutorConfig::default();
    ExecutorLayer {
        max_concurrent_plans: Some(defaults.max_concurrent_plans),
        max_concurrent_tasks: Some(defaults.max_concurrent_tasks),
        max_auto_fix_iterations: Some(defaults.max_auto_fix_iterations),
        max_merge_attempts: Some(defaults.max_merge_attempts),
        task_timeout_secs: Some(defaults.task_timeout_secs),
        budget_usd: defaults.budget_usd,
        auto_replan: Some(defaults.auto_replan),
        use_worktrees: Some(defaults.use_worktrees),
        speculative_threshold_multiplier: Some(defaults.speculative_threshold_multiplier),
    }
}

/// Print the effective merged config with `[source]` tags on each field.
pub fn cmd_show(workdir: &Path) -> Result<()> {
    let resolved = load_layered(workdir)?;
    print_resolved(&resolved);
    Ok(())
}

/// Print the resolved config paths (global + project + env override).
pub fn cmd_path(workdir: &Path) -> Result<()> {
    let resolved = load_layered(workdir)?;
    let global_exists = if resolved.paths.global.is_file() {
        "exists"
    } else {
        "missing"
    };
    println!(
        "global : {} ({global_exists})",
        resolved.paths.global.display()
    );
    match &resolved.paths.project {
        Some(p) => println!("project: {}", p.display()),
        None => println!("project: (none)"),
    }
    if let Some(env) = &resolved.paths.env_override {
        println!("env    : {} (via ROKO_CONFIG)", env.display());
    }
    Ok(())
}

/// Scan the active config file for `${VAR}` references and validate them.
///
/// All referenced env vars must exist and non-empty. `GITHUB_TOKEN` is
/// additionally validated against the GitHub API, and `SLACK_BOT_TOKEN`
/// against Slack's `auth.test` API.
pub fn cmd_check_secrets(workdir: &Path) -> Result<()> {
    let paths = resolve_paths(workdir);
    let config_path = secret_check_config_path(&paths)?;
    let text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;
    let tokens = collect_env_tokens(&text)?;

    if tokens.is_empty() {
        println!("no `${{VAR}}` tokens found in {}", config_path.display());
        return Ok(());
    }

    println!(
        "checking {} referenced secret token(s) in {}",
        tokens.len(),
        config_path.display()
    );

    let client = reqwest::blocking::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_secs(10))
        .build()
        .context("build HTTP client")?;

    let mut missing = Vec::new();
    let mut invalid = Vec::new();

    for token in tokens {
        print!("  {token}: ");
        match std::env::var(&token) {
            Ok(value) if !value.is_empty() => {
                if let Some(target) = secret_validation_target(&token) {
                    match validate_secret_token(&client, target, &value) {
                        Ok(()) => {
                            println!("valid ({})", target.label());
                        }
                        Err(err) => {
                            println!("invalid ({err})");
                            invalid.push(format!("{token}: {err}"));
                        }
                    }
                } else {
                    println!("present");
                }
            }
            _ => {
                println!("missing");
                missing.push(token);
            }
        }
    }

    if missing.is_empty() && invalid.is_empty() {
        println!("all referenced secret tokens are set and valid");
        return Ok(());
    }

    let mut message = String::from("secret check failed");
    if !missing.is_empty() {
        message.push_str(&format!("\nmissing: {}", missing.join(", ")));
    }
    if !invalid.is_empty() {
        message.push_str(&format!("\ninvalid: {}", invalid.join(", ")));
    }
    Err(anyhow!(message))
}

/// Validate the active `roko.toml` in three phases: syntax, schema, semantics.
pub async fn cmd_validate(workdir: &Path) -> Result<()> {
    let paths = resolve_paths(workdir);
    let config_path = validate_config_path(&paths, workdir)?;
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("read config {}", config_path.display()))?;

    if let Err(err) = toml::from_str::<toml::Value>(&text) {
        print_phase_status("Phase 1: TOML syntax", false);
        println!("  ✗ {err}");
        println!();
        println!("Result: 0 warnings, 1 error");
        return Err(anyhow!("config validation failed"));
    }
    print_phase_status("Phase 1: TOML syntax", true);

    let config = match RokoConfig::from_toml(&text) {
        Ok(config) => config,
        Err(err) => {
            print_phase_status("Phase 2: Schema validation", false);
            println!("  ✗ {err}");
            println!();
            println!("Result: 0 warnings, 1 error");
            return Err(anyhow!("config validation failed"));
        }
    };
    print_phase_status("Phase 2: Schema validation", true);

    let client = reqwest::Client::builder()
        .user_agent("roko-cli/0.1")
        .timeout(Duration::from_secs(VALIDATION_REACHABILITY_TIMEOUT_SECS))
        .build()
        .context("build validation HTTP client")?;
    let report = semantic_validate_config(&config, &client).await;

    println!("Phase 3: Semantic validation:");
    print_semantic_result(
        "All model providers exist in [providers.*]",
        &report.provider_reference_errors,
    );
    print_semantic_result(
        "Fallback chain models exist",
        &report.fallback_reference_errors,
    );
    print_semantic_result("Tier model keys exist", &report.tier_model_errors);
    print_semantic_result("API key env vars are set", &report.api_key_errors);
    for warning in &report.warnings {
        println!("  ⚠ {warning}");
    }

    println!();
    println!(
        "Result: {} warnings, {} errors",
        report.warning_count(),
        report.error_count()
    );

    if report.error_count() == 0 {
        Ok(())
    } else {
        Err(anyhow!("config validation failed"))
    }
}

/// Migrate a legacy project-local `roko.toml` into explicit provider/model tables.
pub fn cmd_migrate(workdir: &Path, dry_run: bool, yes: bool) -> Result<()> {
    let paths = resolve_paths(workdir);
    let config_path = validate_config_path(&paths, workdir)?;
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let plan = build_config_migration_plan(&text)?;

    match plan {
        ConfigMigrationPlan::AlreadyCurrent => {
            println!("roko.toml already uses [providers.*]; nothing to migrate");
            Ok(())
        }
        ConfigMigrationPlan::Legacy(plan) => {
            println!("Detected roko.toml version 1 (no [providers] section)");
            println!();
            println!("Proposed changes:");
            for line in render_migration_preview(&plan)? {
                println!("{line}");
            }

            if dry_run {
                println!();
                println!("[dry-run] no changes written");
                return Ok(());
            }

            println!();
            if !yes && !prompt_bool("Apply changes?", false)? {
                return Err(anyhow!("cancelled"));
            }

            fs::write(&config_path, &plan.rendered)
                .with_context(|| format!("write {}", config_path.display()))?;
            println!("updated {}", config_path.display());
            Ok(())
        }
    }
}

/// Set a secret in `~/.roko/.env`, updating an existing key if present.
pub fn cmd_set_secret(name: &str, value: &str) -> Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    let path = PathBuf::from(home).join(".roko").join(".env");
    write_secret_env_file(&path, name, value)?;
    println!("set {name} in {}", path.display());
    Ok(())
}

fn write_secret_env_file(path: &Path, name: &str, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("read {}", path.display())),
    };
    let rendered = upsert_env_assignment(&existing, name, value);
    write_atomic_restricted(path, &rendered)?;
    Ok(())
}

fn upsert_env_assignment(existing: &str, name: &str, value: &str) -> String {
    let mut lines = Vec::new();
    let mut replaced = false;
    let replacement = format!("{name}={value}");

    for line in existing.lines() {
        if env_assignment_name(line).as_deref() == Some(name) {
            lines.push(replacement.clone());
            replaced = true;
        } else {
            lines.push(line.to_string());
        }
    }

    if !replaced {
        lines.push(replacement);
    }

    lines.join("\n")
}

fn env_assignment_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (name, _) = trimmed.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn write_atomic_restricted(path: &Path, text: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path {} has no file name", path.display()))?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_nanos();
    let tmp_path = parent.join(format!(".{}.{}.tmp", file_name.to_string_lossy(), unique));

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp_path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", tmp_path.display()))?;
    }

    fs::rename(&tmp_path, path).with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// Open `$EDITOR` on the global or project config file (creating it if needed).
pub fn cmd_edit(workdir: &Path, which: EditTarget) -> Result<()> {
    let resolved = load_layered(workdir)?;
    let path = match which {
        EditTarget::Global => resolved.paths.global,
        EditTarget::Project => resolved
            .paths
            .project
            .unwrap_or_else(|| workdir.join("roko.toml")),
        EditTarget::Auto => resolved.paths.project.unwrap_or(resolved.paths.global),
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if !path.exists() {
        std::fs::write(&path, "# roko config\n")
            .with_context(|| format!("create {}", path.display()))?;
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("spawn {editor}"))?;
    if !status.success() {
        return Err(anyhow!("{editor} exited non-zero"));
    }
    println!("edited {}", path.display());
    Ok(())
}

/// Set a single dotted-key value and write it to the chosen layer file.
pub fn cmd_set(workdir: &Path, target: EditTarget, key: &str, value: &str) -> Result<()> {
    let resolved = load_layered(workdir)?;
    let path = match target {
        EditTarget::Global | EditTarget::Auto => resolved.paths.global,
        EditTarget::Project => resolved
            .paths
            .project
            .unwrap_or_else(|| workdir.join("roko.toml")),
    };

    let mut layer = if path.exists() {
        ConfigLayer::from_file(&path)?
    } else {
        ConfigLayer::default()
    };
    apply_key_value(&mut layer, key, value).with_context(|| format!("set {key} = {value}"))?;

    let rendered = toml::to_string_pretty(&layer).context("serialize config")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    std::fs::write(&path, rendered).with_context(|| format!("write {}", path.display()))?;
    println!("set {key} = {value} in {}", path.display());
    Ok(())
}

/// Which file `config edit` / `config set` should target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTarget {
    /// `~/.roko/config.toml`.
    Global,
    /// `./roko.toml` (creating it if absent).
    Project,
    /// Project if one exists, else global.
    Auto,
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn print_resolved(r: &ResolvedConfig) {
    println!("effective config:");
    println!(
        "  auto_plan         = {} {}",
        r.config.auto_plan,
        r.sources.auto_plan.tag()
    );
    println!(
        "  agent.command      = {:?} {}",
        r.config.agent.command,
        r.sources.agent_command.tag()
    );
    println!(
        "  agent.args         = {:?} {}",
        r.config.agent.args,
        r.sources.agent_args.tag()
    );
    println!(
        "  agent.model        = {:?} {}",
        r.config.agent.model,
        r.sources.agent_model.tag()
    );
    println!(
        "  agent.effort       = {:?} {}",
        r.config.agent.effort,
        r.sources.agent_effort.tag()
    );
    println!(
        "  agent.bare_mode    = {} {}",
        r.config.agent.bare_mode,
        r.sources.agent_bare_mode.tag()
    );
    println!(
        "  agent.fallback_model = {:?} {}",
        r.config.agent.fallback_model,
        r.sources.agent_fallback_model.tag()
    );
    println!(
        "  agent.timeout_ms   = {} {}",
        r.config.agent.timeout_ms,
        r.sources.agent_timeout_ms.tag()
    );
    println!(
        "  tools.prefer_mcp   = {} {}",
        r.config.tools.prefer_mcp,
        r.sources.tools_prefer_mcp.tag()
    );
    println!(
        "  tools.global_denied = {:?} {}",
        r.config.tools.global_denied,
        r.sources.tools_global_denied.tag()
    );
    println!(
        "  tools.mcp_timeout_secs = {} {}",
        r.config.tools.mcp_timeout_secs,
        r.sources.tools_mcp_timeout_secs.tag()
    );
    println!(
        "  prompt.token_budget= {} {}",
        r.config.prompt.token_budget,
        r.sources.prompt_token_budget.tag()
    );
    println!(
        "  prompt.role        = {:?} {}",
        r.config.prompt.role,
        r.sources.prompt_role.tag()
    );
    println!(
        "  providers         = {:?} {}",
        r.config.providers,
        r.sources.providers.tag()
    );
    println!(
        "  models            = {:?} {}",
        r.config.models,
        r.sources.models.tag()
    );
    println!(
        "  dreams.auto_dream  = {} {}",
        r.config.dreams.auto_dream,
        r.sources.dreams_auto_dream.tag()
    );
    println!(
        "  dreams.idle_threshold_mins = {} {}",
        r.config.dreams.idle_threshold_mins,
        r.sources.dreams_idle_threshold_mins.tag()
    );
    println!(
        "  dreams.min_episodes_for_dream = {} {}",
        r.config.dreams.min_episodes_for_dream,
        r.sources.dreams_min_episodes_for_dream.tag()
    );
    println!(
        "  gates              = {} entries {}",
        r.config.gates.len(),
        r.sources.gates.tag()
    );
    println!();
    println!("sources:");
    println!("  global : {}", r.paths.global.display());
    match &r.paths.project {
        Some(p) => println!("  project: {}", p.display()),
        None => println!("  project: (none)"),
    }
    if let Some(env) = &r.paths.env_override {
        println!("  env    : {} (ROKO_CONFIG)", env.display());
    }
    let fully_default = r.sources.agent_command == Source::Default
        && r.sources.auto_plan == Source::Default
        && r.sources.prompt_token_budget == Source::Default
        && r.sources.prompt_role == Source::Default
        && r.sources.providers == Source::Default
        && r.sources.models == Source::Default
        && r.sources.tools_prefer_mcp == Source::Default
        && r.sources.dreams_auto_dream == Source::Default
        && r.sources.dreams_idle_threshold_mins == Source::Default
        && r.sources.dreams_min_episodes_for_dream == Source::Default;
    if fully_default {
        println!("\nhint: no config files found — run `roko config init` to set one up.");
    }
}

fn apply_key_value(layer: &mut ConfigLayer, key: &str, value: &str) -> Result<()> {
    apply_layer_value(layer, key, value)
}

#[derive(Debug)]
enum ConfigMigrationPlan {
    AlreadyCurrent,
    Legacy(LegacyConfigMigration),
}

#[derive(Debug)]
struct LegacyConfigMigration {
    provider_name: String,
    provider: ProviderConfig,
    models: Vec<(String, ModelProfile)>,
    rendered: String,
}

fn build_config_migration_plan(text: &str) -> Result<ConfigMigrationPlan> {
    let raw_value: toml::Value = toml::from_str(text).context("parse config toml")?;
    let mut raw = raw_value
        .as_table()
        .cloned()
        .ok_or_else(|| anyhow!("config root must be a TOML table"))?;

    if raw
        .get("providers")
        .and_then(toml::Value::as_table)
        .is_some_and(|providers| !providers.is_empty())
    {
        return Ok(ConfigMigrationPlan::AlreadyCurrent);
    }

    let config = RokoConfig::from_toml(text).context("parse roko config")?;
    let provider = legacy_provider_config(&config)?;
    let models = legacy_model_profiles(&config, &provider.0)?;

    let provider_value = toml::Value::try_from(provider.1.clone()).context("serialize provider")?;
    let mut providers_table = toml::map::Map::new();
    providers_table.insert(provider.0.clone(), provider_value);
    raw.insert("providers".to_string(), toml::Value::Table(providers_table));

    let mut models_table = toml::map::Map::new();
    for (model_key, profile) in &models {
        let value = toml::Value::try_from(profile.clone())
            .with_context(|| format!("serialize model '{model_key}'"))?;
        models_table.insert(model_key.clone(), value);
    }
    raw.insert("models".to_string(), toml::Value::Table(models_table));
    raw.insert(
        "schema_version".to_string(),
        toml::Value::Integer(i64::from(CURRENT_SCHEMA_VERSION)),
    );

    let rendered = toml::to_string_pretty(&toml::Value::Table(raw)).context("serialize config")?;
    Ok(ConfigMigrationPlan::Legacy(LegacyConfigMigration {
        provider_name: provider.0,
        provider: provider.1,
        models,
        rendered,
    }))
}

fn legacy_provider_config(config: &RokoConfig) -> Result<(String, ProviderConfig)> {
    let command = config
        .agent
        .command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| anyhow!("legacy config is missing agent.command"))?;

    match command {
        "claude" => Ok((
            "claude_cli".to_string(),
            ProviderConfig {
                kind: ProviderKind::ClaudeCli,
                base_url: None,
                api_key_env: None,
                command: Some(command.to_string()),
                args: config.agent.args.clone(),
                timeout_ms: config.agent.timeout_ms,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        )),
        "ollama" => Ok((
            "ollama".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some(
                    legacy_agent_env(config.agent.env.as_ref(), "OLLAMA_HOST")
                        .unwrap_or("http://localhost:11434")
                        .to_string(),
                ),
                api_key_env: None,
                command: None,
                args: None,
                timeout_ms: config.agent.timeout_ms,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        )),
        other => Err(anyhow!(
            "legacy agent.command '{other}' cannot be migrated safely; only 'claude' and 'ollama' are supported"
        )),
    }
}

fn legacy_model_profiles(
    config: &RokoConfig,
    provider_name: &str,
) -> Result<Vec<(String, ModelProfile)>> {
    let mut model_keys = BTreeSet::new();

    let default_model = config.agent.default_model.trim();
    if !default_model.is_empty() {
        model_keys.insert(default_model.to_string());
    }

    for model in config.agent.tier_models.values() {
        let model = model.trim();
        if !model.is_empty() {
            model_keys.insert(model.to_string());
        }
    }

    if model_keys.is_empty() {
        return Err(anyhow!(
            "legacy config has no agent.model or agent.tier_models to migrate"
        ));
    }

    Ok(model_keys
        .into_iter()
        .map(|model_key| {
            (
                model_key.clone(),
                synthesized_legacy_model_profile(provider_name, &model_key),
            )
        })
        .collect())
}

fn synthesized_legacy_model_profile(provider_name: &str, slug: &str) -> ModelProfile {
    let tool_profile = profile_for_model(slug);
    let tool_format = match provider_name {
        "claude_cli" => ToolFormat::AnthropicBlocks,
        _ => ToolFormat::OpenAiJson,
    };
    let context_window = if matches!(tool_format, ToolFormat::AnthropicBlocks) {
        200_000
    } else {
        128_000
    };

    ModelProfile {
        provider: provider_name.to_string(),
        slug: slug.to_string(),
        context_window,
        max_output: None,
        supports_tools: tool_profile.supports_tools,
        supports_thinking: false,
        supports_vision: false,
        supports_web_search: false,
        supports_mcp_tools: false,
        supports_partial: false,
        provider_routing: None,
        tool_format: tool_format.as_str().to_string(),
        cost_input_per_m: None,
        cost_output_per_m: None,
        cost_cache_read_per_m: None,
        cost_cache_write_per_m: None,
        max_tools: Some(u32::from(tool_profile.max_tools_before_degrade)),
        tokenizer_ratio: None,
        ..Default::default()
    }
}

fn legacy_agent_env<'a>(env: Option<&'a Vec<(String, String)>>, key: &str) -> Option<&'a str> {
    env.and_then(|entries| {
        entries.iter().find_map(|(name, value)| {
            (name.trim().eq_ignore_ascii_case(key) && !value.trim().is_empty())
                .then_some(value.as_str())
        })
    })
}

fn render_migration_preview(plan: &LegacyConfigMigration) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    lines.extend(render_prefixed_toml_block(single_provider_preview(
        &plan.provider_name,
        &plan.provider,
    )?));
    for (model_key, profile) in &plan.models {
        lines.push("  +".to_string());
        lines.extend(render_prefixed_toml_block(single_model_preview(
            model_key, profile,
        )?));
    }
    lines.push("  +".to_string());
    lines.push(format!("  + schema_version = {CURRENT_SCHEMA_VERSION}"));
    Ok(lines)
}

fn single_provider_preview(name: &str, provider: &ProviderConfig) -> Result<String> {
    let mut outer = toml::map::Map::new();
    let mut providers = toml::map::Map::new();
    providers.insert(
        name.to_string(),
        toml::Value::try_from(provider.clone()).context("serialize provider preview")?,
    );
    outer.insert("providers".to_string(), toml::Value::Table(providers));
    toml::to_string_pretty(&toml::Value::Table(outer)).context("serialize provider preview")
}

fn single_model_preview(model_key: &str, profile: &ModelProfile) -> Result<String> {
    let mut outer = toml::map::Map::new();
    let mut models = toml::map::Map::new();
    models.insert(
        model_key.to_string(),
        toml::Value::try_from(profile.clone()).context("serialize model preview")?,
    );
    outer.insert("models".to_string(), toml::Value::Table(models));
    toml::to_string_pretty(&toml::Value::Table(outer)).context("serialize model preview")
}

fn render_prefixed_toml_block(block: String) -> Vec<String> {
    block
        .trim()
        .lines()
        .map(|line| format!("  + {line}"))
        .collect()
}

#[derive(Debug, Default)]
struct SemanticValidationReport {
    provider_reference_errors: Vec<String>,
    fallback_reference_errors: Vec<String>,
    tier_model_errors: Vec<String>,
    api_key_errors: Vec<String>,
    warnings: Vec<String>,
}

impl SemanticValidationReport {
    fn error_count(&self) -> usize {
        self.provider_reference_errors.len()
            + self.fallback_reference_errors.len()
            + self.tier_model_errors.len()
            + self.api_key_errors.len()
    }

    fn warning_count(&self) -> usize {
        self.warnings.len()
    }
}

fn validate_config_path(paths: &ConfigPaths, workdir: &Path) -> Result<PathBuf> {
    if let Some(path) = &paths.project {
        return Ok(path.clone());
    }

    let direct = workdir.join("roko.toml");
    if direct.is_file() {
        return Ok(direct);
    }

    Err(anyhow!("no roko.toml found to validate"))
}

fn print_phase_status(label: &str, ok: bool) {
    let symbol = if ok { "✓" } else { "✗" };
    println!("{label:.<32} {symbol}");
}

fn print_semantic_result(label: &str, errors: &[String]) {
    if errors.is_empty() {
        println!("  ✓ {label}");
        return;
    }

    for error in errors {
        println!("  ✗ {error}");
    }
}

async fn semantic_validate_config(
    config: &RokoConfig,
    client: &reqwest::Client,
) -> SemanticValidationReport {
    let mut report = SemanticValidationReport::default();
    let providers = config.effective_providers();
    let models = config.effective_models();

    let mut model_entries = config.models.iter().collect::<Vec<_>>();
    model_entries.sort_by(|a, b| a.0.cmp(b.0));
    for (model_key, profile) in model_entries {
        let provider_name = profile.provider.trim();
        if provider_name.is_empty() {
            report.provider_reference_errors.push(format!(
                "Model '{model_key}' has an empty provider reference"
            ));
            continue;
        }
        if !providers.contains_key(provider_name) {
            report.provider_reference_errors.push(format!(
                "Model '{model_key}' references missing provider '{provider_name}'"
            ));
        }
    }

    if let Some(fallback_model) = config.agent.fallback_model.as_deref() {
        let fallback_model = fallback_model.trim();
        if fallback_model.is_empty() {
            report
                .fallback_reference_errors
                .push("agent.fallback_model must not be empty".to_string());
        } else if !models.contains_key(fallback_model) {
            report.fallback_reference_errors.push(format!(
                "agent.fallback_model references missing model '{fallback_model}'"
            ));
        }
    }

    let mut tier_models = config.agent.tier_models.iter().collect::<Vec<_>>();
    tier_models.sort_by(|a, b| a.0.cmp(b.0));
    for (tier, model) in tier_models {
        if tier.trim().is_empty() {
            report
                .tier_model_errors
                .push("agent.tier_models contains an empty tier name".to_string());
        }
        if model.trim().is_empty() {
            report
                .tier_model_errors
                .push(format!("agent.tier_models.{tier} must not be empty"));
        }
    }

    let mut provider_entries = providers.iter().collect::<Vec<_>>();
    provider_entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut unreachable_providers = BTreeSet::new();

    for (provider_name, provider) in provider_entries {
        if let Some(env_name) = provider
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|env_name| !env_name.is_empty())
        {
            let is_set = std::env::var(env_name)
                .ok()
                .is_some_and(|value| !value.trim().is_empty());
            if !is_set {
                report.api_key_errors.push(format!(
                    "Provider '{provider_name}' requires env var '{env_name}', but it is not set"
                ));
            }
        } else if provider.api_key_env.is_some() {
            report.api_key_errors.push(format!(
                "Provider '{provider_name}' has an empty api_key_env value"
            ));
        }

        if let Some(base_url) = provider
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|base_url| !base_url.is_empty())
            && let Some(warning) = probe_validation_base_url(client, provider_name, base_url).await
        {
            unreachable_providers.insert(provider_name.to_string());
            report.warnings.push(warning);
        }
    }

    if !unreachable_providers.is_empty() {
        let mut model_entries = config.models.iter().collect::<Vec<_>>();
        model_entries.sort_by(|a, b| a.0.cmp(b.0));
        for (model_key, profile) in model_entries {
            if unreachable_providers.contains(profile.provider.trim()) {
                report.warnings.push(format!(
                    "Model '{model_key}' references provider '{}' which is unreachable",
                    profile.provider.trim()
                ));
            }
        }
    }

    report
}

async fn probe_validation_base_url(
    client: &reqwest::Client,
    provider_name: &str,
    base_url: &str,
) -> Option<String> {
    match client.head(base_url).send().await {
        Ok(_) => None,
        Err(err) if err.is_timeout() => Some(format!(
            "Provider '{provider_name}' base_url unreachable (timeout {}s)",
            VALIDATION_REACHABILITY_TIMEOUT_SECS
        )),
        Err(err) if err.is_builder() => Some(format!(
            "Provider '{provider_name}' base_url is invalid ({err})"
        )),
        Err(err) => Some(format!(
            "Provider '{provider_name}' base_url unreachable ({err})"
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecretValidationTarget {
    GitHub,
    Slack,
}

impl SecretValidationTarget {
    const fn label(self) -> &'static str {
        match self {
            Self::GitHub => "GitHub API",
            Self::Slack => "Slack API",
        }
    }
}

fn secret_check_config_path(paths: &ConfigPaths) -> Result<PathBuf> {
    if let Some(path) = &paths.env_override {
        return Ok(path.clone());
    }
    if let Some(path) = &paths.project {
        return Ok(path.clone());
    }
    if paths.global.is_file() {
        return Ok(paths.global.clone());
    }
    Err(anyhow!("no config file found to check"))
}

fn collect_env_tokens(text: &str) -> Result<BTreeSet<String>> {
    let value: toml::Value = toml::from_str(text).context("parse config toml")?;
    let mut tokens = BTreeSet::new();
    collect_env_tokens_from_value(&value, &mut tokens);
    Ok(tokens)
}

fn collect_env_tokens_from_value(value: &toml::Value, tokens: &mut BTreeSet<String>) {
    match value {
        toml::Value::String(s) => collect_env_tokens_from_string(s, tokens),
        toml::Value::Array(items) => {
            for item in items {
                collect_env_tokens_from_value(item, tokens);
            }
        }
        toml::Value::Table(entries) => {
            for item in entries.values() {
                collect_env_tokens_from_value(item, tokens);
            }
        }
        _ => {}
    }
}

fn collect_env_tokens_from_string(input: &str, tokens: &mut BTreeSet<String>) {
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            break;
        };
        let candidate = &after[..end];
        if !candidate.is_empty()
            && candidate
                .chars()
                .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        {
            tokens.insert(candidate.to_string());
        }
        rest = &after[end + 1..];
    }
}

fn secret_validation_target(var: &str) -> Option<SecretValidationTarget> {
    match var {
        "GITHUB_TOKEN" | "GH_TOKEN" => Some(SecretValidationTarget::GitHub),
        "SLACK_BOT_TOKEN" | "SLACK_TOKEN" => Some(SecretValidationTarget::Slack),
        _ => None,
    }
}

fn validate_secret_token(
    client: &reqwest::blocking::Client,
    target: SecretValidationTarget,
    token: &str,
) -> Result<()> {
    match target {
        SecretValidationTarget::GitHub => validate_github_token(client, token),
        SecretValidationTarget::Slack => validate_slack_token(client, token),
    }
}

fn validate_github_token(client: &reqwest::blocking::Client, token: &str) -> Result<()> {
    let response = client
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .send()
        .context("call GitHub API")?;
    if response.status().is_success() {
        return Ok(());
    }
    Err(anyhow!("GitHub API returned {}", response.status()))
}

fn validate_slack_token(client: &reqwest::blocking::Client, token: &str) -> Result<()> {
    let response = client
        .post("https://slack.com/api/auth.test")
        .bearer_auth(token)
        .send()
        .context("call Slack API")?;
    let status = response.status();
    let body: serde_json::Value = response.json().context("parse Slack API response")?;
    if body.get("ok").and_then(|value| value.as_bool()) == Some(true) {
        return Ok(());
    }
    let error = body
        .get("error")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown error");
    Err(anyhow!("Slack API returned {status} ({error})"))
}

fn resolve_agent_command(preset: Option<String>, detected: &[DetectedCli]) -> Result<String> {
    if let Some(cmd) = preset {
        return Ok(cmd);
    }
    if detected.is_empty() {
        println!("no LLM CLIs found on PATH — defaulting to `cat` (echo smoke-test mode).");
        return Ok("cat".into());
    }
    println!("Detected agent backends:");
    for (i, d) in detected.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, d.command, d.description);
    }
    loop {
        let raw = prompt_string(
            "Pick an agent backend (number or name)",
            &detected[0].command,
        )?;
        if let Ok(idx) = raw.parse::<usize>() {
            if idx >= 1 && idx <= detected.len() {
                return Ok(detected[idx - 1].command.clone());
            }
        }
        if detected.iter().any(|d| d.command == raw) {
            return Ok(raw);
        }
        println!("  (didn't match — try again)");
    }
}

fn resolve_agent_args(
    preset: Option<Vec<String>>,
    command: &str,
    suggested: Vec<String>,
) -> Result<Vec<String>> {
    if let Some(args) = preset {
        return Ok(args);
    }
    // Special-case ollama: ask for model name.
    if command == "ollama" {
        let model = prompt_string(
            "Ollama model (e.g. llama3, codellama, qwen2.5-coder)",
            "llama3",
        )?;
        return Ok(vec!["run".into(), model]);
    }
    let suggested_str = if suggested.is_empty() {
        "(none)".to_string()
    } else {
        suggested.join(" ")
    };
    let raw = prompt_string(
        &format!("Args for `{command}` (space-separated, default: {suggested_str})"),
        &suggested.join(" "),
    )?;
    if raw.trim().is_empty() {
        return Ok(suggested);
    }
    Ok(raw.split_whitespace().map(String::from).collect())
}

fn prompt_string(label: &str, default: &str) -> Result<String> {
    print!("{label} [{default}]: ");
    io::stdout().flush().ok();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .context("read stdin")?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() {
        default.into()
    } else {
        trimmed.into()
    })
}

fn prompt_usize(label: &str, default: usize) -> Result<usize> {
    loop {
        let raw = prompt_string(label, &default.to_string())?;
        match raw.parse::<usize>() {
            Ok(n) => return Ok(n),
            Err(_) => println!("  (not a number — try again)"),
        }
    }
}

fn prompt_bool(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{label} [{hint}]: ");
        io::stdout().flush().ok();
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .context("read stdin")?;
        let trimmed = line.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("  (enter y/n)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use roko_core::agent::ProviderKind;
    use roko_core::config::schema::{ModelProfile, ProviderConfig};
    use std::collections::HashMap;

    #[test]
    fn apply_key_value_sets_agent_command() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "agent.command", "ollama").unwrap();
        assert_eq!(layer.agent.unwrap().command.unwrap(), "ollama");
    }

    #[test]
    fn apply_key_value_sets_prompt_budget() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "prompt.token_budget", "12345").unwrap();
        assert_eq!(layer.prompt.unwrap().token_budget.unwrap(), 12_345);
    }

    #[test]
    fn apply_key_value_sets_tools_prefer_mcp() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "tools.prefer_mcp", "true").unwrap();
        assert!(layer.tools.unwrap().prefer_mcp.unwrap());
    }

    #[test]
    fn apply_key_value_sets_tools_global_denied() {
        let mut layer = ConfigLayer::default();
        apply_key_value(
            &mut layer,
            "tools.global_denied",
            r#"["write_file","bash"]"#,
        )
        .unwrap();
        assert_eq!(
            layer.tools.unwrap().global_denied.unwrap(),
            vec!["write_file".to_string(), "bash".to_string()]
        );
    }

    #[test]
    fn apply_key_value_sets_tools_timeout() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "tools.mcp_timeout_secs", "75").unwrap();
        assert_eq!(layer.tools.unwrap().mcp_timeout_secs.unwrap(), 75);
    }

    #[test]
    fn apply_key_value_rejects_unknown() {
        let mut layer = ConfigLayer::default();
        assert!(apply_key_value(&mut layer, "bogus.key", "x").is_err());
    }

    #[test]
    fn apply_key_value_parses_args_as_json_array() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "agent.args", r#"["run","llama3"]"#).unwrap();
        assert_eq!(layer.agent.unwrap().args.unwrap(), vec!["run", "llama3"]);
    }

    #[test]
    fn apply_key_value_parses_args_as_whitespace() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "agent.args", "run llama3").unwrap();
        assert_eq!(layer.agent.unwrap().args.unwrap(), vec!["run", "llama3"]);
    }

    #[test]
    fn wizard_writes_global_with_yes_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let inputs = WizardInputs {
            agent_command: Some("cat".into()),
            agent_args: Some(vec![]),
            model: None,
            token_budget: Some(4000),
            role: Some("test role".into()),
            enable_gates: Some(false),
            yes: true,
        };
        let written = run_init_wizard(Some(path.clone()), &inputs).unwrap();
        assert_eq!(written, path);
        assert!(path.exists());
        let layer = ConfigLayer::from_file(&path).unwrap();
        assert_eq!(
            layer.agent.as_ref().unwrap().command.as_deref(),
            Some("cat")
        );
        assert_eq!(layer.prompt.as_ref().unwrap().token_budget, Some(4000));
        assert_eq!(layer.tools.as_ref().unwrap().prefer_mcp, Some(false));
        assert_eq!(layer.tools.as_ref().unwrap().global_denied, Some(vec![]));
        assert_eq!(layer.tools.as_ref().unwrap().mcp_timeout_secs, Some(30));
        assert_eq!(
            layer.serve.as_ref().unwrap().auth.as_ref().unwrap().enabled,
            Some(false)
        );
        assert_eq!(
            layer.serve.as_ref().unwrap().auth.as_ref().unwrap().api_key,
            Some(String::new())
        );
        assert_eq!(
            layer.executor.as_ref().unwrap().max_concurrent_plans,
            Some(4)
        );
    }

    #[test]
    fn apply_key_value_sets_serve_auth() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "serve.auth.enabled", "true").unwrap();
        apply_key_value(&mut layer, "serve.auth.api_key", "secret").unwrap();
        let auth = layer.serve.unwrap().auth.unwrap();
        assert_eq!(auth.enabled, Some(true));
        assert_eq!(auth.api_key, Some("secret".to_string()));
    }

    #[test]
    fn apply_key_value_sets_serve_deploy() {
        let mut layer = ConfigLayer::default();
        apply_key_value(&mut layer, "serve.deploy.provider", "fly").unwrap();
        apply_key_value(
            &mut layer,
            "serve.deploy.environment",
            r#"["GITHUB_TOKEN", "SLACK_BOT_TOKEN"]"#,
        )
        .unwrap();
        apply_key_value(
            &mut layer,
            "serve.deploy.webhooks",
            r#"[{"provider":"github","owner":"nunchi","repo":"roko"}]"#,
        )
        .unwrap();
        let deploy = layer.serve.unwrap().deploy.unwrap();
        assert_eq!(deploy.provider, Some("fly".to_string()));
        assert_eq!(
            deploy.environment,
            Some(vec![
                "GITHUB_TOKEN".to_string(),
                "SLACK_BOT_TOKEN".to_string()
            ])
        );
        let webhook = &deploy.webhooks.unwrap()[0];
        assert_eq!(webhook.provider, Some("github".to_string()));
        assert_eq!(webhook.owner, Some("nunchi".to_string()));
        assert_eq!(webhook.repo, Some("roko".to_string()));
    }

    #[test]
    fn collect_env_tokens_dedupes_nested_strings() {
        let text = r#"
            [agent]
            command = "runner-${GITHUB_TOKEN}"
            args = ["--flag=${SLACK_BOT_TOKEN}", "plain", "again-${GITHUB_TOKEN}"]

            [prompt]
            role = "use ${ANTHROPIC_API_KEY}"
        "#;
        let tokens = collect_env_tokens(text).unwrap();
        assert_eq!(
            tokens.into_iter().collect::<Vec<_>>(),
            vec![
                "ANTHROPIC_API_KEY".to_string(),
                "GITHUB_TOKEN".to_string(),
                "SLACK_BOT_TOKEN".to_string(),
            ]
        );
    }

    #[test]
    fn secret_validation_target_recognizes_known_tokens() {
        assert!(matches!(
            secret_validation_target("GITHUB_TOKEN"),
            Some(SecretValidationTarget::GitHub)
        ));
        assert!(matches!(
            secret_validation_target("SLACK_BOT_TOKEN"),
            Some(SecretValidationTarget::Slack)
        ));
        assert!(secret_validation_target("ANTHROPIC_API_KEY").is_none());
    }

    #[test]
    fn upsert_env_assignment_appends_new_key() {
        let rendered = upsert_env_assignment("EXISTING=1\n# comment", "NEW_SECRET", "value");
        assert_eq!(rendered, "EXISTING=1\n# comment\nNEW_SECRET=value");
    }

    #[test]
    fn upsert_env_assignment_updates_existing_key() {
        let rendered = upsert_env_assignment("NAME=old\nOTHER=keep", "NAME", "fresh");
        assert_eq!(rendered, "NAME=fresh\nOTHER=keep");
    }

    #[cfg(unix)]
    #[test]
    fn write_secret_env_file_creates_restricted_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".roko").join(".env");
        write_secret_env_file(&path, "TOKEN", "abc123").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "TOKEN=abc123");
    }

    #[test]
    fn build_config_migration_plan_synthesizes_supported_legacy_claude_config() {
        let text = r#"
[agent]
command = "claude"
args = ["--print", "--output-format", "stream-json"]
model = "claude-sonnet-4-6"
timeout_ms = 300000

[agent.tier_models]
mechanical = "claude-haiku-4-5"
"#;

        let plan = build_config_migration_plan(text).unwrap();
        let ConfigMigrationPlan::Legacy(plan) = plan else {
            panic!("expected legacy migration plan");
        };

        assert_eq!(plan.provider_name, "claude_cli");
        assert_eq!(plan.provider.kind, ProviderKind::ClaudeCli);
        assert_eq!(plan.provider.command.as_deref(), Some("claude"));
        assert_eq!(
            plan.provider.args.as_deref(),
            Some(
                &[
                    "--print".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                ][..]
            )
        );
        assert_eq!(plan.provider.timeout_ms, Some(300_000));
        assert_eq!(plan.models.len(), 2);
        assert_eq!(plan.models[0].0, "claude-haiku-4-5");
        assert_eq!(plan.models[0].1.provider, "claude_cli");
        assert_eq!(plan.models[0].1.tool_format, "anthropic_blocks");
        assert_eq!(plan.models[1].0, "claude-sonnet-4-6");
        assert_eq!(plan.models[1].1.provider, "claude_cli");

        let migrated = RokoConfig::from_toml(&plan.rendered).unwrap();
        assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(migrated.providers.len(), 1);
        assert_eq!(migrated.models.len(), 2);

        let legacy = Config::parse_toml(&plan.rendered).unwrap();
        assert_eq!(legacy.agent.command, "claude");
        assert_eq!(
            legacy.agent.args,
            vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
            ]
        );
        assert_eq!(legacy.agent.model.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn build_config_migration_plan_rejects_unsupported_legacy_backend() {
        let text = r#"
[agent]
command = "mods"
model = "gpt-5"
"#;

        let err = build_config_migration_plan(text).unwrap_err();
        assert!(
            err.to_string()
                .contains("legacy agent.command 'mods' cannot be migrated safely")
        );
    }

    #[test]
    fn build_config_migration_plan_noops_for_current_provider_registry() {
        let text = r#"
schema_version = 2

[agent]
command = "claude"
model = "claude-sonnet-4-6"

[providers.claude_cli]
kind = "claude_cli"
command = "claude"
"#;

        let plan = build_config_migration_plan(text).unwrap();
        assert!(matches!(plan, ConfigMigrationPlan::AlreadyCurrent));
    }

    #[tokio::test]
    async fn semantic_validate_reports_missing_provider_reference() {
        let client = reqwest::Client::builder().build().unwrap();
        let mut config = RokoConfig::default();
        config.models.insert(
            "kimi-k2-5".to_string(),
            ModelProfile {
                provider: "moonshot".to_string(),
                slug: "kimi-k2.5".to_string(),
                context_window: 256_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        let report = semantic_validate_config(&config, &client).await;
        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 0);
        assert_eq!(
            report.provider_reference_errors,
            vec!["Model 'kimi-k2-5' references missing provider 'moonshot'".to_string()]
        );
    }

    #[tokio::test]
    async fn semantic_validate_reports_missing_api_key_env_var() {
        let client = reqwest::Client::builder().build().unwrap();
        let env_name = "ROKO_TEST_VALIDATE_API_KEY_THAT_SHOULD_NOT_EXIST";

        let mut config = RokoConfig::default();
        config.providers.insert(
            "moonshot".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: None,
                api_key_env: Some(env_name.to_string()),
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        );

        let report = semantic_validate_config(&config, &client).await;

        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 0);
        assert_eq!(
            report.api_key_errors,
            vec![format!(
                "Provider 'moonshot' requires env var '{env_name}', but it is not set"
            )]
        );
    }

    #[tokio::test]
    async fn semantic_validate_reports_missing_fallback_model_reference() {
        let client = reqwest::Client::builder().build().unwrap();
        let mut config = RokoConfig::default();
        config.agent.fallback_model = Some("missing-model".to_string());

        let report = semantic_validate_config(&config, &client).await;

        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 0);
        assert_eq!(
            report.fallback_reference_errors,
            vec!["agent.fallback_model references missing model 'missing-model'".to_string()]
        );
    }

    #[tokio::test]
    async fn semantic_validate_allows_fallback_model_from_legacy_effective_models() {
        let client = reqwest::Client::builder().build().unwrap();
        let mut config = RokoConfig::default();
        config.agent.default_model = "claude-sonnet-4-6".to_string();
        config
            .agent
            .tier_models
            .insert("mechanical".to_string(), "claude-haiku-4-5".to_string());
        config.agent.fallback_model = Some("claude-haiku-4-5".to_string());

        let report = semantic_validate_config(&config, &client).await;

        assert!(report.fallback_reference_errors.is_empty());
    }

    #[tokio::test]
    async fn semantic_validate_warns_when_provider_is_unreachable() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let mut config = RokoConfig::default();
        config.providers = HashMap::from([(
            "moonshot".to_string(),
            ProviderConfig {
                kind: ProviderKind::OpenAiCompat,
                base_url: Some("http://127.0.0.1:9".to_string()),
                api_key_env: None,
                command: None,
                args: None,
                timeout_ms: None,
                ttft_timeout_ms: None,
                connect_timeout_ms: None,
                extra_headers: None,
                max_concurrent: None,
            },
        )]);
        config.models.insert(
            "kimi-k2-5".to_string(),
            ModelProfile {
                provider: "moonshot".to_string(),
                slug: "kimi-k2.5".to_string(),
                context_window: 256_000,
                max_output: None,
                supports_tools: true,
                supports_thinking: false,
                supports_vision: false,
                supports_web_search: false,
                supports_mcp_tools: false,
                supports_partial: false,
                provider_routing: None,
                tool_format: "openai_json".to_string(),
                cost_input_per_m: None,
                cost_output_per_m: None,
                cost_cache_read_per_m: None,
                cost_cache_write_per_m: None,
                max_tools: None,
                tokenizer_ratio: None,
                ..Default::default()
            },
        );

        let report = semantic_validate_config(&config, &client).await;

        assert_eq!(report.error_count(), 0);
        assert_eq!(report.warning_count(), 2);
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("Provider 'moonshot' base_url unreachable"))
        );
        assert_eq!(
            report.warnings[1],
            "Model 'kimi-k2-5' references provider 'moonshot' which is unreachable"
        );
    }
}
