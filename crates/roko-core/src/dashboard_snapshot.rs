//! Materialized dashboard state driven by events.
//!
//! [`DashboardSnapshot`] is the single source of truth for all dashboard
//! consumers (TUI, WebSocket, SSE, REST). It is updated atomically via
//! [`apply`] when the [`StateHub`](super::state_hub::StateHub) receives events.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Event type — the only thing DashboardSnapshot knows how to ingest
// ---------------------------------------------------------------------------

/// Events that mutate the dashboard snapshot.
///
/// These map 1:1 to the interesting subset of `ServerEvent` variants from
/// `roko-serve`. The conversion happens at the call-site so that `roko-core`
/// stays free of server dependencies.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DashboardEvent {
    /// A plan execution started.
    PlanStarted { plan_id: String },
    /// A plan execution completed.
    PlanCompleted { plan_id: String, success: bool },
    /// A task started executing.
    TaskStarted {
        plan_id: String,
        task_id: String,
        phase: String,
    },
    /// A task completed.
    TaskCompleted {
        plan_id: String,
        task_id: String,
        outcome: String,
    },
    /// A task changed phase.
    TaskPhaseChanged {
        plan_id: String,
        task_id: String,
        old_phase: String,
        new_phase: String,
    },
    /// An agent was spawned.
    AgentSpawned { agent_id: String, role: String },
    /// Incremental agent output.
    AgentOutput { agent_id: String, content: String },
    /// A gate check completed.
    GateResult {
        plan_id: String,
        task_id: String,
        gate: String,
        passed: bool,
    },
    /// The plan transitioned between phases.
    PhaseTransition {
        plan_id: String,
        from: String,
        to: String,
    },
    /// An efficiency metric was recorded.
    EfficiencyEvent {
        plan_id: String,
        task_id: String,
        metric: String,
        value: f64,
    },
    /// A conductor diagnosis was recorded.
    Diagnosis {
        /// The summarized diagnosis payload to append to the ring buffer.
        summary: DiagnosisSummary,
    },
    /// Prompt experiment winners were refreshed from the learning store.
    ExperimentWinnersUpdated {
        /// Current concluded winners sorted for deterministic rendering.
        winners: Vec<ExperimentWinnerSummary>,
    },
    /// Recent c-factor trend buckets were refreshed from the learning store.
    CFactorTrendUpdated {
        /// Current rolling c-factor buckets for the Learning tab.
        buckets: Vec<CFactorBucket>,
    },
    /// An error occurred.
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// A single plan's live state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanState {
    /// Plan identifier.
    pub plan_id: String,
    /// Current phase name.
    pub phase: String,
    /// Total tasks registered for this plan.
    pub tasks_total: usize,
    /// Tasks that completed successfully.
    pub tasks_done: usize,
    /// Tasks that failed.
    pub tasks_failed: usize,
    /// Whether the plan is still executing.
    pub active: bool,
}

/// A single task's live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskState {
    /// Task identifier.
    pub task_id: String,
    /// Parent plan identifier.
    pub plan_id: String,
    /// Current phase name.
    pub phase: String,
    /// Outcome string, if completed.
    pub outcome: Option<String>,
}

/// A single agent's live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    /// Agent identifier.
    pub agent_id: String,
    /// Agent role.
    pub role: String,
    /// Whether the agent is still running.
    pub active: bool,
    /// Byte count of output received so far.
    pub output_bytes: usize,
}

/// A single gate verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdict {
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// Gate name.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Unix timestamp in milliseconds.
    pub ts_millis: u64,
}

/// Recent error entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEntry {
    /// Error message.
    pub message: String,
    /// Unix timestamp in milliseconds.
    pub ts_millis: u64,
}

/// Operator-facing severity for conductor diagnoses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosisSeverity {
    /// Informational guidance.
    #[default]
    Info,
    /// Operator attention recommended.
    Warn,
    /// Immediate intervention recommended.
    Alert,
}

/// A summarized conductor diagnosis surfaced to the dashboard and HTTP API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisSummary {
    /// Stable identifier for deduplication.
    #[serde(default)]
    pub id: String,
    /// When the diagnosis was produced.
    #[serde(default = "default_diagnosis_timestamp")]
    pub ts: DateTime<Utc>,
    /// Severity bucket for UI rendering.
    #[serde(default)]
    pub severity: DiagnosisSeverity,
    /// Short subject line describing what was diagnosed.
    #[serde(default)]
    pub subject: String,
    /// Human-readable detail or excerpt.
    #[serde(default)]
    pub detail: String,
    /// Suggested next action from the conductor.
    #[serde(default)]
    pub suggested_action: Option<String>,
    /// Action already taken automatically, if any.
    #[serde(default)]
    pub intervention_taken: Option<String>,
}

impl Default for DiagnosisSummary {
    fn default() -> Self {
        Self {
            id: String::new(),
            ts: default_diagnosis_timestamp(),
            severity: DiagnosisSeverity::default(),
            subject: String::new(),
            detail: String::new(),
            suggested_action: None,
            intervention_taken: None,
        }
    }
}

/// Summary row for one concluded prompt experiment winner.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExperimentWinnerSummary {
    /// Stable experiment identifier.
    #[serde(default)]
    pub experiment_id: String,
    /// Parameter under test, typically a prompt section or role name.
    #[serde(default)]
    pub parameter: String,
    /// Human-readable winner label shown to the operator.
    #[serde(default)]
    pub winner: String,
    /// Variant identifier for the winning arm.
    #[serde(default)]
    pub winner_variant_id: String,
    /// Winner empirical success rate in `[0.0, 1.0]`.
    #[serde(default)]
    pub win_rate: f64,
    /// Number of trials observed for the winning arm.
    #[serde(default)]
    pub sample_size: u64,
    /// Lower 95% confidence bound for the winner success rate.
    #[serde(default)]
    pub ci_lower: f64,
    /// Upper 95% confidence bound for the winner success rate.
    #[serde(default)]
    pub ci_upper: f64,
    /// Confidence score used to conclude the experiment.
    #[serde(default)]
    pub confidence: f64,
}

/// Shared agent-topology payload used by the TUI and HTTP API.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTopology {
    /// Flat node roster reported by `roko-serve`.
    #[serde(default)]
    pub nodes: Vec<AgentTopologyNode>,
    /// Directed relationships between agent nodes, when available.
    #[serde(default)]
    pub edges: Vec<AgentTopologyEdge>,
    /// Unix timestamp in seconds for the snapshot.
    #[serde(default)]
    pub timestamp: u64,
}

impl AgentTopology {
    /// Returns `true` when the topology carries no nodes or edges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

/// One node in the shared agent-topology payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTopologyNode {
    /// Stable agent identifier.
    #[serde(default)]
    pub id: String,
    /// Reachable address or card URI, when known.
    #[serde(default)]
    pub address: String,
    /// Count of insights posted by this agent.
    #[serde(default)]
    pub insights_posted: usize,
    /// Count of confirmations emitted by this agent.
    #[serde(default)]
    pub confirmations_given: usize,
    /// Count of challenges emitted by this agent.
    #[serde(default)]
    pub challenges_given: usize,
    /// Aggregate routing weight.
    #[serde(default)]
    pub total_weight: f64,
}

/// One edge in the shared agent-topology payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentTopologyEdge {
    /// Source node identifier.
    #[serde(default)]
    pub from: String,
    /// Target node identifier.
    #[serde(default)]
    pub to: String,
    /// Integer edge weight.
    #[serde(default)]
    pub weight: usize,
    /// Logical edge type.
    #[serde(default, rename = "type")]
    pub edge_type: String,
}

/// One bucket of aggregated efficiency telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EfficiencyBucket {
    /// Bucket start timestamp in UTC.
    #[serde(default = "default_diagnosis_timestamp")]
    pub start: DateTime<Utc>,
    /// Number of turns recorded in the bucket.
    #[serde(default)]
    pub turns: u64,
    /// Sum of input tokens across the bucket.
    #[serde(default)]
    pub tokens_in: u64,
    /// Sum of output tokens across the bucket.
    #[serde(default)]
    pub tokens_out: u64,
    /// Sum of recorded cost in USD cents.
    #[serde(default)]
    pub cost_usd_cents: u64,
    /// Average latency in milliseconds for turns in the bucket.
    #[serde(default)]
    pub latency_ms_avg: f64,
}

