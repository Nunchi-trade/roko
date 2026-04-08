//! Dashboard scaffold container for future TUI wiring.
//!
//! This module does not render anything. It only provides stable data
//! containers that command routing can consume later.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use super::pages::{PageId, PageScaffold, efficiency, operations};

/// In-memory scaffold of all placeholder dashboard pages.
#[derive(Debug, Clone)]
pub struct DashboardScaffold {
    pages: BTreeMap<PageId, PageScaffold>,
    active_page: PageId,
}

impl DashboardScaffold {
    /// Build the full scaffold with all placeholder pages.
    #[must_use]
    pub fn new() -> Self {
        let mut pages = BTreeMap::new();
        for page in efficiency::scaffold_pages()
            .into_iter()
            .chain(operations::scaffold_pages())
        {
            pages.insert(page.id, page);
        }
        Self {
            pages,
            active_page: PageId::Health,
        }
    }

    /// List all pages in stable order.
    #[must_use]
    pub fn pages(&self) -> Vec<&PageScaffold> {
        self.pages.values().collect()
    }

    /// Current active page.
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.active_page
    }

    /// Set active page if it exists in the scaffold.
    pub fn set_active_page(&mut self, page: PageId) -> bool {
        if self.pages.contains_key(&page) {
            self.active_page = page;
            true
        } else {
            false
        }
    }

    /// Return a specific page by ID.
    #[must_use]
    pub fn page(&self, page: PageId) -> Option<&PageScaffold> {
        self.pages.get(&page)
    }

    /// Build a high-level summary used by future command wiring.
    #[must_use]
    pub fn summary(&self) -> DashboardSummary {
        let widget_count = self.pages.values().map(|p| p.widgets.len()).sum();
        DashboardSummary {
            active_page: self.active_page,
            page_count: self.pages.len(),
            widget_count,
        }
    }

    /// Render a plain-text dashboard summary suitable for CLI output.
    #[must_use]
    pub fn render_overview_text(&self) -> String {
        let mut out = self.summary().to_string();
        out.push_str("\npages:\n");
        for page in self.pages.values() {
            let marker = if page.id == self.active_page {
                "*"
            } else {
                " "
            };
            let _ = writeln!(
                out,
                "{marker} {} ({}) [{} widgets]",
                page.title,
                page.id.slug(),
                page.widgets.len()
            );
        }
        out
    }

    /// Render one page as plain text. Returns `None` if the page does not exist.
    #[must_use]
    pub fn render_page_text(&self, page: PageId) -> Option<String> {
        self.page(page).map(PageScaffold::render_text)
    }

    /// Render the current active page as plain text.
    #[must_use]
    pub fn render_active_page_text(&self) -> String {
        self.page(self.active_page).map_or_else(
            || String::from("<missing active page>"),
            PageScaffold::render_text,
        )
    }
}

impl Default for DashboardScaffold {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary metadata for the dashboard scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardSummary {
    /// Currently selected page.
    pub active_page: PageId,
    /// Number of pages scaffolded.
    pub page_count: usize,
    /// Number of widgets scaffolded across all pages.
    pub widget_count: usize,
}

impl fmt::Display for DashboardSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dashboard scaffold: {} pages, {} widgets, active={}",
            self.page_count,
            self.widget_count,
            self.active_page.slug()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_has_expected_page_count() {
        let dashboard = DashboardScaffold::new();
        let summary = dashboard.summary();
        assert_eq!(summary.page_count, 10);
        assert!(summary.widget_count >= 20);
        assert_eq!(summary.active_page, PageId::Health);
    }

    #[test]
    fn can_switch_active_page() {
        let mut dashboard = DashboardScaffold::new();
        assert!(dashboard.set_active_page(PageId::PlanView));
        assert_eq!(dashboard.active_page(), PageId::PlanView);
    }

    #[test]
    fn overview_render_contains_active_page_and_counts() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard.render_overview_text();
        assert!(rendered.contains("dashboard scaffold: 10 pages"));
        assert!(rendered.contains("active=health"));
        assert!(rendered.contains("* Health (health)"));
    }

    #[test]
    fn page_render_includes_widgets() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard
            .render_page_text(PageId::PlanView)
            .expect("plan page should exist");
        assert!(rendered.contains("Plan View (plan-view)"));
        assert!(rendered.contains("widgets:"));
        assert!(rendered.contains("DAG [dag]"));
    }
}
