//! Tab content views for the Mori-style TUI.
//!
//! Each view corresponds to one top-level tab (F1-F9) and renders
//! the full content area for that tab. Views accept the shared
//! dashboard data, theme, and per-view state parameters.
//!
//! ## Region navigation (UI-04)
//!
//! Each top-level tab maps to a **region**. Within a region, number keys
//! `1`-`9` select a **sub-view**. The [`SubView`] enum describes the
//! screens available in each region. The `view_state.sub_tab` field
//! selects the active sub-view (0-indexed).
//!
//! | Tab (F-key) | Region | Sub-views |
//! |-------------|--------|-----------|
//! | F1 Dashboard | Overview | Health, Mesh Status, Cost |
//! | F2 Plans | Plan Detail | DAG View, Task Detail, Wave Progress |
//! | F3 Agents | Agent Detail | Output Stream, Gate Results, Token Burn |
//! | F4 Git | Git Detail | Branch Tree, Commit Graph, Worktrees |
//! | F5 Logs | Logs | Filtered Log, Signal Stream |
//! | F6 Config | System | Config View, Provider Health, Model Comparison |
//! | F7 Inspect | Knowledge | Engram DAG, Episode Replay, Knowledge Browse |
//! | F8 Marketplace | Jobs | Job List, Job Detail, Create Job |
//! | F9 Atelier | Workshop | PRD Workshop, Plan Explorer |

pub mod agents_view;
pub mod atelier_view;
pub mod config_view;
pub mod context_view;
pub mod dashboard_view;
pub mod git_view;
pub mod logs_view;
pub mod marketplace_view;
pub mod plans_view;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::dashboard::{DashboardData, Theme};
use super::state::TuiState;
use super::tabs::Tab;

/// Sub-view identifiers within a tab region (UI-04).
///
/// Each variant maps to a specific screen that can be selected within
/// the parent tab. Use `SubView::for_tab()` to get the available
/// sub-views for a given tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubView {
    // ── Region 1: Dashboard (F1) ──
    /// Health gauges and status overview.
    DashboardHealth,
    /// Agent mesh / collective status.
    MeshStatus,
    /// Cost and budget overview.
    CostOverview,

    // ── Region 2: Plans (F2) ──
    /// Plan DAG visualization.
    PlanDagView,
    /// Task detail for selected task.
    TaskDetail,
    /// Wave progress overview.
    WaveProgress,

    // ── Region 3: Agents (F3) ──
    /// Live output stream from selected agent.
    AgentOutputStream,
    /// Gate results for the selected agent.
    AgentGateResults,
    /// Token burn / cost metrics per agent.
    AgentTokenBurn,

    // ── Region 4: Git (F4) ──
    /// Branch tree / branch list.
    BranchTree,
    /// Commit graph.
    CommitGraph,
    /// Worktree list.
    WorktreeList,

    // ── Region 5: Logs (F5) ──
    /// Filtered log viewer (default).
    FilteredLog,
    /// Signal stream viewer.
    SignalStream,

    // ���─ Region 6: Config / System (F6) ──
    /// Effective configuration view.
    ConfigEditor,
    /// Provider health monitoring.
    ProviderHealth,
    /// Model comparison metrics.
    ModelComparison,

    // ── Region 7: Inspect / Knowledge (F7) ──
    /// Engram DAG inspector.
    EngramDag,
    /// Episode replay viewer.
    EpisodeReplay,
    /// Knowledge browser (Neuro store).
    KnowledgeBrowse,

    // ── Region 8: Marketplace (F8) ──
    /// Job list browser.
    JobList,
    /// Job detail view.
    JobDetail,
    /// Job creation form.
    CreateJob,

    // ── Region 9: Atelier (F9) ──
    /// PRD workshop.
    PrdWorkshop,
    /// Plan explorer.
    PlanExplorer,
}

