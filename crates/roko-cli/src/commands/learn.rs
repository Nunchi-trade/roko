//! learn command handlers.
#![allow(unused_imports)]

use crate::*;

/// Format a cost value for human display.
/// Uses the heuristic: if cost is exactly 0.0 and both token counts are 0,
/// treat the value as unknown.
fn display_cost(cost_usd: f64, input_tokens: u64, output_tokens: u64) -> String {
    if cost_usd == 0.0 && input_tokens == 0 && output_tokens == 0 {
        "unknown".to_string()
    } else {
        format!("${cost_usd:.2}")
    }
}

/// Format a cost value for recent-entry display with four decimal places.
fn display_cost_precise(cost_usd: f64, input_tokens: u64, output_tokens: u64) -> String {
    let display = display_cost(cost_usd, input_tokens, output_tokens);
    if display == "unknown" {
        display
    } else {
        format!("${cost_usd:.4}")
    }
}

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
pub(crate) async fn cmd_tune(
    workdir: &std::path::Path,
    subsystem: &str,
    dry_run: bool,
) -> Result<i32> {
    match subsystem {
        "gates" => {
            let path = learn_gate_thresholds_path(workdir);
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let thresholds: serde_json::Value = serde_json::from_str(&content)?;
                println!("Verify adaptive thresholds ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&thresholds)?);
            } else {
                print_no_data(&path);
            }
        }
        "routing" => {
            let path = learn_router_path(workdir);
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let router: serde_json::Value = serde_json::from_str(&content)?;
                println!("Cascade router state ({}):", path.display());
                println!("{}", serde_json::to_string_pretty(&router)?);
            } else {
                print_no_data(&path);
            }
        }
        "budget" => {
            let path = learn_efficiency_path(workdir);
            if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                let count = content.lines().filter(|l| !l.trim().is_empty()).count();
                println!("Efficiency log: {} entries at {}", count, path.display());
            } else {
                print_no_data(&path);
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
        print_learn_episodes(workdir).await;
    }

    if show_all {
        print_learn_gate_thresholds(workdir);
        print_learn_knowledge(workdir).await;
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
    let path = learn_router_path(workdir);
    print_checked_path(&path);
    if !path.exists() {
        print_no_data(&path);
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Cascade router: 0 entries at {}", path.display());
        return;
    };
    let snapshot = serde_json::from_str::<LearnCascadeRouterSnapshot>(&content).unwrap_or_default();

    let mut first_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut last_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    for transition in &snapshot.stage_transitions {
        first_seen = Some(match first_seen {
            Some(current) => current.min(transition.timestamp.clone()),
            None => transition.timestamp.clone(),
        });
        last_seen = Some(match last_seen {
            Some(current) => current.max(transition.timestamp.clone()),
            None => transition.timestamp.clone(),
        });
    }

    let latest = snapshot
        .stage_transitions
        .last()
        .map(|transition| {
            format!(
                "{} {} -> {} after {} observations",
                transition.timestamp.to_rfc3339(),
                transition.from,
                transition.to,
                transition.observations
            )
        })
        .unwrap_or_else(|| {
            format!(
                "snapshot stage={} total_observations={}",
                cascade_stage_for_observations(snapshot.total_observations),
                snapshot.total_observations
            )
        });

    if snapshot.total_observations == 0 {
        println!("Cascade router: 0 entries at {}", path.display());
    } else {
        println!(
            "Cascade router: {} observations, {} models at {}",
            snapshot.total_observations,
            snapshot.model_slugs.len(),
            path.display()
        );
    }
    println!("  Range: {}", format_range(first_seen, last_seen));
    println!("  Latest: {}", latest);
}

pub(crate) fn print_learn_experiments(workdir: &std::path::Path) {
    // Prompt experiments
    let prompt_path = learn_root(workdir).join("experiments.json");
    print_checked_path(&prompt_path);
    let prompt_store = ExperimentStore::load_or_new(&prompt_path);
    let running = prompt_store.running_count();
    let concluded = prompt_store.concluded_count();
    if running > 0 || concluded > 0 {
        println!(
            "Prompt experiments: {} running, {} concluded",
            running, concluded
        );
    } else if prompt_path.exists() {
        println!("Prompt experiments: 0 entries at {}", prompt_path.display());
    } else {
        println!("Prompt experiments: none");
    }

    // Model experiments
    let model_path = learn_root(workdir).join("model-experiments.json");
    print_checked_path(&model_path);
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
    } else if model_path.exists() {
        println!("Model experiments: 0 entries at {}", model_path.display());
    } else {
        println!("Model experiments: none");
    }
}

