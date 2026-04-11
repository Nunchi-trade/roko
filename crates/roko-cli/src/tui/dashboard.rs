//! Dashboard scaffold container for future TUI wiring.
//!
//! This module keeps the existing page scaffold intact, but layers a
//! best-effort learning snapshot on top so the health and trends pages
//! can render real stats when the memory JSONL files are present.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::{BufRead as _, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan::{PlanSummary, plans_dir};
use crate::task_parser::{TaskDef, TasksFile};
use roko_core::OperatingFrequency;
use roko_core::metric::{Headlines, TaskMetric, compute_headlines};
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_gate::adaptive_threshold::AdaptiveThresholds;
pub use roko_learn::cfactor::{CFactor, CFactorComponents};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::prompt_experiment::{
    ExperimentStatus, ExperimentStore, PromptExperiment, PromptVariant, VariantStats,
};

use super::pages::{PageId, PageScaffold, efficiency, operations};

const MEMORY_DIR: &str = ".roko/memory";
const EPISODES_FILE: &str = "episodes.jsonl";
const TASK_METRICS_FILE: &str = "task-metrics.jsonl";

const LEARN_DIR: &str = ".roko/learn";
const EFFICIENCY_FILE: &str = "efficiency.jsonl";
const EXPERIMENTS_FILE: &str = "experiments.json";
const GATE_THRESHOLDS_FILE: &str = "gate-thresholds.json";
const CASCADE_ROUTER_FILE: &str = "cascade-router.json";
const DAIMON_DIR: &str = "daimon";
const AFFECT_FILE: &str = "affect.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn from_path(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct JsonlState {
    stamp: FileStamp,
    offset: u64,
}

/// Color palette for the dashboard TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Primary foreground text color.
    pub foreground: Color,
    /// Secondary or muted text color.
    pub muted: Color,
    /// Default background color.
    pub background: Color,
    /// Primary accent color.
    pub accent: Color,
    /// Accent foreground color for contrast.
    pub accent_foreground: Color,
    /// Success or completed state color.
    pub success: Color,
    /// Warning or gating state color.
    pub warning: Color,
    /// Error or failed state color.
    pub danger: Color,
    /// Informational or active state color.
    pub info: Color,
    /// Selection background color.
    pub selection_background: Color,
    /// Selection foreground color.
    pub selection_foreground: Color,
}

impl Theme {
    /// Build the default dark palette that matches a typical terminal theme.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            foreground: Color::White,
            muted: Color::DarkGray,
            background: Color::Black,
            accent: Color::Cyan,
            accent_foreground: Color::Black,
            success: Color::Green,
            warning: Color::Yellow,
            danger: Color::Red,
            info: Color::Blue,
            selection_background: Color::Cyan,
            selection_foreground: Color::Black,
        }
    }

    /// Build an uncolored palette for `NO_COLOR` environments.
    #[must_use]
    pub const fn no_color() -> Self {
        Self {
            foreground: Color::Reset,
            muted: Color::Reset,
            background: Color::Reset,
            accent: Color::Reset,
            accent_foreground: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            danger: Color::Reset,
            info: Color::Reset,
            selection_background: Color::Reset,
            selection_foreground: Color::Reset,
        }
    }

    /// Build the active palette from the current environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::from_no_color(std::env::var_os("NO_COLOR").is_some())
    }

    /// Build the active palette from an explicit `NO_COLOR` flag.
    #[must_use]
    pub const fn from_no_color(no_color: bool) -> Self {
        if no_color {
            Self::no_color()
        } else {
            Self::dark()
        }
    }

    /// A plain foreground style.
    #[must_use]
    pub fn text(self) -> Style {
        Style::default().fg(self.foreground)
    }

    /// A muted foreground style.
    #[must_use]
    pub fn muted(self) -> Style {
        Style::default().fg(self.muted)
    }

    /// An accent style used for titles and highlights.
    #[must_use]
    pub fn accent(self) -> Style {
        Style::default().fg(self.accent)
    }

    /// A bold accent style for selected content.
    #[must_use]
    pub fn accent_bold(self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    /// A selected-item style with readable contrast.
    #[must_use]
    pub fn selection(self) -> Style {
        Style::default()
            .fg(self.selection_foreground)
            .bg(self.selection_background)
            .add_modifier(Modifier::BOLD)
    }

    /// A success style for completed or healthy states.
    #[must_use]
    pub fn success(self) -> Style {
        Style::default()
            .fg(self.success)
            .add_modifier(Modifier::BOLD)
    }

    /// A warning style for gating or degraded states.
    #[must_use]
    pub fn warning(self) -> Style {
        Style::default()
            .fg(self.warning)
            .add_modifier(Modifier::BOLD)
    }

    /// A danger style for failed or critical states.
    #[must_use]
    pub fn danger(self) -> Style {
        Style::default()
            .fg(self.danger)
            .add_modifier(Modifier::BOLD)
    }

    /// An informational style for active or in-flight states.
    #[must_use]
    pub fn info(self) -> Style {
        Style::default().fg(self.info).add_modifier(Modifier::BOLD)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_env()
    }
}

/// In-memory scaffold of all placeholder dashboard pages.
#[derive(Debug, Clone)]
pub struct DashboardScaffold {
    pages: BTreeMap<PageId, PageScaffold>,
    active_page: PageId,
    snapshot: DashboardSnapshot,
}

impl DashboardScaffold {
    /// Build the full scaffold with all placeholder pages.
    #[must_use]
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_in(root)
    }

    /// Build the scaffold and load snapshot data relative to `root`.
    #[must_use]
    pub fn new_in(root: impl AsRef<Path>) -> Self {
        let mut pages = BTreeMap::new();
        for page in efficiency::scaffold_pages()
            .into_iter()
            .chain(operations::scaffold_pages())
        {
            pages.insert(page.id, page);
        }

        let root = resolve_snapshot_root(root.as_ref());
        let snapshot = load_snapshot_best_effort(&root);

        Self {
            pages,
            active_page: PageId::Health,
            snapshot,
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
        out.push_str("\nactive page:\n");
        if let Some(page) = self.page(self.active_page) {
            let _ = writeln!(out, "{}", page.render_summary_line(true));
        }
        out.push_str("pages:\n");
        out.push_str(&self.render_page_index_text());
        out
    }

    /// Render the compact page index only.
    #[must_use]
    pub fn render_page_index_text(&self) -> String {
        let mut out = String::new();
        for page in self.pages.values() {
            let _ = writeln!(
                out,
                "{}",
                page.render_summary_line(page.id == self.active_page)
            );
        }
        out
    }

    /// Render one page as plain text. Returns `None` if the page does not exist.
    #[must_use]
    pub fn render_page_text(&self, page: PageId) -> Option<String> {
        let scaffold = self.page(page)?;
        let rendered = match page {
            PageId::Health => self.snapshot.render_health_page(scaffold),
            PageId::Trends => self.snapshot.render_trends_page(scaffold),
            PageId::Correlations => self.snapshot.render_correlations_page(scaffold),
            PageId::GateResults => self.snapshot.render_gate_results_page(scaffold),
            PageId::Learning => self.snapshot.render_learning_page(scaffold),
            PageId::Parameters => self.snapshot.render_parameters_page(scaffold),
            PageId::Experiments => self.snapshot.render_experiments_page(scaffold),
            PageId::Optimizer => self.snapshot.render_optimizer_page(scaffold),
            PageId::AgentStatus => self.snapshot.render_agent_status_page(scaffold),
            PageId::PlanView => self.snapshot.render_plan_view_page(scaffold),
            PageId::LogView => self.snapshot.render_log_view_page(scaffold),
            PageId::Signals => self.snapshot.render_signals_page(scaffold),
            PageId::ConfigView => self.snapshot.render_config_view_page(scaffold),
        };
        rendered.or_else(|| Some(scaffold.render_text()))
    }

    /// Render one page's widget list only. Returns `None` if the page does not exist.
    #[must_use]
    pub fn render_page_list_text(&self, page: PageId) -> Option<String> {
        self.page(page).map(PageScaffold::render_widget_list)
    }

    /// Render the current active page as plain text.
    #[must_use]
    pub fn render_active_page_text(&self) -> String {
        self.render_page_text(self.active_page)
            .unwrap_or_else(|| String::from("<missing active page>"))
    }

    /// Render the health page as plain text.
    #[must_use]
    pub fn render_health_page_text(&self) -> String {
        self.render_page_text(PageId::Health)
            .unwrap_or_else(|| String::from("<missing health page>"))
    }

    /// Render the trends page as plain text.
    #[must_use]
    pub fn render_trends_page_text(&self) -> String {
        self.render_page_text(PageId::Trends)
            .unwrap_or_else(|| String::from("<missing trends page>"))
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

/// Shared dashboard data loaded from `.roko/`.
#[derive(Debug, Clone, Default)]
pub struct DashboardData {
    /// Workspace root used for refreshes.
    root: PathBuf,
    /// Cached executor state from `.roko/state/executor.json`.
    executor_state: Value,
    /// Last observed state file metadata.
    executor_state_stamp: FileStamp,
    /// Plans from executor state.
    pub plans: Vec<PlanSummary>,
    /// Currently executing tasks.
    pub active_tasks: Vec<TaskSummary>,
    /// Agents tracked by the process supervisor.
    pub agents: Vec<AgentSummary>,
    /// Gate verdicts collected from signals.
    pub gate_results: Vec<GateResultSummary>,
    /// Efficiency aggregate from `.roko/learn/efficiency.jsonl`.
    pub efficiency: EfficiencySummary,
    /// Raw efficiency events from `.roko/learn/efficiency.jsonl`.
    pub efficiency_events: Vec<AgentEfficiencyEvent>,
    /// Last observed efficiency file metadata.
    efficiency_stamp: FileStamp,
    /// Cascade router state from `.roko/learn/cascade-router.json`.
    pub cascade_router: CascadeRouterState,
    /// Full experiment store from `.roko/learn/experiments.json`.
    pub experiment_store: ExperimentStore,
    /// Experiments from `.roko/learn/experiments.json`.
    pub experiments: Vec<ExperimentSummary>,
    /// Last observed experiments file metadata.
    experiments_stamp: FileStamp,
    /// Gate-results page data derived from signals and adaptive thresholds.
    pub gate_results_page: GateResultsPageData,
    /// Cached adaptive thresholds from `.roko/learn/gate-thresholds.json`.
    adaptive_thresholds: Option<AdaptiveThresholds>,
    /// Last observed gate-thresholds file metadata.
    gate_thresholds_stamp: FileStamp,
    /// Most recent signals from `.roko/signals.jsonl`.
    pub recent_signals: Vec<SignalSummary>,
    /// Cached signal-derived gate results when executor state does not provide them.
    signal_gate_results: Vec<GateResultSummary>,
    /// Parsed gate-related signals for the gate-results page.
    gate_signal_summaries: Vec<GateSignalSummary>,
    /// Last observed signals file metadata and offset.
    signals_state: JsonlState,
    /// Snapshot of the currently executing plan for the Plan Execution page.
    pub current_plan_execution: Option<PlanExecutionSnapshot>,
    /// Last observed episodes file metadata and offset.
    episodes_state: JsonlState,
    /// Cached episodes for plan execution rendering.
    episodes: Vec<Episode>,
    /// Conductor alerts filtered from signals.
    pub conductor_alerts: Vec<AlertSummary>,
    /// Full C-Factor history from `.roko/learn/c-factor.jsonl`.
    pub cfactor_history: Vec<CFactor>,
    /// Latest C-Factor snapshot, if present.
    pub cfactor: Option<CFactor>,
    /// Last observed C-Factor file metadata.
    cfactor_stamp: FileStamp,
    /// Cascade router file metadata.
    cascade_router_stamp: FileStamp,
    /// Cached Daimon affect state from `.roko/learn/daimon/affect.json`.
    affect_states: HashMap<String, DashboardAffectState>,
    /// Last observed affect file metadata.
    affect_stamp: FileStamp,
}

impl DashboardData {
    /// Load dashboard data from a workspace root, falling back to empty data on errors.
    #[must_use]
    pub fn load_best_effort(root: impl AsRef<Path>) -> Self {
        let root = resolve_snapshot_root(root.as_ref());
        let roko_dir = root.join(".roko");
        let learn_dir = roko_dir.join("learn");
        let state_path = roko_dir.join("state").join("executor.json");
        let signals_path = roko_dir.join("signals.jsonl");
        let episodes_path = roko_dir.join(MEMORY_DIR).join(EPISODES_FILE);
        let efficiency_path = learn_dir.join(EFFICIENCY_FILE);
        let experiments_path = learn_dir.join(EXPERIMENTS_FILE);
        let gate_thresholds_path = learn_dir.join(GATE_THRESHOLDS_FILE);
        let cascade_router_path = learn_dir.join(CASCADE_ROUTER_FILE);
        let affect_path = learn_dir.join(DAIMON_DIR).join(AFFECT_FILE);
        let cfactor_path = learn_dir.join("c-factor.jsonl");

        let state = read_json_value(&state_path).unwrap_or(Value::Null);
        let state_stamp = file_stamp(&state_path);
        let (recent_signals, gate_signal_summaries, signal_gate_results, signals_state) =
            load_signal_state(&signals_path);
        let episodes_state = load_episodes_state(&episodes_path);
        let episodes = load_episodes_from_path(&episodes_path);

        let plans = load_plan_summaries(&root, &state);
        let active_tasks = load_active_tasks(&state);
        let agents = load_agents(&state);
        let gate_results = load_gate_results(&state, &signal_gate_results);
        let efficiency_events = read_efficiency_events_sync(&efficiency_path);
        let efficiency = load_efficiency_summary(&efficiency_path);
        let cascade_router =
            load_json_opt::<CascadeRouterState>(&cascade_router_path).unwrap_or_default();
        let cascade_router_stamp = file_stamp(&cascade_router_path);
        let affect_states = load_json_opt::<DashboardAffectStore>(&affect_path)
            .map(|store| store.states)
            .unwrap_or_default();
        let affect_stamp = file_stamp(&affect_path);
        let experiment_store =
            load_json_opt::<ExperimentStore>(&experiments_path).unwrap_or_default();
        let experiments_stamp = file_stamp(&experiments_path);
        let mut experiments = experiment_store
            .iter()
            .map(ExperimentSummary::from_experiment)
            .collect::<Vec<_>>();
        experiments.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        let adaptive_thresholds = load_json_opt::<AdaptiveThresholds>(&gate_thresholds_path);
        let gate_thresholds_stamp = file_stamp(&gate_thresholds_path);
        let gate_results_page =
            build_gate_results_page_data(&gate_signal_summaries, adaptive_thresholds.as_ref());
        let conductor_alerts = recent_signals
            .iter()
            .filter(|signal| signal.kind.starts_with("conductor:alert:"))
            .map(AlertSummary::from_signal)
            .collect();
        let cfactor_history = load_cfactor_history(&cfactor_path);
        let cfactor = cfactor_history.last().cloned();
        let cfactor_stamp = file_stamp(&cfactor_path);
        let current_plan_execution = load_current_plan_execution(&root, &state, &episodes);
        let efficiency_stamp = file_stamp(&efficiency_path);

        Self {
            root,
            executor_state: state,
            executor_state_stamp: state_stamp,
            plans,
            active_tasks,
            agents,
            gate_results,
            efficiency,
            efficiency_events,
            efficiency_stamp,
            cascade_router,
            experiment_store,
            experiments,
            experiments_stamp,
            gate_results_page,
            adaptive_thresholds,
            gate_thresholds_stamp,
            recent_signals,
            signal_gate_results,
            gate_signal_summaries,
            signals_state,
            current_plan_execution,
            episodes_state,
            episodes,
            conductor_alerts,
            cfactor_history,
            cfactor,
            cfactor_stamp,
            cascade_router_stamp,
            affect_states,
            affect_stamp,
        }
    }

    /// Refresh the snapshot from the stored workspace root.
    pub async fn refresh(&mut self) -> Result<()> {
        let mut snapshot = std::mem::take(self);
        let refreshed = tokio::task::spawn_blocking(move || -> Result<Self> {
            snapshot.refresh_sync()?;
            Ok(snapshot)
        })
        .await??;
        *self = refreshed;
        Ok(())
    }

    fn refresh_sync(&mut self) -> Result<()> {
        let roko_dir = self.root.join(".roko");
        let state_path = roko_dir.join("state").join("executor.json");
        let signals_path = roko_dir.join("signals.jsonl");
        let episodes_path = roko_dir.join(MEMORY_DIR).join(EPISODES_FILE);
        let efficiency_path = roko_dir.join("learn").join(EFFICIENCY_FILE);
        let experiments_path = roko_dir.join("learn").join(EXPERIMENTS_FILE);
        let gate_thresholds_path = roko_dir.join("learn").join(GATE_THRESHOLDS_FILE);
        let cascade_router_path = roko_dir.join("learn").join(CASCADE_ROUTER_FILE);
        let affect_path = roko_dir.join("learn").join(DAIMON_DIR).join(AFFECT_FILE);
        let cfactor_path = roko_dir.join("learn").join("c-factor.jsonl");

        let mut state_changed = false;
        let stamp = file_stamp(&state_path);
        if stamp != self.executor_state_stamp {
            self.executor_state_stamp = stamp;
            self.executor_state = read_json_value(&state_path).unwrap_or(Value::Null);
            state_changed = true;
        }

        let stamp = file_stamp(&signals_path);
        if stamp != self.signals_state.stamp {
            self.refresh_signals(&signals_path, stamp);
        }

        let stamp = file_stamp(&episodes_path);
        if stamp != self.episodes_state.stamp {
            self.refresh_episodes(&episodes_path, stamp);
        }

        let stamp = file_stamp(&efficiency_path);
        if stamp != self.efficiency_stamp {
            self.efficiency_stamp = stamp;
            self.efficiency_events = read_efficiency_events_sync(&efficiency_path);
            self.efficiency = load_efficiency_summary(&efficiency_path);
        }

        let stamp = file_stamp(&experiments_path);
        if stamp != self.experiments_stamp {
            self.experiments_stamp = stamp;
            self.experiment_store =
                load_json_opt::<ExperimentStore>(&experiments_path).unwrap_or_default();
            self.experiments = self
                .experiment_store
                .iter()
                .map(ExperimentSummary::from_experiment)
                .collect::<Vec<_>>();
            self.experiments
                .sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        }

        let stamp = file_stamp(&gate_thresholds_path);
        if stamp != self.gate_thresholds_stamp {
            self.gate_thresholds_stamp = stamp;
            self.adaptive_thresholds = load_json_opt::<AdaptiveThresholds>(&gate_thresholds_path);
            self.rebuild_gate_results_page();
        }

        let stamp = file_stamp(&cascade_router_path);
        if stamp != self.cascade_router_stamp {
            self.cascade_router_stamp = stamp;
            self.cascade_router =
                load_json_opt::<CascadeRouterState>(&cascade_router_path).unwrap_or_default();
        }

        let stamp = file_stamp(&affect_path);
        if stamp != self.affect_stamp {
            self.affect_stamp = stamp;
            self.affect_states = load_json_opt::<DashboardAffectStore>(&affect_path)
                .map(|store| store.states)
                .unwrap_or_default();
        }

        let stamp = file_stamp(&cfactor_path);
        if stamp != self.cfactor_stamp {
            self.cfactor_stamp = stamp;
            self.cfactor_history = load_cfactor_history(&cfactor_path);
            self.cfactor = self.cfactor_history.last().cloned();
        }

        if state_changed {
            self.plans = load_plan_summaries(&self.root, &self.executor_state);
            self.active_tasks = load_active_tasks(&self.executor_state);
            self.agents = load_agents(&self.executor_state);
            self.gate_results = load_gate_results(&self.executor_state, &self.signal_gate_results);
            self.current_plan_execution =
                load_current_plan_execution(&self.root, &self.executor_state, &self.episodes);
        }

        Ok(())
    }

    fn refresh_signals(&mut self, signals_path: &Path, stamp: FileStamp) {
        let should_reset = stamp.len < self.signals_state.offset
            || (stamp.modified != self.signals_state.stamp.modified
                && stamp.len <= self.signals_state.offset);
        if should_reset {
            self.recent_signals.clear();
            self.signal_gate_results.clear();
            self.gate_signal_summaries.clear();
            self.signals_state.offset = 0;
        }

        let values = if should_reset {
            read_jsonl_values(signals_path)
        } else {
            read_jsonl_values_from_offset(signals_path, self.signals_state.offset)
        };

        for value in values {
            if let Some(signal) = SignalSummary::from_value(&value) {
                self.recent_signals.push(signal);
            }
            if let Some(gate_signal) = GateSignalSummary::from_value(&value) {
                self.gate_signal_summaries.push(gate_signal);
            }
            if let Some(gate_result) = signal_gate_result_from_value(&value) {
                self.signal_gate_results.push(gate_result);
            }
        }

        if self.recent_signals.len() > 100 {
            let keep_from = self.recent_signals.len() - 100;
            self.recent_signals.drain(0..keep_from);
        }

        self.signals_state = JsonlState {
            stamp,
            offset: stamp.len,
        };
        self.rebuild_signal_dependent_fields();
    }

    fn refresh_episodes(&mut self, episodes_path: &Path, stamp: FileStamp) {
        let should_reset = stamp.len < self.episodes_state.offset
            || (stamp.modified != self.episodes_state.stamp.modified
                && stamp.len <= self.episodes_state.offset);
        if should_reset {
            self.episodes.clear();
            self.episodes_state.offset = 0;
        }

        let episodes = if should_reset {
            load_episodes_from_path(episodes_path)
        } else {
            load_episodes_from_offset(episodes_path, self.episodes_state.offset)
        };

        if should_reset {
            self.episodes = episodes;
        } else {
            self.episodes.extend(episodes);
        }

        self.episodes_state = JsonlState {
            stamp,
            offset: stamp.len,
        };
        self.current_plan_execution =
            load_current_plan_execution(&self.root, &self.executor_state, &self.episodes);
    }

    fn rebuild_signal_dependent_fields(&mut self) {
        self.gate_results = load_gate_results(&self.executor_state, &self.signal_gate_results);
        self.rebuild_gate_results_page();
        self.conductor_alerts = self
            .recent_signals
            .iter()
            .filter(|signal| signal.kind.starts_with("conductor:alert:"))
            .map(AlertSummary::from_signal)
            .collect();
    }

    fn rebuild_gate_results_page(&mut self) {
        self.gate_results_page = build_gate_results_page_data(
            &self.gate_signal_summaries,
            self.adaptive_thresholds.as_ref(),
        );
    }

    /// Workspace root used to load dashboard artifacts.
    #[must_use]
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Return the affect emoji for a plan or agent key.
    #[must_use]
    pub(crate) fn affect_indicator(&self, key: &str) -> &'static str {
        self.affect_states
            .get(key)
            .map(DashboardAffectState::valence_indicator)
            .unwrap_or("😐")
    }

    /// Convert disk-loaded data into a `DashboardSnapshot` for the new view layer.
    ///
    /// This bridges the old file-polling data model to the new event-driven
    /// snapshot so views render correctly even without a live StateHub.
    #[must_use]
    pub fn to_core_snapshot(&self) -> roko_core::dashboard_snapshot::DashboardSnapshot {
        use roko_core::dashboard_snapshot::*;

        let mut snapshot = DashboardSnapshot::default();

        // Plans.
        for plan in &self.plans {
            let done = if plan.completed {
                plan.task_count
            } else {
                0
            };
            snapshot.plans.insert(
                plan.id.clone(),
                PlanState {
                    plan_id: plan.id.clone(),
                    phase: if plan.completed {
                        "completed".into()
                    } else {
                        "running".into()
                    },
                    tasks_total: plan.task_count,
                    tasks_done: done,
                    tasks_failed: 0,
                    active: !plan.completed,
                },
            );
            if plan.completed {
                snapshot.stats.plans_completed += 1;
            } else {
                snapshot.stats.plans_active += 1;
            }
        }

        // Active tasks.
        for task in &self.active_tasks {
            let key = format!("{}/{}", task.plan_id, task.task_id);
            snapshot.tasks.insert(
                key,
                TaskState {
                    task_id: task.task_id.clone(),
                    plan_id: task.plan_id.clone(),
                    phase: task.status.clone(),
                    outcome: None,
                },
            );
            snapshot.stats.tasks_active += 1;
        }

        // Agents.
        for agent in &self.agents {
            snapshot.agents.insert(
                agent.id.clone(),
                AgentState {
                    agent_id: agent.id.clone(),
                    role: agent.label.clone(),
                    active: agent.status == "running",
                    output_bytes: 0,
                },
            );
            if agent.status == "running" {
                snapshot.stats.agents_active += 1;
            }
        }

        // Gate results.
        for gate in &self.gate_results {
            if gate.passed {
                snapshot.stats.gates_passed += 1;
            } else {
                snapshot.stats.gates_failed += 1;
            }
            snapshot.gates.push(GateVerdict {
                plan_id: gate.plan_id.clone(),
                task_id: String::new(),
                gate: gate.gate_name.clone(),
                passed: gate.passed,
                ts_millis: 0,
            });
        }

        // Gate failure rows → errors.
        for row in &self.gate_results_page.failure_rows {
            snapshot.errors.push(ErrorEntry {
                message: format!("{}: {} ({})", row.gate_name, row.error_excerpt, row.task_id),
                ts_millis: row.created_at_ms as u64,
            });
        }

        snapshot
    }
}

/// Summary of a task that is currently active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSummary {
    pub plan_id: String,
    pub task_id: String,
    pub status: String,
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub assigned_agents: Vec<String>,
}

