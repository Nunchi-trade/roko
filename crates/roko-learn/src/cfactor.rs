//! Composite C-Factor metrics for dashboard and learning feedback.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::episode_logger::Episode;

const BASELINE_TASK_COUNT: usize = 10;

/// Composite C-Factor snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactor {
    /// 0.0-1.0 composite score.
    pub overall: f64,
    /// Component breakdown for the score.
    pub components: CFactorComponents,
    /// Timestamp when the score was computed.
    pub computed_at: DateTime<Utc>,
    /// Number of episodes used in the calculation.
    pub episode_count: usize,
}

/// Individual C-Factor components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorComponents {
    /// % of tasks passing gates on first attempt.
    pub gate_pass_rate: f64,
    /// Inverse of cost per successful task, normalized.
    pub cost_efficiency: f64,
    /// Inverse of time per successful task, normalized.
    pub speed: f64,
    /// % of tasks succeeding without re-plan.
    pub first_try_rate: f64,
    /// Rate of new knowledge entries per episode.
    pub knowledge_growth: f64,
}

impl Default for CFactorComponents {
    fn default() -> Self {
        Self {
            gate_pass_rate: 0.0,
            cost_efficiency: 0.0,
            speed: 0.0,
            first_try_rate: 0.0,
            knowledge_growth: 0.0,
        }
    }
}

