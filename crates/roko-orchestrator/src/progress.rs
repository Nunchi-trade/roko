//! Monotonic progress enforcement, enrichment validation, and fail-loud
//! error publishing.
//!
//! # Progress tracking
//!
//! [`ProgressTracker`] records the current [`PlanPhase`] for each plan and
//! enforces forward-only transitions via [`is_monotonic_progression`].
//! Going backwards requires an explicit [`force_restart`] call — there is
//! no silent rewind.
//!
//! # Enrichment validation
//!
//! [`validate_enrichment`] checks that a JSON payload contains all keys
//! required by a named schema. This is a lightweight structural check, not
//! a full JSON Schema validator; it ensures the enrichment pipeline
//! produced the expected fields before an agent is spawned.
//!
//! # Fail-loud
//!
//! [`publish_error`] converts any [`RokoError`] into a structured
//! [`ErrorEvent`] that can be recorded on an [`EventLog`]. The philosophy
//! is that every failure must produce a visible, auditable event — silent
//! swallowing is a bug.
//!
//! [`force_restart`]: ProgressTracker::force_restart
//! [`is_monotonic_progression`]: roko_core::is_monotonic_progression

use std::collections::HashMap;

use roko_core::error::ErrorKind;
use roko_core::phase::is_monotonic_progression;
use roko_core::{PhaseKind, PlanPhase, RokoError};
use serde::{Deserialize, Serialize};

// ─── ProgressError ──────────────────────────────────────────────────────

/// Error returned when a phase transition violates monotonicity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressError {
    /// The phase the plan was in.
    pub from: PhaseKind,
    /// The phase the caller tried to transition to.
    pub to: PhaseKind,
    /// Human-readable explanation.
    pub reason: String,
}

impl std::fmt::Display for ProgressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "invalid transition {:?} -> {:?}: {}",
            self.from, self.to, self.reason
        )
    }
}

impl std::error::Error for ProgressError {}

// ─── ProgressTracker ────────────────────────────────────────────────────

/// Tracks current phase per plan and enforces forward-only transitions.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    phases: HashMap<String, PlanPhase>,
}

impl ProgressTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the current phase for a plan, or `None` if untracked.
    pub fn current_phase(&self, plan_id: &str) -> Option<&PlanPhase> {
        self.phases.get(plan_id)
    }

    /// Attempt a phase transition. The transition is rejected if:
    /// - the plan is not yet tracked (caller must register with an initial
    ///   `try_transition` from `Queued`),
    /// - the `from` parameter does not match the plan's current phase, or
    /// - the transition violates the [`valid_transitions`] table.
    ///
    /// [`valid_transitions`]: roko_core::valid_transitions
    pub fn try_transition(
        &mut self,
        plan_id: &str,
        from: &PlanPhase,
        to: PlanPhase,
    ) -> Result<(), ProgressError> {
        match self.phases.get(plan_id) {
            None => {
                // First transition: accept only if `from` is Queued
                // (initial state).
                if from.kind() != PhaseKind::Queued {
                    return Err(ProgressError {
                        from: from.kind(),
                        to: to.kind(),
                        reason: "plan not yet tracked; first transition must be from Queued".into(),
                    });
                }
                if !is_monotonic_progression(from, &to) {
                    return Err(ProgressError {
                        from: from.kind(),
                        to: to.kind(),
                        reason: "transition not in valid_transitions table".into(),
                    });
                }
                self.phases.insert(plan_id.to_string(), to);
                Ok(())
            }
            Some(current) => {
                if current.kind() != from.kind() {
                    return Err(ProgressError {
                        from: from.kind(),
                        to: to.kind(),
                        reason: format!(
                            "current phase is {:?}, not {:?}",
                            current.kind(),
                            from.kind()
                        ),
                    });
                }
                if !is_monotonic_progression(from, &to) {
                    return Err(ProgressError {
                        from: from.kind(),
                        to: to.kind(),
                        reason: "transition not in valid_transitions table".into(),
                    });
                }
                self.phases.insert(plan_id.to_string(), to);
                Ok(())
            }
        }
    }

    /// Explicitly reset a plan to [`PlanPhase::Queued`]. This is the
    /// **only** way to go "backwards" — it represents a deliberate
    /// restart, not an accidental rewind.
    pub fn force_restart(&mut self, plan_id: &str) {
        self.phases.insert(plan_id.to_string(), PlanPhase::Queued);
    }

    /// Number of plans currently tracked.
    pub fn tracked_count(&self) -> usize {
        self.phases.len()
    }
}

