//! Agent efficiency events, prompt scoring, and role cost profiles.
//!
//! This module implements the efficiency monitoring pipeline described in
//! `tmp/mori-agents/22-efficiency-monitoring.md`. It bridges per-agent-turn
//! execution data with system-level optimization by providing:
//!
//! - [`AgentEfficiencyEvent`] — rich per-turn cost and quality snapshot
//! - [`PromptSectionMeta`] — per-section token attribution
//! - [`RoleCostProfile`] — aggregate cost profile per agent role
//! - [`PromptEfficiencyScore`] and [`Grade`] — A-D letter grading for
//!   prompt assembly efficiency
//!
//! # Design
//!
//! Efficiency events are computed *after* each agent turn and gate
//! evaluation. They are never mutated — each turn produces one immutable
//! event. Downstream consumers (bandits, dashboards, regression detector)
//! read from the accumulated event stream.

use std::collections::{HashMap, HashSet};

use roko_core::OperatingFrequency;
use serde::{Deserialize, Serialize};

const BASELINE_PLAN_COUNT: usize = 10;

// ─── PromptSectionMeta ──────────────────────────────────────────────────────

/// Metadata for one section in a composed prompt.
///
/// Used to attribute token budget consumption to individual prompt sections
/// so the section bandit and efficiency scorer can reason about which
/// sections pull their weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSectionMeta {
    /// Section name (e.g. `"prd2"`, `"workspace_map"`, `"playbook_hits"`).
    pub name: String,
    /// Number of tokens this section consumed in the final prompt.
    pub tokens: u64,
    /// Compose-assigned priority (0 = highest, 255 = lowest).
    pub priority: u8,
    /// Whether this section was truncated due to budget pressure.
    pub was_truncated: bool,
    /// Whether this section was dropped entirely due to budget pressure.
    pub was_dropped: bool,
}

// ─── ToolCallMeta ───────────────────────────────────────────────────────────

/// Metadata for one tool call made during an agent turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallMeta {
    /// Tool name (e.g. `"Read"`, `"Write"`, `"Bash"`).
    pub tool_name: String,
    /// Wall-clock duration of the tool call in milliseconds.
    pub duration_ms: u64,
    /// Number of tokens in the tool result.
    pub result_tokens: u64,
    /// Whether the tool call succeeded.
    pub succeeded: bool,
    /// Whether this call contributed useful progress toward the final solution.
    #[serde(default)]
    pub advanced_task: bool,
    /// Whether this call was later determined to be unnecessary.
    #[serde(default)]
    pub was_redundant: bool,
    /// Failure category for the call, when one was identified.
    #[serde(default)]
    pub error_category: Option<String>,
}

// ─── AgentEfficiencyEvent ───────────────────────────────────────────────────

/// Emitted once per agent turn completion, summarizing cost and efficiency.
///
/// This is the bridge between agent-level execution and system-level
/// optimization. Contains 20+ fields covering identity, token accounting,
/// cost accounting, prompt composition, tool utilization, and timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEfficiencyEvent {
    // ── Identity ────────────────────────────────────────────────────
    /// Agent identifier.
    pub agent_id: String,
    /// Agent role (e.g. `"Implementer"`, `"Reviewer"`).
    pub role: String,
    /// Backend that ran this turn.
    pub backend: String,
    /// Exact model slug.
    pub model: String,
    /// Plan this turn belongs to.
    pub plan_id: String,
    /// Task within the plan.
    pub task_id: String,
    /// Unique identifier for this dispatch attempt.
    ///
    /// Shared between the dispatch cost event and any gate-failure event for
    /// the same attempt, enabling cross-event joins.
    #[serde(default)]
    pub attempt_id: String,

    // ── Token accounting ────────────────────────────────────────────
    /// Input tokens from provider response.
    pub input_tokens: u64,
    /// Output tokens from provider response.
    pub output_tokens: u64,
    /// Tokens used for reasoning/thinking.
    #[serde(default)]
    pub reasoning_tokens: u64,
    /// Tokens served from cache (subset of input).
    pub cache_read_tokens: u64,
    /// Tokens written to cache.
    pub cache_write_tokens: u64,

    // ── Cost accounting ─────────────────────────────────────────────
    /// Actual cost after cache discount.
    pub cost_usd: f64,
    /// What it would have cost without caching.
    pub cost_usd_without_cache: f64,

    // ── Prompt composition ──────────────────────────────────────────
    /// Per-section metadata.
    pub prompt_sections: Vec<PromptSectionMeta>,
    /// Total tokens in the assembled prompt.
    pub total_prompt_tokens: u64,
    /// Tokens in the system prompt (subset of total).
    pub system_prompt_tokens: u64,

    // ── Tool utilization ────────────────────────────────────────────
    /// Number of tools available to the agent.
    pub tools_available: u32,
    /// Number of distinct tools the agent actually used.
    pub tools_used: u32,
    /// Per-tool-call metadata.
    pub tool_calls: Vec<ToolCallMeta>,

    // ── Timing ──────────────────────────────────────────────────────
    /// Wall-clock milliseconds for the entire turn.
    pub wall_time_ms: u64,
    /// Alias for wall-clock task duration in milliseconds.
    #[serde(default)]
    pub duration_ms: u64,
    /// Time to first token in milliseconds.
    pub time_to_first_token_ms: u64,
    /// Whether this agent was a warm-pool reuse or cold start.
    pub was_warm_start: bool,

    // ── Outcome ─────────────────────────────────────────────────────
    /// Iteration number.
    pub iteration: u32,
    /// Whether the gate passed after this turn.
    pub gate_passed: bool,
    /// Outcome label for the observation.
    #[serde(default)]
    pub outcome: String,
    /// Verify error summaries recorded for failed tasks.
    #[serde(default)]
    pub gate_errors: Vec<String>,
    /// Resolved model used for the task attempt.
    #[serde(rename = "resolved_model", alias = "model_used", default)]
    pub model_used: String,
    /// Operating frequency for the turn.
    #[serde(default = "default_operating_frequency")]
    pub frequency: OperatingFrequency,
    /// Replanning or retry strategy attempted after failure.
    #[serde(default)]
    pub strategy_attempted: String,
    /// ISO-8601 UTC timestamp.
    pub timestamp: String,
}

