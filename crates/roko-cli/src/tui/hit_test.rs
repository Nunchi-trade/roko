//! Mouse coordinate routing for focus zones.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Named focus zones in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusZone {
    /// The plan tree / left sidebar.
    PlanTree,
    /// Task progress panel.
    TaskProgress,
    /// Agent output panel.
    AgentOutput,
    /// Command/tool output panel.
    CommandOutput,
    /// Right-side content area.
    RightContent,
    /// A header tab at the given index.
    HeaderTab(usize),
    /// A detail sub-tab at the given index.
    DetailTab(usize),
}

/// Cached screen regions for hit testing.
#[derive(Debug, Clone, Default)]
pub struct HitZones {
    pub plan_tree: Rect,
    pub task_progress: Rect,
    pub agent_output: Rect,
    pub command_output: Rect,
    pub right_content: Rect,
    pub detail_tab_rects: Vec<(Rect, usize)>,
    pub header_tab_rects: Vec<(Rect, usize)>,
}

impl HitZones {
    /// Replay layout math to compute hit zones for the current terminal size and tab.
    ///
    /// `tab` is the active tab index (0 = Dashboard, 1 = Plans, 2 = Agents, etc.).
    /// `header_tab_count` is how many top-level tabs exist.
    #[must_use]
    pub fn compute(area: Rect, tab: usize, header_tab_count: usize) -> Self {
        let mut zones = Self::default();

        // Top-level layout: header (3 rows) | body | footer (2 rows)
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(2),
            ])
            .split(area);

        let header_area = outer[0];
        let body_area = outer[1];

        // Compute header tab hit rects (evenly spaced across the header).
        if header_tab_count > 0 {
            let tab_width = header_area.width / header_tab_count as u16;
            for i in 0..header_tab_count {
                let x = header_area.x + (i as u16) * tab_width;
                let w = if i == header_tab_count - 1 {
                    header_area.width - (i as u16) * tab_width
                } else {
                    tab_width
                };
                zones
                    .header_tab_rects
                    .push((Rect::new(x, header_area.y, w, header_area.height), i));
            }
        }

        // Per-tab body layout
        match tab {
            0 => {
                // Dashboard: single content area
                zones.right_content = body_area;
            }
            1 => {
                // Plans: left sidebar (30%) | right content (70%)
                let h = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .split(body_area);
                zones.plan_tree = h[0];
                zones.right_content = h[1];

                // Right side: task progress (top 40%) | agent output (bottom 60%)
                let v = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                    .split(h[1]);
                zones.task_progress = v[0];
                zones.agent_output = v[1];
            }
            2 => {
                // Agents: left agent list (25%) | right output (75%)
                let h = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
                    .split(body_area);
                zones.plan_tree = h[0]; // reuse as agent list
                let v = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(h[1]);
                zones.agent_output = v[0];
                zones.command_output = v[1];
            }
            _ => {
                // Generic: full body is right_content
                zones.right_content = body_area;
            }
        }

        zones
    }

    /// Return the focus zone at the given screen coordinate, if any.
    #[must_use]
    pub fn zone_at(&self, x: u16, y: u16) -> Option<FocusZone> {
        // Check header tabs first (highest priority for clicks).
        for &(rect, idx) in &self.header_tab_rects {
            if contains(rect, x, y) {
                return Some(FocusZone::HeaderTab(idx));
            }
        }

        // Check detail tabs.
        for &(rect, idx) in &self.detail_tab_rects {
            if contains(rect, x, y) {
                return Some(FocusZone::DetailTab(idx));
            }
        }

        // Check body zones (order: most specific first).
        if contains(self.plan_tree, x, y) && !is_empty(self.plan_tree) {
            return Some(FocusZone::PlanTree);
        }
        if contains(self.task_progress, x, y) && !is_empty(self.task_progress) {
            return Some(FocusZone::TaskProgress);
        }
        if contains(self.agent_output, x, y) && !is_empty(self.agent_output) {
            return Some(FocusZone::AgentOutput);
        }
        if contains(self.command_output, x, y) && !is_empty(self.command_output) {
            return Some(FocusZone::CommandOutput);
        }
        if contains(self.right_content, x, y) && !is_empty(self.right_content) {
            return Some(FocusZone::RightContent);
        }

        None
    }
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

fn is_empty(rect: Rect) -> bool {
    rect.width == 0 || rect.height == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_tab_has_right_content() {
        let area = Rect::new(0, 0, 120, 50);
        let zones = HitZones::compute(area, 0, 5);
        assert!(zones.right_content.width > 0);
        assert!(zones.right_content.height > 0);
    }

    #[test]
    fn plans_tab_has_sidebar() {
        let area = Rect::new(0, 0, 120, 50);
        let zones = HitZones::compute(area, 1, 5);
        assert!(zones.plan_tree.width > 0);
        assert!(zones.task_progress.width > 0);
    }

    #[test]
    fn header_tabs_are_clickable() {
        let area = Rect::new(0, 0, 120, 50);
        let zones = HitZones::compute(area, 0, 5);
        assert_eq!(zones.header_tab_rects.len(), 5);
        // Click in first tab region
        let (first_rect, _) = zones.header_tab_rects[0];
        let zone = zones.zone_at(first_rect.x + 1, first_rect.y + 1);
        assert_eq!(zone, Some(FocusZone::HeaderTab(0)));
    }

    #[test]
    fn zone_at_returns_none_outside() {
        let area = Rect::new(0, 0, 80, 24);
        let zones = HitZones::compute(area, 0, 3);
        // Way outside
        assert_eq!(zones.zone_at(200, 200), None);
    }
}