/// Summary of an agent tracked by the process supervisor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    pub status: String,
}

/// Aggregated agent-activity row used by the dashboard page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentActivityRow {
    pub agent_id: String,
    pub plan_id: Option<String>,
    pub model: String,
    pub task: String,
    pub role: String,
    pub turns: usize,
    pub tokens_used: u64,
    pub cost_usd: f64,
    pub uptime_ms: u64,
}

/// Model usage count for the bar chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelUsageRow {
    pub label: &'static str,
    pub count: u64,
}

/// Per-model cost row for the breakdown table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelCostRow {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub input_rate: f64,
    pub output_rate: f64,
    pub cost_usd: f64,
}

/// Aggregated agent activity snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AgentActivitySnapshot {
    pub active_agents: Vec<AgentActivityRow>,
    pub model_usage: Vec<ModelUsageRow>,
    pub cost_rows: Vec<ModelCostRow>,
    pub total_session_cost: f64,
}

/// Summary of one gate verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResultSummary {
    pub plan_id: String,
    pub gate_name: String,
    pub passed: bool,
    pub rung: u32,
    pub duration_ms: u64,
    pub summary: String,
}

/// Gate signal summary used to derive the gate-results page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSignalSummary {
    pub id: String,
    pub created_at_ms: i64,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub gate_name: String,
    pub passed: bool,
    pub duration_ms: u64,
    pub excerpt: String,
}

/// Shared gate-results dashboard data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GateResultsPageData {
    pub gate_rows: Vec<GateSummaryRow>,
    pub threshold_rows: Vec<GateThresholdRow>,
    pub failure_rows: Vec<GateFailureRow>,
}

/// Aggregate row for the gate summary table.
#[derive(Debug, Clone, PartialEq)]
pub struct GateSummaryRow {
    pub gate_name: String,
    pub total_runs: u64,
    pub pass_rate: f64,
    pub avg_duration_ms: f64,
    pub last_run: String,
}

/// Row for the adaptive threshold table.
#[derive(Debug, Clone, PartialEq)]
pub struct GateThresholdRow {
    pub rung: u32,
    pub current_threshold: u32,
    pub ema_pass_rate: f64,
    pub trend: GateTrend,
}

/// Recent failing gate row.
#[derive(Debug, Clone, PartialEq)]
pub struct GateFailureRow {
    pub created_at_ms: i64,
    pub task_id: String,
    pub gate_name: String,
    pub error_excerpt: String,
}

/// Derived EMA trend direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateTrend {
    Up,
    Flat,
    Down,
}

/// Snapshot of the currently executing plan.
#[derive(Debug, Clone, Default)]
pub struct PlanExecutionSnapshot {
    pub plan_id: String,
    pub plan_title: String,
    pub tasks_done: usize,
    pub tasks_total: usize,
    pub tasks: Vec<PlanExecutionTaskRow>,
    pub current_task: Option<PlanExecutionTaskDetail>,
    pub agent_output_tail: Vec<String>,
}

/// One row in the execution task table.
#[derive(Debug, Clone)]
pub struct PlanExecutionTaskRow {
    pub task_id: String,
    pub title: String,
    pub phase: String,
    pub frequency: OperatingFrequency,
    pub model: String,
    pub duration: String,
    pub is_current: bool,
}

/// Detail block for the current task.
#[derive(Debug, Clone)]
pub struct PlanExecutionTaskDetail {
    pub task_id: String,
    pub frequency: OperatingFrequency,
    pub description: String,
    pub read_files: Vec<ReadFileSnapshot>,
    pub write_files: Vec<String>,
}

/// Flattened read-file context for display.
#[derive(Debug, Clone)]
pub struct ReadFileSnapshot {
    pub path: String,
    pub lines: Option<String>,
    pub why: String,
}

/// Aggregate learning efficiency snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EfficiencySummary {
    pub event_count: usize,
    pub total_cost_usd: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub passed_count: usize,
    pub average_wall_time_ms: f64,
}

impl Default for EfficiencySummary {
    fn default() -> Self {
        Self {
            event_count: 0,
            total_cost_usd: 0.0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            passed_count: 0,
            average_wall_time_ms: 0.0,
        }
    }
}

/// Cascade router snapshot from `.roko/learn/cascade-router.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CascadeRouterState {
    #[serde(default)]
    pub model_slugs: Vec<String>,
    #[serde(default)]
    pub confidence_stats: HashMap<String, CascadeRouterModelStats>,
}

/// Persisted Daimon affect state snapshot.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct DashboardAffectStore {
    #[serde(default)]
    states: HashMap<String, DashboardAffectState>,
}

/// Persisted affect state for one plan or agent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct DashboardAffectState {
    pleasure: f64,
    arousal: f64,
    dominance: f64,
    confidence: f64,
    updated_at: DateTime<Utc>,
}

impl DashboardAffectState {
    fn valence_indicator(&self) -> &'static str {
        valence_indicator(self.pleasure)
    }
}

/// Per-model cascade-router stats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CascadeRouterModelStats {
    pub trials: u64,
    pub successes: u64,
}

/// Prompt experiment summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentSummary {
    pub experiment_id: String,
    pub section_name: String,
    pub status: String,
    #[serde(default)]
    pub winner_id: Option<String>,
    pub active_variants: usize,
    pub total_trials: u64,
}

/// Recent signal summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSummary {
    pub id: String,
    pub kind: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub parent_hash: Option<String>,
    #[serde(default)]
    pub lineage: Vec<String>,
    #[serde(default)]
    pub payload_preview: String,
}

/// Conductor alert summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertSummary {
    pub id: String,
    pub kind: String,
    pub created_at_ms: i64,
    pub severity: String,
    pub message: String,
}

impl AlertSummary {
    fn from_signal(signal: &SignalSummary) -> Self {
        let severity = signal
            .kind
            .split(':')
            .nth(2)
            .unwrap_or("warning")
            .to_string();
        Self {
            id: signal.id.clone(),
            kind: signal.kind.clone(),
            created_at_ms: signal.created_at_ms,
            severity,
            message: signal.kind.clone(),
        }
    }
}

/// Build the agent activity snapshot from active agents and efficiency events.
pub(crate) fn build_agent_activity_snapshot(
    active_agents: &[AgentSummary],
    efficiency_events: &[AgentEfficiencyEvent],
) -> Option<AgentActivitySnapshot> {
    let agents = if active_agents.is_empty() {
        synthesize_agents_from_events(efficiency_events)
    } else {
        active_agents.to_vec()
    };

    if agents.is_empty() && efficiency_events.is_empty() {
        return None;
    }

    let mut rows_by_agent: HashMap<String, AgentActivityAggregate> = HashMap::new();
    for agent in &agents {
        rows_by_agent
            .entry(agent.id.clone())
            .or_insert_with(AgentActivityAggregate::default);
    }

    for event in efficiency_events {
        let entry = rows_by_agent
            .entry(event.agent_id.clone())
            .or_insert_with(AgentActivityAggregate::default);
        entry.turns += 1;
        entry.tokens_used += event.input_tokens + event.output_tokens;
        entry.cost_usd += event.cost_usd;
        entry.update_from_event(event);
    }

    let now = Utc::now();
    let mut active_rows = agents
        .iter()
        .map(|agent| {
            let aggregate = rows_by_agent
                .entry(agent.id.clone())
                .or_insert_with(AgentActivityAggregate::default);
            aggregate.render_row(agent, now)
        })
        .collect::<Vec<_>>();
    active_rows.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

    let mut model_usage: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut cost_groups: BTreeMap<String, ModelCostAggregate> = BTreeMap::new();
    for event in efficiency_events {
        let (tier, input_rate, output_rate) = model_pricing(&event.model);
        *model_usage.entry(tier).or_default() += 1;
        let aggregate =
            cost_groups
                .entry(event.model.clone())
                .or_insert_with(|| ModelCostAggregate {
                    model: event.model.clone(),
                    input_rate,
                    output_rate,
                    ..ModelCostAggregate::default()
                });
        aggregate.input_tokens += event.input_tokens;
        aggregate.output_tokens += event.output_tokens;
    }

    let mut cost_rows = cost_groups
        .into_values()
        .map(|group| group.into_row())
        .collect::<Vec<_>>();
    cost_rows.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.model.cmp(&b.model))
    });

    let model_usage = ["haiku", "sonnet", "opus"]
        .into_iter()
        .map(|label| ModelUsageRow {
            label,
            count: model_usage.get(label).copied().unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let total_session_cost = cost_rows.iter().map(|row| row.cost_usd).sum();

    Some(AgentActivitySnapshot {
        active_agents: active_rows,
        model_usage,
        cost_rows,
        total_session_cost,
    })
}

fn synthesize_agents_from_events(efficiency_events: &[AgentEfficiencyEvent]) -> Vec<AgentSummary> {
    let mut agents = BTreeMap::<String, AgentSummary>::new();
    for event in efficiency_events {
        agents
            .entry(event.agent_id.clone())
            .or_insert_with(|| AgentSummary {
                id: event.agent_id.clone(),
                label: event.agent_id.clone(),
                plan_id: Some(event.plan_id.clone()),
                status: String::from("active"),
            });
    }
    agents.into_values().collect()
}

fn model_pricing(model: &str) -> (&'static str, f64, f64) {
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        ("haiku", 0.000_000_25, 0.000_001_25)
    } else if lower.contains("opus") {
        ("opus", 0.000_015, 0.000_075)
    } else {
        ("sonnet", 0.000_003, 0.000_015)
    }
}