impl AgentEfficiencyEvent {
    /// Build a default empty event payload.
    #[must_use]
    pub fn default_event() -> Self {
        Self::default()
    }

    /// Compute the cache hit rate for this event.
    #[allow(clippy::cast_precision_loss)]
    pub fn cache_hit_rate(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / self.input_tokens as f64
    }

    /// Compute tool utilization rate (tools used / tools available).
    pub fn tool_utilization(&self) -> f64 {
        if self.tools_available == 0 {
            return 0.0;
        }
        f64::from(self.tools_used) / f64::from(self.tools_available)
    }

    /// Compute cost savings from caching.
    pub fn cache_savings_usd(&self) -> f64 {
        self.cost_usd_without_cache - self.cost_usd
    }

    /// Total tokens consumed (input + output).
    pub const fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

fn default_operating_frequency() -> OperatingFrequency {
    OperatingFrequency::Theta
}

impl Default for AgentEfficiencyEvent {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            role: String::new(),
            backend: String::new(),
            model: String::new(),
            plan_id: String::new(),
            task_id: String::new(),
            attempt_id: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: 0.0,
            cost_usd_without_cache: 0.0,
            prompt_sections: Vec::new(),
            total_prompt_tokens: 0,
            system_prompt_tokens: 0,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: 0,
            duration_ms: 0,
            time_to_first_token_ms: 0,
            was_warm_start: false,
            iteration: 0,
            gate_passed: false,
            outcome: String::new(),
            gate_errors: Vec::new(),
            model_used: String::new(),
            frequency: default_operating_frequency(),
            strategy_attempted: String::new(),
            timestamp: String::new(),
        }
    }
}

// ─── Grade ──────────────────────────────────────────────────────────────────

/// Letter grade for prompt efficiency.
///
/// - **A**: High signal, low budget usage, high cache, passed gate
/// - **B**: Moderate signal, moderate budget, passed gate
/// - **C**: Low signal, high budget usage, or failed gate
/// - **D**: Very low signal, budget-busting, failed gate
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Grade {
    /// Excellent efficiency.
    A,
    /// Good efficiency.
    B,
    /// Fair efficiency.
    C,
    /// Poor efficiency.
    D,
}

impl Grade {
    /// Numeric score: A=4, B=3, C=2, D=1.
    pub const fn numeric(self) -> u8 {
        match self {
            Self::A => 4,
            Self::B => 3,
            Self::C => 2,
            Self::D => 1,
        }
    }

    /// Display label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
        }
    }
}

impl std::fmt::Display for Grade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ─── PromptEfficiencyScore ──────────────────────────────────────────────────

/// Scores a single prompt assembly on how efficiently it used its token budget.
///
/// Combines four sub-scores into a weighted composite that maps to a letter
/// [`Grade`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptEfficiencyScore {
    /// Ratio of "useful" tokens to total tokens (`[0..1]`).
    /// Useful = sections that correlate with pass rate improvement.
    pub signal_ratio: f64,
    /// How much of the budget was used (`actual_tokens / max_tokens`).
    pub budget_utilization: f64,
    /// What fraction of input tokens was served from cache.
    pub cache_efficiency: f64,
    /// Whether the gate passed after this prompt was used.
    pub gate_passed: bool,
}

