//! tune command handler.
#![allow(unused_imports)]

use crate::*;
use std::collections::HashMap;

/// `roko tune ...` - write behavior presets into roko.toml.
pub(crate) async fn cmd_tune(cli: &Cli, cmd: TuneCmd) -> Result<i32> {
    let workdir = tune_workdir(cli, &cmd);
    ensure_project_config(&workdir)?;

    let (label, edits): (&str, Vec<(&str, String)>) = match cmd {
        TuneCmd::Routing { .. } => (
            "routing",
            vec![
                ("routing.mode", "auto_override".to_string()),
                ("routing.algorithm", "linucb".to_string()),
                ("routing.fast_task_model", "claude-haiku-4-5".to_string()),
                (
                    "routing.standard_task_model",
                    "claude-sonnet-4-6".to_string(),
                ),
                ("routing.complex_task_model", "claude-opus-4-6".to_string()),
                ("routing.context_strategy", "hybrid".to_string()),
                ("routing.weights.quality", "0.55".to_string()),
                ("routing.weights.cost", "0.30".to_string()),
                ("routing.weights.latency", "0.15".to_string()),
            ],
        ),
        TuneCmd::Gates { .. } => (
            "gates",
            vec![
                ("gates.clippy_enabled", "true".to_string()),
                ("gates.skip_tests", "false".to_string()),
                ("gates.max_iterations", "2".to_string()),
            ],
        ),
        TuneCmd::Budget { .. } => (
            "budget",
            vec![
                ("budget.max_plan_usd", "10.0".to_string()),
                ("budget.max_turn_usd", "1.0".to_string()),
                ("budget.prompt_token_budget", "20000".to_string()),
            ],
        ),
        TuneCmd::Model { name, .. } => {
            let model = resolve_model_key(&workdir, &name)?;
            ("model", vec![("agent.default_model", model)])
        }
    };

    let pending = edits
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect::<HashMap<_, _>>();
    roko_cli::tui::config_meta::save_pending_edits(&workdir, &pending)
        .map_err(anyhow::Error::msg)?;

    let path = workdir.join("roko.toml");
    println!("tuned {label} in {}", path.display());
    for (key, value) in edits {
        println!("  {key} = {value}");
    }

    Ok(EXIT_SUCCESS)
}

fn tune_workdir(cli: &Cli, cmd: &TuneCmd) -> PathBuf {
    match cmd {
        TuneCmd::Routing { workdir }
        | TuneCmd::Gates { workdir }
        | TuneCmd::Budget { workdir }
        | TuneCmd::Model { workdir, .. } => workdir.clone(),
    }
    .unwrap_or_else(|| resolve_workdir(cli))
}

fn ensure_project_config(workdir: &Path) -> Result<()> {
    let path = workdir.join("roko.toml");
    if path.exists() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let template = Config::default_toml_template(false)?;
    std::fs::write(&path, template).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn resolve_model_key(workdir: &Path, requested: &str) -> Result<String> {
    let requested = requested.trim();
    if requested.is_empty() {
        bail!("provide a model name");
    }

    let config = roko_core::config::loader::load_config_unified(workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let requested_lower = requested.to_ascii_lowercase();
    let normalized = roko_cli::task_parser::normalize_model_alias(requested);
    let normalized_lower = normalized.to_ascii_lowercase();

    if config.models.contains_key(requested) {
        return Ok(requested.to_string());
    }
    if config.models.contains_key(normalized) {
        return Ok(normalized.to_string());
    }

    let mut matches = config
        .models
        .iter()
        .filter_map(|(key, profile)| {
            let key_lower = key.to_ascii_lowercase();
            let slug_lower = profile.slug.to_ascii_lowercase();
            (key_lower == requested_lower
                || key_lower == normalized_lower
                || slug_lower == requested_lower
                || slug_lower == normalized_lower)
                .then(|| key.clone())
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    if let Some(model_key) = matches.into_iter().next() {
        return Ok(model_key);
    }

    if config.models.is_empty() {
        return Ok(normalized.to_string());
    }

    let mut keys = config.models.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    bail!(
        "unknown model '{requested}'. configured models: {}",
        keys.join(", ")
    )
}
