//! Parallel plan executor — the main orchestration loop.
//!
//! [`ParallelExecutor`] is the heart of Roko's orchestration: it holds a
//! queue of plans, advances each through the [`PlanPhase`] state machine
//! via [`tick()`](ParallelExecutor::tick), and emits
//! [`ExecutorAction`](action::ExecutorAction)s that the runtime harness
//! dispatches to agents, gates, and git.
//!
//! # Design
//!
//! The executor is a **pure state machine** — it never performs I/O. All
//! side-effects are expressed as `ExecutorAction` values returned from
//! `tick()`. The runtime (the thing that calls `tick()` in a loop) is
//! responsible for dispatching actions and feeding results back via
//! [`apply_event()`](ParallelExecutor::apply_event).
//!
//! # Sub-modules
//!
//! - [`action`] — the `ExecutorAction` enum
//! - [`plan_state`] — per-plan mutable state
//! - [`snapshot`] — crash-recovery serialization
//! - [`state_machine`] — phase transition logic
//! - [`reorder`] — queue reordering strategies

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::safety::audit_chain::{AuditChain, AuditEntry};
use serde::{Deserialize, Serialize};

use roko_core::{AgentRole, PlanPhase};

pub mod action;
pub mod plan_state;
pub mod recovery;
pub mod reorder;
pub mod snapshot;
pub mod state_machine;

pub use action::ExecutorAction;
pub use plan_state::{GateResult, PlanState};
pub use reorder::{priority_reorder, reorder_queue};
pub use snapshot::{CURRENT_SCHEMA_VERSION, ExecutorSnapshot, current_schema_version};
pub use state_machine::{ExecutorEvent, PlanStateMachine, TransitionError};

/// Live speculative execution tracking for dashboard and recovery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeculativeExecution {
    /// The plan that owns the task.
    pub plan_id: String,
    /// The task being shadowed.
    pub task: String,
    /// The original expectation in minutes.
    pub expected_minutes: u32,
    /// The elapsed runtime that triggered speculation.
    pub elapsed_minutes: u32,
    /// The backup role to spawn.
    pub backup_role: AgentRole,
    /// Projected spend for the backup branch.
    pub projected_cost_usd: f64,
    /// When the speculative branch was recorded.
    pub started_at_ms: u64,
}

/// Configuration for the parallel executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecutorConfig {
    /// Maximum number of plans executing concurrently.
    pub max_concurrent_plans: usize,
    /// Maximum number of tasks executing concurrently within a plan.
    #[serde(default = "ExecutorConfig::default_max_concurrent_tasks")]
    pub max_concurrent_tasks: usize,
    /// Maximum auto-fix iterations before declaring failure.
    pub max_auto_fix_iterations: u32,
    /// Maximum merge retry attempts.
    pub max_merge_attempts: u32,
    /// Per-task timeout in seconds.
    #[serde(default = "ExecutorConfig::default_task_timeout_secs")]
    pub task_timeout_secs: u64,
    /// Optional per-session budget cap in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    /// Multiplier applied to the expected duration before speculation starts.
    #[serde(default = "ExecutorConfig::default_speculative_threshold_multiplier")]
    pub speculative_threshold_multiplier: f64,
    /// Whether to auto-replan after repeated gate failures.
    #[serde(default = "ExecutorConfig::default_auto_replan")]
    pub auto_replan: bool,
    /// Whether task execution should use isolated git worktrees.
    #[serde(default = "ExecutorConfig::default_use_worktrees")]
    pub use_worktrees: bool,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_plans: 4,
            max_concurrent_tasks: Self::default_max_concurrent_tasks(),
            max_auto_fix_iterations: 5,
            max_merge_attempts: 3,
            task_timeout_secs: Self::default_task_timeout_secs(),
            budget_usd: None,
            speculative_threshold_multiplier: Self::default_speculative_threshold_multiplier(),
            auto_replan: Self::default_auto_replan(),
            use_worktrees: Self::default_use_worktrees(),
        }
    }
}