#[derive(Debug, Default)]
struct AgentActivityAggregate {
    model: String,
    task: String,
    role: String,
    turns: usize,
    tokens_used: u64,
    cost_usd: f64,
    first_seen_at: Option<DateTime<Utc>>,
    latest_event_at: Option<DateTime<Utc>>,
}

impl AgentActivityAggregate {
    fn update_from_event(&mut self, event: &AgentEfficiencyEvent) {
        let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) else {
            return;
        };
        if self.first_seen_at.map_or(true, |first| timestamp < first) {
            self.first_seen_at = Some(timestamp);
        }
        if self
            .latest_event_at
            .map_or(true, |latest| timestamp > latest)
        {
            self.latest_event_at = Some(timestamp);
            self.model = event.model.clone();
            self.task = event.task_id.clone();
            self.role = event.role.clone();
        }
    }

    fn render_row(&self, agent: &AgentSummary, now: DateTime<Utc>) -> AgentActivityRow {
        let uptime_ms = self
            .first_seen_at
            .and_then(|first| {
                now.signed_duration_since(first)
                    .num_milliseconds()
                    .try_into()
                    .ok()
            })
            .unwrap_or_default();
        AgentActivityRow {
            agent_id: agent.id.clone(),
            plan_id: agent.plan_id.clone(),
            model: if self.model.is_empty() {
                String::from("-")
            } else {
                self.model.clone()
            },
            task: if self.task.is_empty() {
                agent.plan_id.clone().unwrap_or_else(|| String::from("-"))
            } else {
                self.task.clone()
            },
            role: if self.role.is_empty() {
                agent.status.clone()
            } else {
                self.role.clone()
            },
            turns: self.turns,
            tokens_used: self.tokens_used,
            cost_usd: self.cost_usd,
            uptime_ms,
        }
    }
}

#[derive(Debug, Default)]
struct ModelCostAggregate {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    input_rate: f64,
    output_rate: f64,
}

impl ModelCostAggregate {
    fn into_row(self) -> ModelCostRow {
        ModelCostRow {
            cost_usd: self.input_tokens as f64 * self.input_rate
                + self.output_tokens as f64 * self.output_rate,
            model: self.model,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            input_rate: self.input_rate,
            output_rate: self.output_rate,
        }
    }
}