impl Default for EfficiencyBucket {
    fn default() -> Self {
        Self {
            start: default_diagnosis_timestamp(),
            turns: 0,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd_cents: 0,
            latency_ms_avg: 0.0,
        }
    }
}

/// One bucket of aggregated c-factor telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CFactorBucket {
    /// Bucket start timestamp in UTC.
    #[serde(default = "default_diagnosis_timestamp")]
    pub start: DateTime<Utc>,
    /// Number of c-factor snapshots recorded in the bucket.
    #[serde(default)]
    pub samples: u32,
    /// Average overall c-factor score across snapshots in the bucket.
    #[serde(default)]
    pub avg: f64,
    /// Median overall c-factor score across snapshots in the bucket.
    #[serde(default)]
    pub p50: f64,
    /// p95 overall c-factor score across snapshots in the bucket.
    #[serde(default)]
    pub p95: f64,
}

impl Default for CFactorBucket {
    fn default() -> Self {
        Self {
            start: default_diagnosis_timestamp(),
            samples: 0,
            avg: 0.0,
            p50: 0.0,
            p95: 0.0,
        }
    }
}

/// One hourly gate-trend bucket capturing pass/fail counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrendBucket {
    /// Bucket start timestamp in UTC.
    #[serde(default = "default_diagnosis_timestamp")]
    pub start: DateTime<Utc>,
    /// Number of passing verdicts in the bucket.
    #[serde(default)]
    pub pass: u32,
    /// Number of failing verdicts in the bucket.
    #[serde(default)]
    pub fail: u32,
}

impl Default for TrendBucket {
    fn default() -> Self {
        Self {
            start: default_diagnosis_timestamp(),
            pass: 0,
            fail: 0,
        }
    }
}

/// Fixed-width rolling pass/fail buckets for one gate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrendBuckets {
    /// Width of each bucket in seconds.
    #[serde(default)]
    pub bucket_size_secs: u64,
    /// Buckets ordered from oldest to newest.
    #[serde(default)]
    pub slots: VecDeque<TrendBucket>,
}

impl TrendBuckets {
    /// Create a fixed-width rolling window anchored to `reference`.
    #[must_use]
    pub fn new(bucket_size_secs: u64, bucket_count: usize, reference: DateTime<Utc>) -> Self {
        if bucket_size_secs == 0 || bucket_count == 0 {
            return Self::default();
        }

        let bucket_ms = i64::try_from(bucket_size_secs)
            .unwrap_or(i64::MAX / 1000)
            .saturating_mul(1000);
        let latest_start_ms = current_bucket_start_ms(reference.timestamp_millis(), bucket_ms);
        let oldest_start_ms = latest_start_ms
            .saturating_sub(i64::try_from(bucket_count.saturating_sub(1)).unwrap_or(0) * bucket_ms);

        let slots = (0..bucket_count)
            .map(|idx| TrendBucket {
                start: timestamp_from_millis(
                    oldest_start_ms + i64::try_from(idx).unwrap_or(0) * bucket_ms,
                ),
                ..TrendBucket::default()
            })
            .collect();

        Self {
            bucket_size_secs,
            slots,
        }
    }

    /// Shift the rolling window forward so it covers `reference`.
    pub fn align_to(&mut self, reference: DateTime<Utc>) {
        if self.bucket_size_secs == 0 {
            return;
        }

        let target_len = self.slots.len().max(GATE_TREND_BUCKET_COUNT);
        if self.slots.is_empty() {
            *self = Self::new(self.bucket_size_secs, target_len, reference);
            return;
        }

        let bucket_ms = i64::try_from(self.bucket_size_secs)
            .unwrap_or(i64::MAX / 1000)
            .saturating_mul(1000);
        let latest_start = trend_bucket_start(reference, self.bucket_size_secs);

        while self
            .slots
            .back()
            .is_some_and(|bucket| bucket.start < latest_start)
        {
            self.slots.pop_front();
            let next_start_ms = self
                .slots
                .back()
                .map(|bucket| bucket.start.timestamp_millis().saturating_add(bucket_ms))
                .unwrap_or_else(|| latest_start.timestamp_millis());
            self.slots.push_back(TrendBucket {
                start: timestamp_from_millis(next_start_ms),
                ..TrendBucket::default()
            });
        }

        while self.slots.len() < target_len {
            let first_start_ms = self
                .slots
                .front()
                .map(|bucket| bucket.start.timestamp_millis().saturating_sub(bucket_ms))
                .unwrap_or_else(|| latest_start.timestamp_millis());
            self.slots.push_front(TrendBucket {
                start: timestamp_from_millis(first_start_ms),
                ..TrendBucket::default()
            });
        }
    }

    /// Record a pass/fail observation in the matching bucket.
    pub fn record_gate_result(&mut self, ts: DateTime<Utc>, passed: bool) {
        if self.bucket_size_secs == 0 {
            *self = Self::new(GATE_TREND_BUCKET_SIZE_SECS, GATE_TREND_BUCKET_COUNT, ts);
        }
        self.align_to(ts);
        let bucket_start = trend_bucket_start(ts, self.bucket_size_secs);
        let Some(bucket) = self
            .slots
            .iter_mut()
            .find(|bucket| bucket.start == bucket_start)
        else {
            return;
        };

        if passed {
            bucket.pass = bucket.pass.saturating_add(1);
        } else {
            bucket.fail = bucket.fail.saturating_add(1);
        }
    }
}

/// One recent failing gate verdict surfaced in the dashboard.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FailureEntry {
    /// When the gate failed.
    #[serde(default = "default_diagnosis_timestamp")]
    pub ts: DateTime<Utc>,
    /// Parent plan identifier when known.
    #[serde(default)]
    pub plan_id: String,
    /// Task identifier when known.
    #[serde(default)]
    pub task_id: String,
    /// Gate name.
    #[serde(default)]
    pub gate: String,
    /// Short failure summary or reason when available.
    #[serde(default)]
    pub summary: String,
    /// Optional artifacts directory or file path associated with the failure.
    #[serde(default)]
    pub artifacts: Option<PathBuf>,
}

/// The full materialized dashboard state.
///
/// Updated atomically by [`StateHub`](super::state_hub::StateHub) via
/// `watch::Sender::send_modify`. Consumers (TUI, web, API) borrow this
/// through a `watch::Receiver` for zero-copy reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    /// Active and recently completed plans.
    pub plans: HashMap<String, PlanState>,
    /// Active tasks keyed by `"{plan_id}/{task_id}"`.
    pub tasks: HashMap<String, TaskState>,
    /// Active agents keyed by agent_id.
    pub agents: HashMap<String, AgentState>,
    /// Recent gate verdicts (ring of last 256).
    pub gates: Vec<GateVerdict>,
    /// Recent conductor diagnoses (ring of last 50).
    #[serde(default)]
    pub diagnoses: VecDeque<DiagnosisSummary>,
    /// Concluded prompt experiment winners rendered on the Learning tab.
    #[serde(default)]
    pub experiment_winners: Vec<ExperimentWinnerSummary>,
    /// Latest fetched agent-topology payload for the Agents tab.
    #[serde(default)]
    pub agent_topology: AgentTopology,
    /// Recent efficiency trend buckets for dashboard charts.
    #[serde(default)]
    pub efficiency_trend: Vec<EfficiencyBucket>,
    /// Recent c-factor trend buckets for dashboard charts.
    #[serde(default)]
    pub cfactor_trend: Vec<CFactorBucket>,
    /// Rolling per-gate pass/fail timelines over the last 24 hours.
    #[serde(default)]
    pub gate_trends: HashMap<String, TrendBuckets>,
    /// Recent failing verdicts across all gates.
    #[serde(default)]
    pub gate_recent_failures: Vec<FailureEntry>,
    /// Recent errors (ring of last 64).
    pub errors: Vec<ErrorEntry>,
    /// Overall counts.
    pub stats: SnapshotStats,
}

