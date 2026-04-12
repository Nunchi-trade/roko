//! `roko config` subcommand group — setup wizard, provenance viewer, editor.
//!
//! These commands operate on the global (`$XDG_CONFIG_HOME/roko/config.toml`)
//! and/or project (`./roko.toml`) config files. The `init` wizard is the
//! primary onboarding path: it detects installed LLM CLIs and writes a
//! working global config with one interactive pass.

use crate::config::{
    AgentLayer, ConfigLayer, ConfigPaths, DetectedCli, DreamsLayer, ExecutorLayer, GateConfig,
    PromptLayer, ResolvedConfig, ServeAuthLayer, ServeDeployLayer, ServeDeployWebhookLayer,
    ServeLayer, Source, ToolsLayer, detect_clis, global_config_path, load_layered, resolve_paths,
};
use anyhow::{Context as _, Result, anyhow};
use roko_orchestrator::ExecutorConfig;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        providers: None,
        models: None,
        serve: Some(ServeLayer {
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
    /// `~/.config/roko/config.toml`.
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
    match key {
        "agent.command" => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.command = Some(value.into());
        }
        "agent.args" => {
            // Split on whitespace for simple cases; JSON array for anything richer.
            let args: Vec<String> = if value.trim_start().starts_with('[') {
                serde_json::from_str(value).context("parse JSON array for agent.args")?
            } else {
                value.split_whitespace().map(String::from).collect()
            };
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.args = Some(args);
        }
        "agent.model" => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.model = Some(value.into());
        }
        "agent.effort" => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.effort = Some(value.into());
        }
        "agent.bare_mode" => {
            let bare_mode = value.parse::<bool>().context("parse bare_mode as bool")?;
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.bare_mode = Some(bare_mode);
        }
        "agent.fallback_model" => {
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.fallback_model = Some(value.into());
        }
        "agent.timeout_ms" => {
            let ms: u64 = value.parse().context("parse timeout_ms as u64")?;
            let agent = layer.agent.get_or_insert_with(AgentLayer::default);
            agent.timeout_ms = Some(ms);
        }
        "prompt.token_budget" => {
            let n: usize = value.parse().context("parse token_budget as usize")?;
            let prompt = layer.prompt.get_or_insert_with(PromptLayer::default);
            prompt.token_budget = Some(n);
        }
        "prompt.role" => {
            let prompt = layer.prompt.get_or_insert_with(PromptLayer::default);
            prompt.role = Some(value.into());
        }
        "dreams.auto_dream" => {
            let auto_dream = value.parse::<bool>().context("parse auto_dream as bool")?;
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.auto_dream = Some(auto_dream);
        }
        "dreams.idle_threshold_mins" => {
            let mins = value
                .parse::<u64>()
                .context("parse idle_threshold_mins as u64")?;
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.idle_threshold_mins = Some(mins);
        }
        "dreams.min_episodes_for_dream" => {
            let min_episodes = value
                .parse::<usize>()
                .context("parse min_episodes_for_dream as usize")?;
            let dreams = layer.dreams.get_or_insert_with(DreamsLayer::default);
            dreams.min_episodes_for_dream = Some(min_episodes);
        }
        "tools.prefer_mcp" => {
            let prefer_mcp = value.parse::<bool>().context("parse prefer_mcp as bool")?;
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.prefer_mcp = Some(prefer_mcp);
        }
        "tools.global_denied" => {
            let denied: Vec<String> = if value.trim_start().starts_with('[') {
                serde_json::from_str(value).context("parse JSON array for tools.global_denied")?
            } else {
                value.split_whitespace().map(String::from).collect()
            };
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.global_denied = Some(denied);
        }
        "tools.mcp_timeout_secs" => {
            let secs = value
                .parse::<u64>()
                .context("parse mcp_timeout_secs as u64")?;
            let tools = layer.tools.get_or_insert_with(ToolsLayer::default);
            tools.mcp_timeout_secs = Some(secs);
        }
        "serve.auth.enabled" => {
            let enabled = value.parse::<bool>().context("parse enabled as bool")?;
            let serve = layer.serve.get_or_insert_with(ServeLayer::default);
            let auth = serve.auth.get_or_insert_with(ServeAuthLayer::default);
            auth.enabled = Some(enabled);
        }
        "serve.auth.api_key" => {
            let serve = layer.serve.get_or_insert_with(ServeLayer::default);
            let auth = serve.auth.get_or_insert_with(ServeAuthLayer::default);
            auth.api_key = Some(value.into());
        }
        "serve.deploy.provider" => {
            let serve = layer.serve.get_or_insert_with(ServeLayer::default);
            let deploy = serve.deploy.get_or_insert_with(ServeDeployLayer::default);
            deploy.provider = Some(value.into());
        }
        "serve.deploy.environment" => {
            let environment: Vec<String> = if value.trim_start().starts_with('[') {
                serde_json::from_str(value)
                    .context("parse JSON array for serve.deploy.environment")?
            } else {
                value.split_whitespace().map(String::from).collect()
            };
            let serve = layer.serve.get_or_insert_with(ServeLayer::default);
            let deploy = serve.deploy.get_or_insert_with(ServeDeployLayer::default);
            deploy.environment = Some(environment);
        }
        "serve.deploy.webhooks" => {
            let webhooks: Vec<ServeDeployWebhookLayer> = serde_json::from_str(value)
                .context("parse JSON array for serve.deploy.webhooks")?;
            let serve = layer.serve.get_or_insert_with(ServeLayer::default);
            let deploy = serve.deploy.get_or_insert_with(ServeDeployLayer::default);
            deploy.webhooks = Some(webhooks);
        }
        other => return Err(anyhow!("unknown key: {other}")),
    }
    Ok(())
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
}
