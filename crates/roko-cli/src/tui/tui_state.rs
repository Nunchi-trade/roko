//! Rendering-ready TUI state that bridges DashboardData (disk) +
//! DashboardSnapshot (live events) into a single struct consumed by all widgets.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use roko_core::dashboard_snapshot::DashboardSnapshot;

use super::dashboard::{DashboardData, PlanExecutionSnapshot, SignalSummary};
use super::mori_atmosphere::Atmosphere;
use roko_learn::cfactor::CFactor;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Top-level tab for F1..F6 keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Dashboard,
    Plans,
    Agents,
    Logs,
    Signals,
    Config,
}

static ALL_TABS: [Tab; 6] = [
    Tab::Dashboard,
    Tab::Plans,
    Tab::Agents,
    Tab::Logs,
    Tab::Signals,
    Tab::Config,
];

impl Tab {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Dashboard => "dash",
            Self::Plans => "plans",
            Self::Agents => "agents",
            Self::Logs => "logs",
            Self::Signals => "sigs",
            Self::Config => "cfg",
        }
    }

    pub fn all() -> &'static [Tab] {
        &ALL_TABS
    }

    pub fn from_index(i: usize) -> Option<Tab> {
        ALL_TABS.get(i).copied()
    }

    pub const fn fkey(&self) -> &'static str {
        match self {
            Self::Dashboard => "F1",
            Self::Plans => "F2",
            Self::Agents => "F3",
            Self::Logs => "F4",
            Self::Signals => "F5",
            Self::Config => "F6",
        }
    }
}

/// Sub-tab within the right panel of the dashboard view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailSubTab {
    Agents,
    Output,
    Diff,
    Errors,
    Git,
}

static ALL_DETAIL_TABS: [DetailSubTab; 5] = [
    DetailSubTab::Agents,
    DetailSubTab::Output,
    DetailSubTab::Diff,
    DetailSubTab::Errors,
    DetailSubTab::Git,
];

impl DetailSubTab {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Agents => "a:Agents",
            Self::Output => "o:Output",
            Self::Diff => "d:Diff",
            Self::Errors => "e:Errors",
            Self::Git => "g:Git",
        }
    }

    pub const fn key(&self) -> char {
        match self {
            Self::Agents => 'a',
            Self::Output => 'o',
            Self::Diff => 'd',
            Self::Errors => 'e',
            Self::Git => 'g',
        }
    }

    pub fn all() -> &'static [DetailSubTab] {
        &ALL_DETAIL_TABS
    }

    pub fn from_key(ch: char) -> Option<Self> {
        ALL_DETAIL_TABS.iter().find(|t| t.key() == ch).copied()
    }
}

/// Which panel currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusZone {
    PlanTree,
    PhaseCompact,
    TaskProgress,
    AgentOutput,
    CommandOutput,
}

// ---------------------------------------------------------------------------
// Plan / agent / phase data structs for rendering
// ---------------------------------------------------------------------------

/// A plan entry for the plan tree widget.
#[derive(Debug, Clone)]
pub struct PlanEntry {
    pub id: String,
    pub name: String,
    pub wave: Option<usize>,
    pub tasks_total: usize,
    pub tasks_done: usize,
    pub tasks_failed: usize,
    pub active: bool,
    pub phase: String,
    pub elapsed_secs: f64,
}

/// A wave grouping of plans.
#[derive(Debug, Clone)]
pub struct Wave {
    pub index: usize,
    pub plans: Vec<String>,
    pub done: usize,
    pub total: usize,
    pub expanded: bool,
}

/// An agent entry for the agent pool widget.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub role: String,
    pub model: String,
    pub active: bool,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_limit: u64,
    pub current_plan: String,
    pub current_task: String,
    pub last_output_line: String,
}

/// A gate result for the command output widget.
#[derive(Debug, Clone)]
pub struct GateEntry {
    pub plan_id: String,
    pub task_id: String,
    pub gate: String,
    pub passed: bool,
    pub output: String,
    pub ts_millis: u64,
}

/// A single step in the phase pipeline (left panel).
#[derive(Debug, Clone)]
pub struct PhaseStep {
    pub name: String,
    pub status: PhaseStatus,
    pub elapsed_secs: f64,
    pub pct: f64,
}

