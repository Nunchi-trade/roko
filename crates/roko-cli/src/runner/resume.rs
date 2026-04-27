//! Strict resume validation.
//!
//! ## What "strict" means
//!
//! Plan-id overlap is not enough. Two runs that share plan ids but have
//! divergent task definitions cannot be safely resumed: the executor
//! would skip "completed" tasks whose contents have changed since they
//! ran. This module computes a [`TaskDefFingerprint`] for every task in
//! the current plan set and compares it against the fingerprint stored
//! in the previous run-state snapshot. Any mismatch is a hard
//! [`ResumeError::TaskMismatch`].
//!
//! ## Failure mode
//!
//! When validation fails the caller should refuse to resume and either
//! discard the snapshot (clean restart) or alert the operator. The
//! validator never silently "fixes" the state.
//!
//! ## Recovery integration
//!
//! [`prepare_resume`] additionally invokes
//! [`super::persist::recover_jsonl`] on `episodes.jsonl` and
//! `events.jsonl` so partial-append corruption from a crashed prior run
//! is repaired before the new run begins appending to the same files.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::persist::{
    JsonlRecovery, PersistPaths, RUN_STATE_SCHEMA_VERSION, RunStateSnapshot, TaskDefFingerprint,
    load_run_state, recover_jsonl,
};
use crate::task_parser::TaskDef;

/// Outcome of [`prepare_resume`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeReport {
    /// `true` if a previous run-state snapshot was loaded.
    pub resumed: bool,
    /// Run id of the resumed snapshot (if any).
    pub prior_run_id: Option<String>,
    /// JSONL recovery outcomes per logged file. Reports only the files
    /// the validator inspected.
    pub recovered_files: Vec<RecoveredFile>,
    /// Number of fingerprints compared against the snapshot; useful for
    /// observability when the snapshot is empty.
    pub validated_tasks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecoveredFile {
    pub path: String,
    pub recovery: JsonlRecoveryReport,
}

/// Public-facing snapshot of [`JsonlRecovery`] for serialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JsonlRecoveryReport {
    Clean { lines: usize },
    TruncatedTrailing { valid_lines: usize, truncated_bytes: u64 },
    DroppedInvalid { valid_lines: usize, dropped_lines: usize },
}

impl From<JsonlRecovery> for JsonlRecoveryReport {
    fn from(value: JsonlRecovery) -> Self {
        match value {
            JsonlRecovery::Clean { lines } => Self::Clean { lines },
            JsonlRecovery::TruncatedTrailing {
                valid_lines,
                truncated_bytes,
            } => Self::TruncatedTrailing {
                valid_lines,
                truncated_bytes,
            },
            JsonlRecovery::DroppedInvalid {
                valid_lines,
                dropped_lines,
            } => Self::DroppedInvalid {
                valid_lines,
                dropped_lines,
            },
        }
    }
}

/// Error variants returned when a resume cannot proceed safely.
#[derive(Debug, Error)]
pub enum ResumeError {
    /// The run-state snapshot's schema version is newer than the
    /// runner's. Migration is the operator's responsibility.
    #[error("run-state snapshot schema version {found} is newer than runner version {expected}")]
    UnsupportedSchema { expected: u32, found: u32 },
    /// One or more tasks present in the current plan set have a
    /// fingerprint different from what the snapshot recorded.
    #[error("{} task(s) drifted since the last run", mismatches.len())]
    TaskMismatch { mismatches: Vec<TaskMismatch> },
    /// Plan present in snapshot but missing from the current run.
    #[error("plan `{plan_id}` is in snapshot but not in the current run")]
    PlanMissing { plan_id: String },
    /// Filesystem / parser failure surfacing as anyhow.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskMismatch {
    pub plan_id: String,
    pub task_id: String,
    pub expected_fingerprint: String,
    pub actual_fingerprint: String,
}