/// Aggregate counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotStats {
    /// Number of plans currently executing.
    pub plans_active: usize,
    /// Number of plans that completed successfully.
    pub plans_completed: usize,
    /// Number of plans that failed.
    pub plans_failed: usize,
    /// Number of tasks currently executing.
    pub tasks_active: usize,
    /// Number of tasks that completed successfully.
    pub tasks_completed: usize,
    /// Number of tasks that failed.
    pub tasks_failed: usize,
    /// Number of agents currently running.
    pub agents_active: usize,
    /// Number of gates that passed.
    pub gates_passed: usize,
    /// Number of gates that failed.
    pub gates_failed: usize,
    /// Total errors recorded.
    pub errors_total: usize,
}

// ---------------------------------------------------------------------------
// Ring buffer limits
// ---------------------------------------------------------------------------

const MAX_GATES: usize = 256;
const MAX_DIAGNOSES: usize = 50;
const MAX_ERRORS: usize = 64;
const MAX_GATE_FAILURES: usize = 50;
const GATE_TREND_BUCKET_SIZE_SECS: u64 = 60 * 60;
const GATE_TREND_BUCKET_COUNT: usize = 24;

// ---------------------------------------------------------------------------
// apply()
// ---------------------------------------------------------------------------

impl DashboardSnapshot {
    /// Apply a single event, mutating the snapshot in place.
    ///
    /// This is called inside `watch::Sender::send_modify` so that all
    /// subscribers see a consistent view after each event.
    pub fn apply(&mut self, event: &DashboardEvent) {
        self.apply_with_ts(event, current_ts_millis());
    }

    /// Apply with an explicit timestamp (for testing).
    pub fn apply_with_ts(&mut self, event: &DashboardEvent, ts: u64) {
        match event {
            DashboardEvent::PlanStarted { plan_id } => {
                self.stats.plans_active += 1;
                self.plans.insert(
                    plan_id.clone(),
                    PlanState {
                        plan_id: plan_id.clone(),
                        phase: "started".into(),
                        active: true,
                        ..Default::default()
                    },
                );
            }
            DashboardEvent::PlanCompleted { plan_id, success } => {
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.active = false;
                    plan.phase = if *success {
                        "completed".into()
                    } else {
                        "failed".into()
                    };
                }
                self.stats.plans_active = self.stats.plans_active.saturating_sub(1);
                if *success {
                    self.stats.plans_completed += 1;
                } else {
                    self.stats.plans_failed += 1;
                }
            }
            DashboardEvent::TaskStarted {
                plan_id,
                task_id,
                phase,
            } => {
                self.stats.tasks_active += 1;
                let key = format!("{plan_id}/{task_id}");
                self.tasks.insert(
                    key,
                    TaskState {
                        task_id: task_id.clone(),
                        plan_id: plan_id.clone(),
                        phase: phase.clone(),
                        outcome: None,
                    },
                );
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.tasks_total += 1;
                }
            }
            DashboardEvent::TaskCompleted {
                plan_id,
                task_id,
                outcome,
            } => {
                let key = format!("{plan_id}/{task_id}");
                let failed = outcome.contains("fail") || outcome.contains("error");
                if let Some(task) = self.tasks.get_mut(&key) {
                    task.phase = "completed".into();
                    task.outcome = Some(outcome.clone());
                }
                self.stats.tasks_active = self.stats.tasks_active.saturating_sub(1);
                if failed {
                    self.stats.tasks_failed += 1;
                    if let Some(plan) = self.plans.get_mut(plan_id) {
                        plan.tasks_failed += 1;
                    }
                } else {
                    self.stats.tasks_completed += 1;
                    if let Some(plan) = self.plans.get_mut(plan_id) {
                        plan.tasks_done += 1;
                    }
                }
            }
            DashboardEvent::TaskPhaseChanged {
                plan_id,
                task_id,
                new_phase,
                ..
            } => {
                let key = format!("{plan_id}/{task_id}");
                if let Some(task) = self.tasks.get_mut(&key) {
                    task.phase = new_phase.clone();
                }
            }
            DashboardEvent::AgentSpawned { agent_id, role } => {
                self.stats.agents_active += 1;
                self.agents.insert(
                    agent_id.clone(),
                    AgentState {
                        agent_id: agent_id.clone(),
                        role: role.clone(),
                        active: true,
                        output_bytes: 0,
                    },
                );
            }
            DashboardEvent::AgentOutput { agent_id, content } => {
                if let Some(agent) = self.agents.get_mut(agent_id) {
                    agent.output_bytes += content.len();
                }
            }
            DashboardEvent::GateResult {
                plan_id,
                task_id,
                gate,
                passed,
            } => {
                if *passed {
                    self.stats.gates_passed += 1;
                } else {
                    self.stats.gates_failed += 1;
                }
                if self.gates.len() >= MAX_GATES {
                    self.gates.remove(0);
                }
                self.gates.push(GateVerdict {
                    plan_id: plan_id.clone(),
                    task_id: task_id.clone(),
                    gate: gate.clone(),
                    passed: *passed,
                    ts_millis: ts,
                });
                record_gate_trend(self, gate, diagnosis_timestamp_from_ms(ts), *passed);
                if !passed {
                    push_gate_failure(
                        self,
                        FailureEntry {
                            ts: diagnosis_timestamp_from_ms(ts),
                            plan_id: plan_id.clone(),
                            task_id: task_id.clone(),
                            gate: gate.clone(),
                            summary: String::new(),
                            artifacts: None,
                        },
                    );
                }
            }
            DashboardEvent::PhaseTransition { plan_id, to, .. } => {
                if let Some(plan) = self.plans.get_mut(plan_id) {
                    plan.phase = to.clone();
                }
            }
            DashboardEvent::EfficiencyEvent { .. } => {
                // Efficiency metrics are tracked separately by the learn subsystem.
            }
            DashboardEvent::Diagnosis { summary } => {
                push_diagnosis(self, summary.clone());
            }
            DashboardEvent::ExperimentWinnersUpdated { winners } => {
                self.experiment_winners = winners.clone();
            }
            DashboardEvent::CFactorTrendUpdated { buckets } => {
                self.cfactor_trend = buckets.clone();
            }
            DashboardEvent::Error { message } => {
                self.stats.errors_total += 1;
                if self.errors.len() >= MAX_ERRORS {
                    self.errors.remove(0);
                }
                self.errors.push(ErrorEntry {
                    message: message.clone(),
                    ts_millis: ts,
                });
            }
        }
    }

    /// Load a best-effort snapshot from a workspace root.
    ///
    /// This seeds the live hub from persisted `.roko/` state when it is
    /// available. Missing files are treated as an empty snapshot.
    pub fn load_from_workdir(workdir: &Path) -> Result<Self, io::Error> {
        let root = resolve_snapshot_root(workdir);
        let roko_dir = root.join(".roko");
        let state_dir = roko_dir.join("state");
        let learn_dir = roko_dir.join("learn");

        let state =
            read_json_value(&state_dir.join("executor.json"))?.unwrap_or(serde_json::Value::Null);
        let task_trackers = read_task_trackers(&state_dir.join("task-trackers.json"))?;
        let signal_gates = read_signal_gates(&roko_dir.join("engrams.jsonl"))?;
        let event_entries = read_event_entries(&state_dir.join("events.json"))?;
        let experiment_winners = read_experiment_winners(&learn_dir.join("experiments.json"))?;
        let cfactor_trend = read_cfactor_trend(&learn_dir.join("c-factor.jsonl"))?;

        Ok(snapshot_from_workdir_parts(
            &state,
            &task_trackers,
            &signal_gates,
            &event_entries,
            &experiment_winners,
            &cfactor_trend,
        ))
    }
}

#[allow(clippy::cast_possible_truncation)]
fn current_ts_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn read_json_value(path: &Path) -> Result<Option<serde_json::Value>, io::Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let value = serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("parse {}: {err}", path.display()),
                )
            })?;
            Ok(Some(value))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[derive(Debug, Default)]
struct TaskTrackerSnapshot {
    completed: Vec<String>,
    failed: Vec<String>,
}