// ─── Enrichment validation ──────────────────────────────────────────────

/// Error returned when enrichment data fails schema validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    /// The schema that was validated against.
    pub schema: String,
    /// Human-readable list of violations.
    pub violations: Vec<String>,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "enrichment validation failed for schema '{}': {}",
            self.schema,
            self.violations.join("; ")
        )
    }
}

impl std::error::Error for ValidationError {}

/// Known enrichment schemas and the keys they require.
fn required_keys(schema: &str) -> Option<&'static [&'static str]> {
    match schema {
        "brief" => Some(&["title", "objective", "scope"]),
        "test_plan" => Some(&["tests", "coverage_targets"]),
        "invariants" => Some(&["invariants"]),
        "context" => Some(&["files", "dependencies"]),
        _ => None,
    }
}

/// Validate that `data` contains all keys required by the named `schema`.
///
/// Returns `Ok(())` when every required key is present (regardless of
/// value), or a [`ValidationError`] listing the missing keys.
///
/// Unknown schemas are accepted with no required keys — this is
/// intentionally permissive so that new schemas can be introduced
/// without blocking the pipeline. Callers should log a warning for
/// unknown schemas.
pub fn validate_enrichment(
    data: &serde_json::Value,
    schema: &str,
) -> Result<(), ValidationError> {
    let Some(keys) = required_keys(schema) else {
        return Ok(()); // unknown schema: pass through
    };

    let Some(obj) = data.as_object() else {
        return Err(ValidationError {
            schema: schema.to_string(),
            violations: vec!["enrichment data is not a JSON object".into()],
        });
    };

    let missing: Vec<String> = keys
        .iter()
        .filter(|k| !obj.contains_key(**k))
        .map(|k| format!("missing required key: {k}"))
        .collect();

    if missing.is_empty() {
        Ok(())
    } else {
        Err(ValidationError {
            schema: schema.to_string(),
            violations: missing,
        })
    }
}

// ─── Fail-loud: ErrorEvent ──────────────────────────────────────────────

/// A structured error event suitable for recording on an [`EventLog`].
///
/// Every failure in the orchestrator should produce one of these via
/// [`publish_error`]. The philosophy is "fail loud" — silent error
/// swallowing is a bug.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    /// The stable error kind discriminant.
    pub kind: ErrorKind,
    /// Human-readable error message.
    pub message: String,
    /// Whether the error is transient (may succeed on retry).
    pub transient: bool,
}