impl SubView {
    /// Return the sub-views available for a given tab.
    #[must_use]
    pub const fn for_tab(tab: Tab) -> &'static [SubView] {
        match tab {
            Tab::Dashboard => &[
                SubView::DashboardHealth,
                SubView::MeshStatus,
                SubView::CostOverview,
            ],
            Tab::Plans => &[
                SubView::PlanDagView,
                SubView::TaskDetail,
                SubView::WaveProgress,
            ],
            Tab::Agents => &[
                SubView::AgentOutputStream,
                SubView::AgentGateResults,
                SubView::AgentTokenBurn,
            ],
            Tab::Git => &[
                SubView::BranchTree,
                SubView::CommitGraph,
                SubView::WorktreeList,
            ],
            Tab::Logs => &[SubView::FilteredLog, SubView::SignalStream],
            Tab::Config => &[
                SubView::ConfigEditor,
                SubView::ProviderHealth,
                SubView::ModelComparison,
            ],
            Tab::Inspect => &[
                SubView::EngramDag,
                SubView::EpisodeReplay,
                SubView::KnowledgeBrowse,
            ],
            Tab::Marketplace => &[SubView::JobList, SubView::JobDetail, SubView::CreateJob],
            Tab::Atelier => &[SubView::PrdWorkshop, SubView::PlanExplorer],
        }
    }

    /// Human-readable label for this sub-view.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DashboardHealth => "Health",
            Self::MeshStatus => "Mesh",
            Self::CostOverview => "Cost",
            Self::PlanDagView => "DAG",
            Self::TaskDetail => "Task",
            Self::WaveProgress => "Waves",
            Self::AgentOutputStream => "Output",
            Self::AgentGateResults => "Gates",
            Self::AgentTokenBurn => "Tokens",
            Self::BranchTree => "Branches",
            Self::CommitGraph => "Commits",
            Self::WorktreeList => "Worktrees",
            Self::FilteredLog => "Log",
            Self::SignalStream => "Signals",
            Self::ConfigEditor => "Config",
            Self::ProviderHealth => "Providers",
            Self::ModelComparison => "Models",
            Self::EngramDag => "Engrams",
            Self::EpisodeReplay => "Episodes",
            Self::KnowledgeBrowse => "Knowledge",
            Self::JobList => "Jobs",
            Self::JobDetail => "Detail",
            Self::CreateJob => "New Job",
            Self::PrdWorkshop => "PRDs",
            Self::PlanExplorer => "Plans",
        }
    }

    /// Short hint key (1-based) for display in the sub-view bar.
    #[must_use]
    pub fn hint_for_tab(tab: Tab, index: usize) -> Option<Self> {
        let views = Self::for_tab(tab);
        views.get(index).copied()
    }

    /// Build a label string for the sub-view bar: "1:Health 2:Mesh 3:Cost".
    #[must_use]
    pub fn bar_label(tab: Tab, active_index: usize) -> String {
        let views = Self::for_tab(tab);
        views
            .iter()
            .enumerate()
            .map(|(i, sv)| {
                if i == active_index {
                    format!("[{}:{}]", i + 1, sv.label())
                } else {
                    format!(" {}:{} ", i + 1, sv.label())
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

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
    /// Active sub-tab index (for views with internal tabs / region sub-views).
    ///
    /// Maps to `SubView::for_tab(tab)[sub_tab]`. Number keys 1-9 set this.
    pub sub_tab: usize,
    /// Secondary selection (e.g. wave index in plans view).
    pub secondary_selected: usize,
    /// Whether auto-scroll / tail mode is active.
    pub auto_tail: bool,
    /// Free-text query used by searchable sub-views.
    pub search_query: String,
}

impl ViewState {
    /// Resolve the active [`SubView`] for the given tab.
    #[must_use]
    pub fn active_sub_view(&self, tab: Tab) -> SubView {
        let views = SubView::for_tab(tab);
        views.get(self.sub_tab).copied().unwrap_or(views[0])
    }
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
        Tab::Marketplace => {
            marketplace_view::render(frame, area, data, tui_state, view_state, theme);
        }
        Tab::Atelier => atelier_view::render(frame, area, data, tui_state, view_state, theme),
    }
}