impl PromptEfficiencyScore {
    /// Create a new efficiency score.
    pub const fn new(
        signal_ratio: f64,
        budget_utilization: f64,
        cache_efficiency: f64,
        gate_passed: bool,
    ) -> Self {
        Self {
            signal_ratio: signal_ratio.clamp(0.0, 1.0),
            budget_utilization: budget_utilization.clamp(0.0, 1.0),
            cache_efficiency: cache_efficiency.clamp(0.0, 1.0),
            gate_passed,
        }
    }

    /// Compute the weighted composite score (`[0..1]`).
    ///
    /// Weights: signal 40%, budget headroom 20%, cache 20%, outcome 20%.
    #[allow(clippy::suboptimal_flops)]
    pub fn composite(&self) -> f64 {
        let outcome = if self.gate_passed { 1.0 } else { 0.0 };
        self.signal_ratio * 0.4
            + (1.0 - self.budget_utilization) * 0.2
            + self.cache_efficiency * 0.2
            + outcome * 0.2
    }

    /// Compute the letter grade from the composite score.
    pub fn grade(&self) -> Grade {
        let score = self.composite();
        if score >= 0.75 {
            Grade::A
        } else if score >= 0.50 {
            Grade::B
        } else if score >= 0.25 {
            Grade::C
        } else {
            Grade::D
        }
    }
}

// ─── RoleCostProfile ────────────────────────────────────────────────────────

/// Aggregate cost profile for a single agent role, computed from accumulated
/// efficiency events.
///
/// Answers questions like "What does the average Implementer turn cost?" and
/// "What is the cost per gate pass?"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleCostProfile {
    /// Agent role this profile covers.
    pub role: String,
    /// Number of efficiency events contributing.
    pub observations: u64,

    // ── Token averages ──────────────────────────────────────────────
    /// Average input tokens per turn.
    pub avg_input_tokens: f64,
    /// Average output tokens per turn.
    pub avg_output_tokens: f64,
    /// Average cache hit rate (`cache_read` / `input`).
    pub avg_cache_hit_rate: f64,

    // ── Cost averages ───────────────────────────────────────────────
    /// Average cost in USD per turn.
    pub avg_cost_usd: f64,
    /// 95th percentile cost in USD.
    pub p95_cost_usd: f64,
    /// Total cost / gate passes — true cost of one success.
    pub cost_per_pass: f64,

    // ── Efficiency ──────────────────────────────────────────────────
    /// Average tool utilization (`tools_used` / `tools_available`).
    pub avg_tool_utilization: f64,
    /// Average wall time in milliseconds.
    pub avg_wall_time_ms: f64,
    /// Fraction of turns that were warm starts.
    pub warm_start_pct: f64,
    /// Overall gate pass rate for this role.
    pub pass_rate: f64,
}

impl RoleCostProfile {
    /// Cost of one successful task for this role.
    #[must_use]
    pub fn cost_per_successful_task(&self) -> f64 {
        if self.pass_rate <= 0.0 {
            return f64::INFINITY;
        }
        self.avg_cost_usd / self.pass_rate
    }
}

/// Aggregate cost profile for a single operating frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrequencyCostProfile {
    /// Operating frequency this profile covers.
    pub frequency: OperatingFrequency,
    /// Number of efficiency events contributing.
    pub observations: u64,
    /// Average cost in USD per turn.
    pub avg_cost_usd: f64,
    /// Total cost in USD across all turns.
    pub total_cost_usd: f64,
    /// Overall gate pass rate for this frequency.
    pub pass_rate: f64,
    /// Total cost / gate passes — true cost of one success.
    pub cost_per_pass: f64,
}

/// Composite C-Factor snapshot for a single `roko plan run` session.
///
/// This aggregates efficiency telemetry across all agents participating in the
/// run, grouped by plan. It is the fleet-level counterpart to the episode-based
/// C-Factor in [`crate::cfactor`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetCFactor {
    /// 0.0-1.0 composite score for the session.
    pub overall: f64,
    /// Component breakdown for the score.
    pub components: FleetCFactorComponents,
    /// Number of distinct plans represented in the session.
    pub plan_count: usize,
    /// Number of distinct agents represented in the session.
    pub agent_count: usize,
    /// Number of efficiency events contributing to the snapshot.
    pub observation_count: usize,
}

/// Individual fleet C-Factor components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetCFactorComponents {
    /// Fraction of plans that used more than one agent.
    pub multi_agent_coverage: f64,
    /// Fraction of plan groups that passed at least one gate.
    pub pass_rate: f64,
    /// Inverse of cost per successful plan, normalized against the run's baseline.
    pub cost_efficiency: f64,
    /// Inverse of duration per successful plan, normalized against the run's baseline.
    pub speed: f64,
    /// Evenness of agent participation inside plan groups, normalized to `[0..1]`.
    pub turn_taking_equality: f64,
}

impl Default for FleetCFactorComponents {
    fn default() -> Self {
        Self {
            multi_agent_coverage: 0.0,
            pass_rate: 0.0,
            cost_efficiency: 0.0,
            speed: 0.0,
            turn_taking_equality: 0.0,
        }
    }
}

