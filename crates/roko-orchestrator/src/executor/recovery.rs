//! Deep crash recovery via executor snapshots and event-log replay.
//!
//! After a crash, the orchestrator can recover its state from two sources:
//!
//! 1. **Executor snapshot** (`executor.json`) — a point-in-time capture of
//!    every plan's mutable state plus the execution queue order.
//! 2. **Event log** — an append-only, hash-chained sequence of orchestration
//!    events that can be replayed to reconstruct state from scratch.
//!
//! The [`RecoveryEngine`] orchestrates the full recovery pipeline:
//! deserialize the snapshot, replay the event log, merge both (event log
//! wins on conflict), and validate the result for inconsistencies.

use std::collections::HashMap;

use roko_core::{PhaseKind, PlanPhase};
use serde::{Deserialize, Serialize};

use super::plan_state::PlanResumeDirective;
use super::snapshot::ExecutorSnapshot;
use crate::event_log::{EventEntry, EventKind};

// ─── Error types ────────────────────────────────────────────────────────

/// Errors that can occur during crash recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// The snapshot JSON is corrupt or unparseable.
    CorruptedSnapshot(String),
    /// Event sequence numbers are not monotonically increasing or have gaps.
    InvalidEventSequence(String),
    /// A plan referenced in the event log has no corresponding state.
    MissingPlanState(String),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CorruptedSnapshot(msg) => write!(f, "corrupted snapshot: {msg}"),
            Self::InvalidEventSequence(msg) => write!(f, "invalid event sequence: {msg}"),
            Self::MissingPlanState(msg) => write!(f, "missing plan state: {msg}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

// ─── Warning types ─────────────────────────────────────────────────────

/// Severity level for recovery warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WarningSeverity {
    /// Informational — recovery can proceed but the operator should know.
    Info,
    /// Warning — state may be slightly stale or inconsistent.
    Warning,
    /// Critical — recovered state is likely incorrect; manual inspection needed.
    Critical,
}

/// A non-fatal inconsistency detected during recovery validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryWarning {
    /// The plan that the warning pertains to, if any.
    pub plan_id: String,
    /// Human-readable description of the inconsistency.
    pub message: String,
    /// How severe the inconsistency is.
    pub severity: WarningSeverity,
}

// ─── PlanPhaseInfo ──────────────────────────────────────────────────────

/// Per-plan phase information reconstructed during recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanPhaseInfo {
    /// Stable plan identifier.
    pub plan_id: String,
    /// The plan's phase at the time of the snapshot / last event.
    pub phase: PlanPhase,
    /// Current iteration (starts at 1, bumps on retry).
    pub iteration: u32,
    /// The last gate result summary, if any.
    pub last_gate_result: Option<String>,
    /// Files modified by agents so far.
    pub files_changed: Vec<String>,
}

impl PlanPhaseInfo {
    /// Classify how this recovered plan should behave on resume.
    #[must_use]
    pub fn resume_directive(&self) -> PlanResumeDirective {
        match &self.phase {
            PlanPhase::Failed { reason } if reason.requires_manual_repair() => {
                PlanResumeDirective::AwaitManualRepair {
                    failure: reason.clone(),
                }
            }
            PlanPhase::Failed { reason } if reason.auto_retry_on_resume() => {
                PlanResumeDirective::RetryTerminalFailure {
                    failure: reason.clone(),
                    cooldown_secs: reason.retry_cooldown_secs(),
                }
            }
            phase if phase.kind() == PhaseKind::Complete => PlanResumeDirective::TerminalComplete,
            phase if phase.kind() == PhaseKind::Skipped => PlanResumeDirective::TerminalSkipped,
            _ => PlanResumeDirective::ContinueActive,
        }
    }
}

/// A recovered plan paired with the resume directive derived from its phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveredPlanResume {
    /// Stable plan identifier.
    pub plan_id: String,
    /// Phase recovered for the plan.
    pub phase: PlanPhase,
    /// Iteration recovered for the plan.
    pub iteration: u32,
    /// Resume behavior the runner should apply.
    pub directive: PlanResumeDirective,
    /// Earliest timestamp at which retry should occur, when retryable.
    pub retry_after_ms: Option<u64>,
}

/// Resume plan derived from a recovered state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryResumePlan {
    /// Non-terminal plans that should continue from their recovered phase.
    pub active: Vec<RecoveredPlanResume>,
    /// Terminal failures that can be requeued automatically.
    pub retryable_terminal: Vec<RecoveredPlanResume>,
    /// Terminal failures that require manual repair before requeue.
    pub manual_repair: Vec<RecoveredPlanResume>,
    /// Plans already completed successfully.
    pub completed: Vec<String>,
    /// Plans skipped by policy/operator.
    pub skipped: Vec<String>,
    /// Non-fatal recovery warnings discovered during validation.
    pub warnings: Vec<RecoveryWarning>,
}