impl Default for CFactor {
    fn default() -> Self {
        Self {
            overall: 0.0,
            components: CFactorComponents::default(),
            computed_at: Utc::now(),
            episode_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct TaskAggregate {
    cost_usd: f64,
    duration_ms: f64,
    passed_gate: bool,
    saw_replan: bool,
    first_seen: DateTime<Utc>,
}

/// Compute a C-Factor snapshot from episodes within `window`.
///
/// The calculator groups episodes by task identifier, filters them to the
/// requested time window, and then computes:
///
/// - `gate_pass_rate` over task groups that passed gates
/// - `cost_efficiency` and `speed` against a baseline derived from the first
///   ten task groups in the window
/// - `first_try_rate` over task groups that did not require a replan
/// - `knowledge_growth` from explicit knowledge counters present in episode
///   metadata
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn compute_cfactor(episodes: &[Episode], window: Duration) -> CFactor {
    if episodes.is_empty() {
        return CFactor::default();
    }

    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let filtered: Vec<&Episode> = episodes
        .iter()
        .filter(|episode| episode.timestamp >= cutoff)
        .collect();

    if filtered.is_empty() {
        return CFactor {
            computed_at: Utc::now(),
            ..CFactor::default()
        };
    }

    let mut tasks: HashMap<String, TaskAggregate> = HashMap::new();
    for episode in &filtered {
        let task_key = task_key(episode);
        let entry = tasks.entry(task_key).or_insert_with(|| TaskAggregate {
            cost_usd: 0.0,
            duration_ms: 0.0,
            passed_gate: false,
            saw_replan: false,
            first_seen: episode.timestamp,
        });

        entry.cost_usd += episode.usage.cost_usd;
        entry.duration_ms += episode_duration_ms(episode);
        entry.passed_gate |= episode_passed_gate(episode);
        entry.saw_replan |= episode_is_replan(episode);
        if episode.timestamp < entry.first_seen {
            entry.first_seen = episode.timestamp;
        }
    }

    let mut task_groups: Vec<(String, TaskAggregate)> = tasks.into_iter().collect();
    task_groups.sort_by(|left, right| {
        left.1
            .first_seen
            .cmp(&right.1.first_seen)
            .then_with(|| left.0.cmp(&right.0))
    });

    let total_tasks = task_groups.len();
    let passed_tasks = task_groups.iter().filter(|(_, task)| task.passed_gate).count();
    let first_try_tasks = task_groups
        .iter()
        .filter(|(_, task)| task.passed_gate && !task.saw_replan)
        .count();
    let successful_tasks: Vec<&TaskAggregate> = task_groups
        .iter()
        .filter_map(|(_, task)| task.passed_gate.then_some(task))
        .collect();

    let gate_pass_rate = ratio(passed_tasks, total_tasks);
    let first_try_rate = ratio(first_try_tasks, total_tasks);

    let (avg_cost_per_successful_task, avg_duration_per_successful_task) =
        if successful_tasks.is_empty() {
            (0.0, 0.0)
        } else {
            let count = successful_tasks.len() as f64;
            let total_cost: f64 = successful_tasks.iter().map(|task| task.cost_usd).sum();
            let total_duration: f64 = successful_tasks.iter().map(|task| task.duration_ms).sum();
            (total_cost / count, total_duration / count)
        };

    let baseline_task_count = task_groups.len().min(BASELINE_TASK_COUNT);
    let (baseline_cost, baseline_duration) = if baseline_task_count == 0 {
        (0.0, 0.0)
    } else {
        let baseline_tasks: Vec<&(String, TaskAggregate)> =
            task_groups.iter().take(baseline_task_count).collect();
        let total_cost: f64 = baseline_tasks.iter().map(|(_, task)| task.cost_usd).sum();
        let total_duration: f64 = baseline_tasks.iter().map(|(_, task)| task.duration_ms).sum();
        (
            total_cost / baseline_task_count as f64,
            total_duration / baseline_task_count as f64,
        )
    };

    let cost_efficiency = if baseline_cost > 0.0 && avg_cost_per_successful_task > 0.0 {
        baseline_cost / avg_cost_per_successful_task
    } else {
        0.0
    };

    let speed = if baseline_duration > 0.0 && avg_duration_per_successful_task > 0.0 {
        baseline_duration / avg_duration_per_successful_task
    } else {
        0.0
    };

    let new_knowledge_entries: usize = filtered
        .iter()
        .map(|episode| episode_new_knowledge_entries(episode))
        .sum();
    let knowledge_growth = ratio(new_knowledge_entries, filtered.len());

    let overall = gate_pass_rate * 0.3
        + cost_efficiency * 0.2
        + speed * 0.15
        + first_try_rate * 0.25
        + knowledge_growth * 0.1;

    CFactor {
        overall,
        components: CFactorComponents {
            gate_pass_rate,
            cost_efficiency,
            speed,
            first_try_rate,
            knowledge_growth,
        },
        computed_at: Utc::now(),
        episode_count: filtered.len(),
    }
}

/// Derive the 7-day trend arrow from a history of C-Factor snapshots.
///
/// Compares the oldest and newest snapshot in the requested window.
/// Returns `↑` when the score increased, `↓` when it decreased, and `→`
/// when the window is flat or has insufficient data.
#[must_use]
pub fn trend_arrow(history: &[CFactor], window: Duration) -> &'static str {
    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let mut snapshots: Vec<&CFactor> = history
        .iter()
        .filter(|snapshot| snapshot.computed_at >= cutoff)
        .collect();
    snapshots.sort_by(|left, right| left.computed_at.cmp(&right.computed_at));

    let Some(first) = snapshots.first() else {
        return "→";
    };
    let Some(last) = snapshots.last() else {
        return "→";
    };

    if last.overall > first.overall {
        "↑"
    } else if last.overall < first.overall {
        "↓"
    } else {
        "→"
    }
}

fn ratio(numer: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

fn task_key(episode: &Episode) -> String {
    let task_id = episode.task_id.trim();
    if task_id.is_empty() {
        episode.id.clone()
    } else {
        task_id.to_string()
    }
}

fn episode_duration_ms(episode: &Episode) -> f64 {
    if episode.usage.wall_ms > 0 {
        episode.usage.wall_ms as f64
    } else {
        episode.duration_secs.max(0.0) * 1_000.0
    }
}

fn episode_passed_gate(episode: &Episode) -> bool {
    if !episode.gate_verdicts.is_empty() {
        episode.gate_verdicts.iter().all(|verdict| verdict.passed)
    } else {
        episode.success
    }
}

fn episode_is_replan(episode: &Episode) -> bool {
    if episode.kind.eq_ignore_ascii_case("replan") {
        return true;
    }

    matches!(
        episode.extra.get("strategy").or_else(|| episode.extra.get("replan_strategy")),
        Some(Value::String(_)) | Some(Value::Number(_))
    ) || episode.extra.contains_key("attempt_number")
}

fn episode_new_knowledge_entries(episode: &Episode) -> usize {
    for key in [
        "new_knowledge_entries",
        "knowledge_entries_written",
        "knowledge_entries",
        "knowledge_written",
        "knowledge",
    ] {
        if let Some(value) = episode.extra.get(key) {
            return knowledge_entry_count(value);
        }
    }

    0
}

fn knowledge_entry_count(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::Bool(true) => 1,
        Value::Bool(false) => 0,
        Value::Number(number) => number
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .or_else(|| {
                number
                    .as_i64()
                    .filter(|count| *count > 0)
                    .and_then(|count| usize::try_from(count as u64).ok())
            })
            .unwrap_or(0),
        Value::Object(map) => map.len().max(1),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode_at(
        task_id: &str,
        minutes_ago: i64,
        cost_usd: f64,
        wall_ms: u64,
        success: bool,
    ) -> Episode {
        let mut episode = Episode::new("agent", task_id);
        episode.timestamp = Utc::now() - chrono::Duration::minutes(minutes_ago);
        episode.completed_at = episode.timestamp;
        episode.started_at = episode.timestamp;
        episode.usage.cost_usd = cost_usd;
        episode.usage.wall_ms = wall_ms;
        episode.success = success;
        episode
    }

    #[test]
    fn empty_window_returns_default_snapshot() {
        let cfactor = compute_cfactor(&[], Duration::from_secs(7 * 24 * 60 * 60));
        assert_eq!(cfactor.overall, 0.0);
        assert_eq!(cfactor.components, CFactorComponents::default());
        assert_eq!(cfactor.episode_count, 0);
    }

    #[test]
    fn computes_components_from_recent_task_groups() {
        let mut episodes = Vec::new();

        for i in 0..10 {
            episodes.push(episode_at(&format!("task-{i}"), 60 - i as i64, 10.0, 1_000, true));
        }

        episodes.push(episode_at("task-failed", 5, 10.0, 1_000, false));

        let mut replanned = episode_at("task-replan", 4, 10.0, 1_000, false);
        replanned.kind = "replan".to_string();
        replanned
            .extra
            .insert("strategy".to_string(), Value::String("retry-same".to_string()));
        episodes.push(replanned);
        episodes.push(episode_at("task-replan", 3, 5.0, 500, true));

        let mut knowledge_episode = episode_at("task-knowledge", 2, 10.0, 1_000, false);
        knowledge_episode
            .extra
            .insert("knowledge_entries_written".to_string(), Value::Number(2u64.into()));
        episodes.push(knowledge_episode);

        let cfactor = compute_cfactor(&episodes, Duration::from_secs(7 * 24 * 60 * 60));

        assert_eq!(cfactor.episode_count, 13);
        assert!((cfactor.components.gate_pass_rate - 11.0 / 12.0).abs() < 1e-9);
        assert!((cfactor.components.first_try_rate - 11.0 / 12.0).abs() < 1e-9);
        assert!((cfactor.components.cost_efficiency - 110.0 / 115.0).abs() < 1e-9);
        assert!((cfactor.components.speed - 110.0 / 115.0).abs() < 1e-9);
        assert!((cfactor.components.knowledge_growth - 2.0 / 13.0).abs() < 1e-9);
    }

    #[test]
    fn trend_arrow_uses_snapshots_inside_window() {
        let mut older = CFactor::default();
        older.overall = 0.35;
        older.computed_at = Utc::now() - chrono::Duration::days(8);

        let mut first = CFactor::default();
        first.overall = 0.40;
        first.computed_at = Utc::now() - chrono::Duration::days(6);

        let mut latest = CFactor::default();
        latest.overall = 0.55;
        latest.computed_at = Utc::now() - chrono::Duration::days(1);

        assert_eq!(
            trend_arrow(
                &[older, first, latest],
                Duration::from_secs(7 * 24 * 60 * 60)
            ),
            "↑"
        );
    }

    #[test]
    fn ignores_episodes_outside_window() {
        let recent = episode_at("recent", 5, 10.0, 1_000, true);
        let mut old = episode_at("old", 10_000, 50.0, 5_000, true);
        old.extra
            .insert("knowledge_entries_written".to_string(), Value::Number(5u64.into()));

        let cfactor = compute_cfactor(
            &[recent.clone(), old],
            Duration::from_secs(24 * 60 * 60),
        );

        assert_eq!(cfactor.episode_count, 1);
        assert!((cfactor.components.gate_pass_rate - 1.0).abs() < 1e-9);
        assert!((cfactor.components.first_try_rate - 1.0).abs() < 1e-9);
        assert!((cfactor.components.knowledge_growth - 0.0).abs() < 1e-9);
    }
}
