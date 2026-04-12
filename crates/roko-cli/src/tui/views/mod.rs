//! Tab content views for the Mori-style TUI.
//!
//! Each view corresponds to one top-level tab (F1-F7) and renders
//! the full content area for that tab. Views accept the shared
//! dashboard data, theme, and per-view state parameters.

pub mod agents_view;
pub mod config_view;
pub mod context_view;
pub mod dashboard_view;
pub mod git_view;
pub mod logs_view;
pub mod plans_view;

use ratatui::layout::Rect;
use ratatui::Frame;

use super::dashboard::{DashboardData, Theme};
use super::state::TuiState;
use super::tabs::Tab;

/// Per-view scroll and selection state.
///
/// This is a minimal state struct used by views until the full `TuiState`
/// is wired in by the integration layer. Each field is optional so callers
/// can provide only what the active view needs.
#[derive(Debug, Clone, Default)]
pub struct ViewState {
    /// Vertical scroll offset for scrollable panels.
    pub scroll: u16,
    /// Selected row index in list/table views.
    pub selected: usize,
    /// Active sub-tab index (for views with internal tabs).
    pub sub_tab: usize,
    /// Secondary selection (e.g. wave index in plans view).
    pub secondary_selected: usize,
    /// Whether auto-scroll / tail mode is active.
    pub auto_tail: bool,
}

/// Dispatch rendering to the appropriate view based on the active tab.
pub fn render_tab_content(
    frame: &mut Frame<'_>,
    area: Rect,
    tab: Tab,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    match tab {
        Tab::Dashboard => dashboard_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Plans => plans_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Agents => agents_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Git => git_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Logs => logs_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Config => config_view::render(frame, area, data, tui_state, view_state, theme),
        Tab::Inspect => context_view::render(frame, area, data, tui_state, view_state, theme),
    }
}
