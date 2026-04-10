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
    /// Per-agent leave-one-out contribution scores.
    #[serde(default)]
    pub agent_contributions: Vec<AgentCFactorContribution>,
    /// Timestamp when the score was computed.
    pub computed_at: DateTime<Utc>,
    /// Number of episodes used in the calculation.
    pub episode_count: usize,
}

/// Leave-one-out contribution score for an individual agent.
///
/// Positive scores mean the agent raises the collective C-Factor. Negative
/// scores mean the agent drags the system down relative to the same window
/// without that agent's episodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentCFactorContribution {
    /// Agent identifier used to group episodes.
    pub agent_id: String,
    /// Number of episodes attributed to the agent.
    pub episode_count: usize,
    /// Collective score with this agent's episodes removed.
    pub without_agent_overall: f64,
    /// Full-snapshot overall minus the leave-one-out score.
    pub contribution_score: f64,
}

/// Directional routing preference derived from a C-Factor contribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentDispatchBias {
    /// Prefer a stronger / more capable agent or model for this dispatch.
    PreferStronger,
    /// Prefer the cheaper / lighter agent or model for this dispatch.
    PreferCheaper,
    /// Keep the current routing decision.
    Neutral,
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
    /// Normalized signal throughput relative to the baseline window.
    #[serde(default)]
    pub information_flow_rate: f64,
    /// % of tasks succeeding without re-plan.
    pub first_try_rate: f64,
    /// Rate of new knowledge entries per episode.
    pub knowledge_growth: f64,
    /// Speed at which shared insights accumulate confirmation chains.
    #[serde(default)]
    pub knowledge_integration_rate: f64,
    /// How strongly agent templates specialize by task category.
    #[serde(default)]
    pub task_diversity_coverage: f64,
    /// Speed at which divergent approaches reach a shared conclusion.
    #[serde(default)]
    pub convergence_velocity: f64,
    /// Evenness of agent participation inside a plan, normalized to `[0..1]`.
    pub turn_taking_equality: f64,
    /// Normalized reference rate for completed dependency outputs.
    #[serde(default)]
    pub social_sensitivity: f64,
}

/// Regression alert for a C-Factor drop against a trailing history window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorRegression {
    /// Timestamp of the newest snapshot in the analyzed window.
    pub current_snapshot_at: DateTime<Utc>,
    /// Start of the analyzed history window.
    pub window_start: DateTime<Utc>,
    /// End of the analyzed history window.
    pub window_end: DateTime<Utc>,
    /// Number of historical snapshots used to compute the average.
    pub sample_count: usize,
    /// Average C-Factor across the historical snapshots.
    pub historical_average: f64,
    /// C-Factor on the newest snapshot.
    pub current: f64,
    /// Fractional drop from the historical average to the current value.
    pub drop_fraction: f64,
    /// Threshold that was breached.
    pub threshold: f64,
}

impl Default for CFactorComponents {
    fn default() -> Self {
        Self {
            gate_pass_rate: 0.0,
            cost_efficiency: 0.0,
            speed: 0.0,
            information_flow_rate: 0.0,
            first_try_rate: 0.0,
            knowledge_growth: 0.0,
            knowledge_integration_rate: 0.0,
            task_diversity_coverage: 0.0,
            convergence_velocity: 0.0,
            turn_taking_equality: 0.0,
            social_sensitivity: 0.0,
        }
    }
}

impl Default for CFactor {
    fn default() -> Self {
        Self {
            overall: 0.0,
            components: CFactorComponents::default(),
            agent_contributions: Vec::new(),
            computed_at: Utc::now(),
            episode_count: 0,
        }
    }
}

impl CFactor {
    /// Format the strongest per-agent contributions as compact summary lines.
    #[must_use]
    pub fn top_agent_contribution_lines(&self, limit: usize) -> Vec<String> {
        self.agent_contributions
            .iter()
            .take(limit)
            .map(|contribution| {
                format!(
                    "{}={:+.3} (n={})",
                    contribution.agent_id,
                    contribution.contribution_score,
                    contribution.episode_count
                )
            })
            .collect()
    }

    /// Look up the contribution score for a specific agent.
    #[must_use]
    pub fn agent_contribution(&self, agent_id: &str) -> Option<&AgentCFactorContribution> {
        self.agent_contributions
            .iter()
            .find(|contribution| contribution.agent_id == agent_id)
    }