impl RecoveryResumePlan {
    /// Whether there is any plan the runner can continue or retry.
    #[must_use]
    pub fn has_runnable_work(&self) -> bool {
        !self.active.is_empty() || !self.retryable_terminal.is_empty()
    }

    /// Plan IDs that should appear in the executor queue on resume.
    #[must_use]
    pub fn queueable_plan_ids(&self) -> Vec<String> {
        self.active
            .iter()
            .chain(self.retryable_terminal.iter())
            .map(|plan| plan.plan_id.clone())
            .collect()
    }
}

// ─── RecoveredState ─────────────────────────────────────────────────────

/// The full state reconstructed by crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveredState {
    /// Per-plan phase information, keyed by `plan_id`.
    pub plan_states: HashMap<String, PlanPhaseInfo>,
    /// Queue order: `plan_id`s in execution priority order.
    pub queue_order: Vec<String>,
    /// The highest event sequence number processed (0 if no events).
    pub last_sequence: u64,
    /// Unix millisecond timestamp when recovery was performed.
    pub recovery_timestamp_ms: u64,
}

impl RecoveredState {
    /// Create an empty recovered state.
    fn empty(timestamp_ms: u64) -> Self {
        Self {
            plan_states: HashMap::new(),
            queue_order: Vec::new(),
            last_sequence: 0,
            recovery_timestamp_ms: timestamp_ms,
        }
    }

    /// Build a typed resume plan from the recovered state.
    #[must_use]
    pub fn resume_plan(&self, now_ms: u64) -> RecoveryResumePlan {
        let mut active = Vec::new();
        let mut retryable_terminal = Vec::new();
        let mut manual_repair = Vec::new();
        let mut completed = Vec::new();
        let mut skipped = Vec::new();

        let mut seen = std::collections::BTreeSet::new();
        let mut ids = Vec::new();
        for id in &self.queue_order {
            if self.plan_states.contains_key(id) && seen.insert(id.clone()) {
                ids.push(id.clone());
            }
        }
        let mut missing: Vec<_> = self
            .plan_states
            .keys()
            .filter(|id| !seen.contains(*id))
            .cloned()
            .collect();
        missing.sort();
        ids.extend(missing);

        for id in ids {
            let info = &self.plan_states[&id];
            let directive = info.resume_directive();
            let retry_after_ms = directive
                .cooldown_secs()
                .map(|secs| now_ms.saturating_add(secs.saturating_mul(1_000)));
            let resume = RecoveredPlanResume {
                plan_id: info.plan_id.clone(),
                phase: info.phase.clone(),
                iteration: info.iteration,
                directive: directive.clone(),
                retry_after_ms,
            };

            match directive {
                PlanResumeDirective::ContinueActive => active.push(resume),
                PlanResumeDirective::RetryTerminalFailure { .. } => {
                    retryable_terminal.push(resume);
                }
                PlanResumeDirective::AwaitManualRepair { .. } => manual_repair.push(resume),
                PlanResumeDirective::TerminalComplete => completed.push(info.plan_id.clone()),
                PlanResumeDirective::TerminalSkipped => skipped.push(info.plan_id.clone()),
            }
        }

        RecoveryResumePlan {
            active,
            retryable_terminal,
            manual_repair,
            completed,
            skipped,
            warnings: RecoveryEngine::validate_recovery(self),
        }
    }
}

// ─── RecoveryEngine ─────────────────────────────────────────────────────

/// Orchestrates crash recovery from executor snapshots and event-log replay.
///
/// Usage:
/// ```ignore
/// let engine = RecoveryEngine::new();
/// let snap_state = engine.recover_from_snapshot(json)?;
/// let log_state = engine.recover_from_event_log(&events)?;
/// let merged = RecoveryEngine::merge_recovery(Some(snap_state), Some(log_state));
/// let warnings = RecoveryEngine::validate_recovery(&merged);
/// ```
pub struct RecoveryEngine {
    _private: (),
}

impl RecoveryEngine {
    /// Create a new recovery engine.
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Recover orchestrator state from a serialized [`ExecutorSnapshot`].
    ///
    /// Deserializes the JSON, converts each [`PlanState`](super::plan_state::PlanState)
    /// into a [`PlanPhaseInfo`], and preserves the queue order.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::CorruptedSnapshot`] if the JSON cannot be
    /// parsed into an `ExecutorSnapshot`.
    pub fn recover_from_snapshot(
        &self,
        snapshot_json: &str,
    ) -> Result<RecoveredState, RecoveryError> {
        let snapshot = ExecutorSnapshot::from_json(snapshot_json)
            .map_err(|e| RecoveryError::CorruptedSnapshot(format!("JSON parse error: {e}")))?;

        Ok(self.recover_from_executor_snapshot(snapshot))
    }

