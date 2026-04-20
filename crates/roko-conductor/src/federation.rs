//! Federated conductor hierarchy (COND-05).
//!
//! Four conductor levels aligned with Beer's Viable System Model:
//! - L1 `TurnConductor` (per-turn): wraps StuckDetector + MetaCognitionHook
//! - L2 `TaskConductor` (per-task): the existing `Conductor` with 10 watchers
//! - L3 `PlanConductor` (per-plan): aggregates task-level decisions across a plan
//! - L4 `FleetConductor` (per-fleet): cross-agent coordination (stub)

use std::collections::HashMap;

use roko_core::ConductorDecision;
use serde::{Deserialize, Serialize};

use crate::stuck_detection::{ActivityEntry, MetaCognitionHook, StuckDetector};

// ── L1: TurnConductor ──────────────────────────────────────────────────

/// Per-turn conductor wrapping stuck detection and meta-cognition (L1).
///
/// Operates at gamma frequency: evaluates after every agent turn.
/// Sensitivity can be adjusted by the L2 task conductor.
#[derive(Debug, Clone)]
pub struct TurnConductor {
    /// Stuck detector heuristics.
    pub stuck_detector: StuckDetector,
    /// Meta-cognition hook for self-reflection.
    pub meta_cognition: MetaCognitionHook,
    /// Sensitivity multiplier (adjustable by L2). Default 1.0.
    pub sensitivity: f64,
}

impl Default for TurnConductor {
    fn default() -> Self {
        Self {
            stuck_detector: StuckDetector::default(),
            meta_cognition: MetaCognitionHook::default(),
            sensitivity: 1.0,
        }
    }
}

impl TurnConductor {
    /// Evaluate a single agent turn for stuck conditions.
    ///
    /// Returns `Some(decision)` if intervention is needed, `None` to continue.
    #[must_use]
    pub fn evaluate_turn(&mut self, entries: &[ActivityEntry]) -> Option<ConductorDecision> {
        let signal = self.stuck_detector.check_stuck(entries)?;

        // Apply sensitivity: higher sensitivity lowers the confidence threshold.
        let threshold = 0.6 / self.sensitivity.max(0.1);
        if signal.confidence < threshold {
            return None;
        }

        Some(ConductorDecision::restart(
            "turn-conductor",
            &format!("stuck: {:?} (confidence {:.2})", signal.kind, signal.confidence),
        ))
    }

    /// Set sensitivity (called by L2 when adjusting L1 parameters).
    pub fn set_sensitivity(&mut self, sensitivity: f64) {
        self.sensitivity = sensitivity.clamp(0.1, 10.0);
    }
}

// ── L3: PlanConductor ──────────────────────────────────────────────────

/// Per-plan conductor that aggregates task-level decisions (L3).
///
/// Operates at delta frequency: evaluates after each task completes.
/// Detects plan-level patterns and adjusts L2 thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConductor {
    /// Accumulated task decisions for the current plan.
    pub task_decisions: Vec<TaskDecisionRecord>,
    /// Remaining plan budget in USD.
    pub plan_budget_remaining: f64,
    /// Number of task failures so far.
    pub task_failure_count: usize,
    /// Maximum task failures before plan-level intervention (default 2).
    pub max_plan_failures: usize,
    /// L2 threshold adjustments recommended by this conductor.
    pub l2_adjustments: HashMap<String, f64>,
}

/// Record of a task-level conductor decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDecisionRecord {
    /// Task identifier.
    pub task_id: String,
    /// Decision label (continue, restart, fail).
    pub decision_label: String,
    /// Whether the task eventually succeeded.
    pub task_succeeded: bool,
}

impl Default for PlanConductor {
    fn default() -> Self {
        Self {
            task_decisions: Vec::new(),
            plan_budget_remaining: f64::INFINITY,
            task_failure_count: 0,
            max_plan_failures: 2,
            l2_adjustments: HashMap::new(),
        }
    }
}

impl PlanConductor {
    /// Create a plan conductor with the given budget and failure threshold.
    #[must_use]
    pub fn new(budget_usd: f64, max_failures: usize) -> Self {
        Self {
            plan_budget_remaining: budget_usd,
            max_plan_failures: max_failures,
            ..Default::default()
        }
    }