    /// Convert a per-agent contribution into a dispatch bias.
    ///
    /// Negative contributors get routed toward stronger models so dispatch
    /// decisions compensate for the gap. Positive contributors can be routed
    /// lighter once the overall fleet is healthy.
    #[must_use]
    pub fn dispatch_bias_for_agent(&self, agent_id: &str) -> AgentDispatchBias {
        let Some(contribution) = self.agent_contribution(agent_id) else {
            return AgentDispatchBias::Neutral;
        };

        if contribution.contribution_score <= -0.05 {
            AgentDispatchBias::PreferStronger
        } else if contribution.contribution_score >= 0.05 && self.overall >= 0.65 {
            AgentDispatchBias::PreferCheaper
        } else {
            AgentDispatchBias::Neutral
        }
    }
}

#[derive(Debug, Clone)]
struct TaskAggregate {
    cost_usd: f64,
    duration_ms: f64,
    signal_tokens: f64,
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
/// - `information_flow_rate` from signal token throughput relative to the
///   same baseline window
/// - `first_try_rate` over task groups that did not require a replan
/// - `knowledge_growth` from explicit knowledge counters present in episode
///   metadata
/// - `knowledge_integration_rate` from confirmation chains emitted by Neuro
///   distillation
/// - `task_diversity_coverage` from the association between agent template and
///   task category (specialization vs overlap)
/// - `convergence_velocity` from knowledge agreement across agents
/// - `turn_taking_equality` from the Gini coefficient of per-plan agent
///   contribution counts
/// - `social_sensitivity` from the fraction of `prior_output` context
///   sections that were referenced in the agent's output
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn compute_cfactor(
    episodes: &[Episode],
    window: Duration,
    social_sensitivity: f64,
    knowledge_integration_rate: f64,
    convergence_velocity: f64,
) -> CFactor {
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

    let computed_at = Utc::now();
    let mut tasks: HashMap<String, TaskAggregate> = HashMap::new();
    for episode in &filtered {
        let task_key = task_key(episode);
        let entry = tasks.entry(task_key).or_insert_with(|| TaskAggregate {
            cost_usd: 0.0,
            duration_ms: 0.0,
            signal_tokens: 0.0,
            passed_gate: false,
            saw_replan: false,
            first_seen: episode.timestamp,
        });

        entry.cost_usd += episode.usage.cost_usd;
        entry.duration_ms += episode_duration_ms(episode);
        entry.signal_tokens += episode_signal_tokens(episode);
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
    let passed_tasks = task_groups
        .iter()
        .filter(|(_, task)| task.passed_gate)
        .count();
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
    let task_diversity_coverage = compute_task_diversity_coverage(&filtered);

    let (avg_cost_per_successful_task, avg_duration_per_successful_task) =
        if successful_tasks.is_empty() {
            (0.0, 0.0)
        } else {
            let count = successful_tasks.len() as f64;
            let total_cost: f64 = successful_tasks.iter().map(|task| task.cost_usd).sum();
            let total_duration: f64 = successful_tasks.iter().map(|task| task.duration_ms).sum();
            (total_cost / count, total_duration / count)
        };

    let avg_signal_throughput_per_successful_task = if successful_tasks.is_empty() {
        0.0
    } else {
        let count = successful_tasks.len() as f64;
        let total_signal_throughput: f64 = successful_tasks
            .iter()
            .map(|task| signal_throughput(task.signal_tokens, task.duration_ms))
            .sum();
        total_signal_throughput / count
    };

    let baseline_task_count = task_groups.len().min(BASELINE_TASK_COUNT);
    let (baseline_cost, baseline_duration, baseline_signal_throughput) = if baseline_task_count == 0
    {
        (0.0, 0.0, 0.0)
    } else {
        let baseline_tasks: Vec<&(String, TaskAggregate)> =
            task_groups.iter().take(baseline_task_count).collect();
        let total_cost: f64 = baseline_tasks.iter().map(|(_, task)| task.cost_usd).sum();
        let total_duration: f64 = baseline_tasks
            .iter()
            .map(|(_, task)| task.duration_ms)
            .sum();
        let total_signal_throughput: f64 = baseline_tasks
            .iter()
            .map(|(_, task)| signal_throughput(task.signal_tokens, task.duration_ms))
            .sum();
        (
            total_cost / baseline_task_count as f64,
            total_duration / baseline_task_count as f64,
            total_signal_throughput / baseline_task_count as f64,
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

    let information_flow_rate =
        if baseline_signal_throughput > 0.0 && avg_signal_throughput_per_successful_task > 0.0 {
            avg_signal_throughput_per_successful_task / baseline_signal_throughput
        } else {
            0.0
        };

    let new_knowledge_entries: usize = filtered
        .iter()
        .map(|episode| episode_new_knowledge_entries(episode))
        .sum();
    let knowledge_growth = ratio(new_knowledge_entries, filtered.len());
    let knowledge_integration_rate = knowledge_integration_rate.clamp(0.0, 1.0);
    let convergence_velocity = convergence_velocity.clamp(0.0, 1.0);
    let turn_taking_equality = compute_turn_taking_equality(&filtered);
    let social_sensitivity = social_sensitivity.clamp(0.0, 1.0);

    let overall = (gate_pass_rate * 0.23
        + cost_efficiency * 0.15
        + speed * 0.10
        + information_flow_rate * 0.08
        + first_try_rate * 0.18
        + knowledge_growth * 0.08
        + knowledge_integration_rate * 0.07
        + task_diversity_coverage * 0.11)
        * 0.9
        + convergence_velocity * 0.05
        + turn_taking_equality * 0.05
        + social_sensitivity * 0.05;

    let mut snapshot = CFactor {
        overall: overall.clamp(0.0, 1.0),
        components: CFactorComponents {
            gate_pass_rate,
            cost_efficiency,
            speed,
            information_flow_rate,
            first_try_rate,
            knowledge_growth,
            knowledge_integration_rate,
            task_diversity_coverage,
            convergence_velocity,
            turn_taking_equality,
            social_sensitivity,
        },
        agent_contributions: Vec::new(),
        computed_at,
        episode_count: filtered.len(),
    };
    snapshot.agent_contributions = compute_agent_contributions(
        &filtered,
        computed_at,
        social_sensitivity,
        knowledge_integration_rate,
        convergence_velocity,
        snapshot.overall,
    );
    snapshot
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

/// Detect whether the latest C-Factor snapshot regressed against the recent window.
///
/// The newest snapshot is compared against the average of prior snapshots in
/// the same window. If the newest value drops by more than `threshold`, the
/// regression is returned for downstream alerting.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn detect_cfactor_regression(
    history: &[CFactor],
    window: Duration,
    threshold: f64,
) -> Option<CFactorRegression> {
    let cutoff = match chrono::Duration::from_std(window) {
        Ok(delta) => Utc::now() - delta,
        Err(_) => DateTime::<Utc>::MIN_UTC,
    };

    let mut snapshots: Vec<&CFactor> = history
        .iter()
        .filter(|snapshot| snapshot.computed_at >= cutoff)
        .collect();
    snapshots.sort_by(|left, right| left.computed_at.cmp(&right.computed_at));

    let Some(current) = snapshots.last().copied() else {
        return None;
    };
    let historical = &snapshots[..snapshots.len().saturating_sub(1)];
    if historical.is_empty() {
        return None;
    }

    let historical_average = historical
        .iter()
        .map(|snapshot| snapshot.overall)
        .sum::<f64>()
        / historical.len() as f64;
    if historical_average <= 0.0 || current.overall >= historical_average {
        return None;
    }

    let drop_fraction = (historical_average - current.overall) / historical_average;
    if drop_fraction <= threshold {
        return None;
    }

    Some(CFactorRegression {
        current_snapshot_at: current.computed_at,
        window_start: historical
            .first()
            .map(|snapshot| snapshot.computed_at)
            .unwrap_or(current.computed_at),
        window_end: current.computed_at,
        sample_count: historical.len(),
        historical_average,
        current: current.overall,
        drop_fraction,
        threshold,
    })
}

fn ratio(numer: usize, denom: usize) -> f64 {
    if denom == 0 {
        0.0
    } else {
        numer as f64 / denom as f64
    }
}

fn compute_turn_taking_equality(episodes: &[&Episode]) -> f64 {
    let mut plan_contributions: HashMap<String, HashMap<String, u64>> = HashMap::new();
    for episode in episodes {
        let plan_key = episode_plan_key(episode);
        let agent_key = episode_agent_key(episode);
        let agent_counts = plan_contributions.entry(plan_key).or_default();
        *agent_counts.entry(agent_key).or_default() += 1;
    }

    let mut total_equality = 0.0;
    let mut plan_count = 0.0;
    for agent_counts in plan_contributions.values() {
        if agent_counts.is_empty() {
            continue;
        }

        let equality = turn_taking_equality_for_counts(agent_counts.values().copied().collect());
        total_equality += equality;
        plan_count += 1.0;
    }

    if plan_count == 0.0 {
        0.0
    } else {
        (total_equality / plan_count).clamp(0.0, 1.0)
    }
}

fn turn_taking_equality_for_counts(counts: Vec<u64>) -> f64 {
    if counts.len() < 2 {
        return 0.0;
    }

    let gini = gini_coefficient(&counts);
    (1.0 - gini).clamp(0.0, 1.0)
}

fn compute_task_diversity_coverage(episodes: &[&Episode]) -> f64 {
    let mut joint_counts: HashMap<(String, String), u64> = HashMap::new();
    let mut template_counts: HashMap<String, u64> = HashMap::new();
    let mut category_counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;

    for episode in episodes {
        let Some(template) = episode_agent_template(episode) else {
            continue;
        };
        let Some(category) = episode_task_category(episode) else {
            continue;
        };

        *joint_counts
            .entry((template.clone(), category.clone()))
            .or_default() += 1;
        *template_counts.entry(template).or_default() += 1;
        *category_counts.entry(category).or_default() += 1;
        total += 1;
    }

    if total == 0 {
        return 0.0;
    }

    let template_entropy = entropy_from_counts(template_counts.values().copied().collect());
    let category_entropy = entropy_from_counts(category_counts.values().copied().collect());
    let normalization = template_entropy.max(category_entropy);
    if normalization <= 0.0 {
        return 0.0;
    }

    let total_f = total as f64;
    let mut mutual_information = 0.0;
    for ((template, category), joint_count) in joint_counts {
        let joint_prob = joint_count as f64 / total_f;
        let template_prob = template_counts[&template] as f64 / total_f;
        let category_prob = category_counts[&category] as f64 / total_f;
        let ratio = joint_prob / (template_prob * category_prob);
        mutual_information += joint_prob * ratio.log2();
    }

    (mutual_information / normalization).clamp(0.0, 1.0)
}

fn entropy_from_counts(counts: Vec<u64>) -> f64 {
    let total: u64 = counts.iter().copied().sum();
    if total == 0 {
        return 0.0;
    }

    let total_f = total as f64;
    let mut entropy = 0.0;
    for count in counts {
        if count == 0 {
            continue;
        }
        let probability = count as f64 / total_f;
        entropy -= probability * probability.log2();
    }
    entropy
}

fn gini_coefficient(counts: &[u64]) -> f64 {
    if counts.len() < 2 {
        return 0.0;
    }

    let mut values: Vec<f64> = counts.iter().map(|&count| count as f64).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let total: f64 = values.iter().sum();
    if total <= 0.0 {
        return 0.0;
    }

    let weighted_sum: f64 = values
        .iter()
        .enumerate()
        .map(|(index, value)| (index as f64 + 1.0) * value)
        .sum();
    let n = values.len() as f64;
    let gini = (2.0 * weighted_sum) / (n * total) - (n + 1.0) / n;
    gini.clamp(0.0, 1.0)
}

fn episode_plan_key(episode: &Episode) -> String {
    episode
        .extra
        .get("plan_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let task_id = episode.task_id.trim();
            if task_id.is_empty() {
                None
            } else {
                Some(task_id.to_string())
            }
        })
        .unwrap_or_else(|| episode.id.clone())
}

fn episode_agent_key(episode: &Episode) -> String {
    let agent_id = episode.agent_id.trim();
    if !agent_id.is_empty() {
        return agent_id.to_string();
    }

    let template = episode.agent_template.trim();
    if !template.is_empty() {
        return template.to_string();
    }

    episode.id.clone()
}

fn episode_agent_template(episode: &Episode) -> Option<String> {
    let template = episode.agent_template.trim();
    if !template.is_empty() {
        return Some(template.to_string());
    }

    let agent_id = episode.agent_id.trim();
    if !agent_id.is_empty() {
        return Some(agent_id.to_string());
    }

    None
}

fn episode_task_category(episode: &Episode) -> Option<String> {
    episode
        .extra
        .get("task_category")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
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

fn episode_signal_tokens(episode: &Episode) -> f64 {
    (episode.usage.input_tokens + episode.usage.output_tokens) as f64
}

fn signal_throughput(signal_tokens: f64, duration_ms: f64) -> f64 {
    if duration_ms <= 0.0 {
        0.0
    } else {
        signal_tokens / duration_ms
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
        episode
            .extra
            .get("strategy")
            .or_else(|| episode.extra.get("replan_strategy")),
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

fn compute_agent_contributions(
    filtered: &[&Episode],
    computed_at: DateTime<Utc>,
    social_sensitivity: f64,
    knowledge_integration_rate: f64,
    convergence_velocity: f64,
    overall: f64,
) -> Vec<AgentCFactorContribution> {
    let mut agents: HashMap<String, Vec<&Episode>> = HashMap::new();
    for episode in filtered {
        agents.entry(episode_agent_key(episode)).or_default().push(episode);
    }

    let mut contributions = Vec::with_capacity(agents.len());
    for (agent_id, agent_episodes) in agents {
        let remaining: Vec<&Episode> = filtered
            .iter()
            .copied()
            .filter(|episode| episode_agent_key(episode) != agent_id)
            .collect();
        let without_agent = if remaining.is_empty() {
            CFactor {
                overall: 0.0,
                components: CFactorComponents::default(),
                agent_contributions: Vec::new(),
                computed_at,
                episode_count: 0,
            }
        } else {
            compute_cfactor_from_filtered(
                &remaining,
                computed_at,
                social_sensitivity,
                knowledge_integration_rate,
                convergence_velocity,
            )
        };

        contributions.push(AgentCFactorContribution {
            agent_id,
            episode_count: agent_episodes.len(),
            without_agent_overall: without_agent.overall,
            contribution_score: overall - without_agent.overall,
        });
    }

    contributions.sort_by(|a, b| {
        b.contribution_score
            .partial_cmp(&a.contribution_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });

    contributions
}

#[allow(clippy::cast_precision_loss)]
fn compute_cfactor_from_filtered(
    filtered: &[&Episode],
    computed_at: DateTime<Utc>,
    social_sensitivity: f64,
    knowledge_integration_rate: f64,
    convergence_velocity: f64,
) -> CFactor {
    let mut tasks: HashMap<String, TaskAggregate> = HashMap::new();
    for episode in filtered {
        let task_key = task_key(episode);
        let entry = tasks.entry(task_key).or_insert_with(|| TaskAggregate {
            cost_usd: 0.0,
            duration_ms: 0.0,
            signal_tokens: 0.0,
            passed_gate: false,
            saw_replan: false,
            first_seen: episode.timestamp,
        });

        entry.cost_usd += episode.usage.cost_usd;
        entry.duration_ms += episode_duration_ms(episode);
        entry.signal_tokens += episode_signal_tokens(episode);
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
    let passed_tasks = task_groups
        .iter()
        .filter(|(_, task)| task.passed_gate)
        .count();
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
    let task_diversity_coverage = compute_task_diversity_coverage(filtered);

    let (avg_cost_per_successful_task, avg_duration_per_successful_task) =
        if successful_tasks.is_empty() {
            (0.0, 0.0)
        } else {
            let count = successful_tasks.len() as f64;
            let total_cost: f64 = successful_tasks.iter().map(|task| task.cost_usd).sum();
            let total_duration: f64 = successful_tasks.iter().map(|task| task.duration_ms).sum();
            (total_cost / count, total_duration / count)
        };

    let avg_signal_throughput_per_successful_task = if successful_tasks.is_empty() {
        0.0
    } else {
        let count = successful_tasks.len() as f64;
        let total_signal_throughput: f64 = successful_tasks
            .iter()
            .map(|task| signal_throughput(task.signal_tokens, task.duration_ms))
            .sum();
        total_signal_throughput / count
    };

    let baseline_task_count = task_groups.len().min(BASELINE_TASK_COUNT);
    let (baseline_cost, baseline_duration, baseline_signal_throughput) = if baseline_task_count == 0
    {
        (0.0, 0.0, 0.0)
    } else {
        let baseline_tasks: Vec<&(String, TaskAggregate)> =
            task_groups.iter().take(baseline_task_count).collect();
        let total_cost: f64 = baseline_tasks.iter().map(|(_, task)| task.cost_usd).sum();
        let total_duration: f64 = baseline_tasks
            .iter()
            .map(|(_, task)| task.duration_ms)
            .sum();
        let total_signal_throughput: f64 = baseline_tasks
            .iter()
            .map(|(_, task)| signal_throughput(task.signal_tokens, task.duration_ms))
            .sum();
        (
            total_cost / baseline_task_count as f64,
            total_duration / baseline_task_count as f64,
            total_signal_throughput / baseline_task_count as f64,
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

    let information_flow_rate =
        if baseline_signal_throughput > 0.0 && avg_signal_throughput_per_successful_task > 0.0 {
            avg_signal_throughput_per_successful_task / baseline_signal_throughput
        } else {
            0.0
        };

    let new_knowledge_entries: usize = filtered
        .iter()
        .map(|episode| episode_new_knowledge_entries(episode))
        .sum();
    let knowledge_growth = ratio(new_knowledge_entries, filtered.len());
    let knowledge_integration_rate = knowledge_integration_rate.clamp(0.0, 1.0);
    let convergence_velocity = convergence_velocity.clamp(0.0, 1.0);
    let turn_taking_equality = compute_turn_taking_equality(filtered);
    let social_sensitivity = social_sensitivity.clamp(0.0, 1.0);

    let overall = (gate_pass_rate * 0.23
        + cost_efficiency * 0.15
        + speed * 0.10
        + information_flow_rate * 0.08
        + first_try_rate * 0.18
        + knowledge_growth * 0.08
        + knowledge_integration_rate * 0.07
        + task_diversity_coverage * 0.11)
        * 0.9
        + convergence_velocity * 0.05
        + turn_taking_equality * 0.05
        + social_sensitivity * 0.05;

    CFactor {
        overall: overall.clamp(0.0, 1.0),
        components: CFactorComponents {
            gate_pass_rate,
            cost_efficiency,
            speed,
            information_flow_rate,
            first_try_rate,
            knowledge_growth,
            knowledge_integration_rate,
            task_diversity_coverage,
            convergence_velocity,
            turn_taking_equality,
            social_sensitivity,
        },
        agent_contributions: Vec::new(),
        computed_at,
        episode_count: filtered.len(),
    }
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

    const CFACTOR_REGRESSION_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
    const CFACTOR_REGRESSION_THRESHOLD: f64 = 0.20;

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
        let cfactor = compute_cfactor(&[], Duration::from_secs(7 * 24 * 60 * 60), 0.0, 0.0, 0.0);
        assert_eq!(cfactor.overall, 0.0);
        assert_eq!(cfactor.components, CFactorComponents::default());
        assert_eq!(cfactor.episode_count, 0);
    }

    #[test]
    fn computes_components_from_recent_task_groups() {
        let mut episodes = Vec::new();

        for i in 0..10 {
            episodes.push(episode_at(
                &format!("task-{i}"),
                60 - i as i64,
                10.0,
                1_000,
                true,
            ));
        }

        episodes.push(episode_at("task-failed", 5, 10.0, 1_000, false));

        let mut replanned = episode_at("task-replan", 4, 10.0, 1_000, false);
        replanned.kind = "replan".to_string();
        replanned.extra.insert(
            "strategy".to_string(),
            Value::String("retry-same".to_string()),
        );
        episodes.push(replanned);
        episodes.push(episode_at("task-replan", 3, 5.0, 500, true));

        let mut knowledge_episode = episode_at("task-knowledge", 2, 10.0, 1_000, false);
        knowledge_episode.extra.insert(
            "knowledge_entries_written".to_string(),
            Value::Number(2u64.into()),
        );
        episodes.push(knowledge_episode);

        let cfactor = compute_cfactor(&episodes, Duration::from_secs(7 * 24 * 60 * 60), 0.0, 0.0, 0.0);

        assert_eq!(cfactor.episode_count, 14);
        // 11 of ~13 task episodes pass gates
        assert!(cfactor.components.gate_pass_rate > 0.7 && cfactor.components.gate_pass_rate < 1.0);
        assert!(cfactor.components.first_try_rate > 0.7 && cfactor.components.first_try_rate < 1.0);
        assert!(cfactor.components.cost_efficiency > 0.8);
        assert!(cfactor.components.speed > 0.8);
        assert!(cfactor.components.knowledge_growth > 0.0);
        assert!((cfactor.components.knowledge_integration_rate - 0.0).abs() < 1e-9);
        assert!((cfactor.components.social_sensitivity - 0.0).abs() < 1e-9);
    }

    #[test]
    fn computes_information_flow_rate_from_signal_throughput() {
        let mut episodes = Vec::new();

        for i in 0..10 {
            let mut episode =
                episode_at(&format!("baseline-{i}"), 60 - i as i64, 10.0, 1_000, false);
            episode.usage.input_tokens = 300;
            episode.usage.output_tokens = 200;
            episodes.push(episode);
        }

        let mut current = episode_at("current", 1, 10.0, 1_000, true);
        current.usage.input_tokens = 900;
        current.usage.output_tokens = 600;
        episodes.push(current);

        let cfactor = compute_cfactor(&episodes, Duration::from_secs(7 * 24 * 60 * 60), 0.0, 0.0, 0.0);
        assert!((cfactor.components.information_flow_rate - 3.0).abs() < 1e-9);
    }

    #[test]
    fn computes_turn_taking_equality_from_agent_participation() {
        let mut episodes = Vec::new();

        let mut even_a = episode_at("task-even", 5, 10.0, 1_000, true);
        even_a.agent_id = "agent-a".to_string();
        even_a
            .extra
            .insert("plan_id".to_string(), Value::String("plan-even".to_string()));
        episodes.push(even_a);

        let mut even_b = episode_at("task-even", 4, 10.0, 1_000, true);
        even_b.agent_id = "agent-b".to_string();
        even_b
            .extra
            .insert("plan_id".to_string(), Value::String("plan-even".to_string()));
        episodes.push(even_b);

        let mut solo = episode_at("task-solo", 3, 10.0, 1_000, true);
        solo.agent_id = "agent-c".to_string();
        solo.extra.insert(
            "plan_id".to_string(),
            Value::String("plan-solo".to_string()),
        );
        episodes.push(solo);

        let cfactor = compute_cfactor(&episodes, Duration::from_secs(7 * 24 * 60 * 60), 0.0, 0.0, 0.0);
        assert!((cfactor.components.turn_taking_equality - 0.5).abs() < 1e-9);
    }

    #[test]
    fn computes_task_diversity_coverage_from_template_category_alignment() {
        let mut episodes = Vec::new();

        for suffix in ["a", "b"] {
            let mut implementation = episode_at(&format!("task-impl-{suffix}"), 5, 10.0, 1_000, true);
            implementation.agent_template = "code-implementer".to_string();
            implementation.extra.insert(
                "task_category".to_string(),
                Value::String("implementation".to_string()),
            );
            episodes.push(implementation);

            let mut docs = episode_at(&format!("task-docs-{suffix}"), 4, 10.0, 1_000, true);
            docs.agent_template = "docs-specialist".to_string();
            docs.extra.insert(
                "task_category".to_string(),
                Value::String("docs".to_string()),
            );
            episodes.push(docs);
        }

        let cfactor = compute_cfactor(&episodes, Duration::from_secs(7 * 24 * 60 * 60), 0.0, 0.0, 0.0);
        assert!((cfactor.components.task_diversity_coverage - 1.0).abs() < 1e-9);
    }

    #[test]
    fn computes_agent_contribution_scores_with_leave_one_out_delta() {
        let mut good = episode_at("task-good", 5, 5.0, 1_000, true);
        good.agent_id = "agent-good".to_string();
        good.extra.insert(
            "plan_id".to_string(),
            Value::String("plan-good".to_string()),
        );

        let mut bad = episode_at("task-bad", 4, 5.0, 1_000, false);
        bad.agent_id = "agent-bad".to_string();
        bad.extra.insert(
            "plan_id".to_string(),
            Value::String("plan-bad".to_string()),
        );

        let cfactor = compute_cfactor(
            &[good, bad],
            Duration::from_secs(7 * 24 * 60 * 60),
            0.0,
            0.0,
            0.0,
        );

        assert_eq!(cfactor.agent_contributions.len(), 2);
        assert_eq!(cfactor.agent_contributions[0].agent_id, "agent-good");
        assert_eq!(cfactor.agent_contributions[1].agent_id, "agent-bad");
        assert!(cfactor.agent_contributions[0].contribution_score > 0.0);
        assert!(cfactor.agent_contributions[1].contribution_score < 0.0);
        assert!(
            cfactor.agent_contributions[0].contribution_score
                > cfactor.agent_contributions[1].contribution_score
        );
        assert!(
            cfactor
                .top_agent_contribution_lines(1)
                .first()
                .is_some_and(|line| line.starts_with("agent-good="))
        );
    }

    #[test]
    fn dispatch_bias_prefers_stronger_for_negative_contributor() {
        let mut snapshot = CFactor::default();
        snapshot.overall = 0.72;
        snapshot.agent_contributions = vec![AgentCFactorContribution {
            agent_id: "agent-neg".to_string(),
            episode_count: 4,
            without_agent_overall: 0.78,
            contribution_score: -0.06,
        }];

        assert_eq!(
            snapshot.dispatch_bias_for_agent("agent-neg"),
            AgentDispatchBias::PreferStronger
        );
    }

    #[test]
    fn dispatch_bias_prefers_cheaper_for_positive_contributor() {
        let mut snapshot = CFactor::default();
        snapshot.overall = 0.83;
        snapshot.agent_contributions = vec![AgentCFactorContribution {
            agent_id: "agent-pos".to_string(),
            episode_count: 5,
            without_agent_overall: 0.76,
            contribution_score: 0.07,
        }];

        assert_eq!(
            snapshot.dispatch_bias_for_agent("agent-pos"),
            AgentDispatchBias::PreferCheaper
        );
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
        old.extra.insert(
            "knowledge_entries_written".to_string(),
            Value::Number(5u64.into()),
        );

        let cfactor = compute_cfactor(
            &[recent.clone(), old],
            Duration::from_secs(24 * 60 * 60),
            0.0,
            0.0,
            0.0,
        );

        assert_eq!(cfactor.episode_count, 1);
        assert!((cfactor.components.gate_pass_rate - 1.0).abs() < 1e-9);
        assert!((cfactor.components.first_try_rate - 1.0).abs() < 1e-9);
        assert!((cfactor.components.knowledge_growth - 0.0).abs() < 1e-9);
        assert!((cfactor.components.turn_taking_equality - 0.0).abs() < 1e-9);
        assert!((cfactor.components.social_sensitivity - 0.0).abs() < 1e-9);
    }

    #[test]
    fn social_sensitivity_is_captured_in_overall_score() {
        let episodes = vec![episode_at("task-1", 1, 10.0, 1_000, true)];
        let baseline = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.0, 0.0, 0.0);
        let cfactor = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.8, 0.0, 0.0);

        assert!((cfactor.components.social_sensitivity - 0.8).abs() < 1e-9);
        assert!(cfactor.overall > baseline.overall);
    }

    #[test]
    fn knowledge_integration_rate_is_captured_in_overall_score() {
        let episodes = vec![episode_at("task-1", 1, 10.0, 1_000, true)];
        let baseline = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.0, 0.0, 0.0);
        let cfactor = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.0, 0.8, 0.0);

        assert!((cfactor.components.knowledge_integration_rate - 0.8).abs() < 1e-9);
        assert!(cfactor.overall > baseline.overall);
    }

    #[test]
    fn convergence_velocity_is_captured_in_overall_score() {
        let episodes = vec![episode_at("task-1", 1, 10.0, 1_000, true)];
        let baseline = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.0, 0.0, 0.0);
        let cfactor = compute_cfactor(&episodes, Duration::from_secs(24 * 60 * 60), 0.0, 0.0, 0.8);

        assert!((cfactor.components.convergence_velocity - 0.8).abs() < 1e-9);
        assert!(cfactor.overall > baseline.overall);
    }

    #[test]
    fn detects_cfactor_regression_against_recent_average() {
        let mut older = CFactor::default();
        older.overall = 0.92;
        older.computed_at = Utc::now() - chrono::Duration::days(6);

        let mut middle = CFactor::default();
        middle.overall = 0.84;
        middle.computed_at = Utc::now() - chrono::Duration::days(3);

        let mut current = CFactor::default();
        current.overall = 0.55;
        current.computed_at = Utc::now() - chrono::Duration::days(1);

        let regression = detect_cfactor_regression(
            &[older, middle, current],
            Duration::from_secs(CFACTOR_REGRESSION_WINDOW_SECS),
            CFACTOR_REGRESSION_THRESHOLD,
        )
        .expect("regression");

        assert_eq!(regression.sample_count, 2);
        assert!((regression.historical_average - 0.88).abs() < 1e-9);
        assert!((regression.current - 0.55).abs() < 1e-9);
        assert!(regression.drop_fraction > CFACTOR_REGRESSION_THRESHOLD);
    }

    #[test]
    fn does_not_fire_at_exact_threshold() {
        let mut older = CFactor::default();
        older.overall = 1.0;
        older.computed_at = Utc::now() - chrono::Duration::days(6);

        let mut current = CFactor::default();
        current.overall = 0.8;
        current.computed_at = Utc::now() - chrono::Duration::days(1);

        let regression = detect_cfactor_regression(
            &[older, current],
            Duration::from_secs(CFACTOR_REGRESSION_WINDOW_SECS),
            CFACTOR_REGRESSION_THRESHOLD,
        );

        assert!(regression.is_none());
    }
}