#[allow(clippy::cast_precision_loss)]
pub(crate) async fn print_learn_efficiency(workdir: &std::path::Path) {
    let path = learn_efficiency_path(workdir);
    print_checked_path(&path);
    if !path.exists() {
        print_no_data(&path);
        return;
    }

    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        println!("Efficiency: 0 entries at {}", path.display());
        return;
    };

    let mut count = 0usize;
    let mut first_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut last_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut latest: Option<String> = None;
    let mut events = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(event) =
            serde_json::from_str::<roko_learn::efficiency::AgentEfficiencyEvent>(trimmed)
        else {
            continue;
        };

        count += 1;
        let parsed_timestamp = parse_rfc3339_utc(&event.timestamp);
        if let Some(timestamp) = parsed_timestamp {
            first_seen = Some(match first_seen {
                Some(current) => current.min(timestamp),
                None => timestamp,
            });
            last_seen = Some(match last_seen {
                Some(current) => current.max(timestamp),
                None => timestamp,
            });
        }

        let timestamp = parsed_timestamp
            .map(|ts| ts.to_rfc3339())
            .unwrap_or_else(|| event.timestamp.clone());
        let model = efficiency_model_label(&event);
        let task_id = non_empty_or_unknown(&event.task_id);
        let plan_id = non_empty_or_unknown(&event.plan_id);
        let status = if event.gate_passed { "pass" } else { "fail" };
        latest = Some(format!(
            "{timestamp} model={model} task={task_id} plan={plan_id} {status} cost={}",
            display_cost_precise(event.cost_usd, event.input_tokens, event.output_tokens)
        ));
        events.push(event);
    }

    if count == 0 {
        println!("Efficiency: 0 entries at {}", path.display());
    } else {
        println!("Efficiency: {} events at {}", count, path.display());
    }
    println!("  Range: {}", format_range(first_seen, last_seen));
    println!("  Latest: {}", latest.unwrap_or_else(|| "none".to_string()));
    if let Some(summary) = attempt_correlation_summary(&events) {
        println!("{summary}");
    }
}

pub(crate) async fn print_learn_episodes(workdir: &std::path::Path) {
    let exact_path = learn_episodes_path(workdir);
    let legacy_path = learn_legacy_episodes_path(workdir);
    print_checked_path(&exact_path);
    if legacy_path != exact_path {
        println!("  legacy path: {}", legacy_path.display());
    }
    let path = if exact_path.exists() {
        exact_path
    } else if legacy_path.exists() {
        legacy_path
    } else {
        exact_path
    };
    if !path.exists() {
        print_no_data(&path);
        return;
    }

    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        println!("Episodes: 0 entries at {}", path.display());
        return;
    };

    let mut count = 0usize;
    let mut first_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut last_seen: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut latest: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(episode) = serde_json::from_str::<roko_learn::episode_logger::Episode>(trimmed)
        else {
            continue;
        };

        count += 1;
        first_seen = Some(match first_seen {
            Some(current) => current.min(episode.timestamp.clone()),
            None => episode.timestamp.clone(),
        });
        last_seen = Some(match last_seen {
            Some(current) => current.max(episode.timestamp.clone()),
            None => episode.timestamp.clone(),
        });

        let status = if episode.success { "pass" } else { "fail" };
        let model = non_empty_or_unknown(&episode.model);
        let task_id = non_empty_or_unknown(&episode.task_id);
        latest = Some(format!(
            "{} model={model} task={task_id} {status} cost={}",
            episode.timestamp.to_rfc3339(),
            display_cost_precise(
                episode.usage.cost_usd,
                episode.usage.input_tokens,
                episode.usage.output_tokens
            )
        ));
    }

    if count == 0 {
        println!("Episodes: 0 entries at {}", path.display());
    } else {
        println!("Episodes: {} entries at {}", count, path.display());
    }
    println!("  Range: {}", format_range(first_seen, last_seen));
    println!("  Latest: {}", latest.unwrap_or_else(|| "none".to_string()));
}

