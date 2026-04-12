//! `roko experiment` subcommands.

use anyhow::{Context as _, Result, bail};
use chrono::Utc;
use clap::Subcommand;
use roko_learn::model_experiment::{ModelExperiment, ModelExperimentStore, ModelVariant};
use roko_learn::prompt_experiment::ExperimentStatus;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{Cli, EXIT_SUCCESS, resolve_workdir};

/// Top-level `roko experiment` subcommands.
#[derive(Debug, Subcommand)]
pub enum ExperimentCmd {
    /// Model A/B experiments.
    Model {
        /// Model experiment subcommands.
        #[command(subcommand)]
        cmd: ModelExperimentCmd,
    },
}

/// `roko experiment model` subcommands.
#[derive(Debug, Subcommand)]
pub enum ModelExperimentCmd {
    /// Create a new model experiment.
    Create {
        /// Experiment identifier.
        #[arg(long)]
        id: String,
        /// Scope the experiment to a specific role.
        #[arg(long)]
        role: String,
        /// Variant specification in `id:slug:provider` form.
        #[arg(long = "variant", required = true)]
        variants: Vec<String>,
        /// Minimum trials per variant before the experiment can conclude.
        #[arg(long = "min-trials", default_value_t = 20)]
        min_trials: u64,
    },
    /// Show a model experiment's results.
    Show {
        /// Experiment identifier.
        id: String,
    },
    /// List all model experiments.
    List,
}

/// Dispatch `roko experiment`.
pub fn dispatch_experiment(cli: &Cli, cmd: ExperimentCmd) -> Result<i32> {
    match cmd {
        ExperimentCmd::Model { cmd } => dispatch_model_experiment(cli, cmd),
    }
}

fn dispatch_model_experiment(cli: &Cli, cmd: ModelExperimentCmd) -> Result<i32> {
    match cmd {
        ModelExperimentCmd::Create {
            id,
            role,
            variants,
            min_trials,
        } => cmd_model_create(cli, id, role, variants, min_trials),
        ModelExperimentCmd::Show { id } => cmd_model_show(cli, id),
        ModelExperimentCmd::List => cmd_model_list(cli),
    }
}

fn cmd_model_create(
    cli: &Cli,
    id: String,
    role: String,
    variant_specs: Vec<String>,
    min_trials: u64,
) -> Result<i32> {
    if variant_specs.len() < 2 {
        bail!("model experiments require at least two --variant entries");
    }

    let wd = resolve_workdir(cli);
    let path = model_experiments_path(&wd);
    let mut store = ModelExperimentStore::load_or_new(&path);

    if store.get(&id).is_some() {
        bail!("model experiment '{id}' already exists");
    }

    let mut seen = HashSet::new();
    let variants = variant_specs
        .into_iter()
        .map(|spec| parse_variant_spec(&spec))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|variant| {
            if !seen.insert(variant.id.clone()) {
                bail!("duplicate variant id '{}'", variant.id);
            }
            Ok(variant)
        })
        .collect::<Result<Vec<_>>>()?;

    let experiment = ModelExperiment {
        experiment_id: id.clone(),
        description: format!("Model A/B experiment for {role}"),
        role: Some(role),
        task_category: None,
        variants,
        stats: Default::default(),
        status: ExperimentStatus::Running,
        winner_id: None,
        min_trials_per_variant: min_trials,
        min_effect_size: 0.05,
        created_at: Utc::now().to_rfc3339(),
    };

    store.register(experiment);
    store
        .save(&path)
        .with_context(|| format!("save model experiments to {}", path.display()))?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "created": id,
                "path": path,
            }))?
        );
    } else if !cli.quiet {
        println!("created model experiment '{}' at {}", id, path.display());
    }

    Ok(EXIT_SUCCESS)
}