    /// Recover orchestrator state from an already-deserialized executor snapshot.
    #[must_use]
    pub fn recover_from_executor_snapshot(&self, snapshot: ExecutorSnapshot) -> RecoveredState {
        let now_ms = current_timestamp_ms();

        let mut plan_states = HashMap::with_capacity(snapshot.plan_states.len());
        for (id, ps) in &snapshot.plan_states {
            let last_gate = ps
                .gate_results
                .last()
                .map(|g| format!("{}: {}", g.gate_name, g.summary));

            plan_states.insert(
                id.clone(),
                PlanPhaseInfo {
                    plan_id: id.clone(),
                    phase: ps.current_phase.clone(),
                    iteration: ps.iteration,
                    last_gate_result: last_gate,
                    files_changed: ps.files_changed.clone(),
                },
            );
        }

        RecoveredState {
            plan_states,
            queue_order: snapshot.queue_order,
            last_sequence: 0, // snapshot does not track event sequence
            recovery_timestamp_ms: now_ms,
        }
    }

    /// Recover orchestrator state by replaying a slice of event-log entries.
    ///
    /// Walks the events in order, updating per-plan state as each event is
    /// processed. The resulting [`RecoveredState`] reflects the cumulative
    /// effect of all events.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError::InvalidEventSequence`] if event sequence
    /// numbers are not contiguous from zero.
    pub fn recover_from_event_log(
        &self,
        events: &[EventEntry],
    ) -> Result<RecoveredState, RecoveryError> {
        let now_ms = current_timestamp_ms();

        if events.is_empty() {
            return Ok(RecoveredState::empty(now_ms));
        }

        validate_event_sequence(events)?;

        let mut plan_states: HashMap<String, PlanPhaseInfo> = HashMap::new();
        let mut queue_order: Vec<String> = Vec::new();
        let mut last_sequence = 0u64;

        for event in events {
            last_sequence = event.sequence_number;
            if let Some(plan_id) = extract_plan_id(&event.payload) {
                let info = plan_states
                    .entry(plan_id.clone())
                    .or_insert_with(|| PlanPhaseInfo {
                        plan_id: plan_id.clone(),
                        phase: PlanPhase::Queued,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: Vec::new(),
                    });

                apply_event_to_plan(event, &plan_id, info, &mut queue_order);
            }
        }

        Ok(RecoveredState {
            plan_states,
            queue_order,
            last_sequence,
            recovery_timestamp_ms: now_ms,
        })
    }

    /// Merge state recovered from a snapshot and an event log.
    ///
    /// When both sources provide state for the same plan, the event-log
    /// version takes precedence (it is more recent by definition, since the
    /// snapshot is a point-in-time capture while the event log is append-only
    /// and may contain events that occurred after the snapshot).
    ///
    /// If only one source is `Some`, its state is used directly. If both are
    /// `None`, an empty state is returned.
    #[must_use]
    pub fn merge_recovery(
        snapshot: Option<RecoveredState>,
        event_log: Option<RecoveredState>,
    ) -> RecoveredState {
        match (snapshot, event_log) {
            (None, None) => RecoveredState::empty(current_timestamp_ms()),
            (Some(s), None) => s,
            (None, Some(e)) => e,
            (Some(snap), Some(log)) => {
                let mut merged_plans = snap.plan_states;

                // Event-log state overwrites snapshot state for any plan
                // present in both.
                for (id, log_info) in log.plan_states {
                    merged_plans.insert(id, log_info);
                }

                // Queue order: prefer event log if non-empty, otherwise snapshot.
                let queue_order = if log.queue_order.is_empty() {
                    snap.queue_order
                } else {
                    log.queue_order
                };

                // last_sequence: take the higher of the two.
                let last_sequence = std::cmp::max(snap.last_sequence, log.last_sequence);

                RecoveredState {
                    plan_states: merged_plans,
                    queue_order,
                    last_sequence,
                    recovery_timestamp_ms: current_timestamp_ms(),
                }
            }
        }
    }