/// Convert a [`RokoError`] into a structured [`ErrorEvent`].
///
/// This is the canonical "fail loud" entry point: the returned
/// `ErrorEvent` should be appended to the event log so that every
/// failure is visible and auditable.
pub fn publish_error(error: &RokoError) -> ErrorEvent {
    ErrorEvent {
        kind: error.kind(),
        message: error.to_string(),
        transient: error.is_transient(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::FailureKind;
    use serde_json::json;

    // ── ProgressTracker ─────────────────────────────────────────────

    #[test]
    fn happy_path_transitions() {
        let mut pt = ProgressTracker::new();
        let plan = "plan-1";

        // Queued -> Enriching
        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Enriching)
            .unwrap();
        assert_eq!(pt.current_phase(plan).unwrap().kind(), PhaseKind::Enriching);

        // Enriching -> Implementing
        pt.try_transition(plan, &PlanPhase::Enriching, PlanPhase::Implementing)
            .unwrap();
        assert_eq!(
            pt.current_phase(plan).unwrap().kind(),
            PhaseKind::Implementing
        );

        // Implementing -> Gating
        pt.try_transition(plan, &PlanPhase::Implementing, PlanPhase::Gating)
            .unwrap();
        assert_eq!(pt.current_phase(plan).unwrap().kind(), PhaseKind::Gating);
    }

    #[test]
    fn invalid_transition_rejected() {
        let mut pt = ProgressTracker::new();
        let plan = "plan-2";

        // Queued -> Enriching (valid first)
        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Enriching)
            .unwrap();

        // Enriching -> Complete (illegal: skips many phases)
        let err = pt
            .try_transition(plan, &PlanPhase::Enriching, PlanPhase::Complete)
            .unwrap_err();
        assert_eq!(err.from, PhaseKind::Enriching);
        assert_eq!(err.to, PhaseKind::Complete);
    }

    #[test]
    fn wrong_from_phase_rejected() {
        let mut pt = ProgressTracker::new();
        let plan = "plan-3";

        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Enriching)
            .unwrap();

        // Claim we're in Gating but we're actually in Enriching
        let err = pt
            .try_transition(plan, &PlanPhase::Gating, PlanPhase::Verifying)
            .unwrap_err();
        assert!(err.reason.contains("current phase"));
    }

    #[test]
    fn force_restart_resets_to_queued() {
        let mut pt = ProgressTracker::new();
        let plan = "plan-4";

        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Enriching)
            .unwrap();
        pt.try_transition(plan, &PlanPhase::Enriching, PlanPhase::Implementing)
            .unwrap();

        // Force restart
        pt.force_restart(plan);
        assert_eq!(pt.current_phase(plan).unwrap().kind(), PhaseKind::Queued);

        // Can now start over
        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Enriching)
            .unwrap();
    }

    #[test]
    fn terminal_phase_blocks_further_transition() {
        let mut pt = ProgressTracker::new();
        let plan = "plan-5";

        // Fast-track to Failed
        pt.try_transition(plan, &PlanPhase::Queued, PlanPhase::Failed {
            reason: FailureKind::Deadlock,
        })
        .unwrap();

        // Cannot transition from Failed to anything
        let err = pt
            .try_transition(
                plan,
                &PlanPhase::Failed {
                    reason: FailureKind::Deadlock,
                },
                PlanPhase::Implementing,
            )
            .unwrap_err();
        assert_eq!(err.from, PhaseKind::Failed);
    }

    #[test]
    fn untracked_plan_requires_queued_start() {
        let mut pt = ProgressTracker::new();

        // Try to start from Implementing (not Queued)
        let err = pt
            .try_transition("new-plan", &PlanPhase::Implementing, PlanPhase::Gating)
            .unwrap_err();
        assert!(err.reason.contains("first transition must be from Queued"));
    }

    // ── Enrichment validation ───────────────────────────────────────

    #[test]
    fn valid_brief_passes() {
        let data = json!({
            "title": "Fix auth",
            "objective": "Patch CVE-2024-1234",
            "scope": ["auth.rs", "session.rs"]
        });
        assert!(validate_enrichment(&data, "brief").is_ok());
    }

    #[test]
    fn missing_key_fails() {
        let data = json!({
            "title": "Fix auth"
            // missing "objective" and "scope"
        });
        let err = validate_enrichment(&data, "brief").unwrap_err();
        assert_eq!(err.schema, "brief");
        assert_eq!(err.violations.len(), 2);
    }

    #[test]
    fn non_object_fails() {
        let data = json!([1, 2, 3]);
        let err = validate_enrichment(&data, "brief").unwrap_err();
        assert!(err.violations[0].contains("not a JSON object"));
    }

    #[test]
    fn unknown_schema_passes() {
        let data = json!({"anything": "goes"});
        assert!(validate_enrichment(&data, "future_schema_v99").is_ok());
    }

    // ── Fail-loud ───────────────────────────────────────────────────

    #[test]
    fn publish_error_captures_transient() {
        let err = RokoError::timeout("compile", 30_000);
        let event = publish_error(&err);
        assert_eq!(event.kind, ErrorKind::Timeout);
        assert!(event.transient);
        assert!(event.message.contains("30000"));
    }

    #[test]
    fn publish_error_captures_permanent() {
        let err = RokoError::invalid("bad plan file");
        let event = publish_error(&err);
        assert_eq!(event.kind, ErrorKind::Invalid);
        assert!(!event.transient);
        assert!(event.message.contains("bad plan file"));
    }

    #[test]
    fn progress_error_display() {
        let err = ProgressError {
            from: PhaseKind::Enriching,
            to: PhaseKind::Complete,
            reason: "skipped phases".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("Enriching"));
        assert!(msg.contains("Complete"));
        assert!(msg.contains("skipped phases"));
    }

    #[test]
    fn validation_error_display() {
        let err = ValidationError {
            schema: "brief".into(),
            violations: vec!["missing key: title".into(), "missing key: scope".into()],
        };
        let msg = err.to_string();
        assert!(msg.contains("brief"));
        assert!(msg.contains("missing key: title"));
    }
}
