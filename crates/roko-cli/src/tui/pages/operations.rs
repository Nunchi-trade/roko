//! Operational TUI scaffold pages (§19).

use super::{PageId, PageScaffold, WidgetScaffold};

/// Non-efficiency operational pages as placeholder scaffolds.
#[must_use]
pub fn scaffold_pages() -> Vec<PageScaffold> {
    vec![
        PageScaffold::new(
            PageId::AgentStatus,
            "Agent Status",
            "Live per-agent runtime status and cost counters.",
            vec![
                WidgetScaffold::new(
                    "agent_table",
                    "Agent Table",
                    "Role, backend, state, tokens, and cost.",
                ),
                WidgetScaffold::new(
                    "agent_timeline",
                    "Agent Timeline",
                    "Recent transitions for each active agent.",
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
    ]
}