    /// Validate a recovered state for internal inconsistencies.
    ///
    /// Returns a (possibly empty) list of warnings. An empty list means the
    /// recovered state looks consistent.
    #[must_use]
    pub fn validate_recovery(state: &RecoveredState) -> Vec<RecoveryWarning> {
        let mut warnings = Vec::new();

        // Check 1: plans in queue_order must exist in plan_states.
        for id in &state.queue_order {
            if !state.plan_states.contains_key(id) {
                warnings.push(RecoveryWarning {
                    plan_id: id.clone(),
                    message: "plan in queue_order but missing from plan_states".into(),
                    severity: WarningSeverity::Critical,
                });
            }
        }

        // Check 2: plans in plan_states but not in queue_order (orphans).
        for id in state.plan_states.keys() {
            if !state.queue_order.contains(id) {
                warnings.push(RecoveryWarning {
                    plan_id: id.clone(),
                    message: "plan in plan_states but missing from queue_order".into(),
                    severity: WarningSeverity::Warning,
                });
            }
        }

        // Check 3: iteration must be >= 1.
        for info in state.plan_states.values() {
            if info.iteration == 0 {
                warnings.push(RecoveryWarning {
                    plan_id: info.plan_id.clone(),
                    message: "iteration is 0 (should be >= 1)".into(),
                    severity: WarningSeverity::Warning,
                });
            }
        }

        // Check 4: terminal plans should not have empty files_changed
        // unless they were skipped (this is informational, not critical).
        for info in state.plan_states.values() {
            if info.phase.kind() == PhaseKind::Complete && info.files_changed.is_empty() {
                warnings.push(RecoveryWarning {
                    plan_id: info.plan_id.clone(),
                    message: "completed plan has no files_changed recorded".into(),
                    severity: WarningSeverity::Info,
                });
            }
        }

        // Check 5: duplicate entries in queue_order.
        let mut seen = std::collections::HashSet::new();
        for id in &state.queue_order {
            if !seen.insert(id.clone()) {
                warnings.push(RecoveryWarning {
                    plan_id: id.clone(),
                    message: "duplicate entry in queue_order".into(),
                    severity: WarningSeverity::Critical,
                });
            }
        }

        warnings
    }

    /// Convenience wrapper for [`RecoveredState::resume_plan`].
    #[must_use]
    pub fn build_resume_plan(state: &RecoveredState, now_ms: u64) -> RecoveryResumePlan {
        state.resume_plan(now_ms)
    }
}

impl Default for RecoveryEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────

/// Apply a single event to a plan's recovery state.
#[allow(clippy::match_same_arms)]
fn apply_event_to_plan(
    event: &EventEntry,
    plan_id: &str,
    info: &mut PlanPhaseInfo,
    queue_order: &mut Vec<String>,
) {
    match &event.event_kind {
        EventKind::PlanStarted => {
            if !queue_order.contains(&plan_id.to_owned()) {
                queue_order.push(plan_id.to_owned());
            }
            info.phase = PlanPhase::Enriching;
        }
        EventKind::PhaseTransition => {
            if let Some(phase) = extract_phase(&event.payload) {
                info.phase = phase;
            }
        }
        EventKind::TaskAssigned | EventKind::AgentSpawned => {
            if let Some(arr) = event
                .payload
                .get("files")
                .and_then(serde_json::Value::as_array)
            {
                for f in arr {
                    if let Some(s) = f.as_str() {
                        if !info.files_changed.contains(&s.to_owned()) {
                            info.files_changed.push(s.to_owned());
                        }
                    }
                }
            }
        }
        EventKind::GateResult => {
            apply_gate_result(event, info);
        }
        EventKind::PlanCompleted => {
            info.phase = PlanPhase::Complete;
        }
        EventKind::PlanFailed => {
            let reason = event
                .payload
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            info.phase = PlanPhase::Failed {
                reason: roko_core::FailureKind::Other(reason),
            };
        }
        EventKind::MergeAttempted
            if info.phase.kind() != PhaseKind::Complete
                && info.phase.kind() != PhaseKind::Failed =>
        {
            info.phase = PlanPhase::Merging;
        }
        // All other event kinds (current and future) are no-ops for recovery.
        _ => {}
    }

    // Track iteration bumps from payload.
    if let Some(iter) = event
        .payload
        .get("iteration")
        .and_then(serde_json::Value::as_u64)
    {
        let iter32 = u32::try_from(iter).unwrap_or(u32::MAX);
        if iter32 > info.iteration {
            info.iteration = iter32;
        }
    }
}