/// Strict resume validator + JSONL recovery driver.
///
/// `prepare_resume` is the single entrypoint runners call before
/// re-opening any persistent state file: it loads the prior snapshot,
/// validates every task against the snapshot's fingerprints, and runs
/// JSONL recovery on the durable logs.
pub fn prepare_resume(
    paths: &PersistPaths,
    plans: &HashMap<String, Vec<TaskDef>>,
    snapshot_fingerprints: &[TaskDefFingerprint],
) -> Result<ResumeReport, ResumeError> {
    let snapshot = load_run_state(paths)?;
    let mut report = ResumeReport {
        resumed: snapshot.is_some(),
        prior_run_id: snapshot.as_ref().map(|s| s.run_id.clone()),
        recovered_files: Vec::new(),
        validated_tasks: 0,
    };

    if let Some(prior) = snapshot.as_ref() {
        if prior.schema_version > RUN_STATE_SCHEMA_VERSION {
            return Err(ResumeError::UnsupportedSchema {
                expected: RUN_STATE_SCHEMA_VERSION,
                found: prior.schema_version,
            });
        }

        // Strict validation against the snapshot fingerprints.
        let snapshot_index: HashMap<(&str, &str), &TaskDefFingerprint> = snapshot_fingerprints
            .iter()
            .map(|fp| ((fp.plan_id.as_str(), fp.task_id.as_str()), fp))
            .collect();

        let mut mismatches = Vec::new();
        for (plan_id, tasks) in plans {
            for task in tasks {
                let actual = TaskDefFingerprint::from_task(task, plan_id);
                let key = (plan_id.as_str(), task.id.as_str());
                let Some(expected) = snapshot_index.get(&key) else {
                    // Snapshot does not record this task — it's new.
                    // That's allowed.
                    continue;
                };
                if expected.fingerprint != actual.fingerprint {
                    mismatches.push(TaskMismatch {
                        plan_id: plan_id.clone(),
                        task_id: task.id.clone(),
                        expected_fingerprint: expected.fingerprint.clone(),
                        actual_fingerprint: actual.fingerprint,
                    });
                }
                report.validated_tasks += 1;
            }
        }

        // Plans present in snapshot but missing from current run.
        let current_plans: std::collections::HashSet<&str> =
            plans.keys().map(String::as_str).collect();
        for fp in snapshot_fingerprints {
            if !current_plans.contains(fp.plan_id.as_str()) {
                return Err(ResumeError::PlanMissing {
                    plan_id: fp.plan_id.clone(),
                });
            }
        }

        if !mismatches.is_empty() {
            return Err(ResumeError::TaskMismatch { mismatches });
        }
    }

    // Run JSONL recovery on the two append-only logs the runner writes
    // to, regardless of whether we are resuming. Crash recovery before
    // the first append is the same code path either way.
    for (label, path) in [
        ("episodes", &paths.episodes_jsonl),
        ("events", &paths.events_jsonl),
        ("efficiency", &paths.efficiency_jsonl),
    ] {
        let recovery = recover_jsonl(path, |line: &str| {
            serde_json::from_str::<serde_json::Value>(line)
        })
        .map_err(ResumeError::Other)?;
        report.recovered_files.push(RecoveredFile {
            path: format!("{label}: {}", path.display()),
            recovery: recovery.into(),
        });
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_parser::{TaskDef, VerifyStep};
    use std::collections::HashMap;
    use std::fs;
    use tempfile::tempdir;

    fn task(id: &str, title: &str) -> TaskDef {
        TaskDef {
            id: id.into(),
            title: title.into(),
            description: None,
            role: Some("implementer".into()),
            status: "ready".into(),
            tier: "focused".into(),
            frequency: None,
            model_hint: None,
            replan_strategy: None,
            max_loc: None,
            files: vec![],
            allowed_tools: None,
            denied_tools: None,
            mcp_servers: None,
            depends_on: vec![],
            depends_on_plan: vec![],
            split_into: None,
            context: None,
            verify: vec![VerifyStep {
                phase: "compile".into(),
                command: "cargo check".into(),
                fail_msg: None,
                timeout_ms: 60_000,
            }],
            timeout_secs: 60,
            max_retries: 1,
            acceptance: vec!["compiles".into()],
            acceptance_contract: None,
            domain: None,
        }
    }

    fn paths_for(workdir: &std::path::Path) -> PersistPaths {
        PersistPaths::from_workdir(workdir).expect("paths")
    }

    fn snapshot_with_run_id(run_id: &str) -> RunStateSnapshot {
        RunStateSnapshot {
            schema_version: RUN_STATE_SCHEMA_VERSION,
            run_id: run_id.into(),
            started_at_ms: 0,
            timestamp_ms: 0,
            tasks_total: 0,
            tasks_completed: 0,
            tasks_failed: 0,
            total_tokens_in: 0,
            total_tokens_out: 0,
            total_cost_usd: 0.0,
            total_agent_calls: 0,
            plan_costs: HashMap::new(),
            completed_tasks: HashMap::new(),
            snapshot_fail_streak: 0,
            fingerprints: Vec::new(),
        }
    }

    #[test]
    fn fresh_workdir_returns_empty_resume_report() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        let plans = HashMap::new();
        let report = prepare_resume(&paths, &plans, &[]).unwrap();
        assert!(!report.resumed);
        assert_eq!(report.validated_tasks, 0);
        // All three logs should be reported as Clean.
        assert_eq!(report.recovered_files.len(), 3);
        for f in &report.recovered_files {
            matches!(f.recovery, JsonlRecoveryReport::Clean { lines: 0 });
        }
    }

    #[test]
    fn matching_fingerprints_validate_clean() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        let snap = snapshot_with_run_id("prior");
        super::super::persist::save_run_state(&paths, &snap).unwrap();
        let t = task("a", "Alpha");
        let fp = TaskDefFingerprint::from_task(&t, "p1");
        let mut plans = HashMap::new();
        plans.insert("p1".to_string(), vec![t]);
        let report = prepare_resume(&paths, &plans, &[fp]).unwrap();
        assert!(report.resumed);
        assert_eq!(report.prior_run_id.as_deref(), Some("prior"));
        assert_eq!(report.validated_tasks, 1);
    }

    #[test]
    fn changed_task_definition_is_a_strict_failure() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        let snap = snapshot_with_run_id("prior");
        super::super::persist::save_run_state(&paths, &snap).unwrap();
        let original = task("a", "Alpha original");
        let changed = task("a", "Alpha mutated");
        let fp_original = TaskDefFingerprint::from_task(&original, "p1");
        let mut plans = HashMap::new();
        plans.insert("p1".to_string(), vec![changed]);
        let err = prepare_resume(&paths, &plans, &[fp_original]).unwrap_err();
        match err {
            ResumeError::TaskMismatch { mismatches } => {
                assert_eq!(mismatches.len(), 1);
                assert_eq!(mismatches[0].task_id, "a");
            }
            other => panic!("expected TaskMismatch, got {other:?}"),
        }
    }

    #[test]
    fn missing_plan_in_current_run_is_an_error() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        let snap = snapshot_with_run_id("prior");
        super::super::persist::save_run_state(&paths, &snap).unwrap();
        let t = task("a", "A");
        let fp = TaskDefFingerprint::from_task(&t, "p1");
        let plans = HashMap::new(); // p1 not present
        let err = prepare_resume(&paths, &plans, &[fp]).unwrap_err();
        assert!(matches!(err, ResumeError::PlanMissing { .. }));
    }

    #[test]
    fn unsupported_future_schema_version_rejects_resume() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        let mut snap = snapshot_with_run_id("future");
        snap.schema_version = RUN_STATE_SCHEMA_VERSION + 100;
        super::super::persist::save_run_state(&paths, &snap).unwrap();
        let err = prepare_resume(&paths, &HashMap::new(), &[]).unwrap_err();
        assert!(matches!(err, ResumeError::UnsupportedSchema { .. }));
    }

    #[test]
    fn jsonl_recovery_truncates_partial_trailing_line() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        // Pre-populate events.jsonl with a clean line + a half-written line.
        let valid = "{\"ok\":true}\n";
        let partial = "{\"oops\": "; // no newline, mid-write
        fs::write(&paths.events_jsonl, format!("{valid}{partial}")).unwrap();
        let report = prepare_resume(&paths, &HashMap::new(), &[]).unwrap();
        let events_recovery = report
            .recovered_files
            .iter()
            .find(|f| f.path.starts_with("events: "))
            .unwrap();
        match &events_recovery.recovery {
            JsonlRecoveryReport::TruncatedTrailing {
                valid_lines,
                truncated_bytes,
            } => {
                assert_eq!(*valid_lines, 1);
                assert_eq!(*truncated_bytes, partial.len() as u64);
            }
            other => panic!("expected TruncatedTrailing, got {other:?}"),
        }
        // File on disk should now be just the valid line.
        let content = fs::read_to_string(&paths.events_jsonl).unwrap();
        assert_eq!(content, valid);
    }

    #[test]
    fn jsonl_recovery_drops_invalid_tail_line() {
        let dir = tempdir().unwrap();
        let paths = paths_for(dir.path());
        // Episodes.jsonl: one valid line, then a complete-but-malformed line.
        let valid = "{\"ok\":true}\n";
        let malformed = "this is not json\n";
        fs::write(&paths.episodes_jsonl, format!("{valid}{malformed}")).unwrap();
        let report = prepare_resume(&paths, &HashMap::new(), &[]).unwrap();
        let ep = report
            .recovered_files
            .iter()
            .find(|f| f.path.starts_with("episodes: "))
            .unwrap();
        match &ep.recovery {
            JsonlRecoveryReport::DroppedInvalid {
                valid_lines,
                dropped_lines,
            } => {
                assert_eq!(*valid_lines, 1);
                assert_eq!(*dropped_lines, 1);
            }
            other => panic!("expected DroppedInvalid, got {other:?}"),
        }
        let content = fs::read_to_string(&paths.episodes_jsonl).unwrap();
        assert_eq!(content, valid);
    }
}