pub(crate) fn print_learn_gate_thresholds(workdir: &std::path::Path) {
    let path = learn_gate_thresholds_path(workdir);
    print_checked_path(&path);
    if !path.exists() {
        eprintln!("Gate thresholds: No data at {}", path.display());
        println!("Gate thresholds: 0 entries at {}", path.display());
        return;
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        println!("Gate thresholds: 0 entries at {}", path.display());
        return;
    };
    let count = count_gate_threshold_entries(&content);
    println!("Gate thresholds: {} entries at {}", count, path.display());
}

pub(crate) async fn print_learn_knowledge(workdir: &std::path::Path) {
    let path = learn_knowledge_path(workdir);
    print_checked_path(&path);
    if !path.exists() {
        print_no_data(&path);
        return;
    }
    let Ok(content) = tokio::fs::read_to_string(&path).await else {
        println!("Knowledge: 0 entries at {}", path.display());
        return;
    };
    let count = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter(|line| serde_json::from_str::<serde_json::Value>(line).is_ok())
        .count();
    if count == 0 {
        println!("Knowledge: 0 entries at {}", path.display());
    } else {
        println!("Knowledge: {} durable entries at {}", count, path.display());
    }
}

fn learn_root(workdir: &std::path::Path) -> std::path::PathBuf {
    workdir.join(".roko").join("learn")
}

fn learn_gate_thresholds_path(workdir: &std::path::Path) -> std::path::PathBuf {
    learn_root(workdir).join("gate-thresholds.json")
}

fn learn_router_path(workdir: &std::path::Path) -> std::path::PathBuf {
    learn_root(workdir).join("cascade-router.json")
}

fn learn_efficiency_path(workdir: &std::path::Path) -> std::path::PathBuf {
    learn_root(workdir).join("efficiency.jsonl")
}

fn learn_episodes_path(workdir: &std::path::Path) -> std::path::PathBuf {
    workdir.join(".roko").join("episodes.jsonl")
}

/// Legacy episode log path retained for older worktrees and fixtures.
fn learn_legacy_episodes_path(workdir: &std::path::Path) -> std::path::PathBuf {
    learn_root(workdir).join("episodes.jsonl")
}

fn learn_knowledge_path(workdir: &std::path::Path) -> std::path::PathBuf {
    workdir.join(".roko").join("neuro").join("knowledge.jsonl")
}

fn count_gate_threshold_entries(content: &str) -> usize {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return 0;
    };

    value
        .get("rungs")
        .and_then(serde_json::Value::as_object)
        .map_or(0, |rungs| rungs.len())
}

fn print_checked_path(path: &std::path::Path) {
    println!("  path: {}", path.display());
}

fn print_no_data(path: &std::path::Path) {
    println!("No data at {}", path.display());
}

fn parse_rfc3339_utc(timestamp: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|parsed| parsed.with_timezone(&chrono::Utc))
}

fn format_range(
    first_seen: Option<chrono::DateTime<chrono::Utc>>,
    last_seen: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    match (first_seen, last_seen) {
        (Some(first_seen), Some(last_seen)) => {
            format!("{} .. {}", first_seen.to_rfc3339(), last_seen.to_rfc3339())
        }
        _ => "n/a".to_string(),
    }
}

fn non_empty_or_unknown(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "unknown"
    } else {
        trimmed
    }
}

