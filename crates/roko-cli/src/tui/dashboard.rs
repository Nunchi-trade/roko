//! Dashboard scaffold container for future TUI wiring.
//!
//! This module keeps the existing page scaffold intact, but layers a
//! best-effort learning snapshot on top so the health and trends pages
//! can render real stats when the memory JSONL files are present.

use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan::{PlanSummary, plans_dir};
use crate::task_parser::TasksFile;
use roko_core::metric::{Headlines, TaskMetric, compute_headlines};
use roko_gate::adaptive_threshold::AdaptiveThresholds;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::prompt_experiment::ExperimentStore;

use super::pages::{PageId, PageScaffold, efficiency, operations};

const MEMORY_DIR: &str = ".roko/memory";
const EPISODES_FILE: &str = "episodes.jsonl";
const TASK_METRICS_FILE: &str = "task-metrics.jsonl";

const LEARN_DIR: &str = ".roko/learn";
const EFFICIENCY_FILE: &str = "efficiency.jsonl";
const EXPERIMENTS_FILE: &str = "experiments.json";
const GATE_THRESHOLDS_FILE: &str = "gate-thresholds.json";
const CASCADE_ROUTER_FILE: &str = "cascade-router.json";

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
            PageId::Parameters => self.snapshot.render_parameters_page(scaffold),
            PageId::Experiments => self.snapshot.render_experiments_page(scaffold),
            PageId::Optimizer => self.snapshot.render_optimizer_page(scaffold),
            PageId::AgentStatus => self.snapshot.render_agent_status_page(scaffold),
            PageId::PlanView => self.snapshot.render_plan_view_page(scaffold),
            PageId::LogView => self.snapshot.render_log_view_page(scaffold),
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
    /// Cascade router state from `.roko/learn/cascade-router.json`.
    pub cascade_router: CascadeRouterState,
    /// Experiments from `.roko/learn/experiments.json`.
    pub experiments: Vec<ExperimentSummary>,
    /// Most recent signals from `.roko/signals.jsonl`.
    pub recent_signals: Vec<SignalSummary>,
    /// Conductor alerts filtered from signals.
    pub conductor_alerts: Vec<AlertSummary>,
    /// Latest C-Factor snapshot, if present.
    pub cfactor: Option<CFactor>,
}

impl DashboardData {
    /// Load dashboard data from a workspace root, falling back to empty data on errors.
    #[must_use]
    pub fn load_best_effort(root: impl AsRef<Path>) -> Self {
        let root = resolve_snapshot_root(root.as_ref());
        let roko_dir = root.join(".roko");
        let learn_dir = roko_dir.join("learn");
        let state_path = roko_dir.join("state").join("executor.json");
        let state = read_json_value(&state_path).unwrap_or(Value::Null);
        let signals_path = roko_dir.join("signals.jsonl");

        let plans = load_plan_summaries(&root, &state);
        let active_tasks = load_active_tasks(&state);
        let agents = load_agents(&state);
        let gate_results = load_gate_results(&state, &signals_path);
        let efficiency = load_efficiency_summary(&learn_dir.join(EFFICIENCY_FILE));
        let cascade_router = load_json_opt::<CascadeRouterState>(&learn_dir.join(CASCADE_ROUTER_FILE))
            .unwrap_or_default();
        let experiments = load_experiment_summaries(&learn_dir.join(EXPERIMENTS_FILE));
        let recent_signals = load_recent_signals(&signals_path, 100);
        let conductor_alerts = recent_signals
            .iter()
            .filter(|signal| signal.kind.starts_with("conductor:alert:"))
            .map(AlertSummary::from_signal)
            .collect();
        let cfactor = load_latest_jsonl_value::<CFactor>(&learn_dir.join("c-factor.jsonl"));

        Self {
            root,
            plans,
            active_tasks,
            agents,
            gate_results,
            efficiency,
            cascade_router,
            experiments,
            recent_signals,
            conductor_alerts,
            cfactor,
        }
    }

    /// Refresh the snapshot from the stored workspace root.
    pub async fn refresh(&mut self) -> Result<()> {
        let root = self.root.clone();
        let refreshed = tokio::task::spawn_blocking(move || Self::load_best_effort(root)).await?;
        *self = refreshed;
        Ok(())
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

/// Current C-Factor snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactor {
    pub overall: f64,
    pub components: CFactorComponents,
    pub computed_at: DateTime<Utc>,
    pub episode_count: usize,
}

/// C-Factor components.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorComponents {
    pub gate_pass_rate: f64,
    pub cost_efficiency: f64,
    pub speed: f64,
    pub first_try_rate: f64,
    pub knowledge_growth: f64,
}

impl Default for CFactorComponents {
    fn default() -> Self {
        Self {
            gate_pass_rate: 0.0,
            cost_efficiency: 0.0,
            speed: 0.0,
            first_try_rate: 0.0,
            knowledge_growth: 0.0,
        }
    }
}