/// Apply a gate result event to a plan's recovery state.
fn apply_gate_result(event: &EventEntry, info: &mut PlanPhaseInfo) {
    let gate_name = event
        .payload
        .get("gate")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let passed = event
        .payload
        .get("passed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let summary = event
        .payload
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let status = if passed { "passed" } else { "failed" };
    let suffix = if summary.is_empty() {
        String::new()
    } else {
        format!(" - {summary}")
    };
    info.last_gate_result = Some(format!("{gate_name}: {status}{suffix}"));
}

/// Extract a `plan_id` from an event's JSON payload.
fn extract_plan_id(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("plan_id")
        .or_else(|| payload.get("plan"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

/// Extract a [`PlanPhase`] from an event's JSON payload (for `PhaseTransition` events).
fn extract_phase(payload: &serde_json::Value) -> Option<PlanPhase> {
    payload.get("phase").and_then(|v| {
        // Try deserializing the phase sub-object.
        serde_json::from_value(v.clone()).ok()
    })
}

/// Current wall-clock time in milliseconds since Unix epoch.
fn current_timestamp_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

fn validate_event_sequence(events: &[EventEntry]) -> Result<(), RecoveryError> {
    for (expected, event) in events.iter().enumerate() {
        let expected = expected as u64;
        if event.sequence_number != expected {
            let message = if expected == 0 {
                format!(
                    "sequence {} starts event log (expected 0)",
                    event.sequence_number
                )
            } else {
                format!(
                    "sequence {} follows {} (expected {})",
                    event.sequence_number,
                    expected - 1,
                    expected
                )
            };
            return Err(RecoveryError::InvalidEventSequence(message));
        }
    }

    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::event_log::{EventEntry, EventKind};
    use roko_core::PlanPhase;
    use serde_json::json;

    /// Helper: build a minimal EventEntry for tests.
    fn make_event(seq: u64, kind: EventKind, payload: serde_json::Value) -> EventEntry {
        EventEntry {
            sequence_number: seq,
            timestamp_ms: 1_000_000 + (seq as i64 * 1000),
            event_kind: kind,
            payload,
            content_hash: [0u8; 32], // not validated in recovery
        }
    }

    // ── 1. Snapshot recovery (basic) ────────────────────────────────────

    #[test]
    fn recovery_snapshot_basic() {
        let engine = RecoveryEngine::new();

        let snap_json = json!({
            "plan_states": {
                "plan-1": {
                    "plan_id": "plan-1",
                    "current_phase": {"kind": "implementing"},
                    "assigned_agents": ["impl-1"],
                    "gate_results": [],
                    "iteration": 2,
                    "started_at_ms": 100,
                    "files_changed": ["src/lib.rs"],
                    "merge_attempts": 0,
                    "last_error": null,
                    "paused": false,
                    "priority": 0
                }
            },
            "queue_order": ["plan-1"],
            "timestamp_ms": 5000
        });

        let state = engine
            .recover_from_snapshot(&snap_json.to_string())
            .unwrap();

        assert_eq!(state.plan_states.len(), 1);
        let info = &state.plan_states["plan-1"];
        assert_eq!(info.plan_id, "plan-1");
        assert_eq!(info.phase, PlanPhase::Implementing);
        assert_eq!(info.iteration, 2);
        assert_eq!(info.files_changed, vec!["src/lib.rs"]);
        assert_eq!(state.queue_order, vec!["plan-1"]);
    }

    // ── 2. Snapshot recovery (corrupted) ────────────────────────────────

    #[test]
    fn recovery_snapshot_corrupted() {
        let engine = RecoveryEngine::new();
        let result = engine.recover_from_snapshot("not valid json {{{");
        assert!(result.is_err());
        match result.unwrap_err() {
            RecoveryError::CorruptedSnapshot(msg) => {
                assert!(msg.contains("JSON parse error"));
            }
            other => panic!("expected CorruptedSnapshot, got {other:?}"),
        }
    }

    // ── 3. Snapshot preserves gate results ──────────────────────────────

    #[test]
    fn recovery_snapshot_preserves_gate_result() {
        let engine = RecoveryEngine::new();

        let snap_json = json!({
            "plan_states": {
                "plan-g": {
                    "plan_id": "plan-g",
                    "current_phase": {"kind": "gating"},
                    "assigned_agents": [],
                    "gate_results": [
                        {
                            "gate_name": "compile",
                            "rung": 0,
                            "passed": true,
                            "summary": "all good",
                            "duration_ms": 1200
                        },
                        {
                            "gate_name": "test",
                            "rung": 1,
                            "passed": false,
                            "summary": "3 failures",
                            "duration_ms": 5000
                        }
                    ],
                    "iteration": 1,
                    "started_at_ms": 0,
                    "files_changed": [],
                    "merge_attempts": 0,
                    "last_error": null,
                    "paused": false,
                    "priority": 5
                }
            },
            "queue_order": ["plan-g"],
            "timestamp_ms": 9000
        });

        let state = engine
            .recover_from_snapshot(&snap_json.to_string())
            .unwrap();
        let info = &state.plan_states["plan-g"];
        let gate_str = info.last_gate_result.as_ref().unwrap();
        assert!(gate_str.contains("test"));
        assert!(gate_str.contains("3 failures"));
    }

    #[test]
    fn recovery_snapshot_uses_hardened_parser_path() {
        let engine = RecoveryEngine::new();
        let legacy_json = json!({
            "tasks": [
                {"plan_id": "legacy-a", "status": "completed"},
                {"plan_id": "legacy-a", "status": "completed"},
                {"plan_id": "legacy-b", "status": "running"}
            ],
            "queue_order": ["legacy-b", "legacy-a"],
            "timestamp_ms": 42
        });

        let state = engine
            .recover_from_snapshot(&legacy_json.to_string())
            .unwrap();

        assert_eq!(state.queue_order, vec!["legacy-b", "legacy-a"]);
        assert_eq!(state.plan_states["legacy-a"].phase, PlanPhase::Complete);
        assert_eq!(state.plan_states["legacy-b"].phase, PlanPhase::Implementing);
    }

    // ── 4. Event-log recovery (basic) ───────────────────────────────────

    #[test]
    fn recovery_event_log_basic() {
        let engine = RecoveryEngine::new();

        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "p1"})),
            make_event(
                1,
                EventKind::PhaseTransition,
                json!({"plan_id": "p1", "phase": {"kind": "implementing"}}),
            ),
            make_event(
                2,
                EventKind::GateResult,
                json!({"plan_id": "p1", "gate": "compile", "passed": true, "summary": "ok"}),
            ),
            make_event(3, EventKind::PlanCompleted, json!({"plan_id": "p1"})),
        ];

        let state = engine.recover_from_event_log(&events).unwrap();
        assert_eq!(state.plan_states.len(), 1);
        let info = &state.plan_states["p1"];
        assert_eq!(info.phase, PlanPhase::Complete);
        assert_eq!(state.queue_order, vec!["p1"]);
        assert_eq!(state.last_sequence, 3);

        let gate_str = info.last_gate_result.as_ref().unwrap();
        assert!(gate_str.contains("compile"));
        assert!(gate_str.contains("passed"));
    }

    // ── 5. Event-log recovery (invalid sequence) ────────────────────────

    #[test]
    fn recovery_event_log_invalid_sequence() {
        let engine = RecoveryEngine::new();

        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "p1"})),
            make_event(5, EventKind::PlanCompleted, json!({"plan_id": "p1"})),
            make_event(3, EventKind::PlanStarted, json!({"plan_id": "p2"})),
        ];

        let result = engine.recover_from_event_log(&events);
        assert!(result.is_err());
        match result.unwrap_err() {
            RecoveryError::InvalidEventSequence(msg) => {
                assert!(msg.contains("0"));
                assert!(msg.contains("5"));
                assert!(msg.contains("expected 1"));
            }
            other => panic!("expected InvalidEventSequence, got {other:?}"),
        }
    }

    #[test]
    fn recovery_event_log_rejects_sequence_gap() {
        let engine = RecoveryEngine::new();
        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "p1"})),
            make_event(2, EventKind::PlanCompleted, json!({"plan_id": "p1"})),
        ];

        let result = engine.recover_from_event_log(&events);
        assert!(result.is_err());
        match result.unwrap_err() {
            RecoveryError::InvalidEventSequence(msg) => {
                assert!(msg.contains("2"));
                assert!(msg.contains("expected 1"));
            }
            other => panic!("expected InvalidEventSequence, got {other:?}"),
        }
    }

    // ── 6. Event-log recovery (plan failed + iteration tracking) ────────

    #[test]
    fn recovery_event_log_plan_failed_and_iterations() {
        let engine = RecoveryEngine::new();

        let events = vec![
            make_event(
                0,
                EventKind::PlanStarted,
                json!({"plan_id": "pf", "iteration": 1}),
            ),
            make_event(
                1,
                EventKind::GateResult,
                json!({"plan_id": "pf", "gate": "test", "passed": false, "iteration": 1}),
            ),
            make_event(
                2,
                EventKind::PhaseTransition,
                json!({"plan_id": "pf", "phase": {"kind": "auto-fixing"}, "iteration": 2}),
            ),
            make_event(
                3,
                EventKind::PlanFailed,
                json!({"plan_id": "pf", "reason": "compilation errors", "iteration": 3}),
            ),
        ];

        let state = engine.recover_from_event_log(&events).unwrap();
        let info = &state.plan_states["pf"];
        match &info.phase {
            PlanPhase::Failed { reason } => {
                assert_eq!(reason.to_string(), "compilation errors");
            }
            other => panic!("expected Failed phase, got {other:?}"),
        }
        assert_eq!(info.iteration, 3);
    }

    // ── 7. Merge: event log takes precedence ────────────────────────────

    #[test]
    fn recovery_merge_event_log_precedence() {
        let snap_state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "p1".into(),
                    PlanPhaseInfo {
                        plan_id: "p1".into(),
                        phase: PlanPhase::Implementing,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: vec!["old.rs".into()],
                    },
                );
                m
            },
            queue_order: vec!["p1".into()],
            last_sequence: 0,
            recovery_timestamp_ms: 100,
        };

        let log_state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "p1".into(),
                    PlanPhaseInfo {
                        plan_id: "p1".into(),
                        phase: PlanPhase::Gating,
                        iteration: 2,
                        last_gate_result: Some("compile: passed".into()),
                        files_changed: vec!["new.rs".into()],
                    },
                );
                m
            },
            queue_order: vec!["p1".into()],
            last_sequence: 10,
            recovery_timestamp_ms: 200,
        };

        let merged = RecoveryEngine::merge_recovery(Some(snap_state), Some(log_state));

        let info = &merged.plan_states["p1"];
        assert_eq!(info.phase, PlanPhase::Gating);
        assert_eq!(info.iteration, 2);
        assert_eq!(info.files_changed, vec!["new.rs"]);
        assert_eq!(merged.last_sequence, 10);
    }

    // ── 8. Merge: combines disjoint plans from both sources ─────────────

    #[test]
    fn recovery_merge_combines_disjoint() {
        let snap_state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "snap-only".into(),
                    PlanPhaseInfo {
                        plan_id: "snap-only".into(),
                        phase: PlanPhase::Implementing,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: vec![],
                    },
                );
                m
            },
            queue_order: vec!["snap-only".into()],
            last_sequence: 0,
            recovery_timestamp_ms: 100,
        };

        let log_state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "log-only".into(),
                    PlanPhaseInfo {
                        plan_id: "log-only".into(),
                        phase: PlanPhase::Complete,
                        iteration: 3,
                        last_gate_result: Some("test: passed".into()),
                        files_changed: vec!["main.rs".into()],
                    },
                );
                m
            },
            queue_order: vec!["log-only".into()],
            last_sequence: 5,
            recovery_timestamp_ms: 200,
        };

        let merged = RecoveryEngine::merge_recovery(Some(snap_state), Some(log_state));

        assert_eq!(merged.plan_states.len(), 2);
        assert!(merged.plan_states.contains_key("snap-only"));
        assert!(merged.plan_states.contains_key("log-only"));
        assert_eq!(merged.queue_order, vec!["log-only"]);
    }

    // ── 9. Validation: detects queue/state mismatch ─────────────────────

    #[test]
    fn recovery_validate_queue_without_state() {
        let state = RecoveredState {
            plan_states: HashMap::new(),
            queue_order: vec!["ghost-plan".into()],
            last_sequence: 0,
            recovery_timestamp_ms: 0,
        };

        let warnings = RecoveryEngine::validate_recovery(&state);
        assert!(!warnings.is_empty());
        let w = &warnings[0];
        assert_eq!(w.plan_id, "ghost-plan");
        assert_eq!(w.severity, WarningSeverity::Critical);
        assert!(w.message.contains("missing from plan_states"));
    }

    // ── 10. Validation: detects orphan plans ────────────────────────────

    #[test]
    fn recovery_validate_orphan_plans() {
        let state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "orphan".into(),
                    PlanPhaseInfo {
                        plan_id: "orphan".into(),
                        phase: PlanPhase::Implementing,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: vec![],
                    },
                );
                m
            },
            queue_order: vec![],
            last_sequence: 0,
            recovery_timestamp_ms: 0,
        };

        let warnings = RecoveryEngine::validate_recovery(&state);
        assert!(warnings.iter().any(|w| w.plan_id == "orphan"
            && w.severity == WarningSeverity::Warning
            && w.message.contains("missing from queue_order")));
    }

    // ── 11. Validation: consistent state has no warnings ────────────────

    #[test]
    fn recovery_validate_consistent_state() {
        let state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "good".into(),
                    PlanPhaseInfo {
                        plan_id: "good".into(),
                        phase: PlanPhase::Implementing,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: vec!["lib.rs".into()],
                    },
                );
                m
            },
            queue_order: vec!["good".into()],
            last_sequence: 5,
            recovery_timestamp_ms: 1000,
        };

        let warnings = RecoveryEngine::validate_recovery(&state);
        assert!(
            warnings.is_empty(),
            "expected no warnings, got {warnings:?}"
        );
    }

    // ── 12. End-to-end recovery pipeline ────────────────────────────────

    #[test]
    fn recovery_end_to_end_pipeline() {
        let engine = RecoveryEngine::new();

        // Snapshot: plan-A at Implementing, plan-B at Queued.
        let snap_json = json!({
            "plan_states": {
                "plan-A": {
                    "plan_id": "plan-A",
                    "current_phase": {"kind": "implementing"},
                    "assigned_agents": [],
                    "gate_results": [],
                    "iteration": 1,
                    "started_at_ms": 0,
                    "files_changed": ["a.rs"],
                    "merge_attempts": 0,
                    "last_error": null,
                    "paused": false,
                    "priority": 0
                },
                "plan-B": {
                    "plan_id": "plan-B",
                    "current_phase": {"kind": "queued"},
                    "assigned_agents": [],
                    "gate_results": [],
                    "iteration": 1,
                    "started_at_ms": 0,
                    "files_changed": [],
                    "merge_attempts": 0,
                    "last_error": null,
                    "paused": false,
                    "priority": 0
                }
            },
            "queue_order": ["plan-A", "plan-B"],
            "timestamp_ms": 1000
        });

        // Event log: plan-A completed, plan-B moved to Gating.
        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "plan-A"})),
            make_event(1, EventKind::PlanCompleted, json!({"plan_id": "plan-A"})),
            make_event(2, EventKind::PlanStarted, json!({"plan_id": "plan-B"})),
            make_event(
                3,
                EventKind::PhaseTransition,
                json!({"plan_id": "plan-B", "phase": {"kind": "gating"}}),
            ),
        ];

        let snap_state = engine
            .recover_from_snapshot(&snap_json.to_string())
            .unwrap();
        let log_state = engine.recover_from_event_log(&events).unwrap();
        let merged = RecoveryEngine::merge_recovery(Some(snap_state), Some(log_state));

        // plan-A: event log Complete overrides snapshot Implementing.
        assert_eq!(merged.plan_states["plan-A"].phase, PlanPhase::Complete);
        // plan-B: event log Gating overrides snapshot Queued.
        assert_eq!(merged.plan_states["plan-B"].phase, PlanPhase::Gating);

        let warnings = RecoveryEngine::validate_recovery(&merged);
        // plan-A is Complete with no files_changed in event-log version.
        let completed_no_files = warnings
            .iter()
            .any(|w| w.plan_id == "plan-A" && w.severity == WarningSeverity::Info);
        assert!(
            completed_no_files,
            "expected info warning for completed plan with no files; got {warnings:?}"
        );
    }

    // ── 13. Event log tracks files from agent events ────────────────────

    #[test]
    fn recovery_event_log_tracks_files() {
        let engine = RecoveryEngine::new();

        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "pf"})),
            make_event(
                1,
                EventKind::AgentSpawned,
                json!({"plan_id": "pf", "files": ["src/main.rs", "src/lib.rs"]}),
            ),
            make_event(
                2,
                EventKind::TaskAssigned,
                json!({"plan_id": "pf", "files": ["src/lib.rs", "tests/test.rs"]}),
            ),
        ];

        let state = engine.recover_from_event_log(&events).unwrap();
        let info = &state.plan_states["pf"];
        assert_eq!(info.files_changed.len(), 3);
        assert!(info.files_changed.contains(&"src/main.rs".to_owned()));
        assert!(info.files_changed.contains(&"src/lib.rs".to_owned()));
        assert!(info.files_changed.contains(&"tests/test.rs".to_owned()));
    }

    // ── 14. Multi-plan event log recovery ───────────────────────────────

    #[test]
    fn recovery_event_log_multi_plan() {
        let engine = RecoveryEngine::new();

        let events = vec![
            make_event(0, EventKind::PlanStarted, json!({"plan_id": "alpha"})),
            make_event(1, EventKind::PlanStarted, json!({"plan_id": "beta"})),
            make_event(
                2,
                EventKind::PhaseTransition,
                json!({"plan_id": "alpha", "phase": {"kind": "gating"}}),
            ),
            make_event(3, EventKind::PlanCompleted, json!({"plan_id": "beta"})),
            make_event(
                4,
                EventKind::PlanFailed,
                json!({"plan_id": "alpha", "reason": "test failures"}),
            ),
        ];

        let state = engine.recover_from_event_log(&events).unwrap();
        assert_eq!(state.plan_states.len(), 2);

        match &state.plan_states["alpha"].phase {
            PlanPhase::Failed { .. } => {}
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(state.plan_states["beta"].phase, PlanPhase::Complete);
        assert_eq!(state.queue_order, vec!["alpha", "beta"]);
        assert_eq!(state.last_sequence, 4);
    }

    // ── 15. Error type display ──────────────────────────────────────────

    #[test]
    fn recovery_error_display() {
        let e = RecoveryError::CorruptedSnapshot("bad json".into());
        assert!(e.to_string().contains("corrupted snapshot"));
        assert!(e.to_string().contains("bad json"));

        let e = RecoveryError::InvalidEventSequence("seq 3 before 5".into());
        assert!(e.to_string().contains("invalid event sequence"));

        let e = RecoveryError::MissingPlanState("plan-x".into());
        assert!(e.to_string().contains("missing plan state"));
    }

    // ── 16. Validate detects duplicate queue entries ────────────────────

    #[test]
    fn recovery_validate_duplicate_queue() {
        let state = RecoveredState {
            plan_states: {
                let mut m = HashMap::new();
                m.insert(
                    "dup".into(),
                    PlanPhaseInfo {
                        plan_id: "dup".into(),
                        phase: PlanPhase::Queued,
                        iteration: 1,
                        last_gate_result: None,
                        files_changed: vec![],
                    },
                );
                m
            },
            queue_order: vec!["dup".into(), "dup".into()],
            last_sequence: 0,
            recovery_timestamp_ms: 0,
        };

        let warnings = RecoveryEngine::validate_recovery(&state);
        assert!(warnings.iter().any(|w| w.plan_id == "dup"
            && w.severity == WarningSeverity::Critical
            && w.message.contains("duplicate")));
    }
}
