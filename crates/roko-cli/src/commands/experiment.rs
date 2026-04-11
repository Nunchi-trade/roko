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