impl ExecutorConfig {
    const fn default_max_concurrent_tasks() -> usize {
        4
    }

    const fn default_task_timeout_secs() -> u64 {
        600
    }

    const fn default_speculative_threshold_multiplier() -> f64 {
        2.0
    }

    const fn default_auto_replan() -> bool {
        true
    }

    const fn default_use_worktrees() -> bool {
        false
    }
}

/// The main orchestration engine.
///
/// Holds a queue of plans, each with its own [`PlanState`], and
/// drives them through the [`PlanPhase`] lifecycle via
/// [`tick()`](Self::tick).
///
/// The executor is generic over nothing — it uses concrete types from
/// `roko-core`. External subsystems (agent pool, gate runner, merge
/// queue) are not held here; they are reached through the `ExecutorAction`
/// dispatch loop in the runtime.
#[derive(Debug)]
pub struct ParallelExecutor {
    /// Executor configuration.
    config: ExecutorConfig,
    /// Per-plan state, keyed by `plan_id`.
    plans: HashMap<String, PlanState>,
    /// Execution queue: `plan_id`s in priority order.
    queue: Vec<String>,
    /// Cross-plan dependencies: `plan_id` → list of `plan_id`s it depends on.
    plan_deps: HashMap<String, Vec<String>>,
    /// Record of speculative execution branches.
    speculative_executions: HashMap<String, SpeculativeExecution>,
    /// Optional tamper-evident audit chain for lifecycle transitions.
    audit_chain: Option<AuditChain>,
}