/// Status of a phase step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStatus {
    Done,
    Active,
    Pending,
    Failed,
}

/// A row in the task progress checklist.
#[derive(Debug, Clone)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub status: TaskRowStatus,
    pub elapsed_secs: f64,
}

/// Status of a task row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskRowStatus {
    Done,
    Active,
    Blocked,
    Pending,
    Failed,
}

/// System metrics from sysinfo.
#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    pub cpu_pct: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub net_up_bytes_sec: u64,
    pub net_down_bytes_sec: u64,
    pub disk_read_bytes_sec: u64,
    pub disk_write_bytes_sec: u64,
    pub cpu_history: VecDeque<f32>,
    pub mem_history: VecDeque<f32>,
}

const HISTORY_CAP: usize = 120;
const TOKEN_HISTORY_CAP: usize = 120;

// ---------------------------------------------------------------------------
// TuiState
// ---------------------------------------------------------------------------

/// The combined rendering state consumed by all TUI widgets.
pub struct TuiState {
    // From DashboardData (disk-loaded, always available)
    pub plans: Vec<PlanEntry>,
    pub execution_waves: Vec<Wave>,
    pub agents: Vec<AgentEntry>,
    pub gate_results: Vec<GateEntry>,
    pub recent_signals: Vec<SignalSummary>,
    pub cfactor: Option<CFactor>,
    pub current_plan_execution: Option<PlanExecutionSnapshot>,

    // From DashboardSnapshot (live events, when orchestrator running)
    pub live: DashboardSnapshot,

    // Derived / computed for rendering
    pub token_history: HashMap<String, VecDeque<u64>>,
    pub token_total: u64,
    pub token_rate: f64,
    pub sys: SystemMetrics,
    pub phase_pipeline: Vec<PhaseStep>,
    pub current_task_checklist: Vec<TaskRow>,

    // UI state
    pub active_tab: Tab,
    pub detail_sub_tab: DetailSubTab,
    pub focus: FocusZone,
    pub plan_scroll: usize,
    pub task_scroll: usize,
    pub selected_plan: usize,
    pub selected_agent: usize,
    pub output_scroll: usize,
    pub filter: String,
    pub atmosphere: Atmosphere,

    // Git info
    pub git_branch: String,
    pub git_commit_short: String,
    pub git_age: String,

    // Timing
    pub start_time: Instant,
    pub cost_dollars: f64,
}

impl TuiState {
    /// Build initial TuiState from disk-loaded DashboardData.
    pub fn from_dashboard_data(data: &DashboardData) -> Self {
        let mut state = Self {
            plans: Vec::new(),
            execution_waves: Vec::new(),
            agents: Vec::new(),
            gate_results: Vec::new(),
            recent_signals: data.recent_signals.clone(),
            cfactor: data.cfactor.clone(),
            current_plan_execution: data.current_plan_execution.clone(),
            live: DashboardSnapshot::default(),
            token_history: HashMap::new(),
            token_total: 0,
            token_rate: 0.0,
            sys: SystemMetrics::default(),
            phase_pipeline: Vec::new(),
            current_task_checklist: Vec::new(),
            active_tab: Tab::Dashboard,
            detail_sub_tab: DetailSubTab::Agents,
            focus: FocusZone::PlanTree,
            plan_scroll: 0,
            task_scroll: 0,
            selected_plan: 0,
            selected_agent: 0,
            output_scroll: 0,
            filter: String::new(),
            atmosphere: Atmosphere::new(),
            git_branch: String::new(),
            git_commit_short: String::new(),
            git_age: String::new(),
            start_time: Instant::now(),
            cost_dollars: 0.0,
        };
        state.sync_from_data(data);
        state
    }