impl Default for CFactor {
    fn default() -> Self {
        Self {
            overall: 0.0,
            components: CFactorComponents::default(),
            computed_at: Utc::now(),
            episode_count: 0,
        }
    }
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

impl ExperimentSummary {
    fn from_experiment(experiment: &roko_learn::prompt_experiment::PromptExperiment) -> Self {
        let total_trials: u64 = experiment.stats.values().map(|stats| stats.trials).sum();
        let active_variants = experiment.variants.iter().filter(|variant| variant.active).count();
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

    tasks.sort_by(|a, b| a.plan_id.cmp(&b.plan_id).then_with(|| a.task_id.cmp(&b.task_id)));
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

fn load_gate_results(state: &Value, signals_path: &Path) -> Vec<GateResultSummary> {
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
                    passed: result.get("passed").and_then(Value::as_bool).unwrap_or(false),
                    rung: result.get("rung").and_then(Value::as_u64).unwrap_or_default() as u32,
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
        gate_results.extend(
            read_jsonl_values(signals_path)
                .into_iter()
                .filter(|entry| {
                    entry
                        .get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| is_gate_result_kind(kind))
                })
                .filter_map(|entry| {
            let plan_id = entry
                .pointer("/tags/plan_id")
                .and_then(Value::as_str)
                .or_else(|| entry.pointer("/body/data/plan_id").and_then(Value::as_str))
                .or_else(|| entry.pointer("/body/plan_id").and_then(Value::as_str))
                .unwrap_or("unknown");
                    GateResultSummary::from_signal(&entry, plan_id)
                }),
        );
    }

    gate_results.sort_by(|a, b| {
        a.plan_id
            .cmp(&b.plan_id)
            .then_with(|| a.gate_name.cmp(&b.gate_name))
            .then_with(|| a.rung.cmp(&b.rung))
    });
    gate_results
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
    let average_wall_time_ms = events.iter().map(|event| event.wall_time_ms as f64).sum::<f64>()
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

fn load_experiment_summaries(path: &Path) -> Vec<ExperimentSummary> {
    let store = ExperimentStore::load_or_new(path);
    let mut experiments = store.iter().map(ExperimentSummary::from_experiment).collect::<Vec<_>>();
    experiments.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
    experiments
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

fn load_latest_jsonl_value<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .and_then(|line| serde_json::from_str(line).ok())
}

fn read_json_value(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_jsonl_values(path: &Path) -> Vec<Value> {
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
        .or_else(|| {
            entry
                .pointer("/body/data/passed")
                .and_then(Value::as_bool)
        })
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
        .or_else(|| entry.get("created_at_ms").and_then(Value::as_u64).map(|ts| ts as i64))
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
        let cascade_snapshot =
            load_json_opt::<CascadeSnapshotData>(&learn_dir.join(CASCADE_ROUTER_FILE));

        Ok(Self::from_records(
            root,
            &episodes,
            &task_metrics,
            efficiency_events,
            experiments,
            adaptive_thresholds,
            cascade_snapshot,
        ))
    }

    fn empty(root: PathBuf) -> Self {
        Self::from_records(root, &[], &[], Vec::new(), None, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn from_records(
        root: PathBuf,
        episodes: &[Episode],
        task_metrics: &[TaskMetric],
        efficiency_events: Vec<AgentEfficiencyEvent>,
        experiments: Option<ExperimentStore>,
        adaptive_thresholds: Option<AdaptiveThresholds>,
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
        if self.episodes.is_empty() {
            return None;
        }
        let mut out = page_header(page);

        // Aggregate per-agent stats from episodes.
        let mut agents: BTreeMap<String, AgentStats> = BTreeMap::new();
        for ep in &self.episodes {
            let entry = agents.entry(ep.agent_id.clone()).or_default();
            entry.turns += 1;
            entry.total_cost += ep.usage.cost_usd;
            entry.total_input_tokens += ep.usage.input_tokens;
            entry.total_output_tokens += ep.usage.output_tokens;
            if ep.success {
                entry.successes += 1;
            }
        }

        let _ = writeln!(
            out,
            "  {:>24}  {:>6}  {:>9}  {:>10}  {:>12}  {:>12}",
            "agent", "turns", "pass rate", "cost", "in tokens", "out tokens"
        );
        for (agent_id, stats) in &agents {
            let rate = count_to_f64(stats.successes) / count_to_f64(stats.turns);
            let _ = writeln!(
                out,
                "  {:>24}  {:>6}  {:>9}  {:>10}  {:>12}  {:>12}",
                agent_id,
                stats.turns,
                format_pct(rate),
                format_usd(stats.total_cost),
                stats.total_input_tokens,
                stats.total_output_tokens
            );
        }

        // Also show efficiency event model breakdown if available.
        if !self.efficiency_events.is_empty() {
            let _ = writeln!(out);
            let mut models: BTreeMap<String, usize> = BTreeMap::new();
            for ev in &self.efficiency_events {
                *models.entry(ev.model.clone()).or_default() += 1;
            }
            let _ = writeln!(out, "model usage:");
            for (model, count) in &models {
                let _ = writeln!(out, "  {model}: {count} turns");
            }
        }

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

/// Per-agent aggregated stats.
#[derive(Debug, Default)]
struct AgentStats {
    turns: usize,
    successes: usize,
    total_cost: f64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

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

        let episodes = vec![
            serde_json::to_string(&sample_episode("agent-a", "task-1", true, 0.5, 500))
                .expect("json"),
            serde_json::to_string(&sample_episode("agent-a", "task-2", false, 1.0, 1500))
                .expect("json"),
            serde_json::to_string(&sample_episode("agent-b", "task-3", true, 0.3, 300))
                .expect("json"),
        ];
        write_jsonl(&episodes_path, &episodes);

        let dashboard = DashboardScaffold::new_in(tmpdir.path());
        let rendered = dashboard
            .render_page_text(PageId::AgentStatus)
            .expect("agent status page should render");
        assert!(rendered.contains("Agent Status"));
        assert!(rendered.contains("agent-a"));
        assert!(rendered.contains("agent-b"));
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
