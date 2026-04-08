//! Placeholder page and widget models for the TUI scaffold.

use std::fmt::Write as _;

pub mod efficiency;
pub mod operations;

/// Stable identifiers for scaffolded dashboard pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageId {
    /// Efficiency health gauges.
    Health,
    /// Efficiency trends.
    Trends,
    /// Efficiency correlations.
    Correlations,
    /// Efficiency parameters.
    Parameters,
    /// Efficiency experiments.
    Experiments,
    /// Efficiency optimizer loop.
    Optimizer,
    /// Live per-agent status.
    AgentStatus,
    /// Plan DAG/progress view.
    PlanView,
    /// Log filtering/search view.
    LogView,
    /// Effective config view.
    ConfigView,
}

impl PageId {
    /// Human-readable label for this page.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Health => "Health",
            Self::Trends => "Trends",
            Self::Correlations => "Correlations",
            Self::Parameters => "Parameters",
            Self::Experiments => "Experiments",
            Self::Optimizer => "Optimizer",
            Self::AgentStatus => "Agent Status",
            Self::PlanView => "Plan View",
            Self::LogView => "Log View",
            Self::ConfigView => "Config View",
        }
    }

    /// Stable machine-friendly page slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Health => "health",
            Self::Trends => "trends",
            Self::Correlations => "correlations",
            Self::Parameters => "parameters",
            Self::Experiments => "experiments",
            Self::Optimizer => "optimizer",
            Self::AgentStatus => "agent-status",
            Self::PlanView => "plan-view",
            Self::LogView => "log-view",
            Self::ConfigView => "config-view",
        }
    }

    /// Stable high-level grouping used by CLI renderers.
    #[must_use]
    pub const fn group(self) -> &'static str {
        match self {
            Self::Health
            | Self::Trends
            | Self::Correlations
            | Self::Parameters
            | Self::Experiments
            | Self::Optimizer => "efficiency",
            Self::AgentStatus | Self::PlanView | Self::LogView | Self::ConfigView => "operations",
        }
    }
}

/// Placeholder widget metadata for a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetScaffold {
    /// Stable widget ID.
    pub id: &'static str,
    /// Widget title.
    pub title: &'static str,
    /// Short intended behavior description.
    pub purpose: &'static str,
}

impl WidgetScaffold {
    /// Build a new widget scaffold.
    #[must_use]
    pub const fn new(id: &'static str, title: &'static str, purpose: &'static str) -> Self {
        Self { id, title, purpose }
    }

    /// One-line, command-printable widget summary.
    #[must_use]
    pub fn render_line(&self) -> String {
        format!("- {} [{}]: {}", self.title, self.id, self.purpose)
    }
}

/// Placeholder page metadata and widget list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageScaffold {
    /// Stable page ID.
    pub id: PageId,
    /// Page title.
    pub title: &'static str,
    /// Scope and intent for later implementation.
    pub intent: &'static str,
    /// Placeholder widget layout.
    pub widgets: Vec<WidgetScaffold>,
}

impl PageScaffold {
    /// Build a page scaffold with widgets.
    #[must_use]
    pub const fn new(
        id: PageId,
        title: &'static str,
        intent: &'static str,
        widgets: Vec<WidgetScaffold>,
    ) -> Self {
        Self {
            id,
            title,
            intent,
            widgets,
        }
    }

    /// Number of widgets on this page.
    #[must_use]
    pub fn widget_count(&self) -> usize {
        self.widgets.len()
    }

    /// Compact comma-separated widget-title summary.
    #[must_use]
    pub fn widget_title_summary(&self, limit: usize) -> String {
        if self.widgets.is_empty() {
            return String::from("no widgets");
        }

        let mut titles = self
            .widgets
            .iter()
            .take(limit)
            .map(|widget| widget.title)
            .collect::<Vec<_>>();
        if self.widgets.len() > limit {
            titles.push("...");
        }
        titles.join(", ")
    }

    /// Render a one-line page summary for page indexes and selectors.
    #[must_use]
    pub fn render_summary_line(&self, active: bool) -> String {
        let marker = if active { "*" } else { " " };
        format!(
            "{marker} {} [{}] {} | {} widgets | focus: {}",
            self.title,
            self.id.slug(),
            self.id.group(),
            self.widget_count(),
            self.widget_title_summary(3)
        )
    }

    /// Render the widget list only, suitable for a targeted page selection.
    #[must_use]
    pub fn render_widget_list(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{} [{}]", self.title, self.id.slug());
        let _ = writeln!(out, "group: {}", self.id.group());
        let _ = writeln!(out, "intent: {}", self.intent);
        let _ = writeln!(out, "widgets ({}):", self.widget_count());
        for widget in &self.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        out
    }

    /// Render this page as plain text for command output.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", self.title, self.id.slug(),);
        let _ = writeln!(out, "group: {}", self.id.group());
        let _ = writeln!(out, "intent: {}", self.intent);
        let _ = writeln!(out, "focus: {}", self.widget_title_summary(3));
        let _ = writeln!(out, "widgets ({}):", self.widget_count());
        for widget in &self.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_page() -> PageScaffold {
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
        )
    }

    #[test]
    fn page_group_matches_expected_section() {
        assert_eq!(PageId::Health.group(), "efficiency");
        assert_eq!(PageId::PlanView.group(), "operations");
    }

    #[test]
    fn summary_line_includes_slug_group_and_focus() {
        let line = sample_page().render_summary_line(true);
        assert!(line.contains("* Health [health] efficiency"));
        assert!(line.contains("3 widgets"));
        assert!(line.contains("Pass Rate, Cost / Task, Prompt Size"));
    }

    #[test]
    fn widget_list_renders_count_and_purpose_lines() {
        let rendered = sample_page().render_widget_list();
        assert!(rendered.contains("widgets (3):"));
        assert!(rendered.contains("- Pass Rate [pass_rate]: Rolling gate pass rate."));
    }

    #[test]
    fn full_page_render_includes_group_and_focus() {
        let rendered = sample_page().render_text();
        assert!(rendered.contains("group: efficiency"));
        assert!(rendered.contains("focus: Pass Rate, Cost / Task, Prompt Size"));
    }
}
