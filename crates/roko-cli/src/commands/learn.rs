//! learn command handlers.
#![allow(unused_imports)]

use crate::*;


pub(crate) async fn dispatch_learn(cli: &Cli, cmd: LearnCmd) -> Result<i32> {
    match cmd {
        LearnCmd::All { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "all").await
        }
        LearnCmd::Route { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "router").await
        }
        LearnCmd::Experiments { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "experiments").await
        }
        LearnCmd::Efficiency { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "efficiency").await
        }
        LearnCmd::Episodes { workdir } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_learn(&wd, "episodes").await
        }
        LearnCmd::Tune {
            subsystem,
            dry_run,
            workdir,
        } => {
            let wd = workdir.unwrap_or_else(|| resolve_workdir(cli));
            cmd_tune(&wd, &subsystem, dry_run).await
        }
    }
}


/// `roko tune [subsystem]` — display and optionally adjust adaptive thresholds.
pub(crate) async fn cmd_tune(workdir: &std::path::Path, subsystem: &str, dry_run: bool) -> Result<i32> {
    match subsystem {
        "gates" => {
            let path = workdir.join(".roko/learn/gate-thresholds.json");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let thresholds: serde_json::Value = serde_json::from_str(&content)?;
                println!("Verify adaptive thresholds ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&thresholds)?);
            } else {
                println!("No gate thresholds found at {}.", path.display());
                println!("Run some plans first to generate adaptive thresholds.");
            }
        }
        "routing" => {
            let path = workdir.join(".roko/learn/cascade-router.json");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let router: serde_json::Value = serde_json::from_str(&content)?;
                println!("Cascade router state ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&router)?);
            } else {
                println!("No cascade router state found at {}.", path.display());
            }
        }
        "budget" => {
            let path = workdir.join(".roko/learn/efficiency.jsonl");
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let count = content.lines().filter(|l| !l.trim().is_empty()).count();
                println!("Efficiency log: {} entries at {}", count, path.display());
            } else {
                println!("No efficiency log found at {}.", path.display());
            }
        }
        other => {
            eprintln!("Unknown subsystem '{other}'. Available: gates, routing, budget");
            return Ok(1);
        }
    }
    if dry_run {
        println!("(dry-run: no changes applied)");
    }
    Ok(EXIT_SUCCESS)
}


/// `roko learn [what]` — display learning subsystem state.
pub(crate) async fn cmd_learn(workdir: &std::path::Path, what: &str) -> Result<i32> {
    let show_all = what == "all";

    if show_all || what == "router" {
        print_learn_router(workdir);
    }

    if show_all || what == "experiments" {
        print_learn_experiments(workdir);
    }

    if show_all || what == "efficiency" {
        print_learn_efficiency(workdir).await;
    }

    if show_all || what == "episodes" {
        print_learn_episodes(workdir);
    }

    if !show_all && !["router", "experiments", "efficiency", "episodes"].contains(&what) {
        eprintln!(
            "Unknown learning area '{what}'. Available: router, experiments, efficiency, episodes, all"
        );
        return Ok(1);
    }

    Ok(EXIT_SUCCESS)
}


pub(crate) fn print_learn_router(workdir: &std::path::Path) {
    let path = workdir.join(".roko/learn/cascade-router.json");
    if !path.exists() {
        println!("Cascade router: not initialized");
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Cascade router: unreadable");
        return;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
        println!("Cascade router: parse error");
        return;
    };

    // Model slugs
    let slugs: Vec<&str> = val
        .get("model_slugs")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let total_obs = val
        .get("arms")
        .and_then(|v| v.as_array())
        .map(|arms| {
            arms.iter()
                .filter_map(|arm| arm.get("observations").and_then(|v| v.as_u64()))
                .sum::<u64>()
        })
        .unwrap_or(0);

    println!(
        "Cascade router: {} models, {} total observations",
        slugs.len(),
        total_obs
    );

    // Per-arm summary
    if let Some(arms) = val.get("arms").and_then(|v| v.as_array()) {
        for arm in arms {
            let slug = arm
                .get("model_slug")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let obs = arm
                .get("observations")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let reward = arm
                .get("mean_reward")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if obs > 0 {
                println!("  {slug}: {obs} obs, mean_reward={reward:.3}");
            }
        }
    }
}


