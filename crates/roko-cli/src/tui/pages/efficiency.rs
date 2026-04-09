//! Efficiency dashboard scaffold pages (§18).

use super::{PageId, PageScaffold, WidgetScaffold};

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
            "Gate Results",
            "Gate pass rates, adaptive thresholds, and recent failures.",
            vec![
                WidgetScaffold::new(
                    "gate_summary",
                    "Gate Summary",
                    "Gate name, runs, pass rate, average duration, and last run.",
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
