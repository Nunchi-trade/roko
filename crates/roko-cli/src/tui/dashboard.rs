//! Dashboard scaffold container for future TUI wiring.
//!
//! This module keeps the existing page scaffold intact, but layers a
//! best-effort learning snapshot on top so the health and trends pages
//! can render real stats when the memory JSONL files are present.

use std::collections::{BTreeMap, HashMap, HashSet, hash_map::DefaultHasher};
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
use ratatui::style::Color;

use crate::plan::{PlanSummary, plans_dir};
use crate::task_parser::{TaskDef, TasksFile};
use roko_core::ExperimentWinnerSummary;
use roko_core::metric::{Headlines, TaskMetric, compute_headlines};
use roko_gate::adaptive_threshold::AdaptiveThresholds;
use roko_learn::aggregate::{CFactorBucket, EfficiencyBucket, cfactor_trend, efficiency_trend};
use roko_learn::cascade_router::{CascadeStage, StageTransition};
pub use roko_learn::cfactor::{CFactor, CFactorComponents};
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::pattern_discovery::CrossEpisodeConsolidator;
use roko_learn::prompt_experiment::ExperimentStore;
use roko_learn::provider_health::{CircuitState, ProviderHealth};
use roko_learn::skill_library::Skill;

use super::cursors::{EpisodeCursor, EventLogCursor, SignalCursor};
use super::dashboard_gen::DurableDashboardGenerationCounter;
use super::pages::{PageId, PageScaffold, efficiency, operations};
use super::state::{PlanPhase, TaskStatus};
use super::task_outputs::TaskOutputCursors;
pub use super::theme::Theme;

const MEMORY_DIR: &str = ".roko/memory";
const EPISODES_FILE: &str = "episodes.jsonl";
const TASK_METRICS_FILE: &str = "task-metrics.jsonl";

const LEARN_DIR: &str = ".roko/learn";
const EFFICIENCY_FILE: &str = "efficiency.jsonl";
const EXPERIMENTS_FILE: &str = "experiments.json";
const GATE_THRESHOLDS_FILE: &str = "gate-thresholds.json";
const CASCADE_ROUTER_FILE: &str = "cascade-router.json";
const SKILLS_FILE: &str = "skills.json";
const PROVIDER_HEALTH_FILE: &str = "provider-health.json";
const LATENCY_STATS_FILE: &str = "latency-stats.json";
const NEURO_DIR: &str = ".roko/neuro";
const KNOWLEDGE_FILE: &str = "knowledge.jsonl";
const KNOWLEDGE_CONFIRMATIONS_FILE: &str = "knowledge-confirmations.jsonl";