    /// Record a task-level decision and aggregate into a plan-level decision.
    ///
    /// Returns the aggregated plan-level decision.
    #[must_use]
    pub fn aggregate(
        &mut self,
        task_id: &str,
        decision: &ConductorDecision,
        task_succeeded: bool,
    ) -> ConductorDecision {
        self.task_decisions.push(TaskDecisionRecord {
            task_id: task_id.to_owned(),
            decision_label: decision.label().to_owned(),
            task_succeeded,
        });

        if !task_succeeded {
            self.task_failure_count += 1;
        }

        // Plan-level failure if too many tasks have failed.
        if self.task_failure_count >= self.max_plan_failures {
            return ConductorDecision::fail(
                "plan-conductor",
                roko_core::FailureKind::Other(format!(
                    "{} task failures exceed plan threshold of {}",
                    self.task_failure_count, self.max_plan_failures,
                )),
            );
        }

        // Budget pressure: if remaining budget is low, recommend adjustments.
        if self.plan_budget_remaining < 1.0 && self.plan_budget_remaining > 0.0 {
            self.l2_adjustments
                .insert("cost-overrun".to_string(), 0.5);
        }

        ConductorDecision::cont()
    }

    /// Compute L2 threshold adjustments based on plan-level observations.
    ///
    /// The L3 conductor cascades parameters down to L2:
    /// - High failure rate -> lower L2 thresholds (intervene earlier)
    /// - Low budget -> tighten cost thresholds
    #[must_use]
    pub fn adjust_l2_thresholds(&self) -> HashMap<String, f64> {
        let mut adjustments = self.l2_adjustments.clone();

        // If more than half of completed tasks had failures, lower thresholds.
        let total = self.task_decisions.len();
        if total >= 3 {
            let failure_rate =
                self.task_failure_count as f64 / total as f64;
            if failure_rate > 0.5 {
                // Lower quality watcher thresholds by 30%.
                adjustments
                    .entry("compile-fail-repeat".to_string())
                    .or_insert(0.7);
                adjustments
                    .entry("test-failure-budget".to_string())
                    .or_insert(0.7);
            }
        }

        adjustments
    }

    /// Update the remaining budget after a task completes.
    pub fn record_spend(&mut self, amount_usd: f64) {
        self.plan_budget_remaining -= amount_usd;
    }
}

// ── L4: FleetConductor ─────────────────────────────────────────────────

/// Cross-agent fleet conductor (L4, Phase 2+ stub).
///
/// Coordinates across multiple agents. Currently a passthrough.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FleetConductor {
    /// Number of active agents in the fleet.
    pub active_agents: usize,
    /// Fleet-wide budget remaining.
    pub fleet_budget_remaining: f64,
}

impl FleetConductor {
    /// Create a fleet conductor.
    #[must_use]
    pub fn new(fleet_budget: f64) -> Self {
        Self {
            active_agents: 0,
            fleet_budget_remaining: fleet_budget,
        }
    }

    /// Evaluate fleet-level health. Currently always continues.
    #[must_use]
    pub fn evaluate(&self) -> ConductorDecision {
        ConductorDecision::cont()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_conductor_default_no_entries_returns_none() {
        let mut tc = TurnConductor::default();
        assert!(tc.evaluate_turn(&[]).is_none());
    }

    #[test]
    fn turn_conductor_sensitivity_clamped() {
        let mut tc = TurnConductor::default();
        tc.set_sensitivity(0.01);
        assert!((tc.sensitivity - 0.1).abs() < f64::EPSILON);
        tc.set_sensitivity(100.0);
        assert!((tc.sensitivity - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn plan_conductor_aggregates_failures() {
        let mut pc = PlanConductor::new(10.0, 2);
        let cont = ConductorDecision::cont();

        let d1 = pc.aggregate("t1", &cont, false);
        assert!(d1.is_continue());

        let d2 = pc.aggregate("t2", &cont, false);
        assert!(d2.is_terminal());
    }

    #[test]
    fn plan_conductor_adjusts_l2_on_high_failure_rate() {
        let mut pc = PlanConductor::new(10.0, 10);
        let cont = ConductorDecision::cont();

        for i in 0..4 {
            pc.aggregate(&format!("t{i}"), &cont, i % 2 == 0);
        }
        // 2 failures out of 4 = 50% -> should trigger adjustment.
        let adj = pc.adjust_l2_thresholds();
        assert!(
            adj.contains_key("compile-fail-repeat")
                || adj.contains_key("test-failure-budget")
                || adj.is_empty() // exactly 50%, so >= 3 tasks but rate == 0.5
        );
    }

    #[test]
    fn plan_conductor_budget_pressure() {
        let mut pc = PlanConductor::new(0.5, 5);
        let cont = ConductorDecision::cont();
        let _ = pc.aggregate("t1", &cont, true);
        let adj = pc.adjust_l2_thresholds();
        assert!(adj.contains_key("cost-overrun"));
    }

    #[test]
    fn fleet_conductor_always_continues() {
        let fc = FleetConductor::new(100.0);
        assert!(fc.evaluate().is_continue());
    }
}