impl Default for FleetCFactor {
    fn default() -> Self {
        Self {
            overall: 0.0,
            components: FleetCFactorComponents::default(),
            plan_count: 0,
            agent_count: 0,
            observation_count: 0,
        }
    }
}

/// Compute a [`RoleCostProfile`] for each distinct role in the given events.
#[allow(clippy::cast_precision_loss)]
pub fn compute_role_profiles(events: &[AgentEfficiencyEvent]) -> Vec<RoleCostProfile> {
    let mut groups: HashMap<String, Vec<&AgentEfficiencyEvent>> = HashMap::new();
    for e in events {
        groups.entry(e.role.clone()).or_default().push(e);
    }

    let mut profiles: Vec<RoleCostProfile> = groups
        .into_iter()
        .map(|(role, evts)| {
            let n = evts.len() as f64;
            let n_u64 = evts.len() as u64;

            let avg_input = evts.iter().map(|e| e.input_tokens as f64).sum::<f64>() / n;
            let avg_output = evts.iter().map(|e| e.output_tokens as f64).sum::<f64>() / n;
            let avg_cache = evts.iter().map(|e| e.cache_hit_rate()).sum::<f64>() / n;
            let avg_cost = evts.iter().map(|e| e.cost_usd).sum::<f64>() / n;
            let avg_wall = evts.iter().map(|e| e.wall_time_ms as f64).sum::<f64>() / n;
            let avg_tool = evts.iter().map(|e| e.tool_utilization()).sum::<f64>() / n;

            let warm_count = evts.iter().filter(|e| e.was_warm_start).count();
            let warm_pct = warm_count as f64 / n;

            let pass_count = evts.iter().filter(|e| e.gate_passed).count();
            let pass_rate = pass_count as f64 / n;

            let total_cost: f64 = evts.iter().map(|e| e.cost_usd).sum();
            let cost_per_pass = if pass_count > 0 {
                total_cost / pass_count as f64
            } else {
                0.0
            };

            // P95 cost: sort costs and take the 95th percentile.
            let mut costs: Vec<f64> = evts.iter().map(|e| e.cost_usd).collect();
            costs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // P95 index: 95% of the way through the sorted cost list.
            let p95_idx = (costs.len() * 95 / 100).min(costs.len().saturating_sub(1));
            let p95_cost = costs.get(p95_idx).copied().unwrap_or(0.0);

            RoleCostProfile {
                role,
                observations: n_u64,
                avg_input_tokens: avg_input,
                avg_output_tokens: avg_output,
                avg_cache_hit_rate: avg_cache,
                avg_cost_usd: avg_cost,
                p95_cost_usd: p95_cost,
                cost_per_pass,
                avg_tool_utilization: avg_tool,
                avg_wall_time_ms: avg_wall,
                warm_start_pct: warm_pct,
                pass_rate,
            }
        })
        .collect();

    profiles.sort_by(|a, b| a.role.cmp(&b.role));
    profiles
}

/// Compute a [`FrequencyCostProfile`] for each distinct operating frequency.
#[allow(clippy::cast_precision_loss)]
pub fn compute_frequency_profiles(events: &[AgentEfficiencyEvent]) -> Vec<FrequencyCostProfile> {
    let mut groups: HashMap<OperatingFrequency, Vec<&AgentEfficiencyEvent>> = HashMap::new();
    for e in events {
        groups.entry(e.frequency).or_default().push(e);
    }

    let mut profiles: Vec<FrequencyCostProfile> = groups
        .into_iter()
        .map(|(frequency, evts)| {
            let n = evts.len() as f64;
            let n_u64 = evts.len() as u64;
            let total_cost = evts.iter().map(|e| e.cost_usd).sum::<f64>();
            let avg_cost_usd = if n == 0.0 { 0.0 } else { total_cost / n };
            let pass_count = evts.iter().filter(|e| e.gate_passed).count();
            let pass_rate = if n == 0.0 { 0.0 } else { pass_count as f64 / n };
            let cost_per_pass = if pass_count > 0 {
                total_cost / pass_count as f64
            } else {
                0.0
            };

            FrequencyCostProfile {
                frequency,
                observations: n_u64,
                avg_cost_usd,
                total_cost_usd: total_cost,
                pass_rate,
                cost_per_pass,
            }
        })
        .collect();

    profiles.sort_by_key(|profile| profile.frequency);
    profiles
}

