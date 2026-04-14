//! Operational TUI scaffold pages (§19).

use super::{PageId, PageScaffold, WidgetScaffold};

/// Non-efficiency operational pages as placeholder scaffolds.
#[must_use]
pub fn scaffold_pages() -> Vec<PageScaffold> {
    vec![
        PageScaffold::new(
            PageId::AgentStatus,
            "Agent Activity",
            "Live active-agent roster, model mix, and session cost breakdown.",
            vec![
                WidgetScaffold::new(
                    "agent_table",
                    "Active Agents",
                    "Agent ID, model, task, role, turns, tokens, cost, and uptime.",
                ),
                WidgetScaffold::new(
                    "model_distribution",
                    "Model Distribution",
                    "Horizontal haiku, sonnet, and opus usage counts.",
                ),
                WidgetScaffold::new(
                    "cost_breakdown",
                    "Cost Breakdown",
                    "Per-model token costs plus total session spend.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::PlanView,
            "Plan View",
            "Plan DAG status and per-task progress markers.",
            vec![
                WidgetScaffold::new("dag", "DAG", "Task graph and dependency states."),
                WidgetScaffold::new(
                    "task_detail",
                    "Task Detail",
                    "Selected task metadata and last action.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::LogView,
            "Log View",
            "Filterable event stream and failure drill-down.",
            vec![
                WidgetScaffold::new(
                    "log_stream",
                    "Log Stream",
                    "Structured events in chronological order.",
                ),
                WidgetScaffold::new(
                    "log_filters",
                    "Filters",
                    "Role/gate/error filters and text search.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::Signals,
            "Signals",
            "Recent signals, kind distribution, and parent-chain explorer.",
            vec![
                WidgetScaffold::new(
                    "recent_signals",
                    "Recent Signals",
                    "Timestamp, kind, plan/task ID, and payload preview.",
                ),
                WidgetScaffold::new(
                    "kind_distribution",
                    "Kind Distribution",
                    "Bar chart of signal kind prefixes over the last 100 signals.",
                ),
                WidgetScaffold::new(
                    "signal_tree",
                    "Engram DAG Explorer",
                    "Indented parent-hash chain for the selected signal.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::ConfigView,
            "Config View",
            "Effective config with override/source annotations.",
            vec![
                WidgetScaffold::new(
                    "effective_config",
                    "Effective Config",
                    "Resolved runtime config values.",
                ),
                WidgetScaffold::new(
                    "source_tags",
                    "Source Tags",
                    "Default/global/project/CLI override origin.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::ProviderHealth,
            "Provider Health",
            "Per-provider circuit breaker state, latency, and error rates.",
            vec![
                WidgetScaffold::new(
                    "provider_table",
                    "Provider Status",
                    "Provider name, state (open/half-open/closed), p50/p99 latency, error rate.",
                ),
                WidgetScaffold::new(
                    "request_summary",
                    "Request Summary",
                    "Total requests, failures, and circuit breaker trips.",
                ),
            ],
        ),
        PageScaffold::new(
            PageId::ModelComparison,
            "Model Comparison",
            "Side-by-side model performance, cost, and quality metrics.",
            vec![
                WidgetScaffold::new(
                    "model_table",
                    "Model Metrics",
                    "Model ID, provider, cost/M tokens, latency, gate pass rate.",
                ),
                WidgetScaffold::new(
                    "cost_comparison",
                    "Cost Comparison",
                    "Relative cost per task across models.",
                ),
            ],
        ),
    ]
}