fn parse_efficiency_timestamp(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

impl SignalSummary {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            kind: value.get("kind")?.as_str()?.to_string(),
            created_at_ms: entry_timestamp_ms(value)?,
            plan_id: value
                .pointer("/tags/plan_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    value
                        .pointer("/body/data/plan_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            task_id: value
                .pointer("/tags/task_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    value
                        .pointer("/body/data/task_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            parent_hash: signal_parent_hash(value),
            lineage: signal_lineage(value),
            payload_preview: signal_payload_preview(value),
        })
    }
}

impl GateResultSummary {
    fn from_signal(value: &Value, plan_id: &str) -> Option<Self> {
        let gate_name = extract_gate_name(value)?;
        let passed = extract_gate_passed(value)?;
        let duration_ms = extract_gate_duration_ms(value).unwrap_or(0);
        let rung = value
            .pointer("/tags/rung")
            .and_then(Value::as_u64)
            .or_else(|| value.pointer("/body/data/rung").and_then(Value::as_u64))
            .unwrap_or_default() as u32;
        let summary = value
            .pointer("/body/data/reason")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/body/reason").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();

        Some(Self {
            plan_id: plan_id.to_string(),
            gate_name,
            passed,
            rung,
            duration_ms,
            summary,
        })
    }
}

impl GateSignalSummary {
    fn from_value(value: &Value) -> Option<Self> {
        if !value
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(is_gate_result_kind)
        {
            return None;
        }

        Some(Self {
            id: value.get("id")?.as_str()?.to_string(),
            created_at_ms: entry_timestamp_ms(value)?,
            plan_id: value
                .pointer("/tags/plan_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    value
                        .pointer("/body/data/plan_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .or_else(|| {
                    value
                        .pointer("/body/plan_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            task_id: value
                .pointer("/tags/task_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    value
                        .pointer("/body/data/task_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .or_else(|| {
                    value
                        .pointer("/body/task_id")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            gate_name: extract_gate_name(value)?,
            passed: extract_gate_passed(value)?,
            duration_ms: extract_gate_duration_ms(value).unwrap_or_default(),
            excerpt: gate_excerpt_from_value(value),
        })
    }
}

impl ExperimentSummary {
    fn from_experiment(experiment: &roko_learn::prompt_experiment::PromptExperiment) -> Self {
        let total_trials: u64 = experiment.stats.values().map(|stats| stats.trials).sum();
        let active_variants = experiment
            .variants
            .iter()
            .filter(|variant| variant.active)
            .count();
        Self {
            experiment_id: experiment.experiment_id.clone(),
            section_name: experiment.section_name.clone(),
            status: format!("{:?}", experiment.status),
            winner_id: experiment.winner_id.clone(),
            active_variants,
            total_trials,
        }
    }
}

fn load_plan_summaries(root: &Path, state: &Value) -> Vec<PlanSummary> {
    let mut ids = std::collections::BTreeSet::new();
    if let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) {
        ids.extend(plan_states.keys().cloned());
    }
    if ids.is_empty() {
        if let Ok(entries) = std::fs::read_dir(plans_dir(root)) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    ids.insert(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }

    let mut summaries = Vec::new();
    for id in ids {
        let mut title = id.clone();
        let mut task_count = 0usize;
        let plan_dir = plans_dir(root).join(&id);
        let tasks_path = plan_dir.join("tasks.toml");
        if let Ok(tasks_file) = TasksFile::parse(&tasks_path) {
            if !tasks_file.meta.plan.trim().is_empty() {
                title = tasks_file.meta.plan.clone();
            }
            task_count = tasks_file.tasks.len();
        }

        let completed = state
            .get("plan_states")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&id))
            .and_then(current_phase_label)
            .map(|phase| {
                matches!(
                    phase.to_ascii_lowercase().as_str(),
                    "complete" | "done" | "failed" | "skipped"
                )
            })
            .unwrap_or(false);

        summaries.push(PlanSummary {
            id,
            title,
            task_count,
            completed,
            old_format: false,
        });
    }

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    summaries
}

fn load_active_tasks(state: &Value) -> Vec<TaskSummary> {
    let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut tasks = Vec::new();
    for (plan_id, plan_state) in plan_states {
        let status = current_phase_label(plan_state).unwrap_or_else(|| "unknown".to_string());
        if matches!(
            status.to_ascii_lowercase().as_str(),
            "complete" | "done" | "failed" | "skipped"
        ) {
            continue;
        }
        let task_id = plan_state
            .get("task_id")
            .and_then(Value::as_str)
            .or_else(|| plan_state.get("id").and_then(Value::as_str))
            .unwrap_or(plan_id.as_str())
            .to_string();
        let iteration = plan_state
            .get("iteration")
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32;
        let assigned_agents = plan_state
            .get("assigned_agents")
            .and_then(Value::as_array)
            .map(|agents| {
                agents
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        tasks.push(TaskSummary {
            plan_id: plan_id.clone(),
            task_id,
            status,
            iteration,
            assigned_agents,
        });
    }

    tasks.sort_by(|a, b| {
        a.plan_id
            .cmp(&b.plan_id)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    tasks
}

fn load_agents(state: &Value) -> Vec<AgentSummary> {
    let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut agents = Vec::new();
    for (plan_id, plan_state) in plan_states {
        let status = current_phase_label(plan_state).unwrap_or_else(|| "unknown".to_string());
        let assigned_agents = plan_state
            .get("assigned_agents")
            .and_then(Value::as_array)
            .map(|agents| {
                agents
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for agent in assigned_agents {
            agents.push(AgentSummary {
                id: agent.clone(),
                label: agent,
                plan_id: Some(plan_id.clone()),
                status: status.clone(),
            });
        }
    }

    agents.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.plan_id.cmp(&b.plan_id)));
    agents
}

fn load_gate_results(
    state: &Value,
    signal_gate_results: &[GateResultSummary],
) -> Vec<GateResultSummary> {
    let mut gate_results = Vec::new();

    if let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) {
        for (plan_id, plan_state) in plan_states {
            let Some(results) = plan_state.get("gate_results").and_then(Value::as_array) else {
                continue;
            };
            for result in results {
                gate_results.push(GateResultSummary {
                    plan_id: plan_id.clone(),
                    gate_name: result
                        .get("gate_name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    passed: result
                        .get("passed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    rung: result
                        .get("rung")
                        .and_then(Value::as_u64)
                        .unwrap_or_default() as u32,
                    duration_ms: result
                        .get("duration_ms")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    summary: result
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
    }

    if gate_results.is_empty() {
        gate_results.extend(signal_gate_results.iter().cloned());
    }

    gate_results.sort_by(|a, b| {
        a.plan_id
            .cmp(&b.plan_id)
            .then_with(|| a.gate_name.cmp(&b.gate_name))
            .then_with(|| a.rung.cmp(&b.rung))
    });
    gate_results
}

#[derive(Debug, Clone, Deserialize)]
struct TaskTrackerSnapshot {
    plan_id: String,
    #[serde(default)]
    completed: Vec<String>,
    #[serde(default)]
    failed: Vec<String>,
    #[serde(default)]
    current_group_index: usize,
}

fn load_task_trackers(root: &Path) -> HashMap<String, TaskTrackerSnapshot> {
    let path = root.join(".roko").join("state").join("task-trackers.json");
    let Some(value) = read_json_value(&path) else {
        return HashMap::new();
    };
    let Some(entries) = value.as_array() else {
        return HashMap::new();
    };

    let mut trackers = HashMap::new();
    for entry in entries {
        let Ok(record) = serde_json::from_value::<TaskTrackerSnapshot>(entry.clone()) else {
            continue;
        };
        if !record.plan_id.trim().is_empty() {
            trackers.insert(record.plan_id.clone(), record);
        }
    }
    trackers
}

fn load_current_plan_execution(
    root: &Path,
    state: &Value,
    episodes: &[Episode],
) -> Option<PlanExecutionSnapshot> {
    let plan_states = state.get("plan_states").and_then(Value::as_object)?;
    let trackers = load_task_trackers(root);

    let mut candidates = plan_states
        .iter()
        .filter_map(|(plan_id, plan_state)| {
            let phase = current_phase_label(plan_state)?;
            if matches!(
                phase.to_ascii_lowercase().as_str(),
                "complete" | "done" | "failed" | "skipped"
            ) {
                return None;
            }
            let priority = execution_phase_priority(&phase);
            let started_at_ms = plan_state
                .get("started_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some((priority, started_at_ms, plan_id.clone()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    let plan_id = candidates
        .first()
        .map(|(_, _, plan_id)| plan_id.clone())
        .or_else(|| trackers.keys().next().cloned())?;

    let plan_state = plan_states.get(&plan_id)?;
    let plan_phase = current_phase_label(plan_state).unwrap_or_else(|| String::from("queued"));
    let plan_dir = plans_dir(root).join(&plan_id);
    let tasks_file = TasksFile::parse(&plan_dir.join("tasks.toml")).ok()?;
    let tracker = trackers.get(&plan_id);
    let completed: HashSet<String> = tracker
        .map(|tracker| tracker.completed.iter().cloned().collect())
        .unwrap_or_default();
    let failed: HashSet<String> = tracker
        .map(|tracker| tracker.failed.iter().cloned().collect())
        .unwrap_or_default();
    let current_task_id = current_task_id(&tasks_file, tracker, &completed, &failed);
    let plan_title = if tasks_file.meta.plan.trim().is_empty() {
        plan_id.clone()
    } else {
        tasks_file.meta.plan.clone()
    };
    let current_episode = latest_episode_for_plan(&episodes, &plan_id, current_task_id.as_deref())
        .or_else(|| latest_episode_for_plan(&episodes, &plan_id, None));
    let agent_output_tail = current_episode
        .as_ref()
        .and_then(extract_episode_output_text)
        .map(|text| tail_lines(&text, 20))
        .unwrap_or_default()
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    let current_task = current_task_id.as_ref().and_then(|task_id| {
        tasks_file
            .tasks
            .iter()
            .find(|task| task.id == *task_id)
            .map(|task| build_task_detail(task))
    });

    let mut tasks = Vec::with_capacity(tasks_file.tasks.len());
    for task in &tasks_file.tasks {
        let phase = task_phase_label(
            task,
            &plan_phase,
            current_task_id.as_deref(),
            tracker,
            &completed,
            &failed,
        );
        let model = task
            .model_hint
            .clone()
            .unwrap_or_else(|| default_model_for_tier(&task.tier));
        let duration = latest_episode_for_task(&episodes, &plan_id, &task.id)
            .map(|episode| format_duration_ms(episode.usage.wall_ms))
            .unwrap_or_else(|| String::from("--"));

        tasks.push(PlanExecutionTaskRow {
            task_id: task.id.clone(),
            title: task.title.clone(),
            phase,
            frequency: task.operating_frequency(),
            model,
            duration,
            is_current: current_task_id.as_deref() == Some(task.id.as_str()),
        });
    }

    Some(PlanExecutionSnapshot {
        plan_id: plan_id.clone(),
        plan_title,
        tasks_done: completed.len(),
        tasks_total: tasks_file.tasks.len(),
        tasks,
        current_task,
        agent_output_tail,
    })
}

fn current_task_id(
    tasks_file: &TasksFile,
    tracker: Option<&TaskTrackerSnapshot>,
    completed: &HashSet<String>,
    failed: &HashSet<String>,
) -> Option<String> {
    let groups = tasks_file.parallel_groups();
    if let Some(tracker) = tracker {
        if let Some(group) = groups
            .get(
                tracker
                    .current_group_index
                    .min(groups.len().saturating_sub(1)),
            )
            .or_else(|| groups.last())
        {
            if let Some(task) = group
                .iter()
                .find(|task| !completed.contains(&task.id) && !failed.contains(&task.id))
            {
                return Some(task.id.clone());
            }
        }
    }

    tasks_file
        .tasks
        .iter()
        .find(|task| !completed.contains(&task.id) && !failed.contains(&task.id))
        .map(|task| task.id.clone())
}

fn build_task_detail(task: &TaskDef) -> PlanExecutionTaskDetail {
    let read_files = task
        .context
        .as_ref()
        .map(|ctx| {
            ctx.read_files
                .iter()
                .map(|rf| ReadFileSnapshot {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                    why: rf.why.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    PlanExecutionTaskDetail {
        task_id: task.id.clone(),
        frequency: task.operating_frequency(),
        description: task.title.clone(),
        read_files,
        write_files: task.files.clone(),
    }
}

fn task_phase_label(
    task: &TaskDef,
    plan_phase: &str,
    current_task_id: Option<&str>,
    tracker: Option<&TaskTrackerSnapshot>,
    completed: &HashSet<String>,
    failed: &HashSet<String>,
) -> String {
    let _ = tracker;
    if completed.contains(&task.id) {
        return String::from("Done");
    }
    if failed.contains(&task.id) {
        return String::from("Failed");
    }
    if current_task_id == Some(task.id.as_str()) {
        return match plan_phase.to_ascii_lowercase().as_str() {
            "implementing" => String::from("Implementing"),
            "gating" => String::from("Gating"),
            "verifying" => String::from("Verifying"),
            "reviewing" => String::from("Reviewing"),
            "doc-revision" => String::from("Doc Revision"),
            "auto-fixing" => String::from("Auto Fixing"),
            "regenerating-verify" => String::from("Regenerating Verify"),
            other => title_case_phase(other),
        };
    }
    String::from("Queued")
}

fn title_case_phase(phase: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for ch in phase.chars() {
        if ch == '-' || ch == '_' {
            out.push(' ');
            capitalize = true;
            continue;
        }
        if capitalize {
            out.extend(ch.to_uppercase());
            capitalize = false;
        } else {
            out.push(ch);
        }
    }
    if out.is_empty() {
        String::from("Unknown")
    } else {
        out
    }
}

fn execution_phase_priority(phase: &str) -> u8 {
    match phase.to_ascii_lowercase().as_str() {
        "implementing" => 5,
        "gating" => 4,
        "verifying" => 3,
        "reviewing" => 2,
        "doc-revision" => 2,
        "auto-fixing" => 2,
        "regenerating-verify" => 1,
        "enriching" => 1,
        "queued" => 0,
        _ => 0,
    }
}

fn default_model_for_tier(tier: &str) -> String {
    match tier.to_ascii_lowercase().as_str() {
        "mechanical" => String::from("claude-haiku-4-5"),
        "focused" | "integrative" => String::from("claude-sonnet-4-6"),
        "architectural" => String::from("claude-opus-4-6"),
        _ => String::from("claude-sonnet-4-6"),
    }
}

pub(crate) fn operating_frequency_label(frequency: OperatingFrequency) -> &'static str {
    match frequency {
        OperatingFrequency::Gamma => "gamma",
        OperatingFrequency::Theta => "theta",
        OperatingFrequency::Delta => "delta",
    }
}

fn load_episodes(path: &Path) -> Vec<Episode> {
    read_jsonl_values(path)
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<Episode>(entry).ok())
        .collect()
}

fn latest_episode_for_plan(
    episodes: &[Episode],
    plan_id: &str,
    task_id: Option<&str>,
) -> Option<Episode> {
    episodes
        .iter()
        .rev()
        .find(|episode| episode_matches_plan(episode, plan_id, task_id))
        .cloned()
}

fn latest_episode_for_task(episodes: &[Episode], plan_id: &str, task_id: &str) -> Option<Episode> {
    episodes
        .iter()
        .rev()
        .find(|episode| episode_matches_plan(episode, plan_id, Some(task_id)))
        .cloned()
}

fn episode_matches_plan(episode: &Episode, plan_id: &str, task_id: Option<&str>) -> bool {
    let matches_plan = episode.input_signal_hash == plan_id
        || episode.extra.get("plan_id").and_then(Value::as_str) == Some(plan_id);
    if !matches_plan {
        return false;
    }

    if let Some(task_id) = task_id {
        return episode.task_id == task_id
            || episode.extra.get("task_id").and_then(Value::as_str) == Some(task_id);
    }

    true
}

fn extract_episode_output_text(episode: &Episode) -> Option<String> {
    for key in [
        "stderr",
        "agent_stderr",
        "output",
        "stdout",
        "agent_output",
        "output_tail",
        "detail",
        "text",
    ] {
        if let Some(value) = episode.extra.get(key).and_then(json_value_to_text) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }

    episode
        .failure_reason
        .as_deref()
        .map(ToOwned::to_owned)
        .filter(|text| !text.trim().is_empty())
}

fn json_value_to_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(json_value_to_text)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn tail_lines(text: &str, line_count: usize) -> String {
    let mut lines: Vec<&str> = text.lines().rev().take(line_count).collect();
    lines.reverse();
    lines.join("\n")
}

fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms == 0 {
        return String::from("--");
    }
    if duration_ms < 1000 {
        return format!("{duration_ms}ms");
    }
    let secs = duration_ms / 1000;
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    format!("{mins}m {}s", secs % 60)
}

fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).unwrap_or(u64::MAX)
}

fn format_elapsed_ms(ms: u64) -> String {
    let secs = ms / 1000;
    if secs == 0 {
        return String::from("<1s");
    }
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h {}m", mins % 60);
    }
    format!("{hours}h {}m", mins % 60)
}

fn load_efficiency_summary(path: &Path) -> EfficiencySummary {
    let events = read_efficiency_events_sync(path);
    if events.is_empty() {
        return EfficiencySummary::default();
    }

    let event_count = events.len();
    let total_cost_usd = events.iter().map(|event| event.cost_usd).sum();
    let total_input_tokens = events.iter().map(|event| event.input_tokens).sum();
    let total_output_tokens = events.iter().map(|event| event.output_tokens).sum();
    let passed_count = events.iter().filter(|event| event.gate_passed).count();
    let average_wall_time_ms = events
        .iter()
        .map(|event| event.wall_time_ms as f64)
        .sum::<f64>()
        / count_to_f64(event_count);

    EfficiencySummary {
        event_count,
        total_cost_usd,
        total_input_tokens,
        total_output_tokens,
        passed_count,
        average_wall_time_ms,
    }
}

fn load_recent_signals(path: &Path, limit: usize) -> Vec<SignalSummary> {
    let mut signals = read_jsonl_values(path)
        .into_iter()
        .filter_map(|entry| SignalSummary::from_value(&entry))
        .collect::<Vec<_>>();
    if signals.len() > limit {
        signals = signals.split_off(signals.len() - limit);
    }
    signals
}

fn load_gate_signal_summaries(path: &Path) -> Vec<GateSignalSummary> {
    let mut signals = read_jsonl_values(path)
        .into_iter()
        .filter_map(|entry| GateSignalSummary::from_value(&entry))
        .collect::<Vec<_>>();
    signals.sort_by(|a, b| {
        a.created_at_ms
            .cmp(&b.created_at_ms)
            .then_with(|| a.gate_name.cmp(&b.gate_name))
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    signals
}

fn build_gate_results_page_data(
    signals: &[GateSignalSummary],
    adaptive_thresholds: Option<&AdaptiveThresholds>,
) -> GateResultsPageData {
    let mut by_gate: BTreeMap<String, GateAggregate> = BTreeMap::new();
    for signal in signals {
        let aggregate = by_gate.entry(signal.gate_name.clone()).or_default();
        aggregate.total_runs += 1;
        if signal.passed {
            aggregate.passed_runs += 1;
        }
        aggregate.total_duration_ms += signal.duration_ms as f64;
        aggregate.last_run = Some(signal.clone());
    }

    let mut gate_rows = by_gate
        .into_iter()
        .filter_map(|(gate_name, aggregate)| {
            let last_run = aggregate.last_run?;
            let total_runs = aggregate.total_runs;
            let pass_rate = if total_runs == 0 {
                0.0
            } else {
                aggregate.passed_runs as f64 / total_runs as f64
            };
            let avg_duration_ms = if total_runs == 0 {
                0.0
            } else {
                aggregate.total_duration_ms / total_runs as f64
            };
            Some(GateSummaryRow {
                gate_name,
                total_runs,
                pass_rate,
                avg_duration_ms,
                last_run: format_last_run(&last_run),
            })
        })
        .collect::<Vec<_>>();
    gate_rows.sort_by(|a, b| {
        b.total_runs
            .cmp(&a.total_runs)
            .then_with(|| a.gate_name.cmp(&b.gate_name))
    });

    let mut threshold_rows = Vec::new();
    if let Some(thresholds) = adaptive_thresholds {
        threshold_rows = thresholds
            .all_rungs()
            .map(|(rung, stats)| GateThresholdRow {
                rung: *rung,
                current_threshold: thresholds.suggested_max_retries(*rung),
                ema_pass_rate: stats.ema_pass_rate,
                trend: gate_trend_from_ema(stats.ema_pass_rate),
            })
            .collect::<Vec<_>>();
        threshold_rows.sort_by_key(|row| row.rung);
    }

    let mut failure_rows = signals
        .iter()
        .filter(|signal| !signal.passed)
        .map(|signal| GateFailureRow {
            created_at_ms: signal.created_at_ms,
            task_id: signal
                .task_id
                .clone()
                .unwrap_or_else(|| String::from("unknown")),
            gate_name: signal.gate_name.clone(),
            error_excerpt: signal.excerpt.clone(),
        })
        .collect::<Vec<_>>();
    failure_rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    failure_rows.truncate(10);

    GateResultsPageData {
        gate_rows,
        threshold_rows,
        failure_rows,
    }
}

fn file_stamp(path: &Path) -> FileStamp {
    FileStamp::from_path(path).unwrap_or_default()
}

fn load_cfactor_history(path: &Path) -> Vec<CFactor> {
    read_jsonl_values(path)
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<CFactor>(entry).ok())
        .collect()
}

fn load_signal_state(
    path: &Path,
) -> (
    Vec<SignalSummary>,
    Vec<GateSignalSummary>,
    Vec<GateResultSummary>,
    JsonlState,
) {
    let values = read_jsonl_values(path);
    let (recent_signals, gate_signal_summaries, signal_gate_results) =
        collect_signal_records(values);
    let stamp = file_stamp(path);
    (
        recent_signals,
        gate_signal_summaries,
        signal_gate_results,
        JsonlState {
            stamp,
            offset: stamp.len,
        },
    )
}

fn load_episodes_state(path: &Path) -> JsonlState {
    let stamp = file_stamp(path);
    JsonlState {
        stamp,
        offset: stamp.len,
    }
}

fn load_episodes_from_path(path: &Path) -> Vec<Episode> {
    load_episodes(path)
}

fn load_episodes_from_offset(path: &Path, offset: u64) -> Vec<Episode> {
    read_jsonl_values_from_offset(path, offset)
        .into_iter()
        .filter_map(|entry| serde_json::from_value::<Episode>(entry).ok())
        .collect()
}

fn collect_signal_records(
    values: Vec<Value>,
) -> (
    Vec<SignalSummary>,
    Vec<GateSignalSummary>,
    Vec<GateResultSummary>,
) {
    let mut recent_signals = Vec::new();
    let mut gate_signal_summaries = Vec::new();
    let mut signal_gate_results = Vec::new();
    for value in values {
        if let Some(signal) = SignalSummary::from_value(&value) {
            recent_signals.push(signal);
        }
        if let Some(gate_signal) = GateSignalSummary::from_value(&value) {
            gate_signal_summaries.push(gate_signal);
        }
        if let Some(gate_result) = signal_gate_result_from_value(&value) {
            signal_gate_results.push(gate_result);
        }
    }

    if recent_signals.len() > 100 {
        let keep_from = recent_signals.len() - 100;
        recent_signals = recent_signals.split_off(keep_from);
    }

    (recent_signals, gate_signal_summaries, signal_gate_results)
}

fn read_jsonl_values_from_offset(path: &Path, offset: u64) -> Vec<Value> {
    let Ok(file) = File::open(path) else {
        return Vec::new();
    };
    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return Vec::new();
    }

    let mut values = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let Ok(bytes) = reader.read_line(&mut line) else {
            break;
        };
        if bytes == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            values.push(value);
        }
    }

    values
}

fn signal_gate_result_from_value(value: &Value) -> Option<GateResultSummary> {
    if !value
        .get("kind")
        .and_then(Value::as_str)
        .is_some_and(is_gate_result_kind)
    {
        return None;
    }
    let plan_id = value
        .pointer("/tags/plan_id")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/body/data/plan_id").and_then(Value::as_str))
        .or_else(|| value.pointer("/body/plan_id").and_then(Value::as_str))
        .unwrap_or("unknown");
    GateResultSummary::from_signal(value, plan_id)
}

pub(crate) fn read_json_value(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn read_jsonl_values(path: &Path) -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn read_efficiency_events_sync(path: &Path) -> Vec<AgentEfficiencyEvent> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

#[derive(Debug, Default)]
struct GateAggregate {
    total_runs: u64,
    passed_runs: u64,
    total_duration_ms: f64,
    last_run: Option<GateSignalSummary>,
}

fn gate_trend_from_ema(ema_pass_rate: f64) -> GateTrend {
    if ema_pass_rate >= 0.55 {
        GateTrend::Up
    } else if ema_pass_rate <= 0.45 {
        GateTrend::Down
    } else {
        GateTrend::Flat
    }
}

fn format_last_run(signal: &GateSignalSummary) -> String {
    let age_ms = if signal.created_at_ms <= 0 {
        0
    } else {
        let created_at_ms = u64::try_from(signal.created_at_ms).unwrap_or_default();
        now_ms().saturating_sub(created_at_ms)
    };
    let state = if signal.passed { "pass" } else { "fail" };
    format!("{} {state}", format_elapsed_ms(age_ms))
}

fn gate_excerpt_from_value(value: &Value) -> String {
    for pointer in [
        "/tags/error",
        "/tags/message",
        "/body/data/error",
        "/body/data/message",
        "/body/data/reason",
        "/body/error",
        "/body/message",
        "/body/reason",
    ] {
        if let Some(text) = value.pointer(pointer).and_then(Value::as_str) {
            let first_line = text.lines().next().unwrap_or("").trim();
            if !first_line.is_empty() {
                return first_line.to_string();
            }
        }
    }

    String::new()
}

fn extract_gate_name(entry: &Value) -> Option<String> {
    entry
        .pointer("/tags/gate")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .pointer("/body/data/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .pointer("/body/gate")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .get("kind")
                .and_then(Value::as_str)
                .and_then(|kind| kind.strip_prefix("gate:").or(kind.strip_prefix("gate_")))
                .map(ToOwned::to_owned)
        })
}

fn extract_gate_passed(entry: &Value) -> Option<bool> {
    entry
        .pointer("/tags/passed")
        .and_then(Value::as_str)
        .and_then(|s| match s {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .or_else(|| entry.pointer("/body/data/passed").and_then(Value::as_bool))
        .or_else(|| entry.pointer("/body/passed").and_then(Value::as_bool))
}

fn extract_gate_duration_ms(entry: &Value) -> Option<u64> {
    entry
        .pointer("/body/data/duration_ms")
        .and_then(Value::as_u64)
        .or_else(|| entry.pointer("/body/duration_ms").and_then(Value::as_u64))
        .or_else(|| entry.pointer("/tags/duration_ms").and_then(Value::as_u64))
}

fn entry_timestamp_ms(entry: &Value) -> Option<i64> {
    entry
        .get("created_at_ms")
        .and_then(Value::as_i64)
        .or_else(|| {
            entry
                .get("created_at_ms")
                .and_then(Value::as_u64)
                .map(|ts| ts as i64)
        })
}

fn signal_parent_hash(value: &Value) -> Option<String> {
    value
        .pointer("/parent_hash")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .pointer("/body/data/parent_hash")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .pointer("/body/parent_hash")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .pointer("/lineage")
                .and_then(Value::as_array)
                .and_then(|lineage| lineage.last())
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn signal_lineage(value: &Value) -> Vec<String> {
    value
        .pointer("/lineage")
        .and_then(Value::as_array)
        .map(|lineage| {
            lineage
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .or_else(|| {
            value
                .pointer("/body/data/lineage")
                .and_then(Value::as_array)
                .map(|lineage| {
                    lineage
                        .iter()
                        .filter_map(Value::as_str)
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default()
}

fn signal_payload_preview(value: &Value) -> String {
    let payload = value
        .pointer("/body/data")
        .or_else(|| value.get("body"))
        .or_else(|| value.get("payload"));

    let Some(payload) = payload else {
        return String::new();
    };

    let raw = match payload {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_str(compact.trim(), 60)
}

fn current_phase_label(plan_state: &Value) -> Option<String> {
    plan_state
        .pointer("/current_phase/kind")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            plan_state
                .get("current_phase")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            plan_state
                .pointer("/phase/kind")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            plan_state
                .get("phase")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn is_gate_result_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
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

/// Best-effort learning snapshot for dashboard rendering.
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    root: PathBuf,
    episode_count: usize,
    success_rate: Option<f64>,
    average_cost_usd: Option<f64>,
    average_wall_time_ms: Option<f64>,
    task_metric_count: usize,
    haiku_share: Option<f64>,
    cache_hit_rate: Option<f64>,
    headlines: Headlines,
    /// Raw efficiency events from `.roko/learn/efficiency.jsonl`.
    efficiency_events: Vec<AgentEfficiencyEvent>,
    /// Prompt experiment store from `.roko/learn/experiments.json`.
    experiments: Option<ExperimentStore>,
    /// Adaptive gate thresholds from `.roko/learn/gate-thresholds.json`.
    adaptive_thresholds: Option<AdaptiveThresholds>,
    /// Gate-results page data derived from signals and thresholds.
    gate_results_page: GateResultsPageData,
    /// Most recent signals from `.roko/signals.jsonl`.
    recent_signals: Vec<SignalSummary>,
    /// Cascade router snapshot from `.roko/learn/cascade-router.json` (raw JSON).
    cascade_snapshot: Option<CascadeSnapshotData>,
    /// Raw episodes kept for per-agent analysis.
    episodes: Vec<Episode>,
}

/// Deserialized cascade router snapshot matching the private `CascadeSnapshot`.
#[derive(Debug, Clone, serde::Deserialize)]
struct CascadeSnapshotData {
    #[serde(default)]
    model_slugs: Vec<String>,
    #[serde(default)]
    confidence_stats: HashMap<String, PersistedModelStatsData>,
}

/// Per-model stats from the cascade router JSON.
#[derive(Debug, Clone, serde::Deserialize)]
struct PersistedModelStatsData {
    trials: u64,
    successes: u64,
}

impl DashboardSnapshot {
    /// Load the learning snapshot from a workspace root.
    pub async fn load(root: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let root = resolve_snapshot_root(root.as_ref());
        let memory_dir = root.join(MEMORY_DIR);
        let learn_dir = root.join(LEARN_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let task_metrics_path = memory_dir.join(TASK_METRICS_FILE);
        let signals_path = root.join(".roko").join("signals.jsonl");

        let episodes_logger = EpisodeLogger::new(&episodes_path);
        let episodes = EpisodeLogger::read_all_lossy(episodes_logger.path())
            .await
            .map_err(std::io::Error::other)?;
        let task_metrics = read_task_metrics(&task_metrics_path).await?;

        // Load learning subsystem data (best-effort).
        let efficiency_events = read_efficiency_events(&learn_dir.join(EFFICIENCY_FILE)).await;
        let experiments = load_json_opt::<ExperimentStore>(&learn_dir.join(EXPERIMENTS_FILE));
        let adaptive_thresholds =
            load_json_opt::<AdaptiveThresholds>(&learn_dir.join(GATE_THRESHOLDS_FILE));
        let gate_signals = load_gate_signal_summaries(&signals_path);
        let gate_results_page =
            build_gate_results_page_data(&gate_signals, adaptive_thresholds.as_ref());
        let recent_signals = load_recent_signals(&signals_path, 100);
        let cascade_snapshot =
            load_json_opt::<CascadeSnapshotData>(&learn_dir.join(CASCADE_ROUTER_FILE));

        Ok(Self::from_records(
            root,
            &episodes,
            &task_metrics,
            efficiency_events,
            experiments,
            adaptive_thresholds,
            gate_results_page,
            recent_signals,
            cascade_snapshot,
        ))
    }

    fn empty(root: PathBuf) -> Self {
        Self::from_records(
            root,
            &[],
            &[],
            Vec::new(),
            None,
            None,
            GateResultsPageData::default(),
            Vec::new(),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_records(
        root: PathBuf,
        episodes: &[Episode],
        task_metrics: &[TaskMetric],
        efficiency_events: Vec<AgentEfficiencyEvent>,
        experiments: Option<ExperimentStore>,
        adaptive_thresholds: Option<AdaptiveThresholds>,
        gate_results_page: GateResultsPageData,
        recent_signals: Vec<SignalSummary>,
        cascade_snapshot: Option<CascadeSnapshotData>,
    ) -> Self {
        let episode_count = episodes.len();
        let success_rate = if episode_count == 0 {
            None
        } else {
            let successes = episodes.iter().filter(|episode| episode.success).count();
            Some(count_to_f64(successes) / count_to_f64(episode_count))
        };
        let average_cost_usd = if episode_count == 0 {
            None
        } else {
            Some(
                episodes
                    .iter()
                    .map(|episode| episode.usage.cost_usd)
                    .sum::<f64>()
                    / count_to_f64(episode_count),
            )
        };
        let average_wall_time_ms = if episode_count == 0 {
            None
        } else {
            Some(
                episodes
                    .iter()
                    .map(|episode| wall_ms_to_f64(episode.usage.wall_ms))
                    .sum::<f64>()
                    / count_to_f64(episode_count),
            )
        };

        let task_metric_count = task_metrics.len();
        let haiku_share = if task_metric_count == 0 {
            None
        } else {
            let haiku = task_metrics
                .iter()
                .filter(|metric| metric.model.to_ascii_lowercase().contains("haiku"))
                .count();
            Some(count_to_f64(haiku) / count_to_f64(task_metric_count))
        };
        let cache_hit_rate = if task_metric_count == 0 {
            None
        } else {
            Some(
                task_metrics
                    .iter()
                    .map(|metric| metric.cache_hit_rate)
                    .sum::<f64>()
                    / count_to_f64(task_metric_count),
            )
        };
        let headlines = compute_headlines(task_metrics);

        Self {
            root,
            episode_count,
            success_rate,
            average_cost_usd,
            average_wall_time_ms,
            task_metric_count,
            haiku_share,
            cache_hit_rate,
            headlines,
            efficiency_events,
            experiments,
            adaptive_thresholds,
            gate_results_page,
            recent_signals,
            cascade_snapshot,
            episodes: episodes.to_vec(),
        }
    }

    fn render_health_page(&self, page: &PageScaffold) -> Option<String> {
        if self.episode_count == 0 {
            return None;
        }

        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", page.title, page.id.slug());
        let _ = writeln!(out, "group: {}", page.id.group());
        let _ = writeln!(out, "intent: {}", page.intent);
        let _ = writeln!(
            out,
            "source: {}/{}",
            self.root.join(MEMORY_DIR).display(),
            EPISODES_FILE
        );
        let _ = writeln!(out, "episodes: {}", self.episode_count);
        let _ = writeln!(
            out,
            "success rate: {}",
            format_pct(self.success_rate.unwrap_or(0.0))
        );
        let _ = writeln!(
            out,
            "average cost: {}",
            format_usd(self.average_cost_usd.unwrap_or(0.0))
        );
        let _ = writeln!(
            out,
            "average wall time: {}",
            format_ms(self.average_wall_time_ms.unwrap_or(0.0))
        );
        if let Some(hit_rate) = self.cache_hit_rate {
            let _ = writeln!(out, "cache hit rate: {}", format_pct(hit_rate));
        }
        if let Some(haiku_share) = self.haiku_share {
            let _ = writeln!(out, "haiku share: {}", format_pct(haiku_share));
        }
        if self.task_metric_count > 0 {
            let _ = writeln!(out, "task metrics: {}", self.task_metric_count);
        }
        out.push_str("widgets (scaffold):\n");
        for widget in &page.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        Some(out)
    }

    fn render_trends_page(&self, page: &PageScaffold) -> Option<String> {
        if self.task_metric_count == 0 {
            return None;
        }

        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", page.title, page.id.slug());
        let _ = writeln!(out, "group: {}", page.id.group());
        let _ = writeln!(out, "intent: {}", page.intent);
        let _ = writeln!(
            out,
            "source: {}/{}",
            self.root.join(MEMORY_DIR).display(),
            TASK_METRICS_FILE
        );
        let _ = writeln!(out, "task metrics: {}", self.task_metric_count);
        let _ = writeln!(
            out,
            "first-attempt pass rate: {}",
            format_pct(self.headlines.first_attempt_pass_rate)
        );
        let _ = writeln!(
            out,
            "avg iterations per plan: {}",
            format_float(self.headlines.avg_iterations_per_plan)
        );
        let _ = writeln!(
            out,
            "avg cost per plan: {}",
            format_usd(self.headlines.avg_cost_per_plan)
        );
        let _ = writeln!(
            out,
            "avg input tokens per spawn: {}",
            format_float(self.headlines.avg_input_tokens_per_spawn)
        );
        let _ = writeln!(out, "plans: {}", self.headlines.n_plans);
        let _ = writeln!(out, "records: {}", self.headlines.n_records);
        if let Some(hit_rate) = self.cache_hit_rate {
            let _ = writeln!(out, "cache hit rate: {}", format_pct(hit_rate));
        }
        if let Some(haiku_share) = self.haiku_share {
            let _ = writeln!(out, "haiku share: {}", format_pct(haiku_share));
        }
        out.push_str("headlines:\n");
        let _ = writeln!(
            out,
            "- first_attempt_pass_rate: {}",
            format_pct(self.headlines.first_attempt_pass_rate)
        );
        let _ = writeln!(
            out,
            "- avg_iterations_per_plan: {}",
            format_float(self.headlines.avg_iterations_per_plan)
        );
        let _ = writeln!(
            out,
            "- avg_cost_per_plan: {}",
            format_usd(self.headlines.avg_cost_per_plan)
        );
        let _ = writeln!(
            out,
            "- avg_input_tokens_per_spawn: {}",
            format_float(self.headlines.avg_input_tokens_per_spawn)
        );
        out.push_str("widgets (scaffold):\n");
        for widget in &page.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        Some(out)
    }

    // ── Efficiency pages ────────────────────────────────────────────

    fn render_correlations_page(&self, page: &PageScaffold) -> Option<String> {
        if self.efficiency_events.is_empty() {
            return None;
        }
        let mut out = page_header(page);
        let _ = writeln!(out, "events: {}", self.efficiency_events.len());
        let _ = writeln!(out);

        // prompt_tokens vs gate_passed histogram.
        // Bucket by prompt token count in 1k increments.
        let mut buckets: BTreeMap<u64, (u32, u32)> = BTreeMap::new(); // bucket -> (total, passed)
        for ev in &self.efficiency_events {
            let bucket = ev.total_prompt_tokens / 1000 * 1000; // round down to nearest 1k
            let entry = buckets.entry(bucket).or_default();
            entry.0 += 1;
            if ev.gate_passed {
                entry.1 += 1;
            }
        }
        let _ = writeln!(out, "prompt tokens vs pass rate:");
        let _ = writeln!(
            out,
            "  {:>10}  {:>6}  {:>9}  bar",
            "tokens", "count", "pass rate"
        );
        for (bucket, (total, passed)) in &buckets {
            let rate = if *total > 0 {
                *passed as f64 / *total as f64
            } else {
                0.0
            };
            let bar_len = (rate * 20.0).round() as usize;
            let bar: String = std::iter::repeat_n('#', bar_len).collect();
            let _ = writeln!(
                out,
                "  {:>9}k  {:>6}  {:>8}  {}",
                bucket / 1000,
                total,
                format_pct(rate),
                bar
            );
        }

        // cost vs pass rate.
        let _ = writeln!(out);
        let _ = writeln!(out, "cost vs pass rate:");
        let cost_buckets: Vec<(f64, &str)> = vec![
            (0.001, "<$0.001"),
            (0.01, "<$0.01"),
            (0.1, "<$0.10"),
            (f64::MAX, ">=$0.10"),
        ];
        let _ = writeln!(out, "  {:>10}  {:>6}  {:>9}", "range", "count", "pass rate");
        let mut prev = 0.0_f64;
        for (threshold, label) in &cost_buckets {
            let matching: Vec<&AgentEfficiencyEvent> = self
                .efficiency_events
                .iter()
                .filter(|e| e.cost_usd >= prev && e.cost_usd < *threshold)
                .collect();
            let total = matching.len();
            let passed = matching.iter().filter(|e| e.gate_passed).count();
            if total > 0 {
                let rate = count_to_f64(passed) / count_to_f64(total);
                let _ = writeln!(
                    out,
                    "  {:>10}  {:>6}  {:>9}",
                    label,
                    total,
                    format_pct(rate)
                );
            }
            prev = *threshold;
        }

        Some(out)
    }

    fn render_parameters_page(&self, page: &PageScaffold) -> Option<String> {
        let has_thresholds = self.adaptive_thresholds.is_some();
        let has_cascade = self.cascade_snapshot.is_some();
        if !has_thresholds && !has_cascade {
            return None;
        }

        let mut out = page_header(page);

        // Cascade router model weights.
        if let Some(snap) = &self.cascade_snapshot {
            let _ = writeln!(out, "cascade router:");
            let _ = writeln!(out, "  registered models: {}", snap.model_slugs.len());
            for slug in &snap.model_slugs {
                let _ = writeln!(out, "    - {slug}");
            }
            if !snap.confidence_stats.is_empty() {
                let _ = writeln!(out, "  confidence-stage stats:");
                let _ = writeln!(
                    out,
                    "    {:>20}  {:>8}  {:>8}  {:>9}",
                    "model", "trials", "passes", "pass rate"
                );
                let mut stats: Vec<_> = snap.confidence_stats.iter().collect();
                stats.sort_by(|a, b| b.1.trials.cmp(&a.1.trials));
                for (model, s) in stats {
                    #[allow(clippy::cast_precision_loss)]
                    let rate = if s.trials > 0 {
                        s.successes as f64 / s.trials as f64
                    } else {
                        0.0
                    };
                    let _ = writeln!(
                        out,
                        "    {:>20}  {:>8}  {:>8}  {:>9}",
                        model,
                        s.trials,
                        s.successes,
                        format_pct(rate)
                    );
                }
            }
            let _ = writeln!(out);
        }

        // Adaptive gate thresholds.
        if let Some(at) = &self.adaptive_thresholds {
            let _ = writeln!(out, "adaptive gate thresholds:");
            let _ = writeln!(
                out,
                "  {:>5}  {:>12}  {:>6}  {:>12}  {:>4}",
                "rung", "ema pass rate", "obs", "consec pass", "skip"
            );
            let mut rungs: Vec<_> = at.all_rungs().collect();
            rungs.sort_by_key(|(r, _)| *r);
            for (rung, stats) in rungs {
                let skip = if at.should_skip_rung(*rung) {
                    "yes"
                } else {
                    "no"
                };
                let _ = writeln!(
                    out,
                    "  {:>5}  {:>12}  {:>6}  {:>12}  {:>4}",
                    rung,
                    format_pct(stats.ema_pass_rate),
                    stats.total_observations,
                    stats.consecutive_passes,
                    skip
                );
            }
        }

        Some(out)
    }

    fn render_gate_results_page(&self, page: &PageScaffold) -> Option<String> {
        if self.gate_results_page.gate_rows.is_empty()
            && self.gate_results_page.threshold_rows.is_empty()
            && self.gate_results_page.failure_rows.is_empty()
        {
            return None;
        }

        let mut out = page_header(page);
        let _ = writeln!(
            out,
            "source: {}/signals.jsonl",
            self.root.join(".roko").display()
        );
        let _ = writeln!(
            out,
            "source: {}/gate-thresholds.json",
            self.root.join(LEARN_DIR).display()
        );

        let _ = writeln!(out);
        let _ = writeln!(out, "gate summary:");
        let _ = writeln!(
            out,
            "  {:>18}  {:>10}  {:>9}  {:>12}  {:>12}",
            "gate", "runs", "pass rate", "avg duration", "last run"
        );
        for row in &self.gate_results_page.gate_rows {
            let _ = writeln!(
                out,
                "  {:>18}  {:>10}  {:>9}  {:>12}  {:>12}",
                truncate_str(&row.gate_name, 18),
                row.total_runs,
                format_pct(row.pass_rate),
                format_ms(row.avg_duration_ms),
                truncate_str(&row.last_run, 12)
            );
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "adaptive thresholds:");
        let _ = writeln!(
            out,
            "  {:>6}  {:>10}  {:>9}  {:>8}",
            "rung", "threshold", "ema", "trend"
        );
        for row in &self.gate_results_page.threshold_rows {
            let trend = match row.trend {
                GateTrend::Up => "↑",
                GateTrend::Flat => "→",
                GateTrend::Down => "↓",
            };
            let _ = writeln!(
                out,
                "  {:>6}  {:>10}  {:>9}  {:>8}",
                row.rung,
                row.current_threshold,
                format_pct(row.ema_pass_rate),
                trend
            );
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "recent gate failures:");
        if self.gate_results_page.failure_rows.is_empty() {
            let _ = writeln!(out, "  (none)");
        } else {
            for row in &self.gate_results_page.failure_rows {
                let _ = writeln!(
                    out,
                    "  {} | {} | {}",
                    row.task_id,
                    row.gate_name,
                    truncate_str(&row.error_excerpt, 120)
                );
            }
        }

        Some(out)
    }

    fn render_experiments_page(&self, page: &PageScaffold) -> Option<String> {
        let store = self.experiments.as_ref()?;
        if store.running_count() == 0 && store.concluded_count() == 0 {
            return None;
        }

        let mut out = page_header(page);
        let _ = writeln!(
            out,
            "experiments: {} running, {} concluded",
            store.running_count(),
            store.concluded_count()
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  {:>20}  {:>10}  {:>8}  {:>10}  verdict",
            "section", "status", "trials", "arms"
        );
        for exp in store.iter() {
            let total_trials: u64 = exp.stats.values().map(|s| s.trials).sum();
            let verdict = exp
                .winner_id
                .as_ref()
                .map_or_else(|| "-".to_string(), |winner| format!("winner={winner}"));
            let _ = writeln!(
                out,
                "  {:>20}  {:>10}  {:>8}  {:>10}  {}",
                exp.section_name,
                format!("{:?}", exp.status),
                total_trials,
                exp.variants.len(),
                verdict
            );
        }

        Some(out)
    }

    fn render_learning_page(&self, page: &PageScaffold) -> Option<String> {
        let has_router = self.cascade_snapshot.as_ref().is_some_and(|snapshot| {
            !snapshot.model_slugs.is_empty() || !snapshot.confidence_stats.is_empty()
        });
        let has_experiments = self
            .experiments
            .as_ref()
            .is_some_and(|store| store.running_count() > 0);
        let has_trends = !self.efficiency_events.is_empty();

        if !has_router && !has_experiments && !has_trends {
            return None;
        }

        let mut out = page_header(page);
        let _ = writeln!(
            out,
            "source: {}/{}",
            self.root.join(LEARN_DIR).display(),
            CASCADE_ROUTER_FILE
        );

        let router_rows = self
            .cascade_snapshot
            .as_ref()
            .map(learning_cascade_rows_snapshot)
            .unwrap_or_default();
        let recommendation_counts = learning_recommendation_counts_snapshot(&router_rows);
        let _ = writeln!(out, "cascade router:");
        if router_rows.is_empty() {
            let _ = writeln!(out, "  no cascade-router data");
        } else {
            let _ = writeln!(
                out,
                "  {:>20}  {:>10}  {:>6}  {:>10}",
                "model", "weight", "recs", "ucb"
            );
            for row in &router_rows {
                let recs = recommendation_counts
                    .get(&row.model)
                    .copied()
                    .unwrap_or_default();
                let _ = writeln!(
                    out,
                    "  {:>20}  {:>10}  {:>6}  {:>10}",
                    truncate_str(&row.model, 20),
                    format_pct(row.weight),
                    recs,
                    format_float(row.ucb_score)
                );
            }
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "active experiments:");
        if let Some(store) = self.experiments.as_ref() {
            let experiment_rows = learning_experiment_rows_snapshot(store);
            if experiment_rows.is_empty() {
                let _ = writeln!(out, "  no active experiments");
            } else {
                let _ = writeln!(
                    out,
                    "  {:>20}  {:>18}  {:>18}  {:>14}  {:>14}",
                    "experiment", "variants", "samples", "winner", "significance"
                );
                for row in &experiment_rows {
                    let _ = writeln!(
                        out,
                        "  {:>20}  {:>18}  {:>18}  {:>14}  {:>14}",
                        truncate_str(&row.experiment, 20),
                        truncate_str(&row.variants, 18),
                        truncate_str(&row.sample_sizes, 18),
                        truncate_str(&row.winner, 14),
                        truncate_str(&row.significance, 14)
                    );
                }
            }
        } else {
            let _ = writeln!(out, "  no experiment store");
        }

        let trends = learning_trend_series_snapshot(&self.efficiency_events);
        let _ = writeln!(out);
        let _ = writeln!(out, "efficiency trends:");
        let _ = writeln!(
            out,
            "  cost / task (7d): {}",
            format_series(&trends.cost_per_task)
        );
        let _ = writeln!(
            out,
            "  tokens / task: {}",
            format_series(&trends.tokens_per_task)
        );
        let _ = writeln!(
            out,
            "  success rate: {}",
            format_series(&trends.success_rate)
        );
        let _ = writeln!(
            out,
            "  first-try rate: {}",
            format_series(&trends.first_try_rate)
        );

        Some(out)
    }

    fn render_optimizer_page(&self, page: &PageScaffold) -> Option<String> {
        let at = self.adaptive_thresholds.as_ref()?;
        let mut out = page_header(page);

        // Show EMA confidence per rung.
        let _ = writeln!(out, "gate EMA confidence by rung:");
        let _ = writeln!(
            out,
            "  {:>5}  {:>12}  {:>12}  {:>6}",
            "rung", "ema pass", "observations", "retries"
        );
        let mut rungs: Vec<_> = at.all_rungs().collect();
        rungs.sort_by_key(|(r, _)| *r);
        for (rung, stats) in &rungs {
            let retries = at.suggested_max_retries(**rung);
            let _ = writeln!(
                out,
                "  {:>5}  {:>12}  {:>12}  {:>6}",
                rung,
                format_pct(stats.ema_pass_rate),
                stats.total_observations,
                retries
            );
        }

        // Overall optimization state.
        let _ = writeln!(out);
        let total_obs: u64 = rungs.iter().map(|(_, s)| s.total_observations).sum();
        let avg_ema: f64 = if rungs.is_empty() {
            0.0
        } else {
            rungs.iter().map(|(_, s)| s.ema_pass_rate).sum::<f64>() / count_to_f64(rungs.len())
        };
        let _ = writeln!(out, "optimization cycle:");
        let _ = writeln!(out, "  total observations: {total_obs}");
        let _ = writeln!(out, "  avg ema pass rate: {}", format_pct(avg_ema));
        let skippable: usize = rungs
            .iter()
            .filter(|(r, _)| at.should_skip_rung(**r))
            .count();
        let _ = writeln!(out, "  skippable rungs: {} / {}", skippable, rungs.len());

        // Experiment store summary if present.
        if let Some(store) = &self.experiments {
            let _ = writeln!(out, "  active experiments: {}", store.running_count());
            let _ = writeln!(out, "  concluded experiments: {}", store.concluded_count());
        }

        Some(out)
    }

    // ── Operations pages ────────────────────────────────────────────

    fn render_agent_status_page(&self, page: &PageScaffold) -> Option<String> {
        let mut active_agents = Vec::new();
        let mut seen_agents = HashSet::new();
        for episode in &self.episodes {
            if seen_agents.insert(episode.agent_id.clone()) {
                active_agents.push(AgentSummary {
                    id: episode.agent_id.clone(),
                    label: episode.agent_id.clone(),
                    plan_id: None,
                    status: String::from("active"),
                });
            }
        }
        let snapshot = build_agent_activity_snapshot(&active_agents, &self.efficiency_events)?;

        let mut out = page_header(page);
        let _ = writeln!(out, "active agents:");
        let _ = writeln!(
            out,
            "  {:>20}  {:>14}  {:>16}  {:>12}  {:>5}  {:>12}  {:>10}  {:>10}",
            "agent id", "model", "task", "role", "turns", "tokens used", "cost", "uptime"
        );
        for row in &snapshot.active_agents {
            let _ = writeln!(
                out,
                "  {:>20}  {:>14}  {:>16}  {:>12}  {:>5}  {:>12}  {:>10}  {:>10}",
                truncate_str(&row.agent_id, 20),
                truncate_str(&row.model, 14),
                truncate_str(&row.task, 16),
                truncate_str(&row.role, 12),
                row.turns,
                row.tokens_used,
                format_usd(row.cost_usd),
                format_duration_ms(row.uptime_ms)
            );
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "model distribution:");
        let _ = writeln!(out, "  {:>10}  {:>8}", "model", "count");
        for row in &snapshot.model_usage {
            let _ = writeln!(out, "  {:>10}  {:>8}", row.label, row.count);
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "cost breakdown:");
        let _ = writeln!(
            out,
            "  {:>20}  {:>12}  {:>12}  {:>12}",
            "model", "input tokens", "output tokens", "cost"
        );
        for row in &snapshot.cost_rows {
            let _ = writeln!(
                out,
                "  {:>20}  {:>12}  {:>12}  {:>12}",
                truncate_str(&row.model, 20),
                row.input_tokens,
                row.output_tokens,
                format_usd(row.cost_usd)
            );
        }
        let _ = writeln!(
            out,
            "  total session cost: {}",
            format_usd(snapshot.total_session_cost)
        );

        Some(out)
    }

    fn render_plan_view_page(&self, page: &PageScaffold) -> Option<String> {
        // Try to load executor state from .roko/state/executor.json.
        let state_path = self.root.join(".roko").join("state").join("executor.json");
        let state_text = std::fs::read_to_string(&state_path).ok()?;
        let state: serde_json::Value = serde_json::from_str(&state_text).ok()?;

        let mut out = page_header(page);
        let _ = writeln!(out, "source: {}", state_path.display());

        // Show task list from the executor state.
        if let Some(tasks) = state.get("tasks").and_then(|t| t.as_array()) {
            let total = tasks.len();
            let done = tasks
                .iter()
                .filter(|t| t.get("status").and_then(|s| s.as_str()) == Some("done"))
                .count();
            let failed = tasks
                .iter()
                .filter(|t| t.get("status").and_then(|s| s.as_str()) == Some("failed"))
                .count();
            let running = tasks
                .iter()
                .filter(|t| t.get("status").and_then(|s| s.as_str()) == Some("running"))
                .count();
            let pending = total - done - failed - running;
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "tasks: {total} total, {done} done, {failed} failed, {running} running, {pending} pending"
            );
            let _ = writeln!(out);

            // Task table.
            let _ = writeln!(out, "  {:>4}  {:>10}  {:>30}", "idx", "status", "id");
            for (i, task) in tasks.iter().enumerate() {
                let status = task
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                let id = task.get("id").and_then(|s| s.as_str()).unwrap_or("-");
                let _ = writeln!(out, "  {:>4}  {:>10}  {:>30}", i, status, id);
            }
        } else {
            let _ = writeln!(out, "(no task data in executor state)");
        }

        Some(out)
    }

    fn render_log_view_page(&self, page: &PageScaffold) -> Option<String> {
        let signals_path = self.root.join(".roko").join("signals.jsonl");
        let episodes_path = self.root.join(MEMORY_DIR).join(EPISODES_FILE);

        let signals_exist = signals_path.exists();
        let episodes_exist = episodes_path.exists();
        if !signals_exist && !episodes_exist && self.episodes.is_empty() {
            return None;
        }

        let mut out = page_header(page);

        // Show last N episodes.
        let tail_n = 20;
        let _ = writeln!(out, "recent episodes (last {tail_n}):");
        let start = self.episodes.len().saturating_sub(tail_n);
        if self.episodes.is_empty() {
            let _ = writeln!(out, "  (none)");
        } else {
            let _ = writeln!(
                out,
                "  {:>20}  {:>24}  {:>8}  {:>9}  {:>10}",
                "timestamp", "agent", "task", "success", "cost"
            );
            for ep in &self.episodes[start..] {
                let ts = ep.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
                let _ = writeln!(
                    out,
                    "  {:>20}  {:>24}  {:>8}  {:>9}  {:>10}",
                    ts,
                    truncate_str(&ep.agent_id, 24),
                    truncate_str(&ep.task_id, 8),
                    if ep.success { "pass" } else { "FAIL" },
                    format_usd(ep.usage.cost_usd)
                );
            }
        }

        // Show last N signals if the file exists.
        if signals_exist {
            let _ = writeln!(out);
            let _ = writeln!(out, "recent signals (last {tail_n}):");
            if let Ok(text) = std::fs::read_to_string(&signals_path) {
                let lines: Vec<&str> = text.lines().collect();
                let start = lines.len().saturating_sub(tail_n);
                for line in &lines[start..] {
                    let _ = writeln!(out, "  {line}");
                }
            }
        }

        Some(out)
    }

    fn render_signals_page(&self, page: &PageScaffold) -> Option<String> {
        if self.recent_signals.is_empty() {
            return None;
        }

        let mut signals = self.recent_signals.clone();
        signals.sort_by(|a, b| {
            b.created_at_ms
                .cmp(&a.created_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut out = page_header(page);
        let _ = writeln!(
            out,
            "source: {}/signals.jsonl",
            self.root.join(".roko").display()
        );
        let _ = writeln!(out, "window: last {} signals", signals.len());

        let _ = writeln!(out);
        let _ = writeln!(out, "recent signals:");
        let _ = writeln!(
            out,
            "  {:>8}  {:>18}  {:>18}  {:>60}",
            "time", "kind", "plan/task", "payload preview"
        );
        for signal in signals.iter().take(20) {
            let time = signal_relative_age(signal.created_at_ms);
            let plan_task = signal
                .plan_id
                .as_deref()
                .or(signal.task_id.as_deref())
                .unwrap_or("-");
            let _ = writeln!(
                out,
                "  {:>8}  {:>18}  {:>18}  {:>60}",
                truncate_str(&time, 8),
                truncate_str(&signal.kind, 18),
                truncate_str(plan_task, 18),
                truncate_str(&signal.payload_preview, 60)
            );
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "signal kind distribution:");
        let distribution = signal_kind_distribution(&signals);
        if distribution.is_empty() {
            let _ = writeln!(out, "  (none)");
        } else {
            for (kind, count) in distribution {
                let _ = writeln!(out, "  {:>18}  {:>6}", kind, count);
            }
        }

        let _ = writeln!(out);
        let _ = writeln!(out, "signal DAG explorer:");
        if let Some(selected) = signals.first() {
            let _ = writeln!(
                out,
                "  selected: {} ({})",
                truncate_str(&selected.kind, 24),
                truncate_str(&selected.id, 16)
            );
            for (depth, node) in signal_parent_chain(&signals, selected)
                .into_iter()
                .enumerate()
            {
                let indent = "  ".repeat(depth + 1);
                let label = match node.signal {
                    Some(signal) => format!(
                        "{} [{}] {}",
                        truncate_str(&signal.kind, 24),
                        truncate_str(&signal.id, 16),
                        signal_relative_age(signal.created_at_ms)
                    ),
                    None => truncate_str(&node.hash, 48),
                };
                let _ = writeln!(out, "{indent}- {label}");
            }
        }

        Some(out)
    }

    fn render_config_view_page(&self, page: &PageScaffold) -> Option<String> {
        let config_path = self.root.join("roko.toml");
        let text = std::fs::read_to_string(&config_path).ok()?;

        let mut out = page_header(page);
        let _ = writeln!(out, "source: {}", config_path.display());
        let _ = writeln!(out);

        // Render with section annotations.
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let _ = writeln!(out, "## {line}");
            } else {
                let _ = writeln!(out, "  {line}");
            }
        }

        Some(out)
    }
}

#[derive(Debug, Clone)]
struct LearningCascadeRowSnapshot {
    model: String,
    weight: f64,
    ucb_score: f64,
}

#[derive(Debug, Clone)]
struct LearningExperimentRowSnapshot {
    experiment: String,
    variants: String,
    sample_sizes: String,
    winner: String,
    significance: String,
}

#[derive(Debug, Clone, Default)]
struct LearningTrendSeriesSnapshot {
    cost_per_task: Vec<u64>,
    tokens_per_task: Vec<u64>,
    success_rate: Vec<u64>,
    first_try_rate: Vec<u64>,
}

#[derive(Debug, Clone, Default)]
struct LearningTaskAggregateSnapshot {
    cost_usd: f64,
    tokens: u64,
    first_timestamp: Option<DateTime<Utc>>,
    first_iteration: u32,
    first_passed: bool,
    latest_timestamp: Option<DateTime<Utc>>,
    latest_passed: bool,
}

#[derive(Debug, Clone, Default)]
struct LearningDayAggregateSnapshot {
    tasks: u64,
    cost_usd: f64,
    tokens: u64,
    successes: u64,
    first_try_successes: u64,
}

fn learning_cascade_rows_snapshot(
    snapshot: &CascadeSnapshotData,
) -> Vec<LearningCascadeRowSnapshot> {
    let mut rows = snapshot
        .model_slugs
        .iter()
        .chain(snapshot.confidence_stats.keys())
        .fold(Vec::<String>::new(), |mut acc, slug| {
            if !acc.iter().any(|seen| seen == slug) {
                acc.push(slug.clone());
            }
            acc
        })
        .into_iter()
        .map(|model| {
            let stats = snapshot.confidence_stats.get(&model);
            let trials = stats.map(|stats| stats.trials).unwrap_or_default();
            let successes = stats.map(|stats| stats.successes).unwrap_or_default();
            let ucb_score = confidence_upper_bound_snapshot(trials, successes);
            LearningCascadeRowSnapshot {
                model,
                weight: ucb_score,
                ucb_score,
            }
        })
        .collect::<Vec<_>>();

    let total_weight = rows
        .iter()
        .map(|row| row.weight)
        .sum::<f64>()
        .max(f64::EPSILON);
    for row in &mut rows {
        row.weight /= total_weight;
    }

    rows.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.model.cmp(&b.model))
    });

    rows
}

fn learning_recommendation_counts_snapshot(
    rows: &[LearningCascadeRowSnapshot],
) -> HashMap<String, u64> {
    let mut counts = HashMap::new();
    if rows.is_empty() {
        return counts;
    }

    for category in [
        TaskCategory::Scaffolding,
        TaskCategory::Implementation,
        TaskCategory::Integration,
        TaskCategory::Verification,
        TaskCategory::Research,
        TaskCategory::Refactor,
        TaskCategory::Infra,
        TaskCategory::Docs,
    ] {
        let complexity = complexity_for_category_snapshot(category);
        let tier = tier_for_complexity_snapshot(complexity);
        let selected = select_model_for_tier_snapshot(rows, tier)
            .or_else(|| rows.first())
            .map(|row| row.model.clone());
        if let Some(model) = selected {
            *counts.entry(model).or_default() += 1;
        }
    }

    counts
}

fn learning_experiment_rows_snapshot(
    store: &ExperimentStore,
) -> Vec<LearningExperimentRowSnapshot> {
    let mut rows = store
        .iter()
        .filter(|experiment| experiment.status == ExperimentStatus::Running)
        .map(|experiment| learning_experiment_row_snapshot(experiment))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.experiment.cmp(&b.experiment));
    rows
}

fn learning_experiment_row_snapshot(
    experiment: &PromptExperiment,
) -> LearningExperimentRowSnapshot {
    let mut variants = experiment
        .variants
        .iter()
        .filter(|variant| variant.active)
        .map(|variant| {
            let stats = experiment
                .stats
                .get(&variant.id)
                .cloned()
                .unwrap_or_default();
            (variant, stats)
        })
        .collect::<Vec<_>>();
    variants.sort_by(|(a, _), (b, _)| a.id.cmp(&b.id));

    let sample_sizes = variants
        .iter()
        .map(|(variant, stats)| format!("{}={}", variant.id, stats.trials))
        .collect::<Vec<_>>()
        .join(", ");
    let variant_names = variants
        .iter()
        .map(|(variant, _)| variant.name.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let winner = experiment
        .winner_id
        .clone()
        .or_else(|| {
            variants
                .iter()
                .max_by(|(_, a), (_, b)| {
                    a.success_rate()
                        .partial_cmp(&b.success_rate())
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.trials.cmp(&a.trials))
                })
                .map(|(variant, _)| variant.id.clone())
        })
        .unwrap_or_else(|| String::from("-"));
    let significance = experiment_significance_label_snapshot(experiment, &variants);

    LearningExperimentRowSnapshot {
        experiment: experiment.section_name.clone(),
        variants: if variant_names.is_empty() {
            format!("{} variants", variants.len())
        } else {
            format!("{} variants: {}", variants.len(), variant_names)
        },
        sample_sizes: if sample_sizes.is_empty() {
            String::from("-")
        } else {
            sample_sizes
        },
        winner,
        significance,
    }
}

fn experiment_significance_label_snapshot(
    experiment: &PromptExperiment,
    variants: &[(&PromptVariant, VariantStats)],
) -> String {
    if variants.len() < 2 {
        return String::from("insufficient");
    }

    let mut ranked = variants
        .iter()
        .map(|(variant, stats)| (variant.id.as_str(), stats.clone()))
        .collect::<Vec<_>>();
    ranked.sort_by(|(_, a), (_, b)| {
        b.success_rate()
            .partial_cmp(&a.success_rate())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.trials.cmp(&a.trials))
    });

    let (best_id, best_stats) = &ranked[0];
    let (runner_up_id, runner_up_stats) = &ranked[1];
    let p_value = two_proportion_p_value_snapshot(
        best_stats.successes,
        best_stats.trials,
        runner_up_stats.successes,
        runner_up_stats.trials,
    );
    let gap = best_stats.success_rate() - runner_up_stats.success_rate();
    let significant = p_value
        .map(|p| p < 0.05 && gap >= experiment.min_effect_size)
        .unwrap_or(false);

    match p_value {
        Some(p) if significant => format!("sig p={:.3}", p),
        Some(p) => format!("p={:.3}", p),
        None => format!("n.s. {best_id}/{runner_up_id}"),
    }
}

fn learning_trend_series_snapshot(events: &[AgentEfficiencyEvent]) -> LearningTrendSeriesSnapshot {
    let today = Utc::now().date_naive();
    let mut tasks: HashMap<(String, String), LearningTaskAggregateSnapshot> = HashMap::new();
    for event in events {
        tasks
            .entry((event.plan_id.clone(), event.task_id.clone()))
            .or_default()
            .record(event);
    }

    let mut buckets: BTreeMap<i64, LearningDayAggregateSnapshot> = BTreeMap::new();
    for aggregate in tasks.values() {
        let Some(day) = aggregate.latest_day() else {
            continue;
        };
        let age = today.signed_duration_since(day).num_days();
        if !(0..7).contains(&age) {
            continue;
        }
        let bucket = buckets.entry(age).or_default();
        bucket.tasks += 1;
        bucket.cost_usd += aggregate.cost_usd;
        bucket.tokens += aggregate.tokens;
        if aggregate.latest_passed {
            bucket.successes += 1;
        }
        if aggregate.first_try_passed() {
            bucket.first_try_successes += 1;
        }
    }

    LearningTrendSeriesSnapshot {
        cost_per_task: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    ((bucket.cost_usd / bucket.tasks as f64) * 100.0)
                        .round()
                        .max(0.0) as u64
                }
            })
            .collect(),
        tokens_per_task: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    bucket.tokens / bucket.tasks
                }
            })
            .collect(),
        success_rate: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    ((bucket.successes as f64 / bucket.tasks as f64) * 100.0).round() as u64
                }
            })
            .collect(),
        first_try_rate: (0..7)
            .rev()
            .map(|age| {
                let bucket = buckets.get(&age).cloned().unwrap_or_default();
                if bucket.tasks == 0 {
                    0
                } else {
                    ((bucket.first_try_successes as f64 / bucket.tasks as f64) * 100.0).round()
                        as u64
                }
            })
            .collect(),
    }
}

impl LearningTaskAggregateSnapshot {
    fn record(&mut self, event: &AgentEfficiencyEvent) {
        self.cost_usd += event.cost_usd;
        self.tokens += event.total_tokens();

        let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) else {
            return;
        };

        if self.first_timestamp.map_or(true, |first| timestamp < first) {
            self.first_timestamp = Some(timestamp);
            self.first_iteration = event.iteration;
            self.first_passed = event.gate_passed;
        }

        if self
            .latest_timestamp
            .map_or(true, |latest| timestamp > latest)
        {
            self.latest_timestamp = Some(timestamp);
            self.latest_passed = event.gate_passed;
        }
    }

    fn latest_day(&self) -> Option<chrono::NaiveDate> {
        self.latest_timestamp
            .map(|timestamp| timestamp.date_naive())
    }

    fn first_try_passed(&self) -> bool {
        self.first_iteration == 1 && self.first_passed
    }
}

fn confidence_upper_bound_snapshot(trials: u64, successes: u64) -> f64 {
    if trials == 0 {
        return 1.0;
    }

    let p = successes as f64 / trials as f64;
    let width = 1.96 * (p * (1.0 - p) / trials as f64).sqrt();
    (p + width).min(1.0)
}

fn select_model_for_tier_snapshot<'a>(
    rows: &'a [LearningCascadeRowSnapshot],
    tier: &str,
) -> Option<&'a LearningCascadeRowSnapshot> {
    rows.iter()
        .filter(|row| tier_for_model_snapshot(&row.model) == tier)
        .max_by(|a, b| {
            a.weight
                .partial_cmp(&b.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.model.cmp(&b.model))
        })
}

fn complexity_for_category_snapshot(category: TaskCategory) -> TaskComplexityBand {
    match category {
        TaskCategory::Scaffolding | TaskCategory::Docs => TaskComplexityBand::Fast,
        TaskCategory::Research | TaskCategory::Refactor => TaskComplexityBand::Complex,
        TaskCategory::Implementation
        | TaskCategory::Integration
        | TaskCategory::Verification
        | TaskCategory::Infra => TaskComplexityBand::Standard,
        _ => TaskComplexityBand::Standard,
    }
}

fn tier_for_complexity_snapshot(complexity: TaskComplexityBand) -> &'static str {
    match complexity {
        TaskComplexityBand::Fast => "fast",
        TaskComplexityBand::Complex => "premium",
        _ => "standard",
    }
}

fn tier_for_model_snapshot(model: &str) -> &'static str {
    let lower = model.to_ascii_lowercase();
    if lower.contains("haiku") {
        "fast"
    } else if lower.contains("opus") || lower.contains("premium") {
        "premium"
    } else {
        "standard"
    }
}

fn format_series(values: &[u64]) -> String {
    values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn two_proportion_p_value_snapshot(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    let z = two_proportion_z_score_snapshot(successes_a, trials_a, successes_b, trials_b)?;
    Some(2.0 * (1.0 - standard_normal_cdf_snapshot(z.abs())))
}

fn two_proportion_z_score_snapshot(
    successes_a: u64,
    trials_a: u64,
    successes_b: u64,
    trials_b: u64,
) -> Option<f64> {
    if trials_a == 0 || trials_b == 0 {
        return None;
    }

    let p1 = successes_a as f64 / trials_a as f64;
    let p2 = successes_b as f64 / trials_b as f64;
    let pooled = (successes_a + successes_b) as f64 / (trials_a + trials_b) as f64;
    let standard_error =
        (pooled * (1.0 - pooled) * (1.0 / trials_a as f64 + 1.0 / trials_b as f64)).sqrt();
    if standard_error == 0.0 {
        return None;
    }

    Some((p1 - p2) / standard_error)
}

fn standard_normal_cdf_snapshot(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.231_641_9 * x.abs());
    let d = 0.398_942_3 * (-0.5 * x * x).exp();
    let prob = d
        * t
        * (0.319_381_5 + t * (-0.356_563_8 + t * (1.781_478 + t * (-1.821_256 + t * 1.330_274))));
    if x >= 0.0 { 1.0 - prob } else { prob }
}

/// Per-agent aggregated stats.
/// Render standard page header.
fn page_header(page: &PageScaffold) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} ({})", page.title, page.id.slug());
    let _ = writeln!(out, "group: {}", page.id.group());
    let _ = writeln!(out, "intent: {}", page.intent);
    out
}

/// Truncate a string to `max` chars, adding "..." if truncated.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

fn signal_relative_age(created_at_ms: i64) -> String {
    let created_at_ms = u64::try_from(created_at_ms).unwrap_or_default();
    let age_ms = now_ms().saturating_sub(created_at_ms);
    format_elapsed_ms(age_ms)
}

fn signal_kind_prefix(kind: &str) -> String {
    kind.split(':').next().unwrap_or(kind).to_string()
}

fn signal_kind_distribution(signals: &[SignalSummary]) -> Vec<(String, u64)> {
    let mut counts = BTreeMap::<String, u64>::new();
    for signal in signals {
        *counts.entry(signal_kind_prefix(&signal.kind)).or_default() += 1;
    }

    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

#[derive(Debug)]
struct SignalTreeEntry<'a> {
    hash: String,
    signal: Option<&'a SignalSummary>,
}

fn signal_parent_chain<'a>(
    signals: &'a [SignalSummary],
    selected: &'a SignalSummary,
) -> Vec<SignalTreeEntry<'a>> {
    let by_id = signals
        .iter()
        .map(|signal| (signal.id.as_str(), signal))
        .collect::<HashMap<_, _>>();

    let mut chain = Vec::new();
    chain.push(SignalTreeEntry {
        hash: selected.id.clone(),
        signal: Some(selected),
    });

    let ancestors: Vec<String> = if selected.lineage.is_empty() {
        selected.parent_hash.iter().cloned().collect()
    } else {
        selected.lineage.iter().rev().cloned().collect()
    };

    for hash in ancestors {
        let signal = by_id.get(hash.as_str()).copied();
        chain.push(SignalTreeEntry { hash, signal });
    }

    chain
}

fn load_snapshot_best_effort(root: &Path) -> DashboardSnapshot {
    load_snapshot_blocking(root).unwrap_or_else(|_| DashboardSnapshot::empty(root.to_path_buf()))
}

fn load_snapshot_blocking(root: &Path) -> Result<DashboardSnapshot, std::io::Error> {
    let root = root.to_path_buf();
    let load = move || -> Result<DashboardSnapshot, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(std::io::Error::other)?;
        runtime.block_on(DashboardSnapshot::load(&root))
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(load)
            .join()
            .map_err(|_| std::io::Error::other("dashboard snapshot loader panicked"))?
    } else {
        load()
    }
}

fn resolve_snapshot_root(start: &Path) -> PathBuf {
    let mut cursor = Some(start);
    while let Some(dir) = cursor {
        let memory_dir = dir.join(MEMORY_DIR);
        if memory_dir.join(EPISODES_FILE).exists() || memory_dir.join(TASK_METRICS_FILE).exists() {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    start.to_path_buf()
}

async fn read_task_metrics(path: &Path) -> Result<Vec<TaskMetric>, std::io::Error> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut metrics = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(metric) = TaskMetric::from_jsonl(line) {
            metrics.push(metric);
        }
    }
    Ok(metrics)
}

fn format_pct(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn format_float(value: f64) -> String {
    format!("{value:.2}")
}

fn format_usd(value: f64) -> String {
    format!("${value:.4}")
}

fn format_ms(value: f64) -> String {
    format!("{value:.0} ms")
}

fn valence_indicator(pleasure: f64) -> &'static str {
    if pleasure >= 0.25 {
        "😊"
    } else if pleasure <= -0.25 {
        "😟"
    } else {
        "😐"
    }
}

fn count_to_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn wall_ms_to_f64(wall_ms: u64) -> f64 {
    f64::from(u32::try_from(wall_ms).unwrap_or(u32::MAX))
}

/// Read efficiency events from JSONL (best-effort, returns empty on error).
async fn read_efficiency_events(path: &Path) -> Vec<AgentEfficiencyEvent> {
    let Ok(text) = tokio::fs::read_to_string(path).await else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Best-effort JSON file loader. Returns `None` if missing or corrupt.
fn load_json_opt<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use tempfile::tempdir;

    fn write_jsonl(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().expect("file has parent"))
            .expect("should create parent dir");
        fs::write(path, lines.join("\n") + "\n").expect("should write jsonl");
    }

    fn sample_episode(
        agent: &str,
        task: &str,
        success: bool,
        cost_usd: f64,
        wall_ms: u64,
    ) -> Episode {
        let mut episode = Episode::new(agent, task);
        episode.success = success;
        episode.usage.cost_usd = cost_usd;
        episode.usage.wall_ms = wall_ms;
        episode
    }

    fn sample_metric(
        plan: &str,
        task: &str,
        iteration: u32,
        passed: bool,
        model: &str,
        input_tokens: u64,
        cache_hit_rate: f64,
        cost_usd: f64,
    ) -> TaskMetric {
        let mut metric = TaskMetric::new(
            roko_core::metric::ConfigHash::from("hash".to_string()),
            plan,
            task,
        );
        metric.iteration = iteration;
        metric.gate_passed = passed;
        metric.model = model.to_string();
        metric.input_tokens = input_tokens;
        metric.cached_tokens = (input_tokens as f64 * cache_hit_rate).round() as u64;
        metric.cache_hit_rate = cache_hit_rate;
        metric.cost_usd = cost_usd;
        metric
    }

    fn sample_efficiency_event(
        agent: &str,
        task: &str,
        role: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        timestamp: &str,
    ) -> AgentEfficiencyEvent {
        AgentEfficiencyEvent {
            agent_id: agent.to_string(),
            role: role.to_string(),
            backend: String::from("claude"),
            model: model.to_string(),
            plan_id: String::from("plan-a"),
            task_id: task.to_string(),
            input_tokens,
            output_tokens,
            reasoning_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd,
            cost_usd_without_cache: cost_usd,
            prompt_sections: Vec::new(),
            total_prompt_tokens: input_tokens,
            system_prompt_tokens: 0,
            tools_available: 0,
            tools_used: 0,
            tool_calls: Vec::new(),
            wall_time_ms: 1_000,
            duration_ms: 1_000,
            time_to_first_token_ms: 0,
            was_warm_start: true,
            iteration: 1,
            gate_passed: true,
            outcome: "success".to_string(),
            gate_errors: Vec::new(),
            model_used: model.to_string(),
            frequency: OperatingFrequency::Theta,
            strategy_attempted: "none".to_string(),
            timestamp: timestamp.to_string(),
        }
    }

    #[test]
    fn scaffold_has_expected_page_count() {
        let dashboard = DashboardScaffold::new();
        let summary = dashboard.summary();
        assert_eq!(summary.page_count, 13);
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
    fn theme_defaults_to_a_dark_terminal_palette() {
        let theme = Theme::from_no_color(false);
        assert_eq!(theme.foreground, Color::White);
        assert_eq!(theme.background, Color::Black);
        assert_eq!(theme.accent, Color::Cyan);
        assert_eq!(theme.selection_background, Color::Cyan);
        assert_eq!(theme.selection_foreground, Color::Black);
    }

    #[test]
    fn theme_disables_color_when_requested() {
        let theme = Theme::from_no_color(true);
        assert_eq!(theme.foreground, Color::Reset);
        assert_eq!(theme.background, Color::Reset);
        assert_eq!(theme.accent, Color::Reset);
        assert_eq!(theme.selection_background, Color::Reset);
        assert_eq!(theme.selection_foreground, Color::Reset);
    }

    #[test]
    fn overview_render_contains_active_page_and_counts() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard.render_overview_text();
        assert!(rendered.contains("dashboard scaffold: 13 pages"));
        assert!(rendered.contains("active=health"));
        assert!(rendered.contains("active page:"));
        assert!(rendered.contains("* Health [health] efficiency"));
    }

    #[test]
    fn page_render_includes_widgets() {
        let tmpdir = tempdir().expect("tempdir");
        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::PlanView)
            .expect("plan page should exist");
        assert!(rendered.contains("Plan View (plan-view)"));
        assert!(rendered.contains("widgets (2):"));
        assert!(rendered.contains("DAG [dag]"));
    }

    #[test]
    fn signals_page_renders_recent_signals_and_tree() {
        let tmpdir = tempdir().expect("tempdir");
        let roko_dir = tmpdir.path().join(".roko");
        let memory_dir = roko_dir.join("memory");
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let signals = vec![
            serde_json::json!({
                "id": "sig-1",
                "kind": "gate:compile",
                "created_at_ms": 1_700_000_000_000i64,
                "lineage": [],
                "tags": {
                    "plan_id": "plan-a",
                    "task_id": "task-a"
                },
                "body": {
                    "format": "json",
                    "data": {
                        "message": "compile ok",
                        "detail": "payload"
                    }
                }
            }),
            serde_json::json!({
                "id": "sig-2",
                "kind": "conductor:alert:warning",
                "created_at_ms": 1_700_000_001_000i64,
                "lineage": ["sig-1"],
                "body": {
                    "format": "text",
                    "data": "selected payload text that should preview nicely"
                }
            }),
        ];
        write_jsonl(
            &roko_dir.join("signals.jsonl"),
            &signals
                .into_iter()
                .map(|signal| serde_json::to_string(&signal).expect("signal json"))
                .collect::<Vec<_>>(),
        );

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::Signals)
            .expect("signals page should render");
        assert!(rendered.contains("Signals (signals)"));
        assert!(rendered.contains("recent signals:"));
        assert!(rendered.contains("kind distribution:"));
        assert!(rendered.contains("signal DAG explorer:"));
        assert!(rendered.contains("sig-2"));
        assert!(rendered.contains("sig-1"));
    }

    #[test]
    fn page_index_render_contains_compact_summaries() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard.render_page_index_text();
        assert!(rendered.contains("* Health [health] efficiency | 3 widgets"));
        assert!(rendered.contains("Plan View [plan-view] operations | 2 widgets"));
    }

    #[test]
    fn page_list_render_focuses_on_one_page_widget_list() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard
            .render_page_list_text(PageId::ConfigView)
            .expect("config page should exist");
        assert!(rendered.contains("Config View [config-view]"));
        assert!(rendered.contains("widgets (2):"));
        assert!(rendered.contains("Effective Config [effective_config]"));
    }

    #[test]
    fn snapshot_loader_aggregates_episode_and_metric_stats() {
        let tempdir = tempdir().expect("tempdir");
        let memory_dir = tempdir.path().join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let metrics_path = memory_dir.join(TASK_METRICS_FILE);

        let episodes = vec![
            serde_json::to_string(&sample_episode("agent-a", "task-a", true, 1.50, 1_000))
                .expect("episode json"),
            serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.50, 3_000))
                .expect("episode json"),
        ];
        write_jsonl(&episodes_path, &episodes);

        let metrics = vec![
            sample_metric("plan-a", "t1", 1, true, "claude-haiku-4-5", 100, 0.20, 0.10),
            sample_metric(
                "plan-a",
                "t1",
                2,
                false,
                "claude-sonnet-4-5",
                300,
                0.50,
                0.20,
            ),
            sample_metric("plan-b", "t2", 1, true, "claude-haiku-4-5", 200, 0.25, 0.30),
        ];
        write_jsonl(
            &metrics_path,
            &metrics
                .iter()
                .map(|metric| metric.to_jsonl().expect("metric json"))
                .collect::<Vec<_>>(),
        );

        let snapshot = load_snapshot_blocking(tempdir.path()).expect("snapshot should load");

        assert_eq!(snapshot.episode_count, 2);
        assert_eq!(snapshot.task_metric_count, 3);
        assert_eq!(snapshot.success_rate, Some(0.5));
        assert!((snapshot.average_cost_usd.expect("avg cost") - 1.0).abs() < 1e-9);
        assert!((snapshot.average_wall_time_ms.expect("avg wall") - 2_000.0).abs() < 1e-9);
        assert!((snapshot.haiku_share.expect("haiku share") - (2.0 / 3.0)).abs() < 1e-9);
        assert!((snapshot.cache_hit_rate.expect("cache hit") - (0.95 / 3.0)).abs() < 1e-9);
        assert_eq!(snapshot.headlines.n_plans, 2);
        assert_eq!(snapshot.headlines.n_records, 3);
        assert!((snapshot.headlines.first_attempt_pass_rate - 1.0).abs() < 1e-9);
        assert!((snapshot.headlines.avg_iterations_per_plan - 1.5).abs() < 1e-9);
    }

    #[test]
    fn health_and_trends_render_real_stats_when_snapshot_exists() {
        let tempdir = tempdir().expect("tempdir");
        let memory_dir = tempdir.path().join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let metrics_path = memory_dir.join(TASK_METRICS_FILE);

        write_jsonl(
            &episodes_path,
            &[
                serde_json::to_string(&sample_episode("agent-a", "task-a", true, 1.50, 1_000))
                    .expect("episode json"),
                serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.50, 3_000))
                    .expect("episode json"),
            ],
        );
        write_jsonl(
            &metrics_path,
            &[
                sample_metric("plan-a", "t1", 1, true, "claude-haiku-4-5", 100, 0.20, 0.10)
                    .to_jsonl()
                    .expect("metric json"),
                sample_metric(
                    "plan-a",
                    "t1",
                    2,
                    false,
                    "claude-sonnet-4-5",
                    300,
                    0.50,
                    0.20,
                )
                .to_jsonl()
                .expect("metric json"),
                sample_metric("plan-b", "t2", 1, true, "claude-haiku-4-5", 200, 0.25, 0.30)
                    .to_jsonl()
                    .expect("metric json"),
            ],
        );

        let dashboard = DashboardScaffold::new_in(tempdir.path());
        let health = dashboard.render_health_page_text();
        let trends = dashboard.render_trends_page_text();

        assert!(health.contains("episodes: 2"));
        assert!(health.contains("success rate: 50.0%"));
        assert!(health.contains("average cost: $1.0000"));
        assert!(health.contains("average wall time: 2000 ms"));
        assert!(health.contains("haiku share: 66.7%"));
        assert!(health.contains("cache hit rate: 31.7%"));

        assert!(trends.contains("task metrics: 3"));
        assert!(trends.contains("first-attempt pass rate: 100.0%"));
        assert!(trends.contains("avg iterations per plan: 1.50"));
        assert!(trends.contains("avg cost per plan: $0.3000"));
        assert!(trends.contains("avg input tokens per spawn: 200.00"));
        assert!(trends.contains("haiku share: 66.7%"));
        assert!(trends.contains("cache hit rate: 31.7%"));
        assert!(trends.contains("- avg_cost_per_plan: $0.3000"));
    }

    fn write_json(path: &Path, value: &impl serde::Serialize) {
        fs::create_dir_all(path.parent().expect("file has parent"))
            .expect("should create parent dir");
        let json = serde_json::to_string_pretty(value).expect("should serialize");
        fs::write(path, json).expect("should write json");
    }

    #[test]
    fn parameters_page_renders_cascade_and_thresholds() {
        let tmpdir = tempdir().expect("tempdir");
        let learn_dir = tmpdir.path().join(".roko/learn");

        // Write cascade router data.
        let cascade = serde_json::json!({
            "model_slugs": ["claude-sonnet-4-5", "claude-haiku-4-5"],
            "confidence_stats": {
                "claude-sonnet-4-5": { "trials": 50, "successes": 45 },
                "claude-haiku-4-5": { "trials": 30, "successes": 20 }
            }
        });
        write_json(&learn_dir.join(CASCADE_ROUTER_FILE), &cascade);

        // Write adaptive thresholds.
        let thresholds = AdaptiveThresholds::default();
        write_json(&learn_dir.join(GATE_THRESHOLDS_FILE), &thresholds);

        // Need memory dir to exist for the root resolver.
        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::Parameters)
            .expect("parameters page should render");
        assert!(rendered.contains("Parameters"));
        assert!(rendered.contains("cascade router:"));
        assert!(rendered.contains("registered models: 2"));
    }

    #[test]
    fn gate_results_page_renders_summary_thresholds_and_failures() {
        let tmpdir = tempdir().expect("tempdir");
        let roko_dir = tmpdir.path().join(".roko");
        let learn_dir = roko_dir.join("learn");
        fs::create_dir_all(&learn_dir).expect("learn dir");

        let mut thresholds = AdaptiveThresholds::new();
        thresholds.update(0, true);
        thresholds.update(0, false);
        thresholds.update(1, true);
        write_json(&learn_dir.join(GATE_THRESHOLDS_FILE), &thresholds);

        let signals = vec![
            serde_json::json!({
                "id": "sig-1",
                "kind": "gate:compile",
                "created_at_ms": 1_700_000_000_000i64,
                "tags": {
                    "gate": "compile",
                    "plan_id": "plan-a",
                    "task_id": "task-a",
                    "passed": "true",
                    "duration_ms": 120
                }
            }),
            serde_json::json!({
                "id": "sig-2",
                "kind": "gate:test",
                "created_at_ms": 1_700_000_000_500i64,
                "tags": {
                    "gate": "test",
                    "plan_id": "plan-a",
                    "task_id": "task-b",
                    "passed": "false",
                    "duration_ms": 340
                },
                "body": {
                    "data": {
                        "reason": "assertion failed on line 42\nmore detail"
                    }
                }
            }),
        ];
        write_jsonl(
            &roko_dir.join("signals.jsonl"),
            &signals
                .into_iter()
                .map(|signal| serde_json::to_string(&signal).expect("signal json"))
                .collect::<Vec<_>>(),
        );

        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::GateResults)
            .expect("gate results page should render");
        assert!(rendered.contains("Gate Results"));
        assert!(rendered.contains("gate summary:"));
        assert!(rendered.contains("adaptive thresholds:"));
        assert!(rendered.contains("recent gate failures:"));
        assert!(rendered.contains("compile"));
        assert!(rendered.contains("task-b"));
        assert!(rendered.contains("assertion failed on line 42"));
    }

    #[test]
    fn experiments_page_renders_with_store() {
        let tmpdir = tempdir().expect("tempdir");
        let learn_dir = tmpdir.path().join(".roko/learn");

        // Write experiment store as raw JSON matching ExperimentStore structure.
        let store_json = serde_json::json!({
            "experiments": {
                "exp-1": {
                    "experiment_id": "exp-1",
                    "section_name": "system_prompt",
                    "variants": [
                        { "id": "baseline", "name": "Baseline", "section_name": "system_prompt", "content": "v1", "active": true },
                        { "id": "verbose", "name": "Verbose", "section_name": "system_prompt", "content": "v2", "active": true }
                    ],
                    "stats": {
                        "baseline": { "trials": 10, "successes": 8 },
                        "verbose": { "trials": 10, "successes": 5 }
                    },
                    "status": "Running",
                    "winner_id": null,
                    "min_trials_per_variant": 20,
                    "min_effect_size": 0.1
                }
            }
        });
        write_json(&learn_dir.join(EXPERIMENTS_FILE), &store_json);

        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::Experiments)
            .expect("experiments page should render");
        assert!(rendered.contains("Experiments"));
        assert!(rendered.contains("system_prompt"));
        assert!(rendered.contains("1 running"));
    }

    #[test]
    fn agent_status_page_renders_with_episodes() {
        let tmpdir = tempdir().expect("tempdir");
        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let learn_dir = tmpdir.path().join(LEARN_DIR);
        let efficiency_path = learn_dir.join(EFFICIENCY_FILE);

        let episodes = vec![
            serde_json::to_string(&sample_episode("agent-a", "task-1", true, 0.5, 500))
                .expect("json"),
            serde_json::to_string(&sample_episode("agent-a", "task-2", false, 1.0, 1500))
                .expect("json"),
            serde_json::to_string(&sample_episode("agent-b", "task-3", true, 0.3, 300))
                .expect("json"),
        ];
        write_jsonl(&episodes_path, &episodes);
        write_jsonl(
            &efficiency_path,
            &[
                serde_json::to_string(&sample_efficiency_event(
                    "agent-a",
                    "task-1",
                    "Implementer",
                    "claude-haiku-4-5",
                    120,
                    40,
                    0.10,
                    "2026-04-08T10:00:00Z",
                ))
                .expect("event json"),
                serde_json::to_string(&sample_efficiency_event(
                    "agent-a",
                    "task-2",
                    "Implementer",
                    "claude-sonnet-4-5",
                    300,
                    90,
                    0.30,
                    "2026-04-08T10:05:00Z",
                ))
                .expect("event json"),
                serde_json::to_string(&sample_efficiency_event(
                    "agent-b",
                    "task-3",
                    "Reviewer",
                    "claude-opus-4-6",
                    500,
                    100,
                    1.25,
                    "2026-04-08T10:10:00Z",
                ))
                .expect("event json"),
            ],
        );

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::AgentStatus)
            .expect("agent status page should render");
        assert!(rendered.contains("Agent Activity"));
        assert!(rendered.contains("agent-a"));
        assert!(rendered.contains("agent-b"));
        assert!(rendered.contains("active agents:"));
        assert!(rendered.contains("model distribution:"));
        assert!(rendered.contains("cost breakdown:"));
        assert!(rendered.contains("total session cost:"));
    }

    #[test]
    fn plan_view_renders_with_executor_state() {
        let tmpdir = tempdir().expect("tempdir");
        let state_dir = tmpdir.path().join(".roko/state");
        fs::create_dir_all(&state_dir).expect("state dir");

        let executor_state = serde_json::json!({
            "tasks": [
                { "id": "task-1", "status": "done", "plan": "plan-a" },
                { "id": "task-2", "status": "running", "plan": "plan-a" },
                { "id": "task-3", "status": "pending", "plan": "plan-a" }
            ]
        });
        write_json(&state_dir.join("executor.json"), &executor_state);

        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::PlanView)
            .expect("plan view page should render");
        assert!(rendered.contains("Plan View"));
        assert!(rendered.contains("task-1"));
    }

    #[test]
    fn current_plan_execution_snapshot_uses_tracker_and_episode_tail() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let state_dir = root.join(".roko/state");
        let plan_dir = root.join(".roko/plans/plan-a");
        // load_best_effort resolves episodes via roko_dir.join(MEMORY_DIR)
        // where roko_dir = root/.roko, so episodes end up at root/.roko/.roko/memory/
        let memory_dir = root.join(".roko").join(MEMORY_DIR);

        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&plan_dir).expect("plan dir");
        fs::create_dir_all(&memory_dir).expect("memory dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "implementing" },
                    "started_at_ms": 1_700_000_000_000u64,
                    "assigned_agents": ["agent-a"]
                }
            }
        });
        write_json(&state_dir.join("executor.json"), &executor_state);

        let tracker_state = serde_json::json!([
            {
                "plan_id": "plan-a",
                "completed": ["task-1"],
                "failed": [],
                "current_group_index": 0
            }
        ]);
        write_json(&state_dir.join("task-trackers.json"), &tracker_state);

        fs::write(
            plan_dir.join("tasks.toml"),
            r#"
[meta]
plan = "Plan A"
iteration = 1
total = 2
done = 1
status = "running"

[[task]]
id = "task-1"
title = "Bootstrap"
status = "done"
tier = "focused"
files = ["src/bootstrap.rs"]

  [[task.context.read_files]]
  path = "src/bootstrap.rs"
  why = "history"

[[task]]
id = "task-2"
title = "Wire dashboard"
status = "ready"
tier = "focused"
frequency = "delta"
files = ["src/dashboard.rs"]

  [[task.context.read_files]]
  path = "src/dashboard.rs"
  lines = "1-20"
  why = "current work"
"#,
        )
        .expect("tasks.toml");

        let mut episode = Episode::new("agent-a", "task-2");
        episode.input_signal_hash = "plan-a".to_string();
        episode
            .extra
            .insert("plan_id".to_string(), serde_json::json!("plan-a"));
        episode
            .extra
            .insert("task_id".to_string(), serde_json::json!("task-2"));
        episode.extra.insert(
            "stderr".to_string(),
            serde_json::json!(
                (1..=25)
                    .map(|n| format!("stderr line {n}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        );
        write_jsonl(
            &memory_dir.join("episodes.jsonl"),
            &[serde_json::to_string(&episode).expect("episode json")],
        );

        let data = DashboardData::load_best_effort(root);
        let execution = data
            .current_plan_execution
            .expect("plan execution snapshot should be present");

        assert_eq!(execution.plan_id, "plan-a");
        assert_eq!(execution.plan_title, "Plan A");
        assert_eq!(execution.tasks_done, 1);
        assert_eq!(execution.tasks_total, 2);
        assert_eq!(execution.tasks.len(), 2);
        assert_eq!(execution.tasks[1].frequency, OperatingFrequency::Delta);
        assert_eq!(
            execution
                .current_task
                .as_ref()
                .expect("current task")
                .task_id,
            "task-2"
        );
        assert_eq!(
            execution
                .current_task
                .as_ref()
                .expect("current task")
                .frequency,
            OperatingFrequency::Delta
        );
        assert_eq!(execution.agent_output_tail.len(), 20);
        assert_eq!(
            execution.agent_output_tail.first().expect("tail head"),
            "stderr line 6"
        );
        assert_eq!(
            execution.agent_output_tail.last().expect("tail last"),
            "stderr line 25"
        );
    }

    #[test]
    fn affect_snapshot_renders_valence_indicators() {
        let tmpdir = tempdir().expect("tempdir");
        let learn_dir = tmpdir.path().join(".roko/learn/daimon");
        fs::create_dir_all(&learn_dir).expect("learn dir");
        write_json(
            &learn_dir.join(AFFECT_FILE),
            &serde_json::json!({
                "states": {
                    "agent-happy": {
                        "pleasure": 0.9,
                        "arousal": 0.1,
                        "dominance": 0.2,
                        "confidence": 0.8,
                        "updated_at": "2026-04-10T00:00:00Z"
                    },
                    "plan-sad": {
                        "pleasure": -0.9,
                        "arousal": 0.1,
                        "dominance": -0.2,
                        "confidence": 0.3,
                        "updated_at": "2026-04-10T00:00:00Z"
                    }
                }
            }),
        );

        let data = DashboardData::load_best_effort(tmpdir.path());
        assert_eq!(data.affect_indicator("agent-happy"), "😊");
        assert_eq!(data.affect_indicator("plan-sad"), "😟");
        assert_eq!(data.affect_indicator("missing"), "😐");
    }

    #[test]
    fn config_view_renders_with_roko_toml() {
        let tmpdir = tempdir().expect("tempdir");
        let config_path = tmpdir.path().join("roko.toml");
        fs::write(
            &config_path,
            "[agent]\nmodel = \"claude-sonnet-4-5\"\n\n[gate]\nmax_retries = 3\n",
        )
        .expect("write roko.toml");

        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::ConfigView)
            .expect("config view page should render");
        assert!(rendered.contains("Config View"));
        assert!(rendered.contains("[agent]"));
    }
}
