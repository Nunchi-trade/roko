//! Efficiency dashboard scaffold pages (§18).

use std::collections::{BTreeMap, BTreeSet};

use roko_learn::efficiency::AgentEfficiencyEvent;

use super::{PageId, PageScaffold, WidgetScaffold};
use crate::tui::dashboard::DashboardData;

/// Derived efficiency summary used by the live TUI widgets.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EfficiencySnapshot {
    /// Total recorded efficiency events.
    pub event_count: usize,
    /// Distinct task attempts represented in the efficiency log.
    pub task_count: usize,
    /// Total input + output tokens.
    pub total_tokens: u64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total spend in USD.
    pub total_cost_usd: f64,
    /// Average tokens per task.
    pub average_tokens_per_task: f64,
    /// Average spend per task.
    pub average_cost_per_task: f64,
    /// Success ratio from the persisted efficiency summary.
    pub success_rate: f64,
    /// Tokens emitted per model tier.
    pub tier_counts: BTreeMap<&'static str, u64>,
    /// Token usage history for sparkline rendering.
    pub token_series: Vec<u64>,
}

/// Build a live efficiency snapshot from dashboard data.
#[must_use]
pub fn build_efficiency_snapshot(data: &DashboardData) -> EfficiencySnapshot {
    let event_count = data.efficiency_events.len();
    let total_input_tokens = data.efficiency.total_input_tokens;
    let total_output_tokens = data.efficiency.total_output_tokens;
    let total_tokens = total_input_tokens + total_output_tokens;
    let total_cost_usd = data.efficiency.total_cost_usd;

    let task_count = data
        .efficiency_events
        .iter()
        .map(task_key)
        .collect::<BTreeSet<_>>()
        .len();
    let average_tokens_per_task = if task_count == 0 {
        0.0
    } else {
        total_tokens as f64 / task_count as f64
    };
    let average_cost_per_task = if task_count == 0 {
        0.0
    } else {
        total_cost_usd / task_count as f64
    };
    let success_rate = if event_count == 0 {
        0.0
    } else {
        data.efficiency.passed_count as f64 / event_count as f64
    };

    let mut tier_counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    for tier in ["T0", "T1", "T2"] {
        tier_counts.insert(tier, 0);
    }
    for event in &data.efficiency_events {
        *tier_counts
            .entry(model_tier_label(&event.model))
            .or_default() += 1;
    }

    let token_series = data
        .efficiency_events
        .iter()
        .map(|event| event.input_tokens + event.output_tokens)
        .collect();

    EfficiencySnapshot {
        event_count,
        task_count,
        total_tokens,
        total_input_tokens,
        total_output_tokens,
        total_cost_usd,
        average_tokens_per_task,
        average_cost_per_task,
        success_rate,
        tier_counts,
        token_series,
    }
}

fn task_key(event: &AgentEfficiencyEvent) -> (String, String) {
    (event.plan_id.clone(), event.task_id.clone())
}

fn model_tier_label(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        "T0"
    } else if lower.contains("opus") {
        "T2"
    } else {
        "T1"
    }
}

/// All six efficiency pages from plan 09 as placeholder scaffolds.
#[must_use]
pub fn scaffold_pages() -> Vec<PageScaffold> {
    vec![
        PageScaffold::new(
            PageId::Health,
            "Health",
            "Top-line efficiency gauges for current runs.",
            vec![
                WidgetScaffold::new("pass_rate", "Pass Rate", "Rolling gate pass rate."),
                WidgetScaffold::new("cost_per_task", "Cost / Task", "Average spend per task."),
                WidgetScaffold::new(
                    "prompt_size",
                    "Prompt Size",
                    "Median prompt token footprint.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Trends,
            "Trends",
            "Time-series drift and learning-velocity trends.",
            vec![
                WidgetScaffold::new(
                    "latency_sparkline",
                    "Latency Sparkline",
                    "Turn latency over time.",
                ),
                WidgetScaffold::new(
                    "learning_velocity",
                    "Learning Velocity",
                    "New useful rules/skills over time.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Correlations,
            "Correlations",
            "Relationship views between interventions and outcomes.",
            vec![
                WidgetScaffold::new(
                    "context_vs_pass",
                    "Context vs Pass Rate",
                    "Impact of context packing on gates.",
                ),
                WidgetScaffold::new(
                    "strategy_vs_cost",
                    "Strategy vs Cost",
                    "Cost profile by strategy selection.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::GateResults,
            "Verify Results",
            "Verify pass rates, adaptive thresholds, and recent failures.",
            vec![
                WidgetScaffold::new(
                    "gate_summary",
                    "Verify Summary",
                    "Verify name, runs, pass rate, average duration, and last run.",
                ),
                WidgetScaffold::new(
                    "adaptive_thresholds",
                    "Adaptive Thresholds",
                    "Current rung thresholds, EMA values, and trend arrows.",
                ),
                WidgetScaffold::new(
                    "recent_failures",
                    "Recent Failures",
                    "Last ten gate failures with task ID, gate name, and excerpt.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Learning,
            "Learning",
            "Learning subsystem interactions, update counts, and feedback loop status.",
            vec![
                WidgetScaffold::new(
                    "learning_system_status",
                    "Learning System Status",
                    "Stage transitions, subsystem freshness, and missing feedback loops.",
                ),
                WidgetScaffold::new(
                    "active_experiments",
                    "Active Experiments",
                    "Experiment names, variants, samples, winners, and significance.",
                ),
                WidgetScaffold::new(
                    "efficiency_trends",
                    "Efficiency Trends",
                    "7-day sparklines for cost, tokens, success, and first-try rate.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Parameters,
            "Parameters",
            "Runtime tunables and predicted impact metadata.",
            vec![
                WidgetScaffold::new(
                    "knobs",
                    "Knobs",
                    "Editable runtime and learning parameters.",
                ),
                WidgetScaffold::new(
                    "impact_scores",
                    "Impact Scores",
                    "Estimated sensitivity per knob.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Experiments,
            "Experiments",
            "A/B experiment outcomes and statistical summaries.",
            vec![
                WidgetScaffold::new("ab_runs", "A/B Runs", "Recent active/finished experiments."),
                WidgetScaffold::new(
                    "significance",
                    "Significance",
                    "Simple significance verdicts per experiment.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Optimizer,
            "Optimizer",
            "Closed-loop optimization state and confidence bars.",
            vec![
                WidgetScaffold::new(
                    "loop_state",
                    "Loop State",
                    "Current optimization cycle stage.",
                ),
                WidgetScaffold::new(
                    "confidence_bars",
                    "Confidence Bars",
                    "Confidence by optimization candidate.",
                ),
            ],
        ),
    ]
}