impl ParallelExecutor {
    /// Create a new executor with the given configuration.
    #[must_use]
    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            config,
            plans: HashMap::new(),
            queue: Vec::new(),
            plan_deps: HashMap::new(),
            speculative_executions: HashMap::new(),
            audit_chain: None,
        }
    }

    /// Restore an executor from a crash-recovery snapshot.
    #[must_use]
    pub fn from_snapshot(config: ExecutorConfig, snapshot: ExecutorSnapshot) -> Self {
        Self {
            config,
            plans: snapshot.plan_states,
            queue: snapshot.queue_order,
            plan_deps: HashMap::new(),
            speculative_executions: snapshot.speculative_executions,
            audit_chain: None,
        }
    }

    /// Add a plan to the executor queue.
    ///
    /// If a plan with the same ID already exists, it is **not** replaced
    /// (returns `false`).
    pub fn add_plan(&mut self, plan_state: PlanState) -> bool {
        let id = plan_state.plan_id.clone();
        if self.plans.contains_key(&id) {
            return false;
        }
        self.plans.insert(id.clone(), plan_state);
        self.queue.push(id);
        true
    }

    /// Set cross-plan dependencies (`plan_id` → list of `plan_id`s it depends on).
    ///
    /// Plans whose dependencies are not all terminal will be skipped in `tick()`.
    pub fn set_plan_dependencies(&mut self, deps: HashMap<String, Vec<String>>) {
        self.plan_deps = deps;
    }

    /// View the currently recorded speculative executions.
    #[must_use]
    pub fn speculative_executions(&self) -> Vec<&SpeculativeExecution> {
        self.speculative_executions.values().collect()
    }

    /// Register a speculative execution if the observed runtime exceeds the configured threshold.
    ///
    /// Returns the action the runtime should dispatch, or `None` if speculation
    /// is not warranted or the candidate was already recorded.
    #[must_use]
    pub fn register_speculative_execution(
        &mut self,
        plan_id: impl Into<String>,
        task: impl Into<String>,
        expected_minutes: u32,
        elapsed_minutes: u32,
        backup_role: AgentRole,
        projected_cost_usd: f64,
    ) -> Option<ExecutorAction> {
        if expected_minutes == 0 {
            return None;
        }
        if let Some(budget) = self.config.budget_usd {
            if projected_cost_usd > budget {
                return None;
            }
        }

        let plan_id = plan_id.into();
        let task = task.into();
        let key = format!("{plan_id}:{task}");
        if self.speculative_executions.contains_key(&key) {
            return None;
        }

        let threshold = f64::from(expected_minutes) * self.config.speculative_threshold_multiplier;
        if f64::from(elapsed_minutes) < threshold {
            return None;
        }

        let record = SpeculativeExecution {
            plan_id: plan_id.clone(),
            task: task.clone(),
            expected_minutes,
            elapsed_minutes,
            backup_role,
            projected_cost_usd,
            started_at_ms: current_timestamp_ms(),
        };
        self.speculative_executions.insert(key, record);

        Some(ExecutorAction::StartSpeculativeExecution {
            plan_id,
            task,
            backup_role,
            expected_minutes,
            elapsed_minutes,
        })
    }

    /// Resolve a speculative execution and emit a cancellation action for the losing branch.
    #[must_use]
    pub fn resolve_speculative_execution(
        &mut self,
        plan_id: &str,
        task: &str,
    ) -> Option<ExecutorAction> {
        let key = format!("{plan_id}:{task}");
        self.speculative_executions.remove(&key).map(|_| {
            ExecutorAction::CancelSpeculativeExecution {
                plan_id: plan_id.to_string(),
                task: task.to_string(),
            }
        })
    }

    /// Attach an audit chain to the executor.
    #[must_use]
    pub fn with_audit_chain(mut self, audit_chain: AuditChain) -> Self {
        self.audit_chain = Some(audit_chain);
        self
    }

    /// Inspect the attached audit chain, if any.
    #[must_use]
    pub const fn audit_chain(&self) -> Option<&AuditChain> {
        self.audit_chain.as_ref()
    }

    /// Check whether all dependency plans for `plan_id` are in a terminal state.
    fn deps_satisfied(&self, plan_id: &str) -> bool {
        let Some(deps) = self.plan_deps.get(plan_id) else {
            return true; // no deps declared
        };
        deps.iter().all(|dep| {
            self.plans
                .get(dep.as_str())
                .is_some_and(PlanState::is_terminal)
        })
    }

    /// Public view of the dependency gate for a plan.
    #[must_use]
    pub fn can_dispatch(&self, plan_id: &str) -> bool {
        self.deps_satisfied(plan_id)
    }

    /// One iteration of the main orchestration loop.
    ///
    /// Examines every active (non-terminal, non-paused) plan and returns
    /// the actions the runtime should dispatch. The executor respects
    /// `max_concurrent_plans`: only the first N queued plans are
    /// considered active. Plans whose cross-plan dependencies are not yet
    /// terminal are skipped.
    #[must_use]
    pub fn tick(&self) -> Vec<ExecutorAction> {
        let mut actions = Vec::new();
        let mut active_count = 0;

        for plan_id in &self.queue {
            let Some(state) = self.plans.get(plan_id) else {
                continue;
            };

            // Skip terminal plans.
            if state.is_terminal() {
                continue;
            }

            // Skip paused plans.
            if state.paused {
                continue;
            }

            // Skip plans whose cross-plan dependencies are not yet satisfied.
            if !self.deps_satisfied(plan_id) {
                continue;
            }

            // Respect concurrency limit.
            active_count += 1;
            if active_count > self.config.max_concurrent_plans {
                break;
            }

            // Ask the state machine what action is needed.
            if let Some(action) = PlanStateMachine::next_action(state) {
                actions.push(action);
            }
        }

        actions
    }

    /// Apply an event to a specific plan, transitioning its phase.
    ///
    /// # Errors
    ///
    /// Returns a [`TransitionError`] if the event is not legal for the
    /// plan's current phase, or if the `plan_id` is not found.
    pub fn apply_event(
        &mut self,
        plan_id: &str,
        event: &ExecutorEvent,
    ) -> Result<PlanPhase, TransitionError> {
        let state = self.plans.get(plan_id).ok_or_else(|| TransitionError {
            from: roko_core::PhaseKind::Queued,
            to: roko_core::PhaseKind::Failed,
            reason: format!("plan '{plan_id}' not found"),
        })?;
        let from_kind = state.current_phase.kind();

        let new_phase = PlanStateMachine::transition(state, event)?;
        let to_kind = new_phase.kind();

        // Apply the transition — the plan was just looked up so this
        // branch is unreachable, but we handle it gracefully.
        if let Some(state) = self.plans.get_mut(plan_id) {
            state.current_phase = new_phase.clone();

            // If transitioning to failed, record the reason.
            if let PlanPhase::Failed { reason } = &new_phase {
                state.last_error = Some(reason.to_string());
            }
        }

        if let Some(chain) = &self.audit_chain {
            let kind = format!("phase.{from_kind:?}->{to_kind:?}");
            let entry = AuditEntry::new([0u8; 32], kind, "executor", plan_id.to_string());
            let _ = chain.append(entry);
        }

        Ok(new_phase)
    }

    /// Pause a plan. Returns `false` if the plan was already paused or not found.
    pub fn pause_plan(&mut self, plan_id: &str) -> bool {
        if let Some(state) = self.plans.get_mut(plan_id) {
            if state.paused || state.is_terminal() {
                return false;
            }
            state.paused = true;
            true
        } else {
            false
        }
    }

    /// Resume a paused plan. Returns `false` if the plan was not paused or not found.
    pub fn resume_plan(&mut self, plan_id: &str) -> bool {
        if let Some(state) = self.plans.get_mut(plan_id) {
            if !state.paused {
                return false;
            }
            state.paused = false;
            true
        } else {
            false
        }
    }

    /// Restart a plan after a structural re-plan.
    ///
    /// Returns the plan's previous phase if it was found and reset.
    pub fn restart_plan(&mut self, plan_id: &str) -> Option<PlanPhase> {
        let state = self.plans.get_mut(plan_id)?;
        let previous_phase = state.current_phase.clone();
        state.restart_for_replan();
        Some(previous_phase)
    }

    /// Move a failed plan to the back of the queue.
    pub fn demote_failed(&mut self, plan_id: &str) {
        self.queue = reorder_queue(&self.queue, plan_id);
    }

    /// Re-sort the queue by plan priorities.
    pub fn reorder_by_priority(&mut self) {
        let priorities: HashMap<String, u32> = self
            .plans
            .iter()
            .map(|(id, ps)| (id.clone(), ps.priority))
            .collect();
        self.queue = priority_reorder(&self.queue, &priorities);
    }

    /// Reposition a plan in the queue. Returns `true` if the plan existed.
    pub fn reorder_plan(&mut self, plan_id: &str, new_position: usize) -> bool {
        let Some(current_index) = self.queue.iter().position(|id| id == plan_id) else {
            return false;
        };
        let item = self.queue.remove(current_index);
        let pos = new_position.min(self.queue.len());
        self.queue.insert(pos, item);
        true
    }

    /// List IDs of plans currently active (non-terminal, non-paused, within concurrency limit).
    #[must_use]
    pub fn active_plans(&self) -> Vec<String> {
        let mut result = Vec::new();
        for plan_id in &self.queue {
            if let Some(state) = self.plans.get(plan_id) {
                if !state.is_terminal() && !state.paused {
                    result.push(plan_id.clone());
                    if result.len() >= self.config.max_concurrent_plans {
                        break;
                    }
                }
            }
        }
        result
    }

    /// List IDs of completed plans (terminal phase).
    #[must_use]
    pub fn completed_plans(&self) -> Vec<String> {
        self.plans
            .iter()
            .filter(|(_, ps)| ps.is_terminal())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get a reference to a plan's state.
    #[must_use]
    pub fn plan_state(&self, plan_id: &str) -> Option<&PlanState> {
        self.plans.get(plan_id)
    }

    /// Get a mutable reference to a plan's state.
    pub fn plan_state_mut(&mut self, plan_id: &str) -> Option<&mut PlanState> {
        self.plans.get_mut(plan_id)
    }

    /// Total number of plans (active + completed + paused).
    #[must_use]
    pub fn plan_count(&self) -> usize {
        self.plans.len()
    }

    /// Create a crash-recovery snapshot of the current state.
    #[must_use]
    pub fn snapshot(&self, timestamp_ms: u64) -> ExecutorSnapshot {
        ExecutorSnapshot {
            schema_version: current_schema_version(),
            plan_states: self.plans.clone(),
            queue_order: self.queue.clone(),
            speculative_executions: self.speculative_executions.clone(),
            timestamp_ms,
        }
    }

    /// Current queue order (plan IDs).
    #[must_use]
    pub fn queue_order(&self) -> &[String] {
        &self.queue
    }

    /// Access the executor configuration.
    #[must_use]
    pub const fn config(&self) -> &ExecutorConfig {
        &self.config
    }
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::PhaseKind;

    fn default_executor() -> ParallelExecutor {
        ParallelExecutor::new(ExecutorConfig::default())
    }

    // ── Construction ──

    #[test]
    fn new_executor_is_empty() {
        let ex = default_executor();
        assert_eq!(ex.plan_count(), 0);
        assert!(ex.active_plans().is_empty());
        assert!(ex.completed_plans().is_empty());
        assert!(ex.tick().is_empty());
    }

    #[test]
    fn executor_config_disables_worktrees_by_default() {
        assert!(!ExecutorConfig::default().use_worktrees);
    }

    #[test]
    fn executor_config_sets_speculative_multiplier() {
        assert_eq!(
            ExecutorConfig::default().speculative_threshold_multiplier,
            2.0
        );
    }

    #[test]
    fn register_speculative_execution_respects_threshold() {
        let mut ex = default_executor();
        let action = ex.register_speculative_execution(
            "plan",
            "t1",
            10,
            25,
            roko_core::AgentRole::Implementer,
            3.5,
        );
        assert!(matches!(
            action,
            Some(ExecutorAction::StartSpeculativeExecution { .. })
        ));
        assert_eq!(ex.speculative_executions().len(), 1);

        let cancel = ex.resolve_speculative_execution("plan", "t1");
        assert!(matches!(
            cancel,
            Some(ExecutorAction::CancelSpeculativeExecution { .. })
        ));
        assert!(ex.speculative_executions().is_empty());
    }

    // ── add_plan ──

    #[test]
    fn add_plan_succeeds() {
        let mut ex = default_executor();
        let ps = PlanState::new("plan-1");
        assert!(ex.add_plan(ps));
        assert_eq!(ex.plan_count(), 1);
        assert_eq!(ex.active_plans(), vec!["plan-1"]);
    }

    #[test]
    fn add_duplicate_plan_fails() {
        let mut ex = default_executor();
        assert!(ex.add_plan(PlanState::new("dup")));
        assert!(!ex.add_plan(PlanState::new("dup")));
        assert_eq!(ex.plan_count(), 1);
    }

    // ── tick ──

    #[test]
    fn tick_returns_dispatch_for_queued_plans() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("a"));
        ex.add_plan(PlanState::new("b"));

        let actions = ex.tick();
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0], ExecutorAction::DispatchPlan { plan_id } if plan_id == "a"));
        assert!(matches!(&actions[1], ExecutorAction::DispatchPlan { plan_id } if plan_id == "b"));
    }

    #[test]
    fn tick_respects_concurrency_limit() {
        let mut ex = ParallelExecutor::new(ExecutorConfig {
            max_concurrent_plans: 2,
            ..ExecutorConfig::default()
        });
        for i in 0..5 {
            ex.add_plan(PlanState::new(format!("p{i}")));
        }

        let actions = ex.tick();
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn tick_skips_terminal_plans() {
        let mut ex = default_executor();
        let mut ps = PlanState::new("done");
        ps.current_phase = PlanPhase::Complete;
        ex.add_plan(ps);
        ex.add_plan(PlanState::new("active"));

        let actions = ex.tick();
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(&actions[0], ExecutorAction::DispatchPlan { plan_id } if plan_id == "active")
        );
    }

    #[test]
    fn tick_skips_paused_plans() {
        let mut ex = default_executor();
        let mut ps = PlanState::new("paused");
        ps.paused = true;
        ex.add_plan(ps);
        ex.add_plan(PlanState::new("active"));

        let actions = ex.tick();
        assert_eq!(actions.len(), 1);
    }

    // ── apply_event ──

    #[test]
    fn apply_event_transitions_phase() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("p1"));

        let new_phase = ex.apply_event("p1", &ExecutorEvent::Start).unwrap();
        assert_eq!(new_phase.kind(), PhaseKind::Enriching);

        let state = ex.plan_state("p1").unwrap();
        assert_eq!(state.current_phase.kind(), PhaseKind::Enriching);
    }

    #[test]
    fn apply_event_unknown_plan_errors() {
        let mut ex = default_executor();
        let result = ex.apply_event("ghost", &ExecutorEvent::Start);
        assert!(result.is_err());
    }

    #[test]
    fn apply_event_illegal_transition_errors() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("p"));
        // Queued -> GatePassed is illegal.
        let result = ex.apply_event("p", &ExecutorEvent::GatePassed);
        assert!(result.is_err());
    }

    // ── pause / resume ──

    #[test]
    fn pause_and_resume() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("p1"));

        assert!(ex.pause_plan("p1"));
        assert!(ex.plan_state("p1").unwrap().paused);
        assert!(ex.active_plans().is_empty());

        // Pausing again is a no-op.
        assert!(!ex.pause_plan("p1"));

        assert!(ex.resume_plan("p1"));
        assert!(!ex.plan_state("p1").unwrap().paused);

        // Resuming again is a no-op.
        assert!(!ex.resume_plan("p1"));
    }

    #[test]
    fn pause_unknown_plan() {
        let mut ex = default_executor();
        assert!(!ex.pause_plan("ghost"));
    }

    #[test]
    fn cannot_pause_terminal_plan() {
        let mut ex = default_executor();
        let mut ps = PlanState::new("done");
        ps.current_phase = PlanPhase::Complete;
        ex.add_plan(ps);
        assert!(!ex.pause_plan("done"));
    }

    // ── demote_failed / reorder_by_priority ──

    #[test]
    fn demote_failed_moves_to_back() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("a"));
        ex.add_plan(PlanState::new("b"));
        ex.add_plan(PlanState::new("c"));

        ex.demote_failed("a");
        assert_eq!(ex.queue_order(), &["b", "c", "a"]);
    }

    #[test]
    fn reorder_by_priority_sorts_correctly() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("low").with_priority(1));
        ex.add_plan(PlanState::new("high").with_priority(100));
        ex.add_plan(PlanState::new("mid").with_priority(50));

        ex.reorder_by_priority();
        assert_eq!(ex.queue_order(), &["high", "mid", "low"]);
    }

    // ── snapshot / restore ──

    #[test]
    fn snapshot_roundtrip() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("p1").with_priority(10));
        ex.add_plan(PlanState::new("p2"));
        ex.apply_event("p1", &ExecutorEvent::Start).unwrap();

        let snap = ex.snapshot(12345);
        let json = snap.to_json().unwrap();
        let restored_snap = ExecutorSnapshot::from_json(&json).unwrap();

        let ex2 = ParallelExecutor::from_snapshot(ExecutorConfig::default(), restored_snap);
        assert_eq!(ex2.plan_count(), 2);
        assert_eq!(
            ex2.plan_state("p1").unwrap().current_phase.kind(),
            PhaseKind::Enriching
        );
        assert_eq!(ex2.queue_order(), &["p1", "p2"]);
    }

    // ── completed_plans ──

    #[test]
    fn completed_plans_lists_terminal() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("active"));
        let mut done = PlanState::new("done");
        done.current_phase = PlanPhase::Complete;
        ex.add_plan(done);
        let mut failed = PlanState::new("failed");
        failed.current_phase = PlanPhase::Failed {
            reason: roko_core::FailureKind::Deadlock,
        };
        ex.add_plan(failed);

        let completed = ex.completed_plans();
        assert_eq!(completed.len(), 2);
        assert!(completed.contains(&"done".to_string()));
        assert!(completed.contains(&"failed".to_string()));
    }

    // ── full lifecycle through executor ──

    #[test]
    fn full_lifecycle_through_executor() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("plan-42"));

        // Tick should suggest DispatchPlan.
        let actions = ex.tick();
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], ExecutorAction::DispatchPlan { .. }));

        // Start -> Enriching
        ex.apply_event("plan-42", &ExecutorEvent::Start).unwrap();
        assert_eq!(
            ex.plan_state("plan-42").unwrap().current_phase.kind(),
            PhaseKind::Enriching
        );

        // EnrichmentDone -> Implementing
        ex.apply_event("plan-42", &ExecutorEvent::EnrichmentDone)
            .unwrap();
        let actions = ex.tick();
        assert!(matches!(
            &actions[0],
            ExecutorAction::SpawnAgent {
                role: roko_core::AgentRole::Implementer,
                ..
            }
        ));

        // ImplementationDone -> Gating
        ex.apply_event("plan-42", &ExecutorEvent::ImplementationDone)
            .unwrap();
        let actions = ex.tick();
        assert!(matches!(&actions[0], ExecutorAction::RunGate { .. }));

        // GatePassed -> Verifying
        ex.apply_event("plan-42", &ExecutorEvent::GatePassed)
            .unwrap();
        let actions = ex.tick();
        assert!(matches!(&actions[0], ExecutorAction::RunVerify { .. }));

        // VerifyPassed -> Reviewing
        ex.apply_event("plan-42", &ExecutorEvent::VerifyPassed)
            .unwrap();

        // ReviewApproved -> DocRevision
        ex.apply_event("plan-42", &ExecutorEvent::ReviewApproved)
            .unwrap();

        // DocRevisionDone -> Merging
        ex.apply_event("plan-42", &ExecutorEvent::DocRevisionDone)
            .unwrap();

        // MergeSucceeded -> Complete
        ex.apply_event("plan-42", &ExecutorEvent::MergeSucceeded)
            .unwrap();
        assert!(ex.plan_state("plan-42").unwrap().is_terminal());
        assert!(ex.completed_plans().contains(&"plan-42".to_string()));
    }

    // ── plan_state_mut ──

    #[test]
    fn plan_state_mut_allows_modification() {
        let mut ex = default_executor();
        ex.add_plan(PlanState::new("p"));
        ex.plan_state_mut("p")
            .unwrap()
            .files_changed
            .push("a.rs".into());
        assert_eq!(ex.plan_state("p").unwrap().files_changed, vec!["a.rs"]);
    }

    // ── config access ──

    #[test]
    fn config_access() {
        let ex = ParallelExecutor::new(ExecutorConfig {
            max_concurrent_plans: 7,
            ..ExecutorConfig::default()
        });
        assert_eq!(ex.config().max_concurrent_plans, 7);
    }

    #[test]
    fn restart_plan_for_replan_resets_state() {
        let mut ex = default_executor();
        let mut ps = PlanState::new("p");
        ps.current_phase = PlanPhase::Gating;
        ps.iteration = 3;
        ps.paused = true;
        ps.assigned_agents = vec!["agent-1".into()];
        ps.merge_attempts = 2;
        ex.add_plan(ps);

        let previous = ex.restart_plan("p").unwrap();
        assert_eq!(previous, PlanPhase::Gating);

        let state = ex.plan_state("p").unwrap();
        assert_eq!(state.current_phase, PlanPhase::Queued);
        assert_eq!(state.iteration, 4);
        assert!(!state.paused);
        assert!(state.assigned_agents.is_empty());
        assert_eq!(state.merge_attempts, 0);
    }
}