/// Compute a fleet-level C-Factor for a single plan-run session.
///
/// The snapshot groups efficiency events by `plan_id`, then scores the session
/// using pass rate, baseline-relative cost and speed, multi-agent coverage, and
/// turn-taking equality across agents.
#[allow(clippy::cast_precision_loss)]
pub fn compute_fleet_cfactor(events: &[AgentEfficiencyEvent]) -> FleetCFactor {
    if events.is_empty() {
        return FleetCFactor::default();
    }

    let mut groups: HashMap<String, FleetPlanAggregate> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut agent_ids: HashSet<String> = HashSet::new();

    for event in events {
        let plan_id = event.plan_id.trim();
        if plan_id.is_empty() {
            continue;
        }

        agent_ids.insert(event.agent_id.clone());
        let entry = groups.entry(plan_id.to_string()).or_insert_with(|| {
            order.push(plan_id.to_string());
            FleetPlanAggregate::default()
        });
        entry.observe(event);
    }

    let plans: Vec<(String, FleetPlanAggregate)> = order
        .into_iter()
        .filter_map(|plan_id| groups.remove_entry(&plan_id))
        .collect();
    if plans.is_empty() {
        return FleetCFactor::default();
    }

    let plan_count = plans.len();
    let multi_agent_plan_count = plans
        .iter()
        .filter(|(_, plan)| plan.distinct_agents.len() > 1)
        .count();
    let passed_plan_count = plans.iter().filter(|(_, plan)| plan.passed_gate).count();
    let successful_plans: Vec<&FleetPlanAggregate> = plans
        .iter()
        .filter_map(|(_, plan)| plan.passed_gate.then_some(plan))
        .collect();

    let pass_rate = passed_plan_count as f64 / plan_count as f64;
    let multi_agent_coverage = multi_agent_plan_count as f64 / plan_count as f64;
    let turn_taking_equality = {
        let mut total = 0.0;
        let mut counted = 0.0;
        for (_, plan) in &plans {
            if plan.distinct_agents.len() < 2 {
                continue;
            }
            let equality =
                turn_taking_equality_for_counts(plan.agent_turn_counts.values().copied().collect());
            total += equality;
            counted += 1.0;
        }
        if counted == 0.0 {
            0.0
        } else {
            (total / counted).clamp(0.0, 1.0)
        }
    };

    let (avg_cost_per_successful_plan, avg_duration_per_successful_plan) =
        if successful_plans.is_empty() {
            (0.0, 0.0)
        } else {
            let count = successful_plans.len() as f64;
            let total_cost: f64 = successful_plans.iter().map(|plan| plan.cost_usd).sum();
            let total_duration: f64 = successful_plans.iter().map(|plan| plan.duration_ms).sum();
            (total_cost / count, total_duration / count)
        };

    let baseline_plan_count = plans.len().min(BASELINE_PLAN_COUNT);
    let (baseline_cost, baseline_duration) = if baseline_plan_count == 0 {
        (0.0, 0.0)
    } else {
        let baseline_plans: Vec<&(String, FleetPlanAggregate)> =
            plans.iter().take(baseline_plan_count).collect();
        let total_cost: f64 = baseline_plans.iter().map(|(_, plan)| plan.cost_usd).sum();
        let total_duration: f64 = baseline_plans
            .iter()
            .map(|(_, plan)| plan.duration_ms)
            .sum();
        (
            total_cost / baseline_plan_count as f64,
            total_duration / baseline_plan_count as f64,
        )
    };

    let cost_efficiency = if baseline_cost > 0.0 && avg_cost_per_successful_plan > 0.0 {
        baseline_cost / avg_cost_per_successful_plan
    } else {
        0.0
    };

    let speed = if baseline_duration > 0.0 && avg_duration_per_successful_plan > 0.0 {
        baseline_duration / avg_duration_per_successful_plan
    } else {
        0.0
    };

    let overall = (pass_rate * 0.30
        + cost_efficiency * 0.20
        + speed * 0.15
        + multi_agent_coverage * 0.15
        + turn_taking_equality * 0.20)
        .clamp(0.0, 1.0);

    FleetCFactor {
        overall,
        components: FleetCFactorComponents {
            multi_agent_coverage,
            pass_rate,
            cost_efficiency,
            speed,
            turn_taking_equality,
        },
        plan_count,
        agent_count: agent_ids.len(),
        observation_count: events.len(),
    }
}

#[derive(Debug, Default)]
struct FleetPlanAggregate {
    cost_usd: f64,
    duration_ms: f64,
    passed_gate: bool,
    distinct_agents: HashSet<String>,
    agent_turn_counts: HashMap<String, u64>,
}

impl FleetPlanAggregate {
    fn observe(&mut self, event: &AgentEfficiencyEvent) {
        self.cost_usd += event.cost_usd;
        self.duration_ms += event.wall_time_ms as f64;
        self.passed_gate |= event.gate_passed;
        self.distinct_agents.insert(event.agent_id.clone());
        *self
            .agent_turn_counts
            .entry(event.agent_id.clone())
            .or_default() += 1;
    }
}