fn snapshot_from_workdir_parts(
    state: &serde_json::Value,
    task_trackers: &HashMap<String, TaskTrackerSnapshot>,
    signal_gates: &[GateVerdict],
    event_entries: &[serde_json::Value],
    experiment_winners: &[ExperimentWinnerSummary],
    cfactor_trend: &[CFactorBucket],
) -> DashboardSnapshot {
    let mut snapshot = DashboardSnapshot {
        experiment_winners: experiment_winners.to_vec(),
        cfactor_trend: cfactor_trend.to_vec(),
        ..DashboardSnapshot::default()
    };
    let Some(plan_states) = state
        .get("plan_states")
        .and_then(serde_json::Value::as_object)
    else {
        for gate in signal_gates {
            push_gate(&mut snapshot, gate.clone());
        }
        append_event_diagnoses(&mut snapshot, event_entries);
        append_event_errors(&mut snapshot, event_entries);
        return snapshot;
    };

    let agent_roles = collect_agent_roles(event_entries);
    let mut plan_ids = plan_states.keys().cloned().collect::<Vec<_>>();
    plan_ids.sort();
    let mut plan_gate_results = 0usize;

    for plan_id in plan_ids {
        if let Some(plan_state) = plan_states.get(&plan_id) {
            bootstrap_plan_state(
                &mut snapshot,
                &plan_id,
                plan_state,
                task_trackers.get(&plan_id),
                &agent_roles,
                &mut plan_gate_results,
            );
        }
    }

    if plan_gate_results == 0 {
        for gate in signal_gates {
            push_gate(&mut snapshot, gate.clone());
        }
    }
    append_event_diagnoses(&mut snapshot, event_entries);
    append_event_errors(&mut snapshot, event_entries);
    let fallback_gates = if signal_gates.is_empty() {
        snapshot.gates.clone()
    } else {
        signal_gates.to_vec()
    };
    rebuild_gate_observability(&mut snapshot, &fallback_gates);
    snapshot
}

#[derive(Debug, Default, Deserialize)]
struct PersistedExperimentStore {
    #[serde(default)]
    experiments: HashMap<String, PersistedPromptExperiment>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
enum PersistedExperimentStatus {
    #[default]
    Running,
    Concluded,
}

#[derive(Debug, Default, Deserialize)]
struct PersistedPromptExperiment {
    #[serde(default)]
    experiment_id: String,
    #[serde(default)]
    section_name: String,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    variants: Vec<PersistedPromptVariant>,
    #[serde(default)]
    stats: HashMap<String, PersistedVariantStats>,
    #[serde(default)]
    status: PersistedExperimentStatus,
    #[serde(default)]
    winner_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PersistedPromptVariant {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    active: bool,
}

#[derive(Debug, Default, Deserialize)]
struct PersistedVariantStats {
    #[serde(default)]
    trials: u64,
    #[serde(default)]
    successes: u64,
}

fn read_experiment_winners(path: &Path) -> Result<Vec<ExperimentWinnerSummary>, io::Error> {
    let Some(value) = read_json_value(path)? else {
        return Ok(Vec::new());
    };
    let store = serde_json::from_value::<PersistedExperimentStore>(value).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parse {}: {err}", path.display()),
        )
    })?;
    Ok(experiment_winner_summaries(&store))
}

fn read_cfactor_trend(path: &Path) -> Result<Vec<CFactorBucket>, io::Error> {
    const CFACTOR_TREND_BUCKETS: usize = 24;
    const CFACTOR_TREND_BUCKET_MS: i64 = 60 * 60 * 1000;

    let content = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let now = Utc::now();
    let oldest_start_ms = current_bucket_start_ms(now.timestamp_millis(), CFACTOR_TREND_BUCKET_MS)
        - i64::try_from(CFACTOR_TREND_BUCKETS.saturating_sub(1)).unwrap_or(0)
            * CFACTOR_TREND_BUCKET_MS;
    let mut buckets = (0..CFACTOR_TREND_BUCKETS)
        .map(|idx| CFactorBucket {
            start: timestamp_from_millis(
                oldest_start_ms + i64::try_from(idx).unwrap_or(0) * CFACTOR_TREND_BUCKET_MS,
            ),
            ..CFactorBucket::default()
        })
        .collect::<Vec<_>>();
    let mut bucket_values = vec![Vec::new(); CFACTOR_TREND_BUCKETS];

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let Ok(snapshot) = serde_json::from_str::<PersistedCFactorSnapshot>(trimmed) else {
            continue;
        };
        let bucket_start_ms = current_bucket_start_ms(
            snapshot.computed_at.timestamp_millis(),
            CFACTOR_TREND_BUCKET_MS,
        );
        if bucket_start_ms < oldest_start_ms {
            continue;
        }
        let idx = usize::try_from((bucket_start_ms - oldest_start_ms) / CFACTOR_TREND_BUCKET_MS)
            .unwrap_or(usize::MAX);
        let Some(values) = bucket_values.get_mut(idx) else {
            continue;
        };
        values.push(snapshot.overall);
    }

    for (bucket, values) in buckets.iter_mut().zip(bucket_values.iter_mut()) {
        finalize_cfactor_bucket(bucket, values);
    }

    Ok(buckets)
}

#[derive(Debug, Deserialize)]
struct PersistedCFactorSnapshot {
    computed_at: DateTime<Utc>,
    overall: f64,
}

fn finalize_cfactor_bucket(bucket: &mut CFactorBucket, values: &mut [f64]) {
    if values.is_empty() {
        return;
    }

    values.sort_by(|lhs, rhs| lhs.total_cmp(rhs));
    let sample_count = values.len();
    let sum = values.iter().sum::<f64>();

    bucket.samples = u32::try_from(sample_count).unwrap_or(u32::MAX);
    bucket.avg = sum / sample_count as f64;
    bucket.p50 = quantile(values, 0.50);
    bucket.p95 = quantile(values, 0.95);
}

fn quantile(sorted_values: &[f64], quantile: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }

    let clamped = quantile.clamp(0.0, 1.0);
    let last_idx = sorted_values.len().saturating_sub(1);
    let position = clamped * last_idx as f64;
    let lower_idx = position.floor() as usize;
    let upper_idx = position.ceil() as usize;

    if lower_idx == upper_idx {
        return sorted_values[lower_idx];
    }

    let lower = sorted_values[lower_idx];
    let upper = sorted_values[upper_idx];
    let weight = position - lower_idx as f64;
    lower + (upper - lower) * weight
}

fn current_bucket_start_ms(timestamp_ms: i64, bucket_ms: i64) -> i64 {
    timestamp_ms.div_euclid(bucket_ms) * bucket_ms
}

fn timestamp_from_millis(timestamp_ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .unwrap_or_else(default_diagnosis_timestamp)
}

fn experiment_winner_summaries(store: &PersistedExperimentStore) -> Vec<ExperimentWinnerSummary> {
    let mut winners = store
        .experiments
        .values()
        .filter_map(persisted_experiment_winner_summary)
        .collect::<Vec<_>>();
    winners.sort_by(|lhs, rhs| lhs.experiment_id.cmp(&rhs.experiment_id));
    winners
}

fn persisted_experiment_winner_summary(
    experiment: &PersistedPromptExperiment,
) -> Option<ExperimentWinnerSummary> {
    if experiment.status != PersistedExperimentStatus::Concluded {
        return None;
    }

    let winner_id = experiment.winner_id.as_deref()?;
    let winner_variant = experiment
        .variants
        .iter()
        .find(|variant| variant.id == winner_id)?;
    let winner_stats = experiment.stats.get(winner_id)?;
    let confidence = persisted_winner_confidence(experiment, winner_id)?;
    if confidence < 0.95 {
        return None;
    }

    let (ci_lower, ci_upper) = wilson_confidence_interval(winner_stats);

    Some(ExperimentWinnerSummary {
        experiment_id: experiment.experiment_id.clone(),
        parameter: experiment
            .role
            .clone()
            .unwrap_or_else(|| experiment.section_name.clone()),
        winner: winner_variant_label(winner_variant),
        winner_variant_id: winner_variant.id.clone(),
        win_rate: variant_success_rate(winner_stats),
        sample_size: winner_stats.trials,
        ci_lower,
        ci_upper,
        confidence,
    })
}

fn winner_variant_label(variant: &PersistedPromptVariant) -> String {
    variant
        .slug
        .clone()
        .filter(|slug| !slug.trim().is_empty())
        .or_else(|| (!variant.name.trim().is_empty()).then(|| variant.name.clone()))
        .unwrap_or_else(|| variant.id.clone())
}