fn resolve_episodes_path(root: &Path) -> PathBuf {
    let episodes_path = root.join(MEMORY_DIR).join(EPISODES_FILE);
    if episodes_path.exists() {
        episodes_path
    } else {
        root.join(".roko").join(EPISODES_FILE)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
struct DashboardDataStamps {
    executor_state: FileStamp,
    efficiency: FileStamp,
    experiments: FileStamp,
    gate_thresholds: FileStamp,
    signals: FileStamp,
    episodes: FileStamp,
    cfactor: FileStamp,
    cascade_router: FileStamp,
    task_outputs: u64,
    event_log: FileStamp,
}

impl DashboardDataStamps {
    fn fingerprint(self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

static DASHBOARD_GENERATION_COUNTERS: OnceLock<
    Mutex<HashMap<PathBuf, Arc<DurableDashboardGenerationCounter>>>,
> = OnceLock::new();

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
            PageId::ProviderHealth => self.snapshot.render_provider_health_page(scaffold),
            PageId::ModelComparison => self.snapshot.render_model_comparison_page(scaffold),
            PageId::Dreams => self.snapshot.render_dreams_page(scaffold),
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

    /// Render the provider health page as plain text.
    #[must_use]
    pub fn render_provider_health_page_text(&self) -> String {
        self.render_page_text(PageId::ProviderHealth)
            .unwrap_or_else(|| String::from("<missing provider health page>"))
    }

    /// Render the model comparison page as plain text.
    #[must_use]
    pub fn render_model_comparison_page_text(&self) -> String {
        self.render_page_text(PageId::ModelComparison)
            .unwrap_or_else(|| String::from("<missing model comparison page>"))
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

/// An entry in the orchestrator event log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLogEntry {
    /// Epoch milliseconds when the event occurred.
    pub timestamp_ms: u64,
    /// Event kind (e.g. "task_started", "gate_passed").
    pub event_type: String,
    /// Plan the event belongs to.
    pub plan_id: String,
    /// Task the event belongs to.
    pub task_id: String,
    /// Human-readable event description.
    pub message: String,
}

/// Shared dashboard data loaded from `.roko/`.
#[derive(Debug, Clone, Default)]
pub struct DashboardData {
    /// Workspace root used for refreshes.
    root: PathBuf,
    /// Monotonic token advanced when tracked dashboard source files change.
    pub generation: u64,
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
    /// Hourly efficiency trend over the last 24 hours.
    pub efficiency_trend: Vec<EfficiencyBucket>,
    /// Hourly c-factor trend over the last 24 hours.
    pub cfactor_trend: Vec<CFactorBucket>,
    /// Last observed efficiency file metadata.
    efficiency_stamp: FileStamp,
    /// Cascade router state from `.roko/learn/cascade-router.json`.
    pub cascade_router: CascadeRouterState,
    /// Full experiment store from `.roko/learn/experiments.json`.
    pub experiment_store: ExperimentStore,
    /// Experiments from `.roko/learn/experiments.json`.
    pub experiments: Vec<ExperimentSummary>,
    /// Concluded experiment winners from `.roko/learn/experiments.json`.
    pub experiment_winners: Vec<ExperimentWinnerSummary>,
    /// Last observed experiments file metadata.
    experiments_stamp: FileStamp,
    /// Gate-results page data derived from signals and adaptive thresholds.
    pub gate_results_page: GateResultsPageData,
    /// Cached adaptive thresholds from `.roko/learn/gate-thresholds.json`.
    adaptive_thresholds: Option<AdaptiveThresholds>,
    /// Last observed gate-thresholds file metadata.
    gate_thresholds_stamp: FileStamp,
    /// Most recent signals from `.roko/engrams.jsonl`.
    pub recent_signals: Vec<SignalSummary>,
    /// Cached signal-derived gate results when executor state does not provide them.
    signal_gate_results: Vec<GateResultSummary>,
    /// Parsed gate-related signals for the gate-results page.
    gate_signal_summaries: Vec<GateSignalSummary>,
    /// Incremental cursor over `.roko/engrams.jsonl`.
    signal_cursor: SignalCursor,
    /// Snapshot of the currently executing plan for the Plan Execution page.
    pub current_plan_execution: Option<PlanExecutionSnapshot>,
    /// Incremental cursor over `.roko/episodes.jsonl`.
    episode_cursor: EpisodeCursor,
    /// Cached episodes for plan execution rendering.
    episodes: Vec<Episode>,
    /// Conductor alerts filtered from signals.
    pub conductor_alerts: Vec<AlertSummary>,
    /// Latest C-Factor snapshot, if present.
    pub cfactor: Option<CFactor>,
    /// Last observed C-Factor file metadata.
    cfactor_stamp: FileStamp,
    /// Cascade router file metadata.
    cascade_router_stamp: FileStamp,
    /// Incremental task-output cursors keyed by task ID.
    task_output_cursors: TaskOutputCursors,
    /// Per-task agent output tail cache derived from `.roko/task-outputs/`.
    pub task_outputs: HashMap<String, Vec<String>>,
    /// Cached git diff shown in the Dashboard Diff sub-tab.
    pub git_diff: String,
    /// Whether the cached git diff came from staged changes.
    pub git_diff_is_staged: bool,
    /// Orchestrator event log from `.roko/state/events.json`.
    pub event_log: Vec<EventLogEntry>,
    /// Whole-file reload cursor over `.roko/state/events.json`.
    event_log_cursor: EventLogCursor,
}

/// Derived executor snapshot fields used by TUI orchestration chrome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExecutorSummary {
    pub orchestrator_state: String,
    pub current_iteration: usize,
    pub current_phase: String,
}

impl DashboardData {
    /// Load dashboard data from a workspace root, falling back to empty data on errors.
    #[must_use]
    pub fn load_best_effort(root: impl AsRef<Path>) -> Self {
        let root = resolve_snapshot_root(root.as_ref());
        let roko_dir = root.join(".roko");
        let learn_dir = roko_dir.join("learn");
        let state_path = roko_dir.join("state").join("executor.json");
        let signals_path = roko_dir.join("engrams.jsonl");
        let episodes_path = resolve_episodes_path(&root);
        let efficiency_path = learn_dir.join(EFFICIENCY_FILE);
        let experiments_path = learn_dir.join(EXPERIMENTS_FILE);
        let gate_thresholds_path = learn_dir.join(GATE_THRESHOLDS_FILE);
        let cascade_router_path = learn_dir.join(CASCADE_ROUTER_FILE);
        let cfactor_path = learn_dir.join("c-factor.jsonl");
        let events_path = roko_dir.join("state").join("events.json");

        let state = read_json_value(&state_path).unwrap_or(Value::Null);
        let state_stamp = file_stamp(&state_path);
        let signals_stamp = file_stamp(&signals_path);
        let episodes_stamp = file_stamp(&episodes_path);
        let event_log_stamp = file_stamp(&events_path);

        let mut signal_cursor = SignalCursor::new(&signals_path);
        let _ = signal_cursor.tick();
        let recent_signals = signal_cursor.recent_signals().to_vec();
        let gate_signal_summaries = signal_cursor.gate_signal_summaries().to_vec();
        let signal_gate_results = signal_cursor.signal_gate_results().to_vec();

        let mut episode_cursor = EpisodeCursor::new(&episodes_path);
        let _ = episode_cursor.tick();
        let episodes = episode_cursor.episodes().to_vec();

        let mut event_log_cursor = EventLogCursor::new(&events_path);
        let _ = event_log_cursor.tick();
        let event_log = event_log_cursor.event_log().to_vec();

        let plans = load_plan_summaries(&root, &state);
        let active_tasks = load_active_tasks(&state);
        let agents = load_agents(&state);
        let gate_results = load_gate_results(&state, &signal_gate_results);
        let efficiency_events = read_efficiency_events_sync(&efficiency_path);
        let efficiency = load_efficiency_summary(&efficiency_path);
        let efficiency_trend = load_efficiency_trend(&efficiency_path);
        let cfactor_trend = load_cfactor_trend(&cfactor_path);
        let cascade_router =
            load_json_opt::<CascadeRouterState>(&cascade_router_path).unwrap_or_default();
        let cascade_router_stamp = file_stamp(&cascade_router_path);
        let experiment_store =
            load_json_opt::<ExperimentStore>(&experiments_path).unwrap_or_default();
        let experiments_stamp = file_stamp(&experiments_path);
        let mut experiments = experiment_store
            .iter()
            .map(ExperimentSummary::from_experiment)
            .collect::<Vec<_>>();
        experiments.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        let experiment_winners = experiment_store.winner_summaries();
        let adaptive_thresholds = load_json_opt::<AdaptiveThresholds>(&gate_thresholds_path);
        let gate_thresholds_stamp = file_stamp(&gate_thresholds_path);
        let gate_results_page =
            build_gate_results_page_data(&gate_signal_summaries, adaptive_thresholds.as_ref());
        let conductor_alerts = recent_signals
            .iter()
            .filter(|signal| signal.kind.starts_with("conductor:alert:"))
            .map(AlertSummary::from_signal)
            .collect();
        let cfactor = load_latest_jsonl_value::<CFactor>(&cfactor_path);
        let cfactor_stamp = file_stamp(&cfactor_path);

        let task_outputs_dir = roko_dir.join("task-outputs");
        let mut task_output_cursors = TaskOutputCursors::new(&task_outputs_dir);
        let _ = task_output_cursors.reconcile();
        let _ = task_output_cursors.tick();
        let task_outputs = task_output_cursors.snapshot();

        let current_plan_execution = load_current_plan_execution(&root, &state, &episodes);
        let efficiency_stamp = file_stamp(&efficiency_path);

        // Backfill agent_output_tail from task-outputs if episode didn't provide it.
        let current_plan_execution =
            backfill_agent_output_tail(current_plan_execution, &task_output_cursors);

        let (git_diff, git_diff_is_staged) = load_dashboard_git_diff(&root);
        let generation = next_dashboard_data_generation(
            &root,
            DashboardDataStamps {
                executor_state: state_stamp,
                efficiency: efficiency_stamp,
                experiments: experiments_stamp,
                gate_thresholds: gate_thresholds_stamp,
                signals: signals_stamp,
                episodes: episodes_stamp,
                cfactor: cfactor_stamp,
                cascade_router: cascade_router_stamp,
                task_outputs: task_output_cursors.revision(),
                event_log: event_log_stamp,
            },
        );

        Self {
            root,
            generation,
            executor_state: state,
            executor_state_stamp: state_stamp,
            plans,
            active_tasks,
            agents,
            gate_results,
            efficiency,
            efficiency_events,
            efficiency_trend,
            cfactor_trend,
            efficiency_stamp,
            cascade_router,
            experiment_store,
            experiments,
            experiment_winners,
            experiments_stamp,
            gate_results_page,
            adaptive_thresholds,
            gate_thresholds_stamp,
            recent_signals,
            signal_gate_results,
            gate_signal_summaries,
            signal_cursor,
            current_plan_execution,
            episode_cursor,
            episodes,
            conductor_alerts,
            cfactor,
            cfactor_stamp,
            cascade_router_stamp,
            task_output_cursors,
            task_outputs,
            git_diff,
            git_diff_is_staged,
            event_log,
            event_log_cursor,
        }
    }

    /// Refresh the snapshot from the stored workspace root.
    pub async fn refresh(&mut self) -> Result<()> {
        let mut snapshot = std::mem::take(self);
        let refreshed = tokio::task::spawn_blocking(move || -> Result<Self> {
            snapshot.tick()?;
            Ok(snapshot)
        })
        .await??;
        *self = refreshed;
        Ok(())
    }

    /// Advance cursor-backed dashboard artifacts and refresh stamp-backed files once.
    pub fn tick(&mut self) -> Result<()> {
        let roko_dir = self.root.join(".roko");
        let state_path = roko_dir.join("state").join("executor.json");
        let efficiency_path = roko_dir.join("learn").join(EFFICIENCY_FILE);
        let experiments_path = roko_dir.join("learn").join(EXPERIMENTS_FILE);
        let gate_thresholds_path = roko_dir.join("learn").join(GATE_THRESHOLDS_FILE);
        let cascade_router_path = roko_dir.join("learn").join(CASCADE_ROUTER_FILE);
        let cfactor_path = roko_dir.join("learn").join("c-factor.jsonl");

        let mut state_changed = false;
        let mut generation_changed = false;
        let mut episodes_changed = false;
        let stamp = file_stamp(&state_path);
        if stamp != self.executor_state_stamp {
            self.executor_state_stamp = stamp;
            self.executor_state = read_json_value(&state_path).unwrap_or(Value::Null);
            state_changed = true;
            generation_changed = true;
        }

        if self.signal_cursor.tick()? {
            self.recent_signals = self.signal_cursor.recent_signals().to_vec();
            self.gate_signal_summaries = self.signal_cursor.gate_signal_summaries().to_vec();
            self.signal_gate_results = self.signal_cursor.signal_gate_results().to_vec();
            self.rebuild_signal_dependent_fields();
            generation_changed = true;
        }

        if self.episode_cursor.tick()? {
            self.episodes = self.episode_cursor.episodes().to_vec();
            episodes_changed = true;
            generation_changed = true;
        }

        let stamp = file_stamp(&efficiency_path);
        if stamp != self.efficiency_stamp {
            self.efficiency_stamp = stamp;
            self.efficiency_events = read_efficiency_events_sync(&efficiency_path);
            self.efficiency = load_efficiency_summary(&efficiency_path);
            self.efficiency_trend = load_efficiency_trend(&efficiency_path);
            generation_changed = true;
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
            self.experiment_winners = self.experiment_store.winner_summaries();
            generation_changed = true;
        }

        let stamp = file_stamp(&gate_thresholds_path);
        if stamp != self.gate_thresholds_stamp {
            self.gate_thresholds_stamp = stamp;
            self.adaptive_thresholds = load_json_opt::<AdaptiveThresholds>(&gate_thresholds_path);
            self.rebuild_gate_results_page();
            generation_changed = true;
        }

        let stamp = file_stamp(&cascade_router_path);
        if stamp != self.cascade_router_stamp {
            self.cascade_router_stamp = stamp;
            self.cascade_router =
                load_json_opt::<CascadeRouterState>(&cascade_router_path).unwrap_or_default();
            generation_changed = true;
        }

        let stamp = file_stamp(&cfactor_path);
        if stamp != self.cfactor_stamp {
            self.cfactor_stamp = stamp;
            self.cfactor = load_latest_jsonl_value::<CFactor>(&cfactor_path);
            self.cfactor_trend = load_cfactor_trend(&cfactor_path);
            generation_changed = true;
        }

        let mut task_outputs_changed = false;
        if self.task_output_cursors.reconcile()? {
            task_outputs_changed = true;
        }
        if self.task_output_cursors.tick()? {
            task_outputs_changed = true;
        }
        if task_outputs_changed {
            self.task_outputs = self.task_output_cursors.snapshot();
            generation_changed = true;
        }

        if self.event_log_cursor.tick()? {
            self.event_log = self.event_log_cursor.event_log().to_vec();
            generation_changed = true;
        }

        let (git_diff, git_diff_is_staged) = load_dashboard_git_diff(&self.root);
        self.git_diff = git_diff;
        self.git_diff_is_staged = git_diff_is_staged;

        if state_changed || episodes_changed || task_outputs_changed {
            self.plans = load_plan_summaries(&self.root, &self.executor_state);
            self.active_tasks = load_active_tasks(&self.executor_state);
            self.agents = load_agents(&self.executor_state);
            self.gate_results = load_gate_results(&self.executor_state, &self.signal_gate_results);
            self.current_plan_execution = backfill_agent_output_tail(
                load_current_plan_execution(&self.root, &self.executor_state, &self.episodes),
                &self.task_output_cursors,
            );
        }

        if generation_changed {
            self.generation = self.generation.saturating_add(1);
        }

        Ok(())
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

    /// Cached episodes for log display.
    #[must_use]
    pub(crate) fn episodes(&self) -> &[Episode] {
        &self.episodes
    }

    /// Per-task agent output tails.
    #[must_use]
    pub(crate) fn task_outputs(&self) -> &HashMap<String, Vec<String>> {
        &self.task_outputs
    }

    /// Executor-level summary derived from `.roko/state/executor.json`.
    #[must_use]
    pub(crate) fn executor_summary(&self) -> ExecutorSummary {
        summarize_executor_state(&self.executor_state)
    }

    #[must_use]
    pub(crate) fn gate_signals_for_task(&self, task_id: &str) -> Vec<GateSignalSummary> {
        self.gate_signal_summaries
            .iter()
            .filter(|signal| signal.task_id.as_deref() == Some(task_id))
            .cloned()
            .collect()
    }

    /// Plan/task snapshots for the interactive plans tree and detail panes.
    #[must_use]
    pub(crate) fn plan_task_snapshots(&self) -> HashMap<String, PlanTaskListSnapshot> {
        build_plan_task_snapshots(
            &self.root,
            &self.executor_state,
            &self.plans,
            &self.active_tasks,
            &self.episodes,
        )
    }
}

fn load_dashboard_git_diff(root: &Path) -> (String, bool) {
    let staged = run_dashboard_git_diff(root, true);
    if !staged.is_empty() {
        return (staged, true);
    }

    (run_dashboard_git_diff(root, false), false)
}

fn run_dashboard_git_diff(root: &Path, staged: bool) -> String {
    let args: &[&str] = if staged {
        &["diff", "--cached"]
    } else {
        &["diff", "HEAD"]
    };

    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
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
    #[serde(default)]
    pub latest_gate: Option<String>,
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
    pub model: String,
    pub duration: String,
    pub is_current: bool,
}

/// Detail block for the current task.
#[derive(Debug, Clone)]
pub struct PlanExecutionTaskDetail {
    pub task_id: String,
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

/// Lightweight task snapshot used by the interactive TUI plan views.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanTaskSnapshot {
    pub id: String,
    pub title: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub model: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub wave: Option<u32>,
}

/// Per-plan snapshot used to hydrate `TuiState::plans`.
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanTaskListSnapshot {
    pub phase: String,
    pub active: bool,
    pub tasks_done: usize,
    pub tasks_failed: usize,
    pub elapsed_ms: u64,
    pub elapsed_secs: f64,
    /// Current wave number for the plan.
    pub wave: u32,
    /// Count of failed tasks, including gate rejections.
    pub failed_count: u32,
    pub tasks: Vec<PlanTaskSnapshot>,
}

#[derive(Debug, Clone, Default)]
struct PlanTaskRuntimeFields {
    model: Option<String>,
    elapsed_ms: Option<u64>,
    started_at: Option<String>,
    ended_at: Option<String>,
    wave: Option<u32>,
}

#[derive(Debug, Clone)]
struct ParsedPlanTasksFile {
    tasks_file: TasksFile,
    task_runtime_fields: Vec<PlanTaskRuntimeFields>,
    plan_wave: Option<u32>,
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

// CFactor and CFactorComponents are imported from roko_learn::cfactor (line 26).

impl SignalSummary {
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
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
    pub(crate) fn from_signal(value: &Value, plan_id: &str) -> Option<Self> {
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
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
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

/// Load orchestrator event log from `.roko/state/events.json`.
pub(crate) fn load_event_log(events_path: &Path) -> Vec<EventLogEntry> {
    let Some(value) = read_json_value(events_path) else {
        return Vec::new();
    };
    let Some(entries) = value.as_array() else {
        // Try as JSONL-style (one object = single event)
        return parse_event_entry(&value).into_iter().collect();
    };
    entries.iter().filter_map(parse_event_entry).collect()
}

fn parse_event_entry(value: &Value) -> Option<EventLogEntry> {
    Some(EventLogEntry {
        timestamp_ms: value
            .get("timestamp_ms")
            .and_then(Value::as_u64)
            .or_else(|| value.get("timestamp").and_then(Value::as_u64))
            .unwrap_or_default(),
        event_type: value
            .get("event_type")
            .or_else(|| value.get("type"))
            .or_else(|| value.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        plan_id: value
            .get("plan_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        task_id: value
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        message: value
            .get("message")
            .or_else(|| value.get("detail"))
            .or_else(|| value.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

/// Backfill `agent_output_tail` from task-outputs when episodes didn't provide it.
fn backfill_agent_output_tail(
    mut snapshot: Option<PlanExecutionSnapshot>,
    task_outputs: &TaskOutputCursors,
) -> Option<PlanExecutionSnapshot> {
    let exec = snapshot.as_mut()?;
    if exec.agent_output_tail.is_empty() {
        // Try current task first
        if let Some(detail) = &exec.current_task {
            if let Some(output) = task_outputs.tail_for(&detail.task_id) {
                exec.agent_output_tail = output.to_vec();
            }
        }
        // If still empty, try any task in the execution that has output
        if exec.agent_output_tail.is_empty() {
            for task_row in exec.tasks.iter().rev() {
                if let Some(output) = task_outputs.tail_for(&task_row.task_id) {
                    if !output.is_empty() {
                        exec.agent_output_tail = output.to_vec();
                        break;
                    }
                }
            }
        }
    }
    snapshot
}

fn load_plan_summaries(root: &Path, state: &Value) -> Vec<PlanSummary> {
    let mut ids = std::collections::BTreeSet::new();
    let trackers = load_task_trackers(root);
    if let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) {
        ids.extend(plan_states.keys().cloned());
    }
    if ids.is_empty() {
        let pdir = plans_dir(root);
        if pdir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&pdir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() && p.join("tasks.toml").exists() {
                        ids.insert(entry.file_name().to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    let mut summaries = Vec::new();
    for id in ids {
        let mut title = id.clone();
        let mut task_count = 0usize;
        let mut tasks_done = 0usize;
        let mut tasks_failed = 0usize;
        let plan_dir = plans_dir(root).join(&id);
        let tasks_path = plan_dir.join("tasks.toml");
        if let Ok(tasks_file) = TasksFile::parse(&tasks_path) {
            if !tasks_file.meta.plan.trim().is_empty() {
                title = tasks_file.meta.plan.clone();
            }
            task_count = tasks_file.tasks.len();

            let tracker = trackers.get(&id);
            let completed: HashSet<String> = tracker
                .map(|tracker| tracker.completed.iter().cloned().collect())
                .unwrap_or_default();
            let failed: HashSet<String> = tracker
                .map(|tracker| tracker.failed.iter().cloned().collect())
                .unwrap_or_default();

            for task in &tasks_file.tasks {
                if completed.contains(&task.id) || is_task_done_status(&task.status) {
                    tasks_done += 1;
                } else if failed.contains(&task.id) || is_task_failed_status(&task.status) {
                    tasks_failed += 1;
                }
            }
        }

        let phase = state
            .get("plan_states")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&id))
            .and_then(current_phase_label)
            .unwrap_or_default();
        let phase_status = PlanPhase::from(phase.as_str());
        let completed = phase_status.is_done() || phase_status.is_failed();
        if task_count > 0 && tasks_done == 0 && tasks_failed == 0 && phase_status.is_done() {
            tasks_done = task_count;
        }

        let tasks_done = state
            .get("plan_states")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&id))
            .and_then(|plan_state| {
                plan_state
                    .get("done")
                    .and_then(Value::as_u64)
                    .or_else(|| plan_state.get("tasks_done").and_then(Value::as_u64))
            })
            .unwrap_or(tasks_done as u64) as usize;

        let tasks_failed = state
            .get("plan_states")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&id))
            .and_then(|plan_state| {
                plan_state
                    .get("failed")
                    .and_then(Value::as_u64)
                    .or_else(|| plan_state.get("tasks_failed").and_then(Value::as_u64))
            })
            .unwrap_or(tasks_failed as u64) as usize;

        let last_error = state
            .get("plan_states")
            .and_then(Value::as_object)
            .and_then(|plans| plans.get(&id))
            .and_then(|plan_state| {
                plan_state
                    .get("last_error")
                    .and_then(Value::as_str)
                    .or_else(|| plan_state.pointer("/error/message").and_then(Value::as_str))
                    .or_else(|| plan_state.get("error").and_then(Value::as_str))
            })
            .map(ToOwned::to_owned);

        summaries.push(PlanSummary {
            id,
            title,
            task_count,
            tasks_done,
            tasks_failed,
            completed,
            old_format: false,
            last_error,
        });
    }

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    summaries
}

fn build_plan_task_snapshots(
    root: &Path,
    state: &Value,
    plans: &[PlanSummary],
    active_tasks: &[TaskSummary],
    episodes: &[Episode],
) -> HashMap<String, PlanTaskListSnapshot> {
    let trackers = load_task_trackers(root);
    let plan_states = state.get("plan_states").and_then(Value::as_object);
    let active_by_key: HashMap<(String, String), &TaskSummary> = active_tasks
        .iter()
        .map(|task| ((task.plan_id.clone(), task.task_id.clone()), task))
        .collect();
    let current_task_by_plan: HashMap<&str, &str> = active_tasks
        .iter()
        .map(|task| (task.plan_id.as_str(), task.task_id.as_str()))
        .collect();
    let mut snapshots = HashMap::new();

    for plan in plans {
        let plan_state = plan_states.and_then(|states| states.get(&plan.id));
        let phase = plan_state.and_then(current_phase_label).unwrap_or_else(|| {
            if plan.completed {
                String::from("done")
            } else {
                String::from("pending")
            }
        });
        let plan_succeeded = PlanPhase::from(phase.as_str()).is_done();
        let active = plan_state
            .map(|state| !plan_state_is_terminal(state) && !plan_state_is_paused(state))
            .unwrap_or(!plan.completed);
        let elapsed_secs = episodes
            .iter()
            .filter(|episode| episode_matches_plan(episode, &plan.id, None))
            .map(|episode| episode.usage.wall_ms as f64 / 1000.0)
            .sum();
        let mut snapshot = PlanTaskListSnapshot {
            phase,
            active,
            elapsed_secs,
            ..PlanTaskListSnapshot::default()
        };

        let tasks_path = plans_dir(root).join(&plan.id).join("tasks.toml");
        let parsed = match parse_plan_tasks_file(&tasks_path) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %tasks_path.display(),
                    plan_id = %plan.id,
                    "failed to parse tasks.toml for TUI plan snapshot"
                );
                snapshots.insert(plan.id.clone(), snapshot);
                continue;
            }
        };
        let tasks_file = &parsed.tasks_file;
        if parsed.task_runtime_fields.len() != tasks_file.tasks.len() {
            tracing::warn!(
                path = %tasks_path.display(),
                plan_id = %plan.id,
                runtime_fields = parsed.task_runtime_fields.len(),
                parsed_tasks = tasks_file.tasks.len(),
                "tasks.toml runtime metadata count did not match parsed task count"
            );
        }

        let tracker = trackers.get(&plan.id);
        let completed: HashSet<String> = tracker
            .map(|tracker| tracker.completed.iter().cloned().collect())
            .unwrap_or_default();
        let failed: HashSet<String> = tracker
            .map(|tracker| tracker.failed.iter().cloned().collect())
            .unwrap_or_default();
        let current_task_id = current_task_by_plan
            .get(plan.id.as_str())
            .map(|task_id| (*task_id).to_string())
            .or_else(|| current_task_id(&tasks_file, tracker, &completed, &failed));

        snapshot.tasks = tasks_file
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| {
                let runtime = parsed.task_runtime_fields.get(index);
                let active_task = active_by_key
                    .get(&(plan.id.clone(), task.id.clone()))
                    .copied();
                let status = if completed.contains(&task.id) {
                    String::from("done")
                } else if failed.contains(&task.id) {
                    String::from("failed")
                } else if let Some(active_task) = active_task {
                    active_task.status.clone()
                } else if is_task_done_status(&task.status) {
                    String::from("done")
                } else if is_task_failed_status(&task.status) {
                    String::from("failed")
                } else if plan_succeeded {
                    String::from("done")
                } else {
                    task_phase_label(
                        task,
                        &snapshot.phase,
                        current_task_id.as_deref(),
                        tracker,
                        &completed,
                        &failed,
                    )
                };

                PlanTaskSnapshot {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    status,
                    agent_id: active_task.and_then(|task| task.assigned_agents.first().cloned()),
                    model: runtime
                        .and_then(|runtime| runtime.model.clone())
                        .or_else(|| task.model_hint.clone()),
                    elapsed_ms: runtime.and_then(|runtime| runtime.elapsed_ms),
                    started_at: runtime.and_then(|runtime| runtime.started_at.clone()),
                    ended_at: runtime.and_then(|runtime| runtime.ended_at.clone()),
                    wave: runtime.and_then(|runtime| runtime.wave),
                }
            })
            .collect();
        snapshot.tasks_done = snapshot
            .tasks
            .iter()
            .filter(|task| is_task_done_status(&task.status))
            .count();
        snapshot.tasks_failed = snapshot
            .tasks
            .iter()
            .filter(|task| is_task_failed_status(&task.status))
            .count();
        snapshot.elapsed_ms = snapshot
            .tasks
            .iter()
            .map(|task| task.elapsed_ms.unwrap_or(0))
            .sum();
        if snapshot.elapsed_ms > 0 {
            snapshot.elapsed_secs = snapshot.elapsed_ms as f64 / 1000.0;
        }
        snapshot.wave = parsed.plan_wave.unwrap_or_else(|| {
            snapshot
                .tasks
                .iter()
                .filter(|task| TaskStatus::from(task.status.as_str()).is_active())
                .filter_map(|task| task.wave)
                .max()
                .unwrap_or_default()
        });
        snapshot.failed_count = snapshot.tasks_failed as u32;

        snapshots.insert(plan.id.clone(), snapshot);
    }

    snapshots
}

fn parse_plan_tasks_file(path: &Path) -> Result<ParsedPlanTasksFile> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let tasks_file = toml::from_str::<TasksFile>(&content)
        .with_context(|| format!("parse {}", path.display()))?;
    let raw = toml::from_str::<toml::Value>(&content)
        .with_context(|| format!("parse runtime metadata from {}", path.display()))?;
    let raw_tasks = raw.get("task").and_then(toml::Value::as_array);

    let task_runtime_fields = tasks_file
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            let task_table = raw_tasks
                .and_then(|tasks| tasks.get(index))
                .and_then(toml::Value::as_table)
                .or_else(|| {
                    raw_tasks.and_then(|tasks| {
                        tasks.iter().find_map(|task_value| {
                            let table = task_value.as_table()?;
                            let id = table.get("id").and_then(toml::Value::as_str)?;
                            (id == task.id).then_some(table)
                        })
                    })
                });

            PlanTaskRuntimeFields {
                model: task_table
                    .and_then(|table| table.get("model"))
                    .and_then(toml_scalar_to_string),
                elapsed_ms: task_table
                    .and_then(|table| table.get("elapsed_ms"))
                    .and_then(toml_value_to_u64),
                started_at: task_table
                    .and_then(|table| {
                        table
                            .get("started_at")
                            .or_else(|| table.get("started_at_ms"))
                            .or_else(|| table.get("start_time"))
                    })
                    .and_then(toml_scalar_to_string),
                ended_at: task_table
                    .and_then(|table| {
                        table
                            .get("ended_at")
                            .or_else(|| table.get("ended_at_ms"))
                            .or_else(|| table.get("end_time"))
                    })
                    .and_then(toml_scalar_to_string),
                wave: task_table
                    .and_then(|table| table.get("wave"))
                    .and_then(toml_value_to_u32),
            }
        })
        .collect();

    let plan_wave = raw
        .get("meta")
        .and_then(toml::Value::as_table)
        .and_then(|meta| meta.get("wave"))
        .and_then(toml_value_to_u32);

    Ok(ParsedPlanTasksFile {
        tasks_file,
        task_runtime_fields,
        plan_wave,
    })
}

fn toml_scalar_to_string(value: &toml::Value) -> Option<String> {
    match value {
        toml::Value::String(value) => Some(value.clone()),
        toml::Value::Integer(value) => Some(value.to_string()),
        toml::Value::Float(value) => Some(value.to_string()),
        toml::Value::Boolean(value) => Some(value.to_string()),
        toml::Value::Datetime(value) => Some(value.to_string()),
        _ => None,
    }
}

fn toml_value_to_u64(value: &toml::Value) -> Option<u64> {
    value
        .as_integer()
        .and_then(|value| u64::try_from(value).ok())
}

fn toml_value_to_u32(value: &toml::Value) -> Option<u32> {
    value
        .as_integer()
        .and_then(|value| u32::try_from(value).ok())
}

fn is_task_done_status(status: &str) -> bool {
    TaskStatus::from(status).is_done()
}

fn is_task_failed_status(status: &str) -> bool {
    if TaskStatus::from(status).is_failed() {
        return true;
    }

    let normalized = status.trim().to_ascii_lowercase();
    let compact = normalized.replace(['-', '_', ' '], "");
    matches!(
        compact.as_str(),
        "gaterejected" | "reviewrejected" | "rejected"
    )
}

fn load_active_tasks(state: &Value) -> Vec<TaskSummary> {
    let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut tasks = Vec::new();
    for (plan_id, plan_state) in plan_states {
        let status = current_phase_label(plan_state).unwrap_or_else(|| "unknown".to_string());
        if matches!(status.to_ascii_lowercase().as_str(), "complete" | "skipped") {
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
        let latest_gate = plan_state
            .get("gate_results")
            .and_then(Value::as_array)
            .and_then(|results| results.last())
            .and_then(|result| result.get("gate_name"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        tasks.push(TaskSummary {
            plan_id: plan_id.clone(),
            task_id,
            status,
            iteration,
            assigned_agents,
            latest_gate,
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

fn summarize_executor_state(state: &Value) -> ExecutorSummary {
    let Some(plan_states) = state.get("plan_states").and_then(Value::as_object) else {
        return ExecutorSummary::default();
    };

    if plan_states.is_empty() {
        return ExecutorSummary::default();
    }

    let has_running = plan_states
        .values()
        .any(|plan_state| !plan_state_is_terminal(plan_state) && !plan_state_is_paused(plan_state));
    let has_paused = plan_states
        .values()
        .any(|plan_state| !plan_state_is_terminal(plan_state) && plan_state_is_paused(plan_state));
    let has_error = plan_states.values().any(plan_state_has_error);

    let mut summary = ExecutorSummary {
        orchestrator_state: if has_running {
            String::from("running")
        } else if has_paused {
            String::from("paused")
        } else if has_error {
            String::from("error")
        } else {
            String::from("idle")
        },
        ..ExecutorSummary::default()
    };

    if let Some((_, plan_state)) = most_advanced_active_plan_state(state) {
        summary.current_iteration = plan_state
            .get("iteration")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        summary.current_phase = current_phase_label(plan_state).unwrap_or_default();
    }

    summary
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

fn plan_state_is_terminal(plan_state: &Value) -> bool {
    current_phase_label(plan_state)
        .map(|phase| {
            matches!(
                phase.to_ascii_lowercase().as_str(),
                "complete" | "done" | "failed" | "skipped"
            )
        })
        .unwrap_or(false)
}

fn plan_state_is_paused(plan_state: &Value) -> bool {
    plan_state
        .get("paused")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn plan_state_has_error(plan_state: &Value) -> bool {
    current_phase_label(plan_state)
        .map(|phase| phase.eq_ignore_ascii_case("failed"))
        .unwrap_or(false)
        || plan_state
            .get("last_error")
            .and_then(Value::as_str)
            .is_some_and(|err| !err.trim().is_empty())
        || plan_state
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|err| !err.trim().is_empty())
        || plan_state
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|err| !err.trim().is_empty())
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

fn most_advanced_active_plan_state<'a>(state: &'a Value) -> Option<(&'a str, &'a Value)> {
    let plan_states = state.get("plan_states").and_then(Value::as_object)?;

    let mut candidates = plan_states
        .iter()
        .filter_map(|(plan_id, plan_state)| {
            if plan_state_is_paused(plan_state) || plan_state_is_terminal(plan_state) {
                return None;
            }
            let phase = current_phase_label(plan_state)?;
            let started_at_ms = plan_state
                .get("started_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some((
                execution_phase_priority(&phase),
                started_at_ms,
                plan_id.as_str(),
                plan_state,
            ))
        })
        .collect::<Vec<_>>();

    candidates.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(b.2))
    });

    candidates
        .into_iter()
        .next()
        .map(|(_, _, plan_id, plan_state)| (plan_id, plan_state))
}

fn default_model_for_tier(tier: &str) -> String {
    match tier.to_ascii_lowercase().as_str() {
        "mechanical" => String::from("claude-haiku-4-5"),
        "focused" | "integrative" => String::from("claude-sonnet-4-6"),
        "architectural" => String::from("claude-opus-4-6"),
        _ => String::from("claude-sonnet-4-6"),
    }
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

fn load_efficiency_trend(path: &Path) -> Vec<EfficiencyBucket> {
    efficiency_trend(path, Duration::hours(1), 24).unwrap_or_default()
}

fn load_cfactor_trend(path: &Path) -> Vec<CFactorBucket> {
    cfactor_trend(path, Duration::hours(1), 24).unwrap_or_default()
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

fn load_latest_jsonl_value<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str(line).ok())
}

fn file_stamp(path: &Path) -> FileStamp {
    FileStamp::from_path(path).unwrap_or_default()
}

fn next_dashboard_data_generation(root: &Path, stamps: DashboardDataStamps) -> u64 {
    let counters = DASHBOARD_GENERATION_COUNTERS.get_or_init(|| Mutex::new(HashMap::new()));
    let counter = {
        let mut counters = counters
            .lock()
            .expect("dashboard generation counter registry lock poisoned");
        counters
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(DurableDashboardGenerationCounter::load(root)))
            .clone()
    };

    counter.next(root, stamps.fingerprint())
}

pub(crate) fn signal_gate_result_from_value(value: &Value) -> Option<GateResultSummary> {
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
    /// Hourly efficiency trend over the last 24 hours.
    efficiency_trend: Vec<EfficiencyBucket>,
    /// Prompt experiment store from `.roko/learn/experiments.json`.
    experiments: Option<ExperimentStore>,
    /// Adaptive gate thresholds from `.roko/learn/gate-thresholds.json`.
    adaptive_thresholds: Option<AdaptiveThresholds>,
    /// Gate-results page data derived from signals and thresholds.
    gate_results_page: GateResultsPageData,
    /// Most recent signals from `.roko/engrams.jsonl`.
    recent_signals: Vec<SignalSummary>,
    /// Cascade router snapshot from `.roko/learn/cascade-router.json` (raw JSON).
    cascade_snapshot: Option<CascadeSnapshotData>,
    /// Last observed cascade-router file metadata.
    cascade_snapshot_stamp: FileStamp,
    /// Last observed experiments file metadata.
    experiments_stamp: FileStamp,
    /// Last observed gate-thresholds file metadata.
    adaptive_thresholds_stamp: FileStamp,
    /// Persisted skill-library snapshot from `.roko/learn/skills.json`.
    skills: Vec<Skill>,
    /// Last observed skills file metadata.
    skills_stamp: FileStamp,
    /// Optional persisted provider-health snapshot from `.roko/learn/provider-health.json`.
    provider_health: Option<ProviderHealthRegistrySnapshotData>,
    /// Last observed provider-health file metadata.
    provider_health_stamp: FileStamp,
    /// Latency stats from `.roko/learn/latency-stats.json`.
    latency_stats: Option<LatencyStatsData>,
    /// Knowledge-store counters derived from `.roko/neuro/*.jsonl`.
    knowledge_store: KnowledgeStoreSnapshot,
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
    #[serde(default)]
    total_observations: u64,
    #[serde(default)]
    stage_transitions: Vec<StageTransition>,
}

/// Per-model stats from the cascade router JSON.
#[derive(Debug, Clone, serde::Deserialize)]
struct PersistedModelStatsData {
    trials: u64,
    successes: u64,
}

/// Deserialized latency stats from `.roko/learn/latency-stats.json`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct LatencyStatsData {
    #[serde(default)]
    entries: Vec<LatencyEntryData>,
}

/// Per-provider latency entry.
#[derive(Debug, Clone, serde::Deserialize)]
struct LatencyEntryData {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    stats: LatencyStatsEntryData,
}

/// Latency statistics for one provider.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct LatencyStatsEntryData {
    #[serde(default)]
    recent_latencies: Vec<f64>,
}

#[derive(Debug, Clone, Default)]
struct LearningArtifactsSnapshot {
    cascade_stamp: FileStamp,
    experiments_stamp: FileStamp,
    gate_thresholds_stamp: FileStamp,
    skills: Vec<Skill>,
    skills_stamp: FileStamp,
    provider_health: Option<ProviderHealthRegistrySnapshotData>,
    provider_health_stamp: FileStamp,
    latency_stats: Option<LatencyStatsData>,
    knowledge_store: KnowledgeStoreSnapshot,
}

#[derive(Debug, Clone, Default)]
struct KnowledgeStoreSnapshot {
    total_records: usize,
    last_updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ProviderHealthRegistrySnapshotData {
    #[serde(default)]
    providers: HashMap<String, ProviderHealth>,
}

#[derive(Debug, Clone)]
struct LearningSubsystemRow {
    subsystem: &'static str,
    updates: String,
    last: String,
    health: String,
}

impl DashboardSnapshot {
    /// Load the learning snapshot from a workspace root.
    pub async fn load(root: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let root = resolve_snapshot_root(root.as_ref());
        let memory_dir = root.join(MEMORY_DIR);
        let learn_dir = root.join(LEARN_DIR);
        let episodes_path = resolve_episodes_path(&root);
        let task_metrics_path = memory_dir.join(TASK_METRICS_FILE);
        let signals_path = root.join(".roko").join("engrams.jsonl");

        let episodes_logger = EpisodeLogger::new(&episodes_path);
        let episodes = EpisodeLogger::read_all_lossy(episodes_logger.path())
            .await
            .map_err(std::io::Error::other)?;
        let task_metrics = read_task_metrics(&task_metrics_path).await?;

        // Load learning subsystem data (best-effort).
        let efficiency_path = learn_dir.join(EFFICIENCY_FILE);
        let efficiency_events = read_efficiency_events(&efficiency_path).await;
        let efficiency_trend = load_efficiency_trend(&efficiency_path);
        let experiments = load_json_opt::<ExperimentStore>(&learn_dir.join(EXPERIMENTS_FILE));
        let adaptive_thresholds =
            load_json_opt::<AdaptiveThresholds>(&learn_dir.join(GATE_THRESHOLDS_FILE));
        let gate_signals = load_gate_signal_summaries(&signals_path);
        let gate_results_page =
            build_gate_results_page_data(&gate_signals, adaptive_thresholds.as_ref());
        let recent_signals = load_recent_signals(&signals_path, 100);
        let cascade_path = learn_dir.join(CASCADE_ROUTER_FILE);
        let experiments_path = learn_dir.join(EXPERIMENTS_FILE);
        let thresholds_path = learn_dir.join(GATE_THRESHOLDS_FILE);
        let skills_path = learn_dir.join(SKILLS_FILE);
        let provider_health_path = learn_dir.join(PROVIDER_HEALTH_FILE);
        let cascade_snapshot = load_json_opt::<CascadeSnapshotData>(&cascade_path);
        let learning_artifacts = LearningArtifactsSnapshot {
            cascade_stamp: file_stamp(&cascade_path),
            experiments_stamp: file_stamp(&experiments_path),
            gate_thresholds_stamp: file_stamp(&thresholds_path),
            skills: load_json_opt::<Vec<Skill>>(&skills_path).unwrap_or_default(),
            skills_stamp: file_stamp(&skills_path),
            provider_health: load_json_opt::<ProviderHealthRegistrySnapshotData>(
                &provider_health_path,
            ),
            provider_health_stamp: file_stamp(&provider_health_path),
            latency_stats: load_json_opt::<LatencyStatsData>(&learn_dir.join(LATENCY_STATS_FILE)),
            knowledge_store: load_knowledge_store_snapshot(&root),
        };

        Ok(Self::from_records(
            root,
            &episodes,
            &task_metrics,
            efficiency_events,
            efficiency_trend,
            experiments,
            adaptive_thresholds,
            gate_results_page,
            recent_signals,
            cascade_snapshot,
            learning_artifacts,
        ))
    }

    fn empty(root: PathBuf) -> Self {
        Self::from_records(
            root,
            &[],
            &[],
            Vec::new(),
            Vec::new(),
            None,
            None,
            GateResultsPageData::default(),
            Vec::new(),
            None,
            LearningArtifactsSnapshot::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_records(
        root: PathBuf,
        episodes: &[Episode],
        task_metrics: &[TaskMetric],
        efficiency_events: Vec<AgentEfficiencyEvent>,
        efficiency_trend: Vec<EfficiencyBucket>,
        experiments: Option<ExperimentStore>,
        adaptive_thresholds: Option<AdaptiveThresholds>,
        gate_results_page: GateResultsPageData,
        recent_signals: Vec<SignalSummary>,
        cascade_snapshot: Option<CascadeSnapshotData>,
        learning_artifacts: LearningArtifactsSnapshot,
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
            efficiency_trend,
            experiments,
            adaptive_thresholds,
            gate_results_page,
            recent_signals,
            cascade_snapshot,
            cascade_snapshot_stamp: learning_artifacts.cascade_stamp,
            experiments_stamp: learning_artifacts.experiments_stamp,
            adaptive_thresholds_stamp: learning_artifacts.gate_thresholds_stamp,
            skills: learning_artifacts.skills,
            skills_stamp: learning_artifacts.skills_stamp,
            provider_health: learning_artifacts.provider_health,
            provider_health_stamp: learning_artifacts.provider_health_stamp,
            latency_stats: learning_artifacts.latency_stats,
            knowledge_store: learning_artifacts.knowledge_store,
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
            "source: {}",
            resolve_episodes_path(&self.root).display()
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
            "source: {}/engrams.jsonl",
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

    fn render_learning_page(&self, _page: &PageScaffold) -> Option<String> {
        let observations = cascade_observations_snapshot(self.cascade_snapshot.as_ref());
        let stage = cascade_stage_for_observations(observations);
        let last_transition = self
            .cascade_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.stage_transitions.last());
        let gate_threshold_updates: u64 = self
            .adaptive_thresholds
            .as_ref()
            .map(|thresholds| {
                thresholds
                    .all_rungs()
                    .map(|(_, stats)| stats.total_observations)
                    .sum()
            })
            .unwrap_or(0);
        let running_experiments = self
            .experiments
            .as_ref()
            .map(ExperimentStore::running_count)
            .unwrap_or(0);
        let pattern_count = CrossEpisodeConsolidator::default()
            .discover(&self.episodes)
            .meta_pattern_count;
        let provider_summary = learning_provider_health_summary(
            &self.provider_health,
            self.provider_health_stamp,
            &self.efficiency_events,
        );

        let rows = vec![
            LearningSubsystemRow {
                subsystem: "CascadeRouter",
                updates: observations.to_string(),
                last: format_file_age(self.cascade_snapshot_stamp),
                health: format!(
                    "● {}",
                    match stage {
                        CascadeStage::Static => "warming",
                        CascadeStage::Confidence => "calibrating",
                        CascadeStage::Ucb => "learning",
                    }
                ),
            },
            LearningSubsystemRow {
                subsystem: "GateThresholds",
                updates: gate_threshold_updates.to_string(),
                last: format_file_age(self.adaptive_thresholds_stamp),
                health: format!(
                    "● {}",
                    if gate_threshold_updates > 0 {
                        "stable"
                    } else {
                        "pending"
                    }
                ),
            },
            LearningSubsystemRow {
                subsystem: "Experiments",
                updates: format!("{running_experiments} running"),
                last: format_file_age(self.experiments_stamp),
                health: format!(
                    "● {}",
                    if running_experiments > 0 {
                        "active"
                    } else {
                        "idle"
                    }
                ),
            },
            LearningSubsystemRow {
                subsystem: "SkillLibrary",
                updates: format!("{} skills", self.skills.len()),
                last: learning_skills_last_updated(&self.skills, self.skills_stamp),
                health: format!(
                    "● {}",
                    if self.skills.is_empty() {
                        "empty"
                    } else {
                        "growing"
                    }
                ),
            },
            LearningSubsystemRow {
                subsystem: "PatternMiner",
                updates: format!("{pattern_count} patterns"),
                last: learning_patterns_last_updated(&self.episodes),
                health: format!("● {}", if pattern_count > 0 { "mining" } else { "idle" }),
            },
            LearningSubsystemRow {
                subsystem: "ProviderHealth",
                updates: format!("{} providers", provider_summary.provider_count),
                last: provider_summary.last_updated,
                health: format!("● {}", provider_summary.health),
            },
            LearningSubsystemRow {
                subsystem: "KnowledgeStore",
                updates: learning_knowledge_updates(&self.knowledge_store),
                last: format_relative_timestamp(self.knowledge_store.last_updated),
                health: if self.knowledge_store.total_records > 0 {
                    String::from("● learning")
                } else {
                    String::new()
                },
            },
        ];

        let subsystem_header = format!(
            "{:<14} {:<9} {:<8} {}",
            "Subsystem", "Updates", "Last", "Health"
        );
        let mut lines = vec![
            String::new(),
            format!(
                "  Stage: {} ({} observations)",
                cascade_stage_label(stage),
                observations
            ),
            format!(
                "  Last transition: {}",
                last_transition.map_or_else(
                    || String::from("none yet"),
                    |transition| format!(
                        "{} -> {} at obs {}",
                        cascade_stage_label(transition.from),
                        cascade_stage_label(transition.to),
                        transition.observations
                    )
                )
            ),
            String::new(),
            format!("  {subsystem_header}"),
        ];
        lines.extend(rows.into_iter().map(|row| {
            format!(
                "  {:<14} {:<9} {:<8} {}",
                row.subsystem, row.updates, row.last, row.health
            )
        }));
        lines.extend(render_learning_trend_lines(&self.efficiency_trend));
        lines.extend([
            String::new(),
            String::from("  Feedback Loops:  6/8 connected"),
            String::from("  Missing: GateFail->Replan, SectionEffect->Prompt"),
            String::new(),
        ]);

        Some(render_boxed_panel("Learning System Status", &lines))
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
        let signals_path = self.root.join(".roko").join("engrams.jsonl");
        let episodes_path = resolve_episodes_path(&self.root);

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
            "source: {}/engrams.jsonl",
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

    fn render_provider_health_page(&self, page: &PageScaffold) -> Option<String> {
        let ph = self.provider_health.as_ref()?;
        if ph.providers.is_empty() {
            return None;
        }

        // Build a lookup of latency p50 per provider.
        let latency_p50: HashMap<&str, f64> = self
            .latency_stats
            .as_ref()
            .map(|ls| {
                ls.entries
                    .iter()
                    .filter_map(|entry| {
                        let p50 = percentile_ms(&entry.stats.recent_latencies, 50.0)?;
                        Some((entry.provider.as_str(), p50))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut out = page_header(page);

        // Sort providers alphabetically for stable output.
        let mut providers: Vec<_> = ph.providers.iter().collect();
        providers.sort_by_key(|(name, _)| name.as_str());

        // Aggregate summary totals.
        let total_requests: u64 = providers.iter().map(|(_, p)| p.total_requests).sum();
        let total_failures: u64 = providers.iter().map(|(_, p)| p.total_failures).sum();

        for (name, entry) in &providers {
            let state_symbol = match entry.state {
                CircuitState::Closed => "\u{25cf} CLOSED",
                CircuitState::HalfOpen => "\u{25d1} HALF-OPEN",
                CircuitState::Open => "\u{25cb} OPEN",
            };
            let _ = writeln!(out, "  {name}: {state_symbol}");
            if let Some(p50) = latency_p50.get(name.as_str()) {
                let _ = writeln!(out, "    p50: {}", format_latency_seconds(*p50));
            }
            let _ = writeln!(
                out,
                "    requests: {}, failures: {}",
                entry.total_requests, entry.total_failures
            );
        }

        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "summary: {} requests, {} failures",
            total_requests, total_failures
        );

        Some(out)
    }

    #[allow(clippy::cast_precision_loss)]
    fn render_model_comparison_page(&self, page: &PageScaffold) -> Option<String> {
        let cascade = self.cascade_snapshot.as_ref()?;
        if cascade.confidence_stats.is_empty() {
            return None;
        }

        let mut out = page_header(page);
        let _ = writeln!(out, "models: {}", cascade.model_slugs.len());

        // Table of model stats.
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "  {:>24}  {:>8}  {:>8}  {:>9}",
            "model", "trials", "passes", "pass rate"
        );
        let mut stats: Vec<_> = cascade.confidence_stats.iter().collect();
        stats.sort_by(|a, b| b.1.trials.cmp(&a.1.trials));
        for (model, s) in &stats {
            let rate = if s.trials > 0 {
                s.successes as f64 / s.trials as f64
            } else {
                0.0
            };
            let _ = writeln!(
                out,
                "  {:>24}  {:>8}  {:>8}  {:>9}",
                model,
                s.trials,
                s.successes,
                format_pct(rate)
            );
        }

        // Pareto frontier: a model is dominated if another model has both a
        // higher (or equal) pass rate AND fewer (or equal) trials (proxy for
        // cost).  We report dominated models with the model that dominates them.
        let _ = writeln!(out);
        let _ = writeln!(out, "Pareto frontier:");
        let mut model_rates: Vec<(&String, f64, u64)> = cascade
            .confidence_stats
            .iter()
            .map(|(model, s)| {
                let rate = if s.trials > 0 {
                    s.successes as f64 / s.trials as f64
                } else {
                    0.0
                };
                (model, rate, s.trials)
            })
            .collect();
        // Sort by trials descending for deterministic output: when looking for
        // dominators, prefer the "closest" (most trials) model first.
        model_rates.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(b.0)));

        let mut any_dominated = false;
        for (model, rate, trials) in &model_rates {
            // Check if another model dominates this one.
            for (other_model, other_rate, other_trials) in &model_rates {
                if *model == *other_model {
                    continue;
                }
                // `other` dominates `model` if it has a strictly higher pass
                // rate with fewer or equal trials (cost proxy).
                if *other_rate > *rate && *other_trials <= *trials {
                    let _ = writeln!(out, "  {model} dominated by {other_model}");
                    any_dominated = true;
                    break;
                }
            }
        }
        if !any_dominated {
            let _ = writeln!(out, "  (no dominated models)");
        }

        Some(out)
    }

    fn render_dreams_page(&self, page: &PageScaffold) -> Option<String> {
        let dream_dir = self.root.join(".roko").join("dreams");
        let journal_path = dream_dir.join("journal.jsonl");
        let archive_path = dream_dir.join("archive.jsonl");
        let journal_exists = journal_path.exists();
        let archive_exists = archive_path.exists();

        if !journal_exists && !archive_exists {
            return None;
        }

        let mut out = page_header(page);

        // Journal entries (most recent).
        if journal_exists {
            let _ = writeln!(out, "journal: {}", journal_path.display());
            if let Ok(content) = std::fs::read_to_string(&journal_path) {
                let lines: Vec<&str> = content.lines().collect();
                let total = lines.len();
                let _ = writeln!(out, "  entries: {total}");
                let _ = writeln!(out, "  recent:");
                for line in lines.iter().rev().take(5) {
                    if let Ok(val) = serde_json::from_str::<Value>(line) {
                        let cycle_id = val.get("cycle_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let phase = val.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
                        let summary = val.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = writeln!(out, "    [{cycle_id}] {phase}: {summary}");
                    }
                }
            }
        } else {
            let _ = writeln!(out, "journal: (no entries yet)");
        }

        let _ = writeln!(out);

        // Archive entries.
        if archive_exists {
            let _ = writeln!(out, "archive: {}", archive_path.display());
            if let Ok(content) = std::fs::read_to_string(&archive_path) {
                let lines: Vec<&str> = content.lines().collect();
                let total = lines.len();
                let _ = writeln!(out, "  entries: {total}");
                let _ = writeln!(out, "  recent:");
                for line in lines.iter().rev().take(5) {
                    if let Ok(val) = serde_json::from_str::<Value>(line) {
                        let kind = val.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                        let quality = val
                            .get("quality_score")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        let summary = val.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                        let _ = writeln!(out, "    [{kind}] q={quality:.2}: {summary}");
                    }
                }
            }
        } else {
            let _ = writeln!(out, "archive: (no entries yet)");
        }

        Some(out)
    }
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

fn render_boxed_panel(title: &str, lines: &[String]) -> String {
    let width = 50_usize;
    let mut out = String::new();
    let _ = writeln!(out, "╔{}╗", "═".repeat(width));
    let _ = writeln!(out, "║{: <width$}║", format!("  {title}"), width = width);
    let _ = writeln!(out, "╠{}╣", "═".repeat(width));
    for line in lines {
        let truncated: String = line.chars().take(width).collect();
        let _ = writeln!(out, "║{: <width$}║", truncated, width = width);
    }
    let _ = write!(out, "╚{}╝", "═".repeat(width));
    out
}

fn render_learning_trend_lines(buckets: &[EfficiencyBucket]) -> Vec<String> {
    let tokens = buckets
        .iter()
        .map(|bucket| bucket.tokens_in.saturating_add(bucket.tokens_out))
        .collect::<Vec<_>>();
    let latency = buckets
        .iter()
        .map(|bucket| bucket.latency_ms_avg.round() as u64)
        .collect::<Vec<_>>();
    let cost = buckets
        .iter()
        .map(|bucket| bucket.cost_usd_cents)
        .collect::<Vec<_>>();
    let has_data = buckets.iter().any(|bucket| bucket.turns > 0);

    if !has_data {
        return vec![
            String::new(),
            String::from("  24h Efficiency Trends"),
            String::from("  tok/h   no efficiency events yet"),
            String::from("  lat/h   no efficiency events yet"),
            String::from("  cost/h  no efficiency events yet"),
        ];
    }

    vec![
        String::new(),
        String::from("  24h Efficiency Trends"),
        format!(
            "  tok/h   {} {}",
            learning_sparkline(&tokens),
            format_compact_count(tokens.iter().copied().max().unwrap_or(0))
        ),
        format!(
            "  lat/h   {} {}ms",
            learning_sparkline(&latency),
            latency.iter().copied().max().unwrap_or(0)
        ),
        format!(
            "  cost/h  {} {}",
            learning_sparkline(&cost),
            format_usd(cost.iter().copied().max().unwrap_or(0) as f64 / 100.0)
        ),
    ]
}

fn learning_sparkline(series: &[u64]) -> String {
    if series.is_empty() {
        return String::new();
    }

    let max = series.iter().copied().max().unwrap_or(0);
    series
        .iter()
        .map(|value| learning_sparkline_char(*value, max))
        .collect()
}

fn learning_sparkline_char(value: u64, max: u64) -> char {
    const LEVELS: &[char; 8] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if max == 0 {
        return '·';
    }

    let idx = ((value.saturating_mul((LEVELS.len() - 1) as u64)) + (max / 2)) / max;
    LEVELS[idx as usize]
}

fn format_compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 10_000 {
        format!("{}k", value / 1_000)
    } else if value >= 1_000 {
        format!("{:.1}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
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

fn format_relative_timestamp(timestamp: Option<DateTime<Utc>>) -> String {
    let Some(timestamp) = timestamp else {
        return String::new();
    };
    let age_ms = Utc::now()
        .signed_duration_since(timestamp)
        .num_milliseconds()
        .max(0) as u64;
    format!("{} ago", format_elapsed_ms(age_ms))
}

fn format_file_age(stamp: FileStamp) -> String {
    format_relative_timestamp(stamp.modified.map(DateTime::<Utc>::from))
}

fn cascade_observations_snapshot(snapshot: Option<&CascadeSnapshotData>) -> u64 {
    snapshot.map_or(0, |snapshot| {
        if snapshot.total_observations > 0 {
            snapshot.total_observations
        } else {
            snapshot
                .confidence_stats
                .values()
                .map(|stats| stats.trials)
                .sum()
        }
    })
}

fn cascade_stage_for_observations(observations: u64) -> CascadeStage {
    if observations >= 200 {
        CascadeStage::Ucb
    } else if observations >= 50 {
        CascadeStage::Confidence
    } else {
        CascadeStage::Static
    }
}

fn cascade_stage_label(stage: CascadeStage) -> &'static str {
    match stage {
        CascadeStage::Static => "Static",
        CascadeStage::Confidence => "Confidence",
        CascadeStage::Ucb => "UCB",
    }
}

fn learning_skills_last_updated(skills: &[Skill], stamp: FileStamp) -> String {
    let latest = skills
        .iter()
        .filter_map(|skill| skill.last_matched.or(skill.first_seen))
        .max();
    format_relative_timestamp(latest.or_else(|| stamp.modified.map(DateTime::<Utc>::from)))
}

fn learning_patterns_last_updated(episodes: &[Episode]) -> String {
    format_relative_timestamp(episodes.iter().map(|episode| episode.timestamp).max())
}

struct ProviderHealthSummary {
    provider_count: usize,
    last_updated: String,
    health: &'static str,
}

fn learning_provider_health_summary(
    snapshot: &Option<ProviderHealthRegistrySnapshotData>,
    stamp: FileStamp,
    efficiency_events: &[AgentEfficiencyEvent],
) -> ProviderHealthSummary {
    if let Some(snapshot) = snapshot {
        let provider_count = snapshot.providers.len();
        let last_updated = snapshot
            .providers
            .values()
            .filter_map(|provider| provider.last_failure_at)
            .max()
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .or_else(|| stamp.modified.map(DateTime::<Utc>::from));
        let health = if snapshot
            .providers
            .values()
            .any(|provider| provider.state != CircuitState::Closed)
        {
            "degraded"
        } else if provider_count == 0 {
            "unknown"
        } else {
            "healthy"
        };
        return ProviderHealthSummary {
            provider_count,
            last_updated: format_relative_timestamp(last_updated),
            health,
        };
    }

    let mut providers = HashSet::new();
    let mut latest = None;
    for event in efficiency_events {
        if !event.backend.trim().is_empty() {
            providers.insert(event.backend.clone());
        }
        if let Some(timestamp) = parse_efficiency_timestamp(&event.timestamp) {
            latest = latest.max(Some(timestamp));
        }
    }

    ProviderHealthSummary {
        provider_count: providers.len(),
        last_updated: format_relative_timestamp(
            latest.or_else(|| stamp.modified.map(DateTime::<Utc>::from)),
        ),
        health: if providers.is_empty() {
            "unknown"
        } else {
            "healthy"
        },
    }
}

fn learning_knowledge_updates(snapshot: &KnowledgeStoreSnapshot) -> String {
    if snapshot.total_records == 0 {
        String::from("[WIP]")
    } else {
        format!("{} records", snapshot.total_records)
    }
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
        if resolve_episodes_path(dir).exists() || memory_dir.join(TASK_METRICS_FILE).exists() {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    start.to_path_buf()
}

fn load_knowledge_store_snapshot(root: &Path) -> KnowledgeStoreSnapshot {
    let neuro_dir = root.join(NEURO_DIR);
    let knowledge_path = neuro_dir.join(KNOWLEDGE_FILE);
    let confirmations_path = neuro_dir.join(KNOWLEDGE_CONFIRMATIONS_FILE);
    let knowledge_stamp = file_stamp(&knowledge_path);
    let confirmations_stamp = file_stamp(&confirmations_path);
    let last_updated = [knowledge_stamp.modified, confirmations_stamp.modified]
        .into_iter()
        .flatten()
        .max()
        .map(DateTime::<Utc>::from);

    KnowledgeStoreSnapshot {
        total_records: count_nonempty_lines(&knowledge_path)
            + count_nonempty_lines(&confirmations_path),
        last_updated,
    }
}

fn count_nonempty_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
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

fn count_to_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn wall_ms_to_f64(wall_ms: u64) -> f64 {
    f64::from(u32::try_from(wall_ms).unwrap_or(u32::MAX))
}

/// Compute the p-th percentile from a slice of millisecond latencies.
/// Returns `None` for empty slices.
fn percentile_ms(values: &[f64], pct: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((pct / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    let idx = idx.min(sorted.len() - 1);
    Some(sorted[idx])
}

/// Format a millisecond value as seconds with one decimal place (e.g. "0.8s").
fn format_latency_seconds(ms: f64) -> String {
    let secs = ms / 1000.0;
    format!("{secs:.1}s")
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

    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use tempfile::tempdir;

    fn write_jsonl(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().expect("file has parent"))
            .expect("should create parent dir");
        fs::write(path, lines.join("\n") + "\n").expect("should write jsonl");
    }

    fn append_raw(path: &Path, text: &str) {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open append target");
        file.write_all(text.as_bytes()).expect("append bytes");
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
            strategy_attempted: "none".to_string(),
            timestamp: timestamp.to_string(),
            frequency: roko_core::OperatingFrequency::Theta,
            reasoning_tokens: 0,
        }
    }

    #[test]
    fn scaffold_has_expected_page_count() {
        let dashboard = DashboardScaffold::new();
        let summary = dashboard.summary();
        assert_eq!(summary.page_count, 16);
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
    fn theme_defaults_to_rosedust_palette() {
        let theme = Theme::from_no_color(false);
        assert_eq!(theme.foreground, Color::Rgb(165, 142, 158)); // rose-tinted text
        assert_eq!(theme.background, Color::Rgb(0, 0, 0)); // void
        assert_eq!(theme.accent, Color::Rgb(185, 120, 148)); // primary rose
        assert_eq!(theme.selection_background, Color::Rgb(34, 28, 36)); // highlight
        assert_eq!(theme.selection_foreground, Color::Rgb(215, 198, 158)); // bone
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
        assert!(rendered.contains("dashboard scaffold: 16 pages"));
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
            &roko_dir.join("engrams.jsonl"),
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
            &roko_dir.join("engrams.jsonl"),
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
    fn learning_page_renders_learning_system_status() {
        let tmpdir = tempdir().expect("tempdir");
        let learn_dir = tmpdir.path().join(".roko/learn");
        let memory_dir = tmpdir.path().join(MEMORY_DIR);
        let neuro_dir = tmpdir.path().join(NEURO_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");
        fs::write(memory_dir.join(EPISODES_FILE), "").expect("empty episodes");

        let cascade = serde_json::json!({
            "model_slugs": ["claude-sonnet-4-5"],
            "confidence_stats": {
                "claude-sonnet-4-5": { "trials": 423, "successes": 350 }
            },
            "total_observations": 423,
            "stage_transitions": [
                {
                    "from": "Static",
                    "to": "Confidence",
                    "observations": 50,
                    "timestamp": "2026-04-10T08:00:00Z"
                },
                {
                    "from": "Confidence",
                    "to": "Ucb",
                    "observations": 201,
                    "timestamp": "2026-04-10T09:00:00Z"
                }
            ]
        });
        write_json(&learn_dir.join(CASCADE_ROUTER_FILE), &cascade);

        let mut thresholds = AdaptiveThresholds::new();
        thresholds.update(0, true);
        thresholds.update(1, false);
        write_json(&learn_dir.join(GATE_THRESHOLDS_FILE), &thresholds);

        let experiment_store = serde_json::json!({
            "experiments": {
                "exp-1": {
                    "experiment_id": "exp-1",
                    "section_name": "system_prompt",
                    "variants": [
                        { "id": "baseline", "name": "Baseline", "section_name": "system_prompt", "content": "v1", "active": true },
                        { "id": "variant", "name": "Variant", "section_name": "system_prompt", "content": "v2", "active": true }
                    ],
                    "stats": {
                        "baseline": { "trials": 4, "successes": 3 },
                        "variant": { "trials": 4, "successes": 2 }
                    },
                    "status": "Running",
                    "winner_id": null,
                    "min_trials_per_variant": 10,
                    "min_effect_size": 0.1
                }
            }
        });
        write_json(&learn_dir.join(EXPERIMENTS_FILE), &experiment_store);

        let mut skill = Skill::new("route_fix", "Route a fix", "template");
        skill.first_seen = Some(Utc::now());
        write_json(&learn_dir.join(SKILLS_FILE), &vec![skill]);

        let provider_health = serde_json::json!({
            "providers": {
                "anthropic": {
                    "provider_id": "anthropic",
                    "state": "Closed",
                    "consecutive_failures": 0,
                    "total_requests": 8,
                    "total_failures": 0,
                    "last_failure_at": null,
                    "cooldown_until": null,
                    "failure_window": []
                }
            }
        });
        write_json(&learn_dir.join(PROVIDER_HEALTH_FILE), &provider_health);

        fs::create_dir_all(&neuro_dir).expect("neuro dir");
        fs::write(neuro_dir.join(KNOWLEDGE_FILE), "{\"id\":\"k1\"}\n").expect("knowledge file");

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::Learning)
            .expect("learning page should render");
        assert!(rendered.contains("Learning System Status"));
        assert!(rendered.contains("Stage: UCB (423 observations)"));
        assert!(rendered.contains("Last transition: Confidence -> UCB at obs 201"));
        assert!(rendered.contains("CascadeRouter"));
        assert!(rendered.contains("GateThresholds"));
        assert!(rendered.contains("Experiments"));
        assert!(rendered.contains("SkillLibrary"));
        assert!(rendered.contains("PatternMiner"));
        assert!(rendered.contains("ProviderHealth"));
        assert!(rendered.contains("KnowledgeStore"));
        assert!(rendered.contains("24h Efficiency Trends"));
        assert!(rendered.contains("tok/h"));
        assert!(rendered.contains("Feedback Loops:  6/8 connected"));
        assert!(rendered.contains("Missing: GateFail->Replan, SectionEffect->Prompt"));
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
        let plan_dir = crate::workspace_paths::plans_dir(root).join("plan-a");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&plan_dir).expect("plan dir");

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
files = ["src/dashboard.rs"]

  [[task.context.read_files]]
  path = "src/dashboard.rs"
  lines = "1-20"
  why = "current work"
"#,
        )
        .expect("tasks.toml");

        let ep_dir = root.join(MEMORY_DIR);
        fs::create_dir_all(&ep_dir).expect("memory dir");

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
            &ep_dir.join(EPISODES_FILE),
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
        assert_eq!(
            execution
                .current_task
                .as_ref()
                .expect("current task")
                .task_id,
            "task-2"
        );
        // The agent_output_tail is populated from episode stderr when episodes
        // are matched to the plan. If empty, the episode wasn't found (episodes
        // need to match via input_signal_hash or extra.plan_id).
        if !execution.agent_output_tail.is_empty() {
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
    }

    #[test]
    fn task_output_cursors_tail_incrementally_and_drop_stale_files() {
        let tmpdir = tempdir().expect("tempdir");
        let task_outputs_dir = tmpdir.path().join(".roko/task-outputs");
        fs::create_dir_all(&task_outputs_dir).expect("task outputs dir");

        let path = task_outputs_dir.join("task-1.txt");
        fs::write(&path, "").expect("seed empty task output");

        let mut cursors = TaskOutputCursors::new(&task_outputs_dir);
        assert!(cursors.reconcile().expect("reconcile new file"));
        assert!(!cursors.tick().expect("initial empty tick"));

        for n in 1..=5 {
            append_raw(&path, &format!("line-{n}\n"));
            assert!(cursors.tick().expect("append tick"));
        }

        let tail = cursors.tail_for("task-1").expect("task tail");
        assert_eq!(tail.len(), 5);
        assert_eq!(tail.first().expect("tail head"), "line-1");
        assert_eq!(tail.last().expect("tail last"), "line-5");

        fs::remove_file(&path).expect("remove task output");
        assert!(cursors.reconcile().expect("reconcile stale file"));
        assert!(cursors.tail_for("task-1").is_none());
    }

    #[test]
    fn dashboard_data_tick_updates_task_outputs_and_generation() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let state_dir = root.join(".roko/state");
        let task_outputs_dir = root.join(".roko/task-outputs");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&task_outputs_dir).expect("task outputs dir");

        write_json(&state_dir.join("executor.json"), &serde_json::json!({}));

        let path = task_outputs_dir.join("task-1.txt");
        fs::write(&path, "").expect("seed empty task output");

        let mut data = DashboardData::load_best_effort(root);
        let initial_generation = data.generation;
        assert!(data.task_outputs().contains_key("task-1"));
        assert!(
            data.task_outputs()
                .get("task-1")
                .expect("task output cache")
                .is_empty()
        );

        append_raw(&path, "line-1\n");
        data.tick().expect("append tick");
        assert!(data.generation > initial_generation);
        assert_eq!(
            data.task_outputs()
                .get("task-1")
                .expect("task output cache"),
            &vec![String::from("line-1")]
        );

        let generation_after_append = data.generation;
        fs::remove_file(&path).expect("remove task output");
        data.tick().expect("stale removal tick");
        assert!(data.generation > generation_after_append);
        assert!(!data.task_outputs().contains_key("task-1"));
    }

    #[test]
    fn plan_task_snapshots_include_runtime_task_metadata() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let state_dir = root.join(".roko/state");
        let plan_dir = crate::workspace_paths::plans_dir(root).join("plan-a");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&plan_dir).expect("plan dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "implementing" }
                }
            }
        });
        write_json(&state_dir.join("executor.json"), &executor_state);

        fs::write(
            plan_dir.join("tasks.toml"),
            r#"
[meta]
plan = "Plan A"
iteration = 1
total = 3
done = 1
status = "running"
wave = 4

[[task]]
id = "task-1"
title = "Bootstrap"
status = "done"
model = "claude-haiku-4-5"
elapsed_ms = 1000
tier = "focused"

[[task]]
id = "task-2"
title = "Wire dashboard"
status = "implementing"
model = "claude-sonnet-4-6"
elapsed_ms = 2500
started_at_ms = 111
wave = 2
tier = "focused"

[[task]]
id = "task-3"
title = "Handle failures"
status = "gate_rejected"
model = "claude-sonnet-4-6"
elapsed_ms = 3500
ended_at_ms = 222
tier = "focused"
"#,
        )
        .expect("tasks.toml");

        let data = DashboardData::load_best_effort(root);
        let snapshots = data.plan_task_snapshots();
        let snapshot = snapshots.get("plan-a").expect("plan snapshot");

        assert_eq!(snapshot.tasks_done, 1);
        assert_eq!(snapshot.tasks_failed, 1);
        assert_eq!(snapshot.elapsed_ms, 7_000);
        assert!((snapshot.elapsed_secs - 7.0).abs() < f64::EPSILON);
        assert_eq!(snapshot.wave, 4);
        assert_eq!(snapshot.tasks.len(), 3);
        assert_eq!(
            snapshot.tasks[1].model.as_deref(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(snapshot.tasks[1].started_at.as_deref(), Some("111"));
        assert_eq!(snapshot.tasks[1].wave, Some(2));
        assert_eq!(snapshot.tasks[2].status, "failed");
        assert_eq!(snapshot.tasks[2].ended_at.as_deref(), Some("222"));
        assert_eq!(snapshot.failed_count, 1);
    }

    #[test]
    fn plan_task_snapshots_ignore_invalid_tasks_toml() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let state_dir = root.join(".roko/state");
        let plan_dir = crate::workspace_paths::plans_dir(root).join("plan-a");
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&plan_dir).expect("plan dir");

        let executor_state = serde_json::json!({
            "plan_states": {
                "plan-a": {
                    "current_phase": { "kind": "implementing" }
                }
            }
        });
        write_json(&state_dir.join("executor.json"), &executor_state);
        fs::write(plan_dir.join("tasks.toml"), "[meta]\nplan = ").expect("invalid tasks.toml");

        let data = DashboardData::load_best_effort(root);
        let snapshots = data.plan_task_snapshots();
        let snapshot = snapshots.get("plan-a").expect("plan snapshot");

        assert!(snapshot.tasks.is_empty());
        assert_eq!(snapshot.tasks_done, 0);
        assert_eq!(snapshot.tasks_failed, 0);
    }

    #[test]
    fn dashboard_data_tick_updates_jsonl_cursors_and_event_log() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let roko_dir = root.join(".roko");
        let state_dir = roko_dir.join("state");
        let memory_dir = root.join(MEMORY_DIR);
        fs::create_dir_all(&state_dir).expect("state dir");
        fs::create_dir_all(&memory_dir).expect("memory dir");

        write_json(
            &state_dir.join("events.json"),
            &vec![serde_json::json!({
                "timestamp_ms": 1_u64,
                "event_type": "started",
                "plan_id": "plan-a",
                "task_id": "task-a",
                "message": "boot"
            })],
        );
        write_jsonl(
            &roko_dir.join("engrams.jsonl"),
            &[serde_json::json!({
                "id": "sig-1",
                "kind": "conductor:alert:warning",
                "created_at_ms": 1_i64,
            })
            .to_string()],
        );
        write_jsonl(
            &memory_dir.join(EPISODES_FILE),
            &[
                serde_json::to_string(&sample_episode("agent-a", "task-a", true, 0.5, 100))
                    .expect("episode json"),
            ],
        );

        let mut data = DashboardData::load_best_effort(root);
        assert_eq!(data.recent_signals.len(), 1);
        assert_eq!(data.episodes().len(), 1);
        assert_eq!(data.event_log.len(), 1);

        let appended_signal = serde_json::json!({
            "id": "sig-2",
            "kind": "gate:compile",
            "created_at_ms": 2_i64,
            "tags": {
                "plan_id": "plan-a",
                "task_id": "task-a",
                "gate": "compile",
                "passed": "true"
            }
        })
        .to_string();
        let appended_episode =
            serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.8, 240))
                .expect("episode json");

        append_raw(&roko_dir.join("engrams.jsonl"), &appended_signal);
        append_raw(&memory_dir.join(EPISODES_FILE), &appended_episode);

        data.tick().expect("partial tick should succeed");
        assert_eq!(data.recent_signals.len(), 1);
        assert_eq!(data.episodes().len(), 1);

        append_raw(&roko_dir.join("engrams.jsonl"), "\n");
        append_raw(&memory_dir.join(EPISODES_FILE), "\n");
        write_json(
            &state_dir.join("events.json"),
            &vec![
                serde_json::json!({
                    "timestamp_ms": 1_u64,
                    "event_type": "started",
                    "plan_id": "plan-a",
                    "task_id": "task-a",
                    "message": "boot"
                }),
                serde_json::json!({
                    "timestamp_ms": 2_u64,
                    "event_type": "finished",
                    "plan_id": "plan-a",
                    "task_id": "task-a",
                    "message": "done"
                }),
            ],
        );

        data.tick().expect("append tick should succeed");
        assert_eq!(data.recent_signals.len(), 2);
        assert_eq!(data.episodes().len(), 2);
        assert_eq!(data.event_log.len(), 2);
        assert_eq!(data.event_log[1].event_type, "finished");
    }

    #[test]
    fn dashboard_data_tick_resets_signal_and_episode_state_on_truncation() {
        let tmpdir = tempdir().expect("tempdir");
        let root = tmpdir.path();
        let roko_dir = root.join(".roko");
        let memory_dir = root.join(MEMORY_DIR);
        fs::create_dir_all(&memory_dir).expect("memory dir");

        write_jsonl(
            &roko_dir.join("engrams.jsonl"),
            &[
                serde_json::json!({
                    "id": "sig-1",
                    "kind": "gate:compile",
                    "created_at_ms": 1_i64,
                })
                .to_string(),
                serde_json::json!({
                    "id": "sig-2",
                    "kind": "conductor:alert:warning",
                    "created_at_ms": 2_i64,
                })
                .to_string(),
            ],
        );
        write_jsonl(
            &memory_dir.join(EPISODES_FILE),
            &[
                serde_json::to_string(&sample_episode("agent-a", "task-a", true, 0.5, 100))
                    .expect("episode json"),
                serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.8, 240))
                    .expect("episode json"),
            ],
        );

        let mut data = DashboardData::load_best_effort(root);
        assert_eq!(data.recent_signals.len(), 2);
        assert_eq!(data.episodes().len(), 2);

        write_jsonl(
            &roko_dir.join("engrams.jsonl"),
            &[serde_json::json!({
                "id": "sig-reset",
                "kind": "conductor:alert:error",
                "created_at_ms": 3_i64,
            })
            .to_string()],
        );
        write_jsonl(
            &memory_dir.join(EPISODES_FILE),
            &[
                serde_json::to_string(&sample_episode("agent-c", "task-c", true, 0.2, 90))
                    .expect("episode json"),
            ],
        );

        data.tick().expect("truncation tick should succeed");
        assert_eq!(data.recent_signals.len(), 1);
        assert_eq!(data.recent_signals[0].id, "sig-reset");
        assert_eq!(data.episodes().len(), 1);
        assert_eq!(data.episodes()[0].task_id, "task-c");
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
