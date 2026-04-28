//! End-to-end test for the strict resume cycle.
//!
//! Drives the runner-side persistence + resume helpers directly:
//!
//! 1. `save_run_state` with a populated `RunStateSnapshot` (including
//!    fingerprints of two tasks).
//! 2. Append a clean line + a partial line to `events.jsonl` to simulate
//!    a crash mid-write.
//! 3. Call `prepare_resume` against the same plan set.
//! 4. Assert: snapshot loaded, fingerprints validated, partial line
//!    truncated.
//!
//! Then drift the second task and re-run `prepare_resume` to assert the
//! validator hard-fails with `TaskMismatch`.

use std::collections::HashMap;
use std::fs;

use roko_cli::runner::persist::{
    JsonlRecovery, PersistPaths, RUN_STATE_SCHEMA_VERSION, RunStateSnapshot, TaskDefFingerprint,
    save_run_state,
};
use roko_cli::runner::resume::{JsonlRecoveryReport, ResumeError, prepare_resume};
use roko_cli::task_parser::{TaskDef, VerifyStep};
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

fn baseline_snapshot(run_id: &str, fingerprints: Vec<TaskDefFingerprint>) -> RunStateSnapshot {
    RunStateSnapshot {
        schema_version: RUN_STATE_SCHEMA_VERSION,
        run_id: run_id.into(),
        started_at_ms: 0,
        timestamp_ms: 0,
        tasks_total: 2,
        tasks_completed: 1,
        tasks_failed: 0,
        total_tokens_in: 100,
        total_tokens_out: 50,
        total_cost_usd: 0.05,
        total_agent_calls: 1,
        plan_costs: HashMap::new(),
        completed_tasks: HashMap::from([("p1".to_string(), vec!["a".to_string()])]),
        snapshot_fail_streak: 0,
        fingerprints,
    }
}

#[test]
fn full_resume_cycle_validates_fingerprints_and_recovers_jsonl() {
    let dir = tempdir().expect("tempdir");
    let paths = PersistPaths::from_workdir(dir.path()).expect("paths");

    let task_a = task("a", "Alpha");
    let task_b = task("b", "Beta");
    let fp_a = TaskDefFingerprint::from_task(&task_a, "p1");
    let fp_b = TaskDefFingerprint::from_task(&task_b, "p1");

    // Persist the snapshot (this is what save_snapshot writes during a
    // real run).
    save_run_state(
        &paths,
        &baseline_snapshot("prior-run", vec![fp_a.clone(), fp_b.clone()]),
    )
    .unwrap();

    // Simulate a crash: events.jsonl ends with a partial line.
    let valid_line = "{\"type\":\"plan.started\",\"plan_id\":\"p1\"}\n";
    let partial_line = "{\"type\":\"plan.compl"; // missing closing + newline
    fs::write(&paths.events_jsonl, format!("{valid_line}{partial_line}")).unwrap();

    // Call prepare_resume against the same plan set. Validator should
    // succeed and the partial line should be truncated.
    let mut plans = HashMap::new();
    plans.insert("p1".to_string(), vec![task_a.clone(), task_b.clone()]);
    let report = prepare_resume(&paths, &plans, &[fp_a.clone(), fp_b.clone()]).expect("validates");

    assert!(report.resumed);
    assert_eq!(report.prior_run_id.as_deref(), Some("prior-run"));
    assert_eq!(report.validated_tasks, 2);

    // events.jsonl recovery should show TruncatedTrailing.
    let events_recovery = report
        .recovered_files
        .iter()
        .find(|f| f.path.starts_with("events: "))
        .expect("events log reported");
    match &events_recovery.recovery {
        JsonlRecoveryReport::TruncatedTrailing {
            valid_lines,
            truncated_bytes,
        } => {
            assert_eq!(*valid_lines, 1);
            assert_eq!(*truncated_bytes, partial_line.len() as u64);
        }
        other => panic!("expected TruncatedTrailing, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(&paths.events_jsonl).unwrap(),
        valid_line,
        "partial line must be truncated to last clean newline",
    );
}

#[test]
fn drifted_task_definition_aborts_resume_with_typed_error() {
    let dir = tempdir().expect("tempdir");
    let paths = PersistPaths::from_workdir(dir.path()).expect("paths");

    let task_a_original = task("a", "Alpha original");
    let fp_a_original = TaskDefFingerprint::from_task(&task_a_original, "p1");
    save_run_state(
        &paths,
        &baseline_snapshot("prior-run", vec![fp_a_original.clone()]),
    )
    .unwrap();

    // Drift: same id, different title -> different fingerprint.
    let task_a_drifted = task("a", "Alpha drifted");
    let mut plans = HashMap::new();
    plans.insert("p1".to_string(), vec![task_a_drifted]);

    let err = prepare_resume(&paths, &plans, &[fp_a_original.clone()]).unwrap_err();
    match err {
        ResumeError::TaskMismatch { mismatches } => {
            assert_eq!(mismatches.len(), 1);
            assert_eq!(mismatches[0].plan_id, "p1");
            assert_eq!(mismatches[0].task_id, "a");
            assert_ne!(
                mismatches[0].expected_fingerprint, mismatches[0].actual_fingerprint,
                "fingerprints should disagree on drift",
            );
        }
        other => panic!("expected TaskMismatch, got {other:?}"),
    }
}

#[test]
fn missing_plan_in_current_run_is_rejected() {
    let dir = tempdir().expect("tempdir");
    let paths = PersistPaths::from_workdir(dir.path()).expect("paths");

    let task_a = task("a", "A");
    let fp_a = TaskDefFingerprint::from_task(&task_a, "p1");
    save_run_state(&paths, &baseline_snapshot("prior", vec![fp_a.clone()])).unwrap();

    // Current run only has p2 — p1 (which the snapshot recorded) is gone.
    let task_x = task("x", "X");
    let mut plans = HashMap::new();
    plans.insert("p2".to_string(), vec![task_x]);

    let err = prepare_resume(&paths, &plans, &[fp_a]).unwrap_err();
    assert!(matches!(err, ResumeError::PlanMissing { .. }));
}

#[test]
fn fresh_workdir_resume_is_treated_as_new_run() {
    let dir = tempdir().expect("tempdir");
    let paths = PersistPaths::from_workdir(dir.path()).expect("paths");

    // No run-state.json — fresh workdir.
    let report = prepare_resume(&paths, &HashMap::new(), &[]).unwrap();
    assert!(!report.resumed);
    assert!(report.prior_run_id.is_none());
    assert_eq!(report.validated_tasks, 0);
    // All three logs must report Clean.
    assert_eq!(report.recovered_files.len(), 3);
    for f in &report.recovered_files {
        match f.recovery {
            JsonlRecoveryReport::Clean { lines } => assert_eq!(lines, 0),
            ref other => panic!("expected Clean, got {other:?}"),
        }
    }
}

#[test]
fn jsonl_recovery_helper_returns_clean_for_consistent_file() {
    // Direct test of the recover_jsonl helper (sanity for the mod).
    let dir = tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    fs::write(&path, "{\"a\":1}\n{\"b\":2}\n").unwrap();
    let outcome = roko_cli::runner::persist::recover_jsonl(&path, |line: &str| {
        serde_json::from_str::<serde_json::Value>(line)
    })
    .unwrap();
    matches!(outcome, JsonlRecovery::Clean { lines: 2 });
}