fn cmd_model_list(cli: &Cli) -> Result<i32> {
    let wd = resolve_workdir(cli);
    let path = model_experiments_path(&wd);
    let store = ModelExperimentStore::load_or_new(&path);

    let mut experiments: Vec<_> = store.iter().collect();
    experiments.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));

    if cli.json {
        let json = serde_json::json!({
            "path": path,
            "running": store.running_count(),
            "concluded": store.concluded_experiments().len(),
            "experiments": experiments.iter().map(|experiment| serde_json::json!({
                "id": experiment.experiment_id.clone(),
                "description": experiment.description.clone(),
                "role": experiment.role.clone(),
                "task_category": experiment.task_category.clone(),
                "status": format!("{:?}", experiment.status),
                "winner_id": experiment.winner_id.clone(),
                "min_trials_per_variant": experiment.min_trials_per_variant,
                "variants": experiment.variants.iter().map(|variant| serde_json::json!({
                    "id": variant.id.clone(),
                    "model_key": variant.model_key.clone(),
                    "slug": variant.slug.clone(),
                    "provider": variant.provider.clone(),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(EXIT_SUCCESS);
    }

    if experiments.is_empty() {
        if !cli.quiet {
            println!("no model experiments found in {}", path.display());
        }
        return Ok(EXIT_SUCCESS);
    }

    if !cli.quiet {
        println!(
            "model experiments: {} running, {} concluded",
            store.running_count(),
            store.concluded_experiments().len()
        );
        for experiment in experiments {
            println!(
                "- {} [{}] role={} category={} variants={} min_trials={} winner={}",
                experiment.experiment_id.as_str(),
                format!("{:?}", experiment.status),
                experiment.role.as_deref().unwrap_or("any"),
                experiment.task_category.as_deref().unwrap_or("any"),
                experiment.variants.len(),
                experiment.min_trials_per_variant,
                experiment.winner_id.as_deref().unwrap_or("-"),
            );
            for variant in &experiment.variants {
                println!(
                    "  - {} ({}, {}, {})",
                    variant.id.as_str(),
                    variant.model_key.as_str(),
                    variant.slug.as_str(),
                    variant.provider.as_str()
                );
            }
        }
    }

    Ok(EXIT_SUCCESS)
}

fn cmd_model_show(cli: &Cli, id: String) -> Result<i32> {
    let wd = resolve_workdir(cli);
    let path = model_experiments_path(&wd);
    let store = ModelExperimentStore::load_or_new(&path);
    let experiment = store.get(&id).ok_or_else(|| {
        anyhow::anyhow!("model experiment '{id}' not found in {}", path.display())
    })?;

    if cli.json {
        let total_trials: u64 = experiment.stats.values().map(|stats| stats.trials).sum();
        let json = serde_json::json!({
            "path": path,
            "experiment": {
                "id": experiment.experiment_id.clone(),
                "description": experiment.description.clone(),
                "role": experiment.role.clone(),
                "task_category": experiment.task_category.clone(),
                "status": format!("{:?}", experiment.status),
                "winner_id": experiment.winner_id.clone(),
                "min_trials_per_variant": experiment.min_trials_per_variant,
                "min_effect_size": experiment.min_effect_size,
                "created_at": experiment.created_at.clone(),
                "total_trials": total_trials,
                "variants": experiment
                    .variants
                    .iter()
                    .map(|variant| {
                        let stats = experiment.stats.get(&variant.id).cloned().unwrap_or_default();
                        let ucb_score = if stats.trials == 0 || total_trials == 0 {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(model_variant_ucb_score(&stats, total_trials))
                        };

                        serde_json::json!({
                            "id": variant.id.clone(),
                            "model_key": variant.model_key.clone(),
                            "slug": variant.slug.clone(),
                            "provider": variant.provider.clone(),
                            "stats": {
                                "trials": stats.trials,
                                "successes": stats.successes,
                                "total_cost_usd": stats.total_cost_usd,
                                "total_tokens": stats.total_tokens,
                                "total_duration_ms": stats.total_duration_ms,
                                "pass_rate": stats.pass_rate,
                                "avg_cost_usd": stats.avg_cost_usd,
                                "cost_per_success": stats.cost_per_success,
                                "avg_duration_ms": stats.avg_duration_ms,
                                "ucb_score": ucb_score,
                            },
                        })
                    })
                    .collect::<Vec<_>>(),
            }
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(EXIT_SUCCESS);
    }

    if !cli.quiet {
        println!(
            "Experiment: {} ({:?})",
            experiment.experiment_id, experiment.status
        );
        println!(
            "Role: {} | Category: {}",
            experiment.role.as_deref().unwrap_or("any"),
            experiment.task_category.as_deref().unwrap_or("any")
        );
        println!();
        println!("{}", render_model_experiment_table(experiment));
        println!();
        println!("{}", render_model_experiment_status(experiment));
    }

    Ok(EXIT_SUCCESS)
}

fn parse_variant_spec(spec: &str) -> Result<ModelVariant> {
    let parts: Vec<&str> = spec.split(':').collect();
    if parts.len() != 3 {
        bail!("invalid variant '{spec}'; expected id:slug:provider");
    }

    let id = parts[0].trim();
    let slug = parts[1].trim();
    let provider = parts[2].trim();
    if id.is_empty() || slug.is_empty() || provider.is_empty() {
        bail!("invalid variant '{spec}'; fields must not be empty");
    }

    Ok(ModelVariant {
        id: id.to_string(),
        model_key: id.to_string(),
        slug: slug.to_string(),
        provider: provider.to_string(),
    })
}

fn model_experiments_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("model-experiments.json")
}

fn render_model_experiment_table(experiment: &ModelExperiment) -> String {
    let total_trials: u64 = experiment.stats.values().map(|stats| stats.trials).sum();
    let headers = [
        "Variant".to_string(),
        "Trials".to_string(),
        "Pass %".to_string(),
        "Avg Cost".to_string(),
        "$/Success".to_string(),
        "UCB Score".to_string(),
    ];
    let mut rows = Vec::with_capacity(experiment.variants.len());

    for variant in &experiment.variants {
        let stats = experiment
            .stats
            .get(&variant.id)
            .cloned()
            .unwrap_or_default();
        let ucb_score = if stats.trials == 0 || total_trials == 0 {
            "∞".to_string()
        } else {
            format!("{:.2}", model_variant_ucb_score(&stats, total_trials))
        };
        rows.push([
            variant.id.clone(),
            stats.trials.to_string(),
            format!("{:.1}%", stats.pass_rate * 100.0),
            format!("${:.2}", stats.avg_cost_usd),
            format!("${:.2}", stats.cost_per_success),
            ucb_score,
        ]);
    }

    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.len());
        }
    }

    let mut out = String::new();
    out.push_str(&render_table_border('┌', '┬', '┐', &widths));
    out.push('\n');
    out.push_str(&render_table_row(&headers, &widths, false));
    out.push('\n');
    out.push_str(&render_table_border('├', '┼', '┤', &widths));
    out.push('\n');
    for (idx, row) in rows.iter().enumerate() {
        out.push_str(&render_table_row(row, &widths, true));
        if idx + 1 != rows.len() {
            out.push('\n');
        }
    }
    if rows.is_empty() {
        out.push_str(&render_table_row(&[], &widths, true));
    }
    out.push('\n');
    out.push_str(&render_table_border('└', '┴', '┘', &widths));
    out
}

fn render_table_border(left: char, middle: char, right: char, widths: &[usize]) -> String {
    let mut out = String::new();
    out.push(left);
    for (idx, width) in widths.iter().enumerate() {
        out.push_str(&"─".repeat(*width + 2));
        out.push(if idx + 1 == widths.len() {
            right
        } else {
            middle
        });
    }
    out
}

fn render_table_row(cells: &[String], widths: &[usize], numeric: bool) -> String {
    let mut out = String::new();
    out.push('│');
    for (idx, width) in widths.iter().enumerate() {
        let cell = cells.get(idx).map(String::as_str).unwrap_or("");
        out.push(' ');
        if numeric && idx != 0 {
            out.push_str(&format!("{cell:^width$}", width = *width));
        } else if idx == 0 && cells.len() == widths.len() {
            out.push_str(&format!("{cell:<width$}", width = *width));
        } else {
            out.push_str(&format!("{cell:^width$}", width = *width));
        }
        out.push(' ');
        out.push('│');
    }
    out
}

fn render_model_experiment_status(experiment: &ModelExperiment) -> String {
    match experiment.status {
        ExperimentStatus::Concluded => match experiment.winner_id.as_deref() {
            Some(winner) => format!("Status: Concluded (winner: {winner})"),
            None => "Status: Concluded".to_string(),
        },
        ExperimentStatus::Running => {
            let mut needs = Vec::new();
            let mut first_remaining = true;
            for variant in &experiment.variants {
                let trials = experiment
                    .stats
                    .get(&variant.id)
                    .map(|stats| stats.trials)
                    .unwrap_or(0);
                let remaining = experiment.min_trials_per_variant.saturating_sub(trials);
                if remaining == 0 {
                    continue;
                }
                if first_remaining {
                    needs.push(format!("{remaining} more trials for {}", variant.id));
                    first_remaining = false;
                } else {
                    needs.push(format!("{remaining} more for {}", variant.id));
                }
            }

            if needs.is_empty() {
                "Status: Minimum trials met; awaiting effect-size separation".to_string()
            } else {
                format!("Status: Need {}", needs.join(", "))
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn model_variant_ucb_score(
    stats: &roko_learn::model_experiment::ModelVariantStats,
    total_trials: u64,
) -> f64 {
    if stats.trials == 0 || total_trials == 0 {
        return f64::INFINITY;
    }

    let mean = stats.successes as f64 / stats.trials as f64;
    let exploration = (2.0 * (total_trials as f64).ln() / stats.trials as f64).sqrt();
    mean + exploration
}