fn persisted_winner_confidence(
    experiment: &PersistedPromptExperiment,
    winner_id: &str,
) -> Option<f64> {
    let mut ranked = experiment
        .variants
        .iter()
        .filter(|variant| variant.active)
        .filter_map(|variant| {
            experiment
                .stats
                .get(&variant.id)
                .map(|stats| (variant.id.as_str(), stats, variant_success_rate(stats)))
        })
        .collect::<Vec<_>>();
    if ranked.is_empty() {
        return None;
    }

    ranked.sort_by(|lhs, rhs| rhs.2.total_cmp(&lhs.2));
    let (winner_ranked_id, winner_stats, winner_rate) = ranked
        .iter()
        .find(|(id, _, _)| *id == winner_id)
        .copied()
        .unwrap_or(ranked[0]);
    let second = ranked.iter().find(|(id, _, _)| *id != winner_ranked_id);
    let second_rate = second.map_or(0.0, |(_, _, rate)| *rate);
    let second_stats = second.map(|(_, stats, _)| *stats);

    let se = match second_stats {
        Some(second_stats) => {
            let winner_trials = winner_stats.trials.max(1) as f64;
            let second_trials = second_stats.trials.max(1) as f64;
            let winner_var = winner_rate * (1.0 - winner_rate) / winner_trials;
            let second_var = second_rate * (1.0 - second_rate) / second_trials;
            (winner_var + second_var).sqrt()
        }
        None => 0.0,
    };
    let gap = (winner_rate - second_rate).max(0.0);
    if se == 0.0 {
        Some(1.0)
    } else {
        Some((gap / (gap + se)).clamp(0.0, 1.0))
    }
}

#[allow(clippy::cast_precision_loss)]
fn variant_success_rate(stats: &PersistedVariantStats) -> f64 {
    if stats.trials == 0 {
        0.0
    } else {
        stats.successes as f64 / stats.trials as f64
    }
}

#[allow(clippy::cast_precision_loss)]
fn wilson_confidence_interval(stats: &PersistedVariantStats) -> (f64, f64) {
    if stats.trials == 0 {
        return (0.0, 0.0);
    }

    let n = stats.trials as f64;
    let p = stats.successes as f64 / n;
    let z = 1.96_f64;
    let z_sq = z * z;
    let denom = 1.0 + z_sq / n;
    let center = (p + z_sq / (2.0 * n)) / denom;
    let margin = (z / denom) * ((p * (1.0 - p) / n + z_sq / (4.0 * n * n)).sqrt());
    (
        (center - margin).clamp(0.0, 1.0),
        (center + margin).clamp(0.0, 1.0),
    )
}

fn bootstrap_plan_state(
    snapshot: &mut DashboardSnapshot,
    plan_id: &str,
    plan_state: &serde_json::Value,
    task_tracker: Option<&TaskTrackerSnapshot>,
    agent_roles: &HashMap<String, String>,
    plan_gate_results: &mut usize,
) {
    let phase = current_phase_label(plan_state).unwrap_or_else(|| String::from("pending"));
    let terminal = is_terminal_phase(&phase);
    let paused = plan_state
        .get("paused")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let active = !terminal && !paused;
    let active_task_id = plan_state
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| plan_state.get("id").and_then(serde_json::Value::as_str))
        .filter(|task_id| !task_id.trim().is_empty());
    let completed_tasks = task_tracker
        .map(|tracker| tracker.completed.clone())
        .unwrap_or_default();
    let failed_tasks = task_tracker
        .map(|tracker| tracker.failed.clone())
        .unwrap_or_default();
    let mut seen_task_ids = HashSet::new();
    let mut tasks_done = 0usize;
    let mut tasks_failed = 0usize;

    for task_id in completed_tasks {
        if !seen_task_ids.insert(task_id.clone()) {
            continue;
        }
        tasks_done += 1;
        snapshot.stats.tasks_completed += 1;
        snapshot.tasks.insert(
            format!("{plan_id}/{task_id}"),
            TaskState {
                task_id,
                plan_id: plan_id.to_string(),
                phase: String::from("completed"),
                outcome: Some(String::from("success")),
            },
        );
    }

    for task_id in failed_tasks {
        if !seen_task_ids.insert(task_id.clone()) {
            continue;
        }
        tasks_failed += 1;
        snapshot.stats.tasks_failed += 1;
        snapshot.tasks.insert(
            format!("{plan_id}/{task_id}"),
            TaskState {
                task_id,
                plan_id: plan_id.to_string(),
                phase: String::from("completed"),
                outcome: Some(String::from("failed")),
            },
        );
    }

    if active {
        if let Some(task_id) = active_task_id {
            if seen_task_ids.insert(task_id.to_string()) {
                snapshot.stats.tasks_active += 1;
                snapshot.tasks.insert(
                    format!("{plan_id}/{task_id}"),
                    TaskState {
                        task_id: task_id.to_string(),
                        plan_id: plan_id.to_string(),
                        phase: phase.clone(),
                        outcome: None,
                    },
                );
            }
        }
    } else if terminal {
        if let Some(task_id) = active_task_id {
            if seen_task_ids.insert(task_id.to_string()) {
                let failed =
                    phase.eq_ignore_ascii_case("failed") || phase.eq_ignore_ascii_case("error");
                if failed {
                    tasks_failed += 1;
                    snapshot.stats.tasks_failed += 1;
                } else {
                    tasks_done += 1;
                    snapshot.stats.tasks_completed += 1;
                }
                snapshot.tasks.insert(
                    format!("{plan_id}/{task_id}"),
                    TaskState {
                        task_id: task_id.to_string(),
                        plan_id: plan_id.to_string(),
                        phase: String::from("completed"),
                        outcome: Some(if failed {
                            String::from("failed")
                        } else {
                            String::from("success")
                        }),
                    },
                );
            }
        }
    }

    snapshot.plans.insert(
        plan_id.to_string(),
        PlanState {
            plan_id: plan_id.to_string(),
            phase: phase.clone(),
            tasks_total: seen_task_ids.len(),
            tasks_done,
            tasks_failed,
            active,
        },
    );

    if active {
        snapshot.stats.plans_active += 1;
    } else if phase.eq_ignore_ascii_case("failed") || phase.eq_ignore_ascii_case("error") {
        snapshot.stats.plans_failed += 1;
    } else if terminal {
        snapshot.stats.plans_completed += 1;
    }

    if let Some(agents) = plan_state
        .get("assigned_agents")
        .and_then(serde_json::Value::as_array)
    {
        for agent in agents {
            let Some(agent_id) = agent.as_str() else {
                continue;
            };
            let role = agent_roles
                .get(agent_id)
                .cloned()
                .unwrap_or_else(|| String::from("unknown"));
            let entry = snapshot
                .agents
                .entry(agent_id.to_string())
                .or_insert_with(|| AgentState {
                    agent_id: agent_id.to_string(),
                    role: role.clone(),
                    active: false,
                    output_bytes: 0,
                });
            if entry.role == "unknown" && role != "unknown" {
                entry.role = role;
            }
            if active && !entry.active {
                entry.active = true;
                snapshot.stats.agents_active += 1;
            }
        }
    }

    if let Some(results) = plan_state
        .get("gate_results")
        .and_then(serde_json::Value::as_array)
    {
        for result in results {
            *plan_gate_results += 1;
            push_gate(
                snapshot,
                GateVerdict {
                    plan_id: plan_id.to_string(),
                    task_id: result
                        .get("task_id")
                        .and_then(serde_json::Value::as_str)
                        .or(active_task_id)
                        .unwrap_or(plan_id)
                        .to_string(),
                    gate: result
                        .get("gate_name")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| result.get("gate").and_then(serde_json::Value::as_str))
                        .unwrap_or("unknown")
                        .to_string(),
                    passed: result
                        .get("passed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    ts_millis: result
                        .get("timestamp_ms")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default(),
                },
            );
        }
    }

    if let Some(message) = plan_state
        .get("last_error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            plan_state
                .pointer("/error/message")
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| plan_state.get("error").and_then(serde_json::Value::as_str))
        .filter(|message| !message.trim().is_empty())
    {
        push_error(
            snapshot,
            ErrorEntry {
                message: message.to_string(),
                ts_millis: plan_state
                    .get("timestamp_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default(),
            },
        );
    }
}