fn turn_taking_equality_for_counts(counts: Vec<u64>) -> f64 {
    if counts.len() < 2 {
        return 0.0;
    }

    let gini = gini_coefficient(&counts);
    (1.0 - gini).clamp(0.0, 1.0)
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

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Create a minimal test fixture [`AgentEfficiencyEvent`].
#[cfg(test)]
fn make_test_event(
    role: &str,
    cost: f64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read: u64,
    wall_time_ms: u64,
    tools_available: u32,
    tools_used: u32,
    warm: bool,
    passed: bool,
) -> AgentEfficiencyEvent {
    AgentEfficiencyEvent {
        agent_id: "agent-1".into(),
        role: role.into(),
        backend: "claude".into(),
        model: "claude-sonnet-4-5".into(),
        plan_id: "plan-1".into(),
        task_id: "t1".into(),
        attempt_id: "test-attempt".into(),
        input_tokens,
        output_tokens,
        reasoning_tokens: 0,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        cost_usd: cost,
        cost_usd_without_cache: cost * 1.5,
        prompt_sections: Vec::new(),
        total_prompt_tokens: input_tokens,
        system_prompt_tokens: 200,
        tools_available,
        tools_used,
        tool_calls: Vec::new(),
        wall_time_ms,
        duration_ms: wall_time_ms,
        time_to_first_token_ms: 500,
        was_warm_start: warm,
        iteration: 1,
        gate_passed: passed,
        outcome: if passed {
            "success".into()
        } else {
            "failure".into()
        },
        gate_errors: if passed {
            Vec::new()
        } else {
            vec!["test gate failed".into()]
        },
        model_used: "claude-sonnet-4-5".into(),
        frequency: OperatingFrequency::Theta,
        strategy_attempted: if passed {
            "none".into()
        } else {
            "retry_same".into()
        },
        timestamp: "2026-04-06T12:00:00Z".into(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Grade tests ─────────────────────────────────────────────────

    #[test]
    fn efficiency_grade_a_high_signal_low_budget_passed() {
        let s = PromptEfficiencyScore::new(1.0, 0.2, 0.9, true);
        assert_eq!(s.grade(), Grade::A);
    }

    #[test]
    fn efficiency_grade_b_moderate() {
        let s = PromptEfficiencyScore::new(0.6, 0.5, 0.5, true);
        assert_eq!(s.grade(), Grade::B);
    }

    #[test]
    fn efficiency_grade_c_low_signal() {
        // 0.4*0.4 + (1-0.6)*0.2 + 0.3*0.2 + 0.0*0.2 = 0.16 + 0.08 + 0.06 = 0.30
        let s = PromptEfficiencyScore::new(0.4, 0.6, 0.3, false);
        assert_eq!(s.grade(), Grade::C);
    }

    #[test]
    fn efficiency_grade_d_worst_case() {
        let s = PromptEfficiencyScore::new(0.0, 1.0, 0.0, false);
        assert_eq!(s.grade(), Grade::D);
    }

    #[test]
    fn efficiency_composite_score_range() {
        // Best case: 1.0*0.4 + (1-0)*0.2 + 1.0*0.2 + 1.0*0.2 = 1.0
        let best = PromptEfficiencyScore::new(1.0, 0.0, 1.0, true);
        assert!((best.composite() - 1.0).abs() < 1e-9);

        // Worst case: 0.0*0.4 + (1-1)*0.2 + 0.0*0.2 + 0.0*0.2 = 0.0
        let worst = PromptEfficiencyScore::new(0.0, 1.0, 0.0, false);
        assert!((worst.composite()).abs() < 1e-9);
    }

    #[test]
    fn efficiency_grade_numeric_values() {
        assert_eq!(Grade::A.numeric(), 4);
        assert_eq!(Grade::B.numeric(), 3);
        assert_eq!(Grade::C.numeric(), 2);
        assert_eq!(Grade::D.numeric(), 1);
    }

    #[test]
    fn efficiency_grade_ordering() {
        assert!(Grade::A < Grade::B);
        assert!(Grade::B < Grade::C);
        assert!(Grade::C < Grade::D);
    }

    #[test]
    fn efficiency_grade_display() {
        assert_eq!(Grade::A.to_string(), "A");
        assert_eq!(Grade::D.to_string(), "D");
    }

    // ── AgentEfficiencyEvent tests ──────────────────────────────────

    #[test]
    fn efficiency_event_cache_hit_rate() {
        let e = make_test_event("Impl", 0.50, 1000, 200, 300, 5000, 10, 5, false, true);
        assert!((e.cache_hit_rate() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn efficiency_event_cache_hit_rate_zero_input() {
        let e = make_test_event("Impl", 0.0, 0, 0, 0, 0, 0, 0, false, false);
        assert!((e.cache_hit_rate()).abs() < 1e-9);
    }

    #[test]
    fn efficiency_event_tool_utilization() {
        let e = make_test_event("Impl", 0.50, 1000, 200, 0, 5000, 10, 4, false, true);
        assert!((e.tool_utilization() - 0.4).abs() < 1e-9);
    }

    #[test]
    fn efficiency_event_tool_utilization_zero_available() {
        let e = make_test_event("Impl", 0.50, 1000, 200, 0, 5000, 0, 0, false, true);
        assert!((e.tool_utilization()).abs() < 1e-9);
    }

    #[test]
    fn efficiency_event_cache_savings() {
        let e = make_test_event("Impl", 0.50, 1000, 200, 300, 5000, 10, 5, false, true);
        // cost_usd_without_cache = 0.50 * 1.5 = 0.75
        assert!((e.cache_savings_usd() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn efficiency_event_total_tokens() {
        let e = make_test_event("Impl", 0.50, 1000, 200, 0, 5000, 10, 5, false, true);
        assert_eq!(e.total_tokens(), 1200);
    }

    #[test]
    fn efficiency_event_serialization_roundtrip() {
        let e = make_test_event("Implementer", 0.42, 1500, 300, 200, 45000, 8, 3, true, true);
        let json = serde_json::to_string(&e).expect("serialize");
        let e2: AgentEfficiencyEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(e, e2);
    }

    #[test]
    fn efficiency_event_serializes_resolved_model() {
        let event = AgentEfficiencyEvent {
            model_used: "claude-sonnet-4-6".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_value(&event).expect("serialize event");
        assert_eq!(json["resolved_model"], "claude-sonnet-4-6");
        assert!(json.get("model_used").is_none());

        let roundtrip: AgentEfficiencyEvent =
            serde_json::from_value(json).expect("deserialize event");
        assert_eq!(roundtrip.model_used, "claude-sonnet-4-6");
    }

    #[test]
    fn efficiency_reasoning_tokens() {
        let mut e = make_test_event("Implementer", 0.42, 1500, 300, 200, 45000, 8, 3, true, true);
        e.reasoning_tokens = 120;

        let json = serde_json::to_string(&e).expect("serialize");
        assert!(json.contains("\"reasoning_tokens\":120"));

        let e2: AgentEfficiencyEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(e2.reasoning_tokens, 120);
        assert_eq!(e, e2);
    }

    // ── RoleCostProfile tests ───────────────────────────────────────

    #[test]
    fn efficiency_role_profile_single_role() {
        let events = vec![
            make_test_event(
                "Implementer",
                0.50,
                1000,
                200,
                300,
                10000,
                10,
                5,
                true,
                true,
            ),
            make_test_event("Implementer", 0.30, 800, 150, 200, 8000, 10, 3, false, true),
            make_test_event(
                "Implementer",
                0.70,
                1200,
                250,
                400,
                12000,
                10,
                7,
                true,
                false,
            ),
        ];

        let profiles = compute_role_profiles(&events);
        assert_eq!(profiles.len(), 1);

        let p = &profiles[0];
        assert_eq!(p.role, "Implementer");
        assert_eq!(p.observations, 3);
        assert!((p.avg_cost_usd - 0.5).abs() < 1e-9);
        assert!((p.pass_rate - 2.0 / 3.0).abs() < 1e-9);
        assert!((p.warm_start_pct - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn efficiency_role_profile_multiple_roles() {
        let events = vec![
            make_test_event("Implementer", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Reviewer", 0.20, 500, 100, 0, 5000, 5, 2, false, true),
        ];

        let profiles = compute_role_profiles(&events);
        assert_eq!(profiles.len(), 2);
        // Sorted by role name
        assert_eq!(profiles[0].role, "Implementer");
        assert_eq!(profiles[1].role, "Reviewer");
    }

    #[test]
    fn efficiency_role_profile_cost_per_pass() {
        // 3 events: 2 pass, total cost = 1.50 → cost_per_pass = 0.75
        let events = vec![
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, false),
        ];

        let profiles = compute_role_profiles(&events);
        assert!((profiles[0].cost_per_pass - 0.75).abs() < 1e-9);
    }

    #[test]
    fn efficiency_role_profile_no_passes() {
        let events = vec![make_test_event(
            "Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, false,
        )];

        let profiles = compute_role_profiles(&events);
        assert!((profiles[0].cost_per_pass).abs() < 1e-9);
    }

    #[test]
    fn efficiency_role_profile_cost_per_successful_task() {
        let events = vec![
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, true),
            make_test_event("Impl", 0.50, 1000, 200, 0, 10000, 10, 5, false, false),
        ];

        let profiles = compute_role_profiles(&events);
        assert_eq!(profiles.len(), 1);

        let p = &profiles[0];
        assert!((p.avg_cost_usd - 0.50).abs() < 1e-9);
        assert!((p.pass_rate - 0.80).abs() < 1e-9);
        assert!((p.cost_per_successful_task() - 0.625).abs() < 1e-9);
    }

    #[test]
    fn efficiency_frequency_profile_groups_by_frequency() {
        let mut gamma = make_test_event("Impl", 1.0, 100, 20, 0, 1000, 5, 2, false, false);
        gamma.frequency = OperatingFrequency::Gamma;
        let mut delta = make_test_event("Reviewer", 3.0, 150, 40, 0, 1500, 5, 3, false, true);
        delta.frequency = OperatingFrequency::Delta;

        let profiles = compute_frequency_profiles(&[gamma, delta]);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].frequency, OperatingFrequency::Gamma);
        assert_eq!(profiles[0].observations, 1);
        assert_eq!(profiles[0].pass_rate, 0.0);
        assert_eq!(profiles[1].frequency, OperatingFrequency::Delta);
        assert_eq!(profiles[1].observations, 1);
        assert_eq!(profiles[1].pass_rate, 1.0);
    }

    #[test]
    fn efficiency_fleet_cfactor_groups_by_plan_and_agents() {
        let mut a1 = make_test_event("Implementer", 0.40, 900, 100, 0, 4000, 8, 4, false, true);
        a1.plan_id = "plan-a".into();
        a1.agent_id = "agent-a".into();
        let mut a2 = make_test_event("Reviewer", 0.20, 600, 80, 0, 3000, 8, 2, false, true);
        a2.plan_id = "plan-a".into();
        a2.agent_id = "agent-b".into();
        let mut b1 = make_test_event("Implementer", 0.10, 400, 50, 0, 1500, 8, 2, false, false);
        b1.plan_id = "plan-b".into();
        b1.agent_id = "agent-c".into();

        let fleet = compute_fleet_cfactor(&[a1, a2, b1]);
        assert_eq!(fleet.plan_count, 2);
        assert_eq!(fleet.agent_count, 3);
        assert_eq!(fleet.observation_count, 3);
        assert!(fleet.components.multi_agent_coverage > 0.0);
        assert!(fleet.components.turn_taking_equality > 0.0);
        assert!(fleet.overall >= 0.0);
    }

    #[test]
    fn efficiency_prompt_section_meta_serialization() {
        let s = PromptSectionMeta {
            name: "prd2".into(),
            tokens: 500,
            priority: 1,
            was_truncated: false,
            was_dropped: false,
        };
        let json = serde_json::to_string(&s).expect("serialize");
        let s2: PromptSectionMeta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, s2);
    }

    #[test]
    fn efficiency_tool_call_meta_serialization() {
        let t = ToolCallMeta {
            tool_name: "Read".into(),
            duration_ms: 150,
            result_tokens: 800,
            succeeded: true,
            advanced_task: true,
            was_redundant: false,
            error_category: None,
        };
        let json = serde_json::to_string(&t).expect("serialize");
        let t2: ToolCallMeta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(t, t2);
    }

    #[test]
    fn tool_call_reward_roundtrip_preserves_reward_indicators() {
        let meta = ToolCallMeta {
            tool_name: "Bash".into(),
            duration_ms: 875,
            result_tokens: 120,
            succeeded: false,
            advanced_task: false,
            was_redundant: true,
            error_category: Some("timeout".into()),
        };

        let json = serde_json::to_string(&meta).expect("serialize");
        let restored: ToolCallMeta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, meta);
    }

    #[test]
    fn tool_call_reward_defaults_for_legacy_payloads() {
        let json = r#"{
            "tool_name":"Read",
            "duration_ms":150,
            "result_tokens":800,
            "succeeded":true
        }"#;

        let restored: ToolCallMeta = serde_json::from_str(json).expect("deserialize");
        assert_eq!(restored.tool_name, "Read");
        assert!(restored.succeeded);
        assert!(!restored.advanced_task);
        assert!(!restored.was_redundant);
        assert_eq!(restored.error_category, None);
    }

    #[test]
    fn efficiency_score_clamping() {
        // Values outside [0,1] should be clamped
        let s = PromptEfficiencyScore::new(1.5, -0.5, 2.0, true);
        assert!((s.signal_ratio - 1.0).abs() < 1e-9);
        assert!((s.budget_utilization).abs() < 1e-9);
        assert!((s.cache_efficiency - 1.0).abs() < 1e-9);
    }

    #[test]
    fn efficiency_profile_p95_cost() {
        // 20 events with increasing cost: 0.01, 0.02, ..., 0.20
        let events: Vec<AgentEfficiencyEvent> = (1..=20)
            .map(|i| {
                let cost = i as f64 * 0.01;
                make_test_event("Impl", cost, 1000, 200, 0, 10000, 10, 5, false, true)
            })
            .collect();

        let profiles = compute_role_profiles(&events);
        assert_eq!(profiles.len(), 1);
        // P95 index for 20 elements: 20 * 95 / 100 = 19 → costs[19] = 0.20
        assert!((profiles[0].p95_cost_usd - 0.20).abs() < 1e-9);
    }
}
