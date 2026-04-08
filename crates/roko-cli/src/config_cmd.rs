//! `roko config` subcommand group — setup wizard, provenance viewer, editor.
//!
//! These commands operate on the global (`$XDG_CONFIG_HOME/roko/config.toml`)
//! and/or project (`./roko.toml`) config files. The `init` wizard is the
//! primary onboarding path: it detects installed LLM CLIs and writes a
//! working global config with one interactive pass.

use crate::config::{
    AgentLayer, ConfigLayer, DetectedCli, GateConfig, PromptLayer, ResolvedConfig, Source,
    detect_clis, global_config_path, load_layered,
};
use anyhow::{Context as _, Result, anyhow};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

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
        }),
        prompt: Some(PromptLayer {
            token_budget: Some(token_budget),
            role: Some(role),
            files: None,
        }),
        gates,
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
        && r.sources.prompt_token_budget == Source::Default
        && r.sources.prompt_role == Source::Default;
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
        other => return Err(anyhow!("unknown key: {other}")),
    }
    Ok(())
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
    }
}