fn current_phase_label(plan_state: &serde_json::Value) -> Option<String> {
    plan_state
        .pointer("/current_phase/kind")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            plan_state
                .get("current_phase")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            plan_state
                .pointer("/phase/kind")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            plan_state
                .get("phase")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn is_terminal_phase(phase: &str) -> bool {
    matches!(
        phase.trim().to_ascii_lowercase().as_str(),
        "done" | "completed" | "complete" | "failed" | "error" | "skipped"
    )
}

fn resolve_snapshot_root(start: &Path) -> PathBuf {
    let mut cursor = Some(start);
    while let Some(dir) = cursor {
        if dir.join(".roko").is_dir() {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    start.to_path_buf()
}

fn read_task_trackers(path: &Path) -> Result<HashMap<String, TaskTrackerSnapshot>, io::Error> {
    let mut trackers = HashMap::new();
    let Some(value) = read_json_value(path)? else {
        return Ok(trackers);
    };
    let Some(entries) = value.as_array() else {
        return Ok(trackers);
    };

    for entry in entries {
        let Some(plan_id) = entry.get("plan_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if plan_id.trim().is_empty() {
            continue;
        }
        let completed = entry
            .get("completed")
            .and_then(serde_json::Value::as_array)
            .map(|values| string_array(values))
            .unwrap_or_default();
        let failed = entry
            .get("failed")
            .and_then(serde_json::Value::as_array)
            .map(|values| string_array(values))
            .unwrap_or_default();
        trackers.insert(
            plan_id.to_string(),
            TaskTrackerSnapshot { completed, failed },
        );
    }

    Ok(trackers)
}

fn read_signal_gates(path: &Path) -> Result<Vec<GateVerdict>, io::Error> {
    let Some(values) = read_jsonl_values(path)? else {
        return Ok(Vec::new());
    };

    let mut gates = Vec::new();
    for value in values {
        let Some(kind) = value.get("kind").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !is_gate_result_kind(kind) {
            continue;
        }
        let Some(gate) = extract_gate_name(&value) else {
            continue;
        };
        let Some(passed) = extract_gate_passed(&value) else {
            continue;
        };
        gates.push(GateVerdict {
            plan_id: value
                .pointer("/tags/plan_id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    value
                        .pointer("/body/data/plan_id")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    value
                        .pointer("/body/plan_id")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("unknown")
                .to_string(),
            task_id: value
                .pointer("/tags/task_id")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    value
                        .pointer("/body/data/task_id")
                        .and_then(serde_json::Value::as_str)
                })
                .or_else(|| {
                    value
                        .pointer("/body/task_id")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or_default()
                .to_string(),
            gate,
            passed,
            ts_millis: entry_timestamp_ms(&value).unwrap_or_default(),
        });
    }

    Ok(gates)
}

fn read_jsonl_values(path: &Path) -> Result<Option<Vec<serde_json::Value>>, io::Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).map_err(|err| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("parse {}: {err}", path.display()),
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn read_event_entries(path: &Path) -> Result<Vec<serde_json::Value>, io::Error> {
    let Some(value) = read_json_value(path)? else {
        return Ok(Vec::new());
    };
    Ok(extract_event_entries(&value))
}

fn extract_event_entries(value: &serde_json::Value) -> Vec<serde_json::Value> {
    if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
        return entries.clone();
    }
    if let Some(entries) = value.as_array() {
        return entries.clone();
    }
    vec![value.clone()]
}

fn collect_agent_roles(event_entries: &[serde_json::Value]) -> HashMap<String, String> {
    let mut roles = HashMap::new();

    for entry in event_entries {
        let event_kind = event_kind_label(entry);
        if !matches!(
            event_kind.as_deref(),
            Some("AgentSpawned" | "agent.spawned" | "agent_spawned")
        ) {
            continue;
        }

        let payload = entry.get("payload").unwrap_or(entry);
        let plan_id = payload
            .get("plan_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let task = payload
            .get("task")
            .or_else(|| payload.get("task_id"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let agent_id = payload
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .filter(|agent_id| !agent_id.trim().is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| {
                if plan_id.is_empty() || task.is_empty() {
                    None
                } else {
                    Some(format!("{plan_id}:{task}"))
                }
            });
        let role = payload
            .get("role")
            .and_then(serde_json::Value::as_str)
            .filter(|role| !role.trim().is_empty());

        if let (Some(agent_id), Some(role)) = (agent_id, role) {
            roles.insert(agent_id, role.to_string());
        }
    }

    roles
}

fn append_event_errors(snapshot: &mut DashboardSnapshot, event_entries: &[serde_json::Value]) {
    for entry in event_entries {
        let event_kind = event_kind_label(entry);
        if !matches!(
            event_kind.as_deref(),
            Some("ErrorOccurred" | "error.occurred" | "error_occurred" | "error")
        ) {
            continue;
        }
        let payload = entry.get("payload").unwrap_or(entry);
        let message = payload
            .get("message")
            .or_else(|| payload.get("detail"))
            .or_else(|| payload.get("description"))
            .or_else(|| payload.get("reason"))
            .or_else(|| payload.get("err"))
            .and_then(serde_json::Value::as_str)
            .filter(|message| !message.trim().is_empty());
        if let Some(message) = message {
            push_error(
                snapshot,
                ErrorEntry {
                    message: message.to_string(),
                    ts_millis: event_timestamp_ms(entry),
                },
            );
        }
    }
}

fn append_event_diagnoses(snapshot: &mut DashboardSnapshot, event_entries: &[serde_json::Value]) {
    for entry in event_entries {
        let event_kind = event_kind_label(entry);
        if !matches!(
            event_kind.as_deref(),
            Some("InterventionFired" | "intervention.fired" | "intervention_fired")
        ) {
            continue;
        }

        if let Some(summary) = diagnosis_from_event_entry(entry) {
            push_diagnosis(snapshot, summary);
        }
    }
}

fn event_kind_label(entry: &serde_json::Value) -> Option<String> {
    entry
        .get("event_kind")
        .or_else(|| entry.get("event_type"))
        .or_else(|| entry.get("type"))
        .or_else(|| entry.get("kind"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn event_timestamp_ms(entry: &serde_json::Value) -> u64 {
    entry
        .get("timestamp_ms")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| entry.get("timestamp").and_then(serde_json::Value::as_u64))
        .unwrap_or_default()
}

fn push_gate(snapshot: &mut DashboardSnapshot, gate: GateVerdict) {
    if gate.passed {
        snapshot.stats.gates_passed += 1;
    } else {
        snapshot.stats.gates_failed += 1;
    }
    if snapshot.gates.len() >= MAX_GATES {
        snapshot.gates.remove(0);
    }
    snapshot.gates.push(gate);
}

fn record_gate_trend(
    snapshot: &mut DashboardSnapshot,
    gate_name: &str,
    ts: DateTime<Utc>,
    passed: bool,
) {
    snapshot
        .gate_trends
        .entry(gate_name.to_string())
        .or_insert_with(|| {
            TrendBuckets::new(GATE_TREND_BUCKET_SIZE_SECS, GATE_TREND_BUCKET_COUNT, ts)
        })
        .record_gate_result(ts, passed);
}

fn push_gate_failure(snapshot: &mut DashboardSnapshot, failure: FailureEntry) {
    if snapshot.gate_recent_failures.len() >= MAX_GATE_FAILURES {
        snapshot.gate_recent_failures.remove(0);
    }
    snapshot.gate_recent_failures.push(failure);
}

fn rebuild_gate_observability(snapshot: &mut DashboardSnapshot, gates: &[GateVerdict]) {
    snapshot.gate_trends.clear();
    snapshot.gate_recent_failures.clear();

    let reference = Utc::now();
    for gate in gates {
        let ts = timestamp_from_millis(i64::try_from(gate.ts_millis).unwrap_or_default());
        record_gate_trend(snapshot, &gate.gate, ts, gate.passed);
        if !gate.passed {
            push_gate_failure(
                snapshot,
                FailureEntry {
                    ts,
                    plan_id: gate.plan_id.clone(),
                    task_id: gate.task_id.clone(),
                    gate: gate.gate.clone(),
                    summary: String::new(),
                    artifacts: None,
                },
            );
        }
    }

    for trend in snapshot.gate_trends.values_mut() {
        trend.align_to(reference);
    }
}

fn push_error(snapshot: &mut DashboardSnapshot, error: ErrorEntry) {
    snapshot.stats.errors_total += 1;
    if snapshot.errors.len() >= MAX_ERRORS {
        snapshot.errors.remove(0);
    }
    snapshot.errors.push(error);
}

fn push_diagnosis(snapshot: &mut DashboardSnapshot, mut diagnosis: DiagnosisSummary) {
    if diagnosis.id.trim().is_empty() {
        diagnosis.id = format!(
            "{}:{}:{}",
            diagnosis.subject,
            diagnosis.suggested_action.as_deref().unwrap_or_default(),
            diagnosis.ts.timestamp_millis()
        );
    }

    if let Some(existing_idx) = snapshot
        .diagnoses
        .iter()
        .position(|existing| existing.id == diagnosis.id)
    {
        snapshot.diagnoses.remove(existing_idx);
    }
    while snapshot.diagnoses.len() >= MAX_DIAGNOSES {
        snapshot.diagnoses.pop_front();
    }
    snapshot.diagnoses.push_back(diagnosis);
}

fn diagnosis_from_event_entry(entry: &serde_json::Value) -> Option<DiagnosisSummary> {
    let payload = entry.get("payload").unwrap_or(entry);
    let plan_id = payload
        .get("plan_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let watcher = payload
        .get("watcher")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("conductor");
    let action = payload
        .get("action")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("observe");
    let ts = diagnosis_timestamp_from_ms(event_timestamp_ms(entry));

    if let Some(primary) = payload.get("primary_diagnosis") {
        let pattern_name = primary
            .get("pattern_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let detail = primary
            .get("matched_excerpt")
            .and_then(serde_json::Value::as_str)
            .filter(|detail| !detail.trim().is_empty())
            .or_else(|| {
                payload
                    .get("error_output")
                    .and_then(serde_json::Value::as_str)
                    .filter(|detail| !detail.trim().is_empty())
            })
            .unwrap_or_default()
            .to_string();

        return Some(DiagnosisSummary {
            id: format!("plan:{plan_id}:watcher:{watcher}:pattern:{pattern_name}"),
            ts,
            severity: diagnosis_severity_from_payload(primary, action),
            subject: format!(
                "{}: {}",
                titleize_token(watcher),
                titleize_token(pattern_name)
            ),
            detail,
            suggested_action: primary
                .get("suggested_intervention")
                .and_then(serde_json::Value::as_str)
                .map(titleize_token),
            intervention_taken: Some(titleize_token(action)),
        });
    }

    let reason = payload
        .get("reason")
        .or_else(|| payload.get("message"))
        .and_then(serde_json::Value::as_str)
        .filter(|reason| !reason.trim().is_empty())?;

    Some(DiagnosisSummary {
        id: format!("plan:{plan_id}:watcher:{watcher}:action:{action}"),
        ts,
        severity: diagnosis_severity_from_action(action),
        subject: titleize_token(watcher),
        detail: reason.to_string(),
        suggested_action: Some(titleize_token(action)),
        intervention_taken: None,
    })
}

fn diagnosis_severity_from_payload(
    diagnosis: &serde_json::Value,
    action: &str,
) -> DiagnosisSeverity {
    match diagnosis
        .get("suggested_intervention")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
    {
        "abort_plan" | "restart_agent" | "switch_model" | "merge_resolution" => {
            DiagnosisSeverity::Alert
        }
        "autofix" | "backoff_retry" | "reduce_context" => DiagnosisSeverity::Warn,
        _ => diagnosis_severity_from_action(action),
    }
}

fn diagnosis_severity_from_action(action: &str) -> DiagnosisSeverity {
    match action {
        "fail" | "pause" | "abort" | "restart" => DiagnosisSeverity::Alert,
        "retry" | "warn" | "backoff" => DiagnosisSeverity::Warn,
        _ => DiagnosisSeverity::Info,
    }
}

fn default_diagnosis_timestamp() -> DateTime<Utc> {
    diagnosis_timestamp_from_ms(0)
}

fn trend_bucket_start(timestamp: DateTime<Utc>, bucket_size_secs: u64) -> DateTime<Utc> {
    let bucket_ms = i64::try_from(bucket_size_secs)
        .unwrap_or(i64::MAX / 1000)
        .saturating_mul(1000);
    timestamp_from_millis(current_bucket_start_ms(
        timestamp.timestamp_millis(),
        bucket_ms,
    ))
}

fn diagnosis_timestamp_from_ms(ts_millis: u64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(i64::try_from(ts_millis).unwrap_or_default())
        .single()
        .unwrap_or_else(default_epoch)
}

fn default_epoch() -> DateTime<Utc> {
    Utc.timestamp_opt(0, 0).single().unwrap_or_else(Utc::now)
}

fn titleize_token(value: &str) -> String {
    value
        .split(['.', '-', '_', ':'])
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut title = String::new();
                    title.extend(first.to_uppercase());
                    title.push_str(chars.as_str());
                    title
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn string_array(values: &[serde_json::Value]) -> Vec<String> {
    values
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn is_gate_result_kind(kind: &str) -> bool {
    kind == "gate_verdict" || kind.starts_with("gate:") || kind.starts_with("gate_")
}

fn extract_gate_name(entry: &serde_json::Value) -> Option<String> {
    entry
        .pointer("/tags/gate")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            entry
                .pointer("/body/data/gate")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .pointer("/body/gate")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            entry
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .and_then(|kind| kind.strip_prefix("gate:").or(kind.strip_prefix("gate_")))
                .map(ToOwned::to_owned)
        })
}

fn extract_gate_passed(entry: &serde_json::Value) -> Option<bool> {
    entry
        .pointer("/tags/passed")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| match value {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
        .or_else(|| {
            entry
                .pointer("/body/data/passed")
                .and_then(serde_json::Value::as_bool)
        })
        .or_else(|| {
            entry
                .pointer("/body/passed")
                .and_then(serde_json::Value::as_bool)
        })
}

fn entry_timestamp_ms(entry: &serde_json::Value) -> Option<u64> {
    entry
        .get("created_at_ms")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            entry
                .get("created_at_ms")
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| u64::try_from(value).ok())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn plan_lifecycle() {
        let mut snap = DashboardSnapshot::default();

        snap.apply(&DashboardEvent::PlanStarted {
            plan_id: "p1".into(),
        });
        assert_eq!(snap.stats.plans_active, 1);
        assert!(snap.plans["p1"].active);

        snap.apply(&DashboardEvent::TaskStarted {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            phase: "compose".into(),
        });
        assert_eq!(snap.stats.tasks_active, 1);
        assert_eq!(snap.plans["p1"].tasks_total, 1);

        snap.apply(&DashboardEvent::GateResult {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            gate: "compile".into(),
            passed: true,
        });
        assert_eq!(snap.stats.gates_passed, 1);

        snap.apply(&DashboardEvent::TaskCompleted {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            outcome: "success".into(),
        });
        assert_eq!(snap.stats.tasks_completed, 1);
        assert_eq!(snap.plans["p1"].tasks_done, 1);

        snap.apply(&DashboardEvent::PlanCompleted {
            plan_id: "p1".into(),
            success: true,
        });
        assert_eq!(snap.stats.plans_active, 0);
        assert_eq!(snap.stats.plans_completed, 1);
        assert!(!snap.plans["p1"].active);
    }

    #[test]
    fn gate_ring_eviction() {
        let mut snap = DashboardSnapshot::default();
        for i in 0..300 {
            snap.apply(&DashboardEvent::GateResult {
                plan_id: "p1".into(),
                task_id: format!("t{i}"),
                gate: "compile".into(),
                passed: true,
            });
        }
        assert_eq!(snap.gates.len(), MAX_GATES);
        assert_eq!(snap.stats.gates_passed, 300);
    }

    #[test]
    fn gate_results_update_trends_and_recent_failures() {
        let mut snap = DashboardSnapshot::default();

        snap.apply(&DashboardEvent::GateResult {
            plan_id: "plan-a".into(),
            task_id: "task-1".into(),
            gate: "compile".into(),
            passed: true,
        });
        snap.apply(&DashboardEvent::GateResult {
            plan_id: "plan-a".into(),
            task_id: "task-2".into(),
            gate: "compile".into(),
            passed: false,
        });

        let trend = snap.gate_trends.get("compile").expect("compile trend");
        assert_eq!(trend.bucket_size_secs, GATE_TREND_BUCKET_SIZE_SECS);
        assert_eq!(trend.slots.len(), GATE_TREND_BUCKET_COUNT);
        let latest = trend.slots.back().expect("latest bucket");
        assert_eq!(latest.pass, 1);
        assert_eq!(latest.fail, 1);

        assert_eq!(snap.gate_recent_failures.len(), 1);
        assert_eq!(snap.gate_recent_failures[0].gate, "compile");
        assert_eq!(snap.gate_recent_failures[0].task_id, "task-2");
    }

    #[test]
    fn error_ring_eviction() {
        let mut snap = DashboardSnapshot::default();
        for i in 0..100 {
            snap.apply(&DashboardEvent::Error {
                message: format!("err {i}"),
            });
        }
        assert_eq!(snap.errors.len(), MAX_ERRORS);
        assert_eq!(snap.stats.errors_total, 100);
    }

    #[test]
    fn diagnosis_ring_dedupes_by_id_and_keeps_latest_copy() {
        let mut snap = DashboardSnapshot::default();

        snap.apply(&DashboardEvent::Diagnosis {
            summary: DiagnosisSummary {
                id: "diag-1".into(),
                ts: diagnosis_timestamp_from_ms(10),
                severity: DiagnosisSeverity::Warn,
                subject: "Circuit Breaker".into(),
                detail: "first copy".into(),
                suggested_action: Some("Restart Agent".into()),
                intervention_taken: Some("Paused plan".into()),
            },
        });
        snap.apply(&DashboardEvent::Diagnosis {
            summary: DiagnosisSummary {
                id: "diag-1".into(),
                ts: diagnosis_timestamp_from_ms(20),
                severity: DiagnosisSeverity::Alert,
                subject: "Circuit Breaker".into(),
                detail: "latest copy".into(),
                suggested_action: Some("Abort Plan".into()),
                intervention_taken: Some("Paused plan".into()),
            },
        });

        assert_eq!(snap.diagnoses.len(), 1);
        assert_eq!(snap.diagnoses[0].detail, "latest copy");
        assert_eq!(snap.diagnoses[0].severity, DiagnosisSeverity::Alert);
    }

    #[test]
    fn agent_output_accumulates() {
        let mut snap = DashboardSnapshot::default();
        snap.apply(&DashboardEvent::AgentSpawned {
            agent_id: "a1".into(),
            role: "coder".into(),
        });
        snap.apply(&DashboardEvent::AgentOutput {
            agent_id: "a1".into(),
            content: "hello world".into(),
        });
        assert_eq!(snap.agents["a1"].output_bytes, 11);
    }

    #[test]
    fn load_from_workdir_bootstraps_executor_trackers_and_events() {
        let dir = tempdir().unwrap();
        let roko_dir = dir.path().join(".roko");
        let state_dir = roko_dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();

        std::fs::write(
            state_dir.join("executor.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "plan_states": {
                    "plan-1": {
                        "current_phase": { "kind": "implementing" },
                        "task_id": "task-2",
                        "assigned_agents": ["plan-1:task-2"],
                        "gate_results": [
                            { "gate_name": "compile", "passed": true, "timestamp_ms": 42 }
                        ]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            state_dir.join("task-trackers.json"),
            serde_json::to_vec_pretty(&serde_json::json!([
                {
                    "plan_id": "plan-1",
                    "completed": ["task-0"],
                    "failed": ["task-1"]
                }
            ]))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            state_dir.join("events.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "entries": [
                    {
                        "timestamp_ms": 99,
                        "event_kind": "AgentSpawned",
                        "payload": { "plan_id": "plan-1", "task": "task-2", "role": "Implementer" }
                    },
                    {
                        "timestamp_ms": 100,
                        "event_kind": "ErrorOccurred",
                        "payload": { "message": "boom" }
                    },
                    {
                        "timestamp_ms": 101,
                        "event_kind": "intervention.fired",
                        "payload": {
                            "plan_id": "plan-1",
                            "watcher": "circuit-breaker",
                            "action": "pause",
                            "error_output": "tool timeout",
                            "primary_diagnosis": {
                                "pattern_name": "timeout_error",
                                "suggested_intervention": "restart_agent",
                                "matched_excerpt": "tool timeout"
                            }
                        }
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let snapshot = DashboardSnapshot::load_from_workdir(dir.path()).unwrap();
        assert_eq!(snapshot.stats.plans_active, 1);
        assert_eq!(snapshot.stats.tasks_active, 1);
        assert_eq!(snapshot.stats.tasks_completed, 1);
        assert_eq!(snapshot.stats.tasks_failed, 1);
        assert_eq!(snapshot.stats.agents_active, 1);
        assert_eq!(snapshot.stats.gates_passed, 1);
        assert_eq!(snapshot.stats.errors_total, 1);
        assert_eq!(snapshot.plans["plan-1"].tasks_total, 3);
        assert_eq!(snapshot.agents["plan-1:task-2"].role, "Implementer");
        assert_eq!(snapshot.tasks["plan-1/task-2"].phase, "implementing");
        assert_eq!(snapshot.errors[0].message, "boom");
        assert_eq!(snapshot.diagnoses.len(), 1);
        assert_eq!(snapshot.diagnoses[0].severity, DiagnosisSeverity::Alert);
        assert!(snapshot.diagnoses[0].subject.contains("Circuit Breaker"));
    }

    #[test]
    fn load_from_workdir_uses_signal_gates_when_executor_has_none() {
        let dir = tempdir().unwrap();
        let roko_dir = dir.path().join(".roko");
        let state_dir = roko_dir.join("state");
        std::fs::create_dir_all(&state_dir).unwrap();

        std::fs::write(
            state_dir.join("executor.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "plan_states": {
                    "plan-1": {
                        "current_phase": { "kind": "failed" },
                        "task_id": "task-1"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            roko_dir.join("engrams.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({
                    "kind": "gate:compile",
                    "created_at_ms": 7,
                    "tags": {
                        "plan_id": "plan-1",
                        "task_id": "task-1",
                        "passed": "false"
                    }
                })
            ),
        )
        .unwrap();

        let snapshot = DashboardSnapshot::load_from_workdir(dir.path()).unwrap();
        assert_eq!(snapshot.stats.plans_failed, 1);
        assert_eq!(snapshot.stats.gates_failed, 1);
        assert_eq!(snapshot.gates.len(), 1);
        assert_eq!(snapshot.gates[0].gate, "compile");
    }

    #[test]
    fn load_from_workdir_bootstraps_experiment_winners() {
        let dir = tempdir().unwrap();
        let learn_dir = dir.path().join(".roko").join("learn");
        std::fs::create_dir_all(&learn_dir).unwrap();

        std::fs::write(
            learn_dir.join("experiments.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "experiments": {
                    "exp-01": {
                        "experiment_id": "exp-01",
                        "section_name": "constraints",
                        "role": "implementer",
                        "variants": [
                            {
                                "id": "winner",
                                "name": "Winner",
                                "slug": "claude-opus-4-6",
                                "active": true
                            },
                            {
                                "id": "runner-up",
                                "name": "Runner Up",
                                "slug": "gpt-5.4",
                                "active": true
                            }
                        ],
                        "stats": {
                            "winner": { "trials": 120, "successes": 114 },
                            "runner-up": { "trials": 120, "successes": 18 }
                        },
                        "status": "Concluded",
                        "winner_id": "winner"
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let snapshot = DashboardSnapshot::load_from_workdir(dir.path()).unwrap();
        assert_eq!(snapshot.experiment_winners.len(), 1);
        assert_eq!(snapshot.experiment_winners[0].experiment_id, "exp-01");
        assert_eq!(snapshot.experiment_winners[0].winner, "claude-opus-4-6");
        assert_eq!(snapshot.experiment_winners[0].sample_size, 120);
        assert!(snapshot.experiment_winners[0].ci_lower <= snapshot.experiment_winners[0].win_rate);
        assert!(snapshot.experiment_winners[0].ci_upper >= snapshot.experiment_winners[0].win_rate);
    }
}