fn efficiency_model_label(event: &roko_learn::efficiency::AgentEfficiencyEvent) -> &str {
    let model_used = event.model_used.trim();
    if model_used.is_empty() {
        non_empty_or_unknown(&event.model)
    } else {
        model_used
    }
}

fn attempt_correlation_summary(
    events: &[roko_learn::efficiency::AgentEfficiencyEvent],
) -> Option<String> {
    let events_with_task_id = events
        .iter()
        .filter(|event| !event.task_id.is_empty())
        .count();
    if events_with_task_id == 0 {
        return None;
    }

    let linked_gate_failures = events
        .iter()
        .filter(|event| !event.task_id.is_empty() && !event.gate_passed)
        .count();

    Some(format!(
        "  Attempt correlation: {} events with task_id, {} gate failures linked",
        events_with_task_id, linked_gate_failures
    ))
}

fn cascade_stage_for_observations(observations: u64) -> &'static str {
    if observations >= 200 {
        "ucb"
    } else if observations >= 50 {
        "confidence"
    } else {
        "static"
    }
}

#[derive(Default, serde::Deserialize)]
struct LearnCascadeRouterSnapshot {
    #[serde(default)]
    model_slugs: Vec<String>,
    #[serde(default)]
    total_observations: u64,
    #[serde(default)]
    stage_transitions: Vec<roko_learn::cascade::StageTransition>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_cost_uses_unknown_for_zero_usage() {
        assert_eq!(display_cost(0.0, 0, 0), "unknown");
    }

    #[test]
    fn display_cost_shows_zero_for_reported_free_usage() {
        assert_eq!(display_cost(0.0, 1, 0), "$0.00");
        assert_eq!(display_cost(0.0, 0, 1), "$0.00");
    }

    #[test]
    fn display_cost_shows_formatted_value() {
        assert_eq!(display_cost(1.42, 0, 0), "$1.42");
    }

    #[test]
    fn display_cost_precise_uses_unknown_for_zero_usage() {
        assert_eq!(display_cost_precise(0.0, 0, 0), "unknown");
    }

    #[test]
    fn display_cost_precise_shows_zero_for_reported_free_usage() {
        assert_eq!(display_cost_precise(0.0, 2, 3), "$0.0000");
    }

    #[test]
    fn display_cost_precise_shows_formatted_value() {
        assert_eq!(display_cost_precise(1.42, 7, 9), "$1.4200");
    }

    #[test]
    fn attempt_correlation_summary_counts_only_attempted_events() {
        let mut success = roko_learn::efficiency::AgentEfficiencyEvent::default();
        success.attempt_id = "attempt-1".into();
        success.gate_passed = true;

        let mut failure = roko_learn::efficiency::AgentEfficiencyEvent::default();
        failure.attempt_id = "attempt-2".into();
        failure.gate_passed = false;

        let mut unlabeled = roko_learn::efficiency::AgentEfficiencyEvent::default();
        unlabeled.gate_passed = false;

        let events = vec![success, failure, unlabeled];
        let summary = attempt_correlation_summary(&events);

        assert_eq!(
            summary.as_deref(),
            Some("  Attempt correlation: 2 events with attempt_id, 1 gate failures linked")
        );
    }

    #[test]
    fn attempt_correlation_summary_skips_empty_attempt_ids() {
        let mut unlabeled = roko_learn::efficiency::AgentEfficiencyEvent::default();
        unlabeled.gate_passed = false;

        assert!(attempt_correlation_summary(&[unlabeled]).is_none());
    }

    #[test]
    fn learn_episodes_path_targets_root_log() {
        let workdir = std::path::Path::new("/tmp/workdir");
        assert_eq!(
            learn_episodes_path(workdir),
            workdir.join(".roko").join("episodes.jsonl")
        );
    }

    #[test]
    fn count_gate_threshold_entries_uses_rungs_map() {
        let content = r#"{"rungs":{"1":{"ema_pass_rate":0.5},"2":{"ema_pass_rate":0.75}}}"#;
        assert_eq!(count_gate_threshold_entries(content), 2);
    }
}