    /// Re-sync derived fields from refreshed DashboardData.
    pub fn sync_from_data(&mut self, data: &DashboardData) {
        // Plans — map from PlanSummary (id, title, task_count, completed)
        self.plans = data
            .plans
            .iter()
            .map(|p| PlanEntry {
                id: p.id.clone(),
                name: p.title.clone(),
                wave: None,
                tasks_total: p.task_count,
                tasks_done: if p.completed { p.task_count } else { 0 },
                tasks_failed: 0,
                active: !p.completed,
                phase: if p.completed {
                    "done".to_string()
                } else {
                    "pending".to_string()
                },
                elapsed_secs: 0.0,
            })
            .collect();

        // Enrich plans from current_plan_execution if available
        if let Some(exec) = &data.current_plan_execution {
            if let Some(pe) = self.plans.iter_mut().find(|p| p.id == exec.plan_id) {
                pe.tasks_done = exec.tasks_done;
                pe.tasks_total = exec.tasks_total;
                pe.active = true;
                pe.phase = "executing".to_string();
            }
        }

        // Agents — map from AgentSummary (id, label, plan_id, status)
        self.agents = data
            .agents
            .iter()
            .map(|a| AgentEntry {
                id: a.id.clone(),
                role: a.label.clone(),
                model: String::new(),
                active: a.status == "running" || a.status == "active",
                input_tokens: 0,
                output_tokens: 0,
                context_limit: 200_000,
                current_plan: a.plan_id.clone().unwrap_or_default(),
                current_task: String::new(),
                last_output_line: String::new(),
            })
            .collect();

        // Gate results — map from GateResultSummary (plan_id, gate_name, passed, ...)
        self.gate_results = data
            .gate_results
            .iter()
            .map(|g| GateEntry {
                plan_id: g.plan_id.clone(),
                task_id: String::new(),
                gate: g.gate_name.clone(),
                passed: g.passed,
                output: g.summary.clone(),
                ts_millis: 0,
            })
            .collect();

        // Signals + cfactor
        self.recent_signals = data.recent_signals.clone();
        self.cfactor = data.cfactor.clone();
        self.current_plan_execution = data.current_plan_execution.clone();

        // Build phase pipeline from current execution
        self.rebuild_phase_pipeline();
        self.rebuild_task_checklist();
        self.rebuild_waves();

        // Token totals from efficiency events
        let mut total = 0u64;
        for ev in &data.efficiency_events {
            let tokens = ev.input_tokens + ev.output_tokens;
            total += tokens;
        }
        self.token_total = total;
    }

    /// Update from a live DashboardSnapshot (event-driven).
    pub fn update_from_snapshot(&mut self, snap: &DashboardSnapshot) {
        self.live = snap.clone();

        // Merge live plan state over disk plans
        for (id, ps) in &snap.plans {
            if let Some(pe) = self.plans.iter_mut().find(|p| p.id == *id) {
                pe.active = ps.active;
                pe.phase = ps.phase.clone();
                pe.tasks_done = ps.tasks_done;
                pe.tasks_failed = ps.tasks_failed;
                pe.tasks_total = ps.tasks_total;
            }
        }

        // Merge live agents
        for (id, agent) in &snap.agents {
            if let Some(ae) = self.agents.iter_mut().find(|a| a.id == *id) {
                ae.active = agent.active;
                ae.role = agent.role.clone();
            }
        }
    }

    /// Record a token sample for an agent role's sparkline.
    pub fn push_token_sample(&mut self, role: &str, cumulative: u64) {
        let history = self
            .token_history
            .entry(role.to_string())
            .or_insert_with(|| VecDeque::with_capacity(TOKEN_HISTORY_CAP));
        if history.len() >= TOKEN_HISTORY_CAP {
            history.pop_front();
        }
        history.push_back(cumulative);
    }

    /// Update system metrics (call periodically).
    pub fn update_sys_metrics(&mut self, cpu: f32, mem_used: u64, mem_total: u64) {
        self.sys.cpu_pct = cpu;
        self.sys.mem_used_bytes = mem_used;
        self.sys.mem_total_bytes = mem_total;
        if self.sys.cpu_history.len() >= HISTORY_CAP {
            self.sys.cpu_history.pop_front();
        }
        self.sys.cpu_history.push_back(cpu);
        if self.sys.mem_history.len() >= HISTORY_CAP {
            self.sys.mem_history.pop_front();
        }
        let mem_pct = if mem_total > 0 {
            (mem_used as f32 / mem_total as f32) * 100.0
        } else {
            0.0
        };
        self.sys.mem_history.push_back(mem_pct);
    }

    /// Tick the atmosphere (call every frame).
    pub fn tick(&mut self) {
        self.atmosphere.tick();
    }

    // -- Internal helpers --

