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

    /// Render this page as plain text for command output.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", self.title, self.id.slug(),);
        let _ = writeln!(out, "{}", self.intent);
        out.push_str("widgets:\n");
        for widget in &self.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        out
    }
}