pub(crate) fn print_learn_experiments(workdir: &std::path::Path) {
    // Prompt experiments
    let prompt_path = workdir.join(".roko/learn/experiments.json");
    let prompt_store = ExperimentStore::load_or_new(&prompt_path);
    let running = prompt_store.running_count();
    let concluded = prompt_store.concluded_count();
    if running > 0 || concluded > 0 {
        println!(
            "Prompt experiments: {} running, {} concluded",
            running, concluded
        );
    } else {
        println!("Prompt experiments: none");
    }

    // Model experiments
    let model_path = workdir
        .join(".roko")
        .join("learn")
        .join("model-experiments.json");
    let model_store = roko_learn::model_experiment::ModelExperimentStore::load_or_new(&model_path);
    let model_running = model_store.running_count();
    let model_concluded = model_store.concluded_experiments().len();
    if model_running > 0 || model_concluded > 0 {
        println!(
            "Model experiments: {} running, {} concluded",
            model_running, model_concluded
        );
        for exp in model_store.iter() {
            println!(
                "  {} [{:?}] role={} variants={} winner={}",
                exp.experiment_id,
                exp.status,
                exp.role.as_deref().unwrap_or("any"),
                exp.variants.len(),
                exp.winner_id.as_deref().unwrap_or("-"),
            );
        }
    } else {
        println!("Model experiments: none");
    }
}


#[allow(clippy::cast_precision_loss)]
pub(crate) async fn print_learn_efficiency(workdir: &std::path::Path) {
    let path = workdir.join(".roko/learn/efficiency.jsonl");
    let events = match read_efficiency_events(&path).await {
        Ok(events) => events,
        Err(_) => {
            println!("Efficiency log: empty");
            return;
        }
    };
    if events.is_empty() {
        println!("Efficiency log: empty");
        return;
    }

    let total_cost: f64 = events.iter().map(|e| e.cost_usd).sum();
    let total_input: u64 = events.iter().map(|e| e.input_tokens).sum();
    let total_output: u64 = events.iter().map(|e| e.output_tokens).sum();
    let success_count = events.iter().filter(|e| e.gate_passed).count();
    let pass_rate = if events.is_empty() {
        0.0
    } else {
        success_count as f64 / events.len() as f64 * 100.0
    };

    println!(
        "Efficiency: {} events, ${:.2} total, {:.0}% pass rate",
        events.len(),
        total_cost,
        pass_rate
    );
    println!(
        "  Tokens: {}K input, {}K output",
        total_input / 1000,
        total_output / 1000
    );

    // Per-model breakdown
    let mut by_model: std::collections::HashMap<&str, (usize, f64, usize)> =
        std::collections::HashMap::new();
    for ev in &events {
        let entry = by_model.entry(ev.model.as_str()).or_default();
        entry.0 += 1;
        entry.1 += ev.cost_usd;
        if ev.gate_passed {
            entry.2 += 1;
        }
    }
    let mut model_summary: Vec<_> = by_model.into_iter().collect();
    model_summary.sort_by(|a, b| {
        b.1.1
            .partial_cmp(&a.1.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (model, (count, cost, passed)) in &model_summary {
        let model_pass = if *count == 0 {
            0.0
        } else {
            *passed as f64 / *count as f64 * 100.0
        };
        println!("  {model}: {count} runs, ${cost:.2}, {model_pass:.0}% pass",);
    }
}


pub(crate) fn print_learn_episodes(workdir: &std::path::Path) {
    let path = workdir.join(".roko/episodes.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Episodes: none");
        return;
    };

    let episodes: Vec<Episode> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    if episodes.is_empty() {
        println!("Episodes: none");
        return;
    }

    let success_count = episodes.iter().filter(|e| e.success).count();
    let total_cost: f64 = episodes.iter().map(|e| f64::from(e.usage.cost_usd)).sum();
    let pass_rate = if episodes.is_empty() {
        0.0
    } else {
        success_count as f64 / episodes.len() as f64 * 100.0
    };

    println!(
        "Episodes: {} recorded, {:.0}% success, ${:.2} total cost",
        episodes.len(),
        pass_rate,
        total_cost,
    );

    // Per-model summary
    let mut by_model: std::collections::HashMap<&str, (usize, usize)> =
        std::collections::HashMap::new();
    for ep in &episodes {
        let entry = by_model.entry(ep.model.as_str()).or_default();
        entry.0 += 1;
        if ep.success {
            entry.1 += 1;
        }
    }
    let mut model_summary: Vec<_> = by_model.into_iter().collect();
    model_summary.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    for (model, (count, passed)) in &model_summary {
        let model_pass = if *count == 0 {
            0.0
        } else {
            *passed as f64 / *count as f64 * 100.0
        };
        println!("  {model}: {count} episodes, {model_pass:.0}% success");
    }

    // Last 5 episodes
    let last_n = episodes.len().min(5);
    if last_n > 0 {
        println!("  Recent:");
        for ep in episodes.iter().rev().take(last_n) {
            let status = if ep.success { "pass" } else { "fail" };
            println!(
                "    {} {} {} [{}] ${:.4}",
                ep.timestamp.format("%Y-%m-%d %H:%M"),
                ep.model,
                ep.task_id,
                status,
                ep.usage.cost_usd,
            );
        }
    }
}