    fn rebuild_phase_pipeline(&mut self) {
        // Standard phase pipeline for roko plans
        let phase_names = [
            "preflight",
            "strategist",
            "implementer",
            "compile-gate",
            "test-gate",
            "reviewing",
            "critic-review",
            "verdict",
            "committing",
        ];

        let current_phase = self
            .current_plan_execution
            .as_ref()
            .and_then(|e| {
                e.tasks
                    .iter()
                    .find(|t| t.is_current)
                    .or_else(|| e.tasks.first())
                    .map(|t| t.phase.as_str())
            })
            .unwrap_or("");

        let mut found_current = false;
        self.phase_pipeline = phase_names
            .iter()
            .map(|&name| {
                let status = if found_current {
                    PhaseStatus::Pending
                } else if name == current_phase {
                    found_current = true;
                    PhaseStatus::Active
                } else {
                    PhaseStatus::Done
                };
                PhaseStep {
                    name: name.to_string(),
                    status,
                    elapsed_secs: 0.0,
                    pct: 0.0,
                }
            })
            .collect();

        // If no current phase matched, all are pending
        if !found_current && !current_phase.is_empty() {
            // Check for "done" / "failed"
            if current_phase == "done" || current_phase == "complete" {
                for step in &mut self.phase_pipeline {
                    step.status = PhaseStatus::Done;
                }
            }
        }
    }

    fn rebuild_task_checklist(&mut self) {
        self.current_task_checklist.clear();
        if let Some(exec) = &self.current_plan_execution {
            for t in &exec.tasks {
                // PlanExecutionTaskRow has: task_id, title, phase, frequency, model,
                // duration (String), is_current
                let status = if t.is_current {
                    TaskRowStatus::Active
                } else if t.phase == "done" || t.phase == "completed" {
                    TaskRowStatus::Done
                } else if t.phase == "failed" {
                    TaskRowStatus::Failed
                } else if t.phase == "blocked" {
                    TaskRowStatus::Blocked
                } else {
                    TaskRowStatus::Pending
                };
                self.current_task_checklist.push(TaskRow {
                    id: t.task_id.clone(),
                    title: t.title.clone(),
                    status,
                    elapsed_secs: 0.0,
                });
            }
        }
    }

    fn rebuild_waves(&mut self) {
        // Group plans by wave if they have wave info
        let mut wave_map: HashMap<usize, Vec<String>> = HashMap::new();
        for plan in &self.plans {
            if let Some(w) = plan.wave {
                wave_map.entry(w).or_default().push(plan.id.clone());
            }
        }
        if wave_map.is_empty() {
            self.execution_waves.clear();
            return;
        }
        let mut indices: Vec<usize> = wave_map.keys().copied().collect();
        indices.sort_unstable();
        self.execution_waves = indices
            .into_iter()
            .map(|idx| {
                let plan_ids = wave_map.remove(&idx).unwrap_or_default();
                let done = plan_ids
                    .iter()
                    .filter(|id| {
                        self.plans
                            .iter()
                            .find(|p| p.id == **id)
                            .map_or(false, |p| !p.active && p.tasks_failed == 0)
                    })
                    .count();
                let total = plan_ids.len();
                Wave {
                    index: idx,
                    plans: plan_ids,
                    done,
                    total,
                    expanded: true,
                }
            })
            .collect();
    }

    /// Total elapsed seconds since the TUI started.
    pub fn elapsed_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }

    /// Progress ratio: tasks done / total across all plans.
    pub fn progress_ratio(&self) -> f64 {
        let total: usize = self.plans.iter().map(|p| p.tasks_total).sum();
        let done: usize = self.plans.iter().map(|p| p.tasks_done).sum();
        if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        }
    }

    /// Total tasks done / total.
    pub fn task_counts(&self) -> (usize, usize) {
        let total: usize = self.plans.iter().map(|p| p.tasks_total).sum();
        let done: usize = self.plans.iter().map(|p| p.tasks_done).sum();
        (done, total)
    }

    /// Number of active agents.
    pub fn active_agent_count(&self) -> usize {
        self.agents.iter().filter(|a| a.active).count()
    }

    /// Current wave index (0-based).
    pub fn current_wave(&self) -> usize {
        self.execution_waves
            .iter()
            .position(|w| w.done < w.total)
            .unwrap_or(0)
    }

    /// Total wave count.
    pub fn wave_count(&self) -> usize {
        self.execution_waves.len()
    }
}
