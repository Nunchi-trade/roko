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

use roko_core::PlanPhase;

pub mod action;
pub mod plan_state;
pub mod recovery;
pub mod reorder;
pub mod snapshot;
pub mod state_machine;

pub use action::ExecutorAction;
pub use plan_state::{GateResult, PlanState};
pub use reorder::{priority_reorder, reorder_queue};
pub use snapshot::ExecutorSnapshot;
pub use state_machine::{ExecutorEvent, PlanStateMachine, TransitionError};

/// Configuration for the parallel executor.
#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    /// Maximum number of plans executing concurrently.
    pub max_concurrent_plans: usize,
    /// Maximum auto-fix iterations before declaring failure.
    pub max_auto_fix_iterations: u32,
    /// Maximum merge retry attempts.
    pub max_merge_attempts: u32,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            max_concurrent_plans: 4,
            max_auto_fix_iterations: 5,
            max_merge_attempts: 3,
        }
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
}

impl ParallelExecutor {
    /// Create a new executor with the given configuration.
    #[must_use]
    pub fn new(config: ExecutorConfig) -> Self {
        Self {
            config,
            plans: HashMap::new(),
            queue: Vec::new(),
        }
    }

    /// Restore an executor from a crash-recovery snapshot.
    #[must_use]
    pub fn from_snapshot(config: ExecutorConfig, snapshot: ExecutorSnapshot) -> Self {
        Self {
            config,
            plans: snapshot.plan_states,
            queue: snapshot.queue_order,
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

    /// One iteration of the main orchestration loop.
    ///
    /// Examines every active (non-terminal, non-paused) plan and returns
    /// the actions the runtime should dispatch. The executor respects
    /// `max_concurrent_plans`: only the first N queued plans are
    /// considered active.
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

        let new_phase = PlanStateMachine::transition(state, event)?;

        // Apply the transition — the plan was just looked up so this
        // branch is unreachable, but we handle it gracefully.
        if let Some(state) = self.plans.get_mut(plan_id) {
            state.current_phase = new_phase.clone();

            // If transitioning to failed, record the reason.
            if let PlanPhase::Failed { reason } = &new_phase {
                state.last_error = Some(reason.to_string());
            }
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
            plan_states: self.plans.clone(),
            queue_order: self.queue.clone(),
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
        assert!(matches!(&actions[0], ExecutorAction::DispatchPlan { plan_id } if plan_id == "active"));
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
        assert!(matches!(&actions[0], ExecutorAction::SpawnAgent { role: roko_core::AgentRole::Implementer, .. }));

        // ImplementationDone -> Gating
        ex.apply_event("plan-42", &ExecutorEvent::ImplementationDone)
            .unwrap();
        let actions = ex.tick();
        assert!(matches!(&actions[0], ExecutorAction::RunGate { .. }));

        // GatePassed -> Verifying
        ex.apply_event("plan-42", &ExecutorEvent::GatePassed)
            .unwrap();

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
        ex.plan_state_mut("p").unwrap().files_changed.push("a.rs".into());
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
}
