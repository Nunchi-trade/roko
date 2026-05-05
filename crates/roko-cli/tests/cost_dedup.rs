//! Integration test: exactly one efficiency event is emitted per dispatch
//! attempt.
//!
//! The test uses the real mock-backed plan workspace, then forces a single
//! gate failure so the run exercises the failure accounting path. That catches
//! duplicate persistence for the same attempt.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use roko_learn::runtime_feedback::read_efficiency_events;
use serde_json::Value;
use tempfile::TempDir;

fn force_failing_verify_step(workdir: &Path) {
    let tasks_path = workdir
        .join("plans")
        .join(common::SAMPLE_PLAN_ID)
        .join("tasks.toml");
    let tasks = fs::read_to_string(&tasks_path).expect("read sample tasks.toml");

    let mut patched = String::with_capacity(tasks.len() + 128);
    let mut inserted_verify = false;
    let mut saw_max_retries = false;

    for line in tasks.lines() {
        if line.trim() == "verify = []" {
            continue;
        }

        if line.trim() == "max_retries = 1" {
            patched.push_str("max_retries = 0\n");
            patched.push('\n');
            patched.push_str("[[task.verify]]\n");
            patched.push_str("phase = \"compile\"\n");
            patched.push_str("command = \"false\"\n");
            patched.push_str("timeout_ms = 1000\n");
            patched.push('\n');
            inserted_verify = true;
            saw_max_retries = true;
            continue;
        }

        patched.push_str(line);
        patched.push('\n');
    }

    assert!(
        saw_max_retries,
        "sample tasks.toml did not contain max_retries = 1"
    );
    assert!(
        inserted_verify,
        "failed to inject a failing verify step into sample tasks.toml"
    );

    fs::write(&tasks_path, patched).expect("write patched sample tasks.toml");
}

#[tokio::test]
async fn single_task_plan_emits_one_efficiency_event_per_attempt() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();

    common::setup_sample_plan_workspace(workdir);
    force_failing_verify_step(workdir);

    let report = common::run_sample_plan(workdir);
    let total_agent_calls = report
        .get("total_agent_calls")
        .and_then(Value::as_u64)
        .expect("plan run report should include total_agent_calls");
    assert!(
        total_agent_calls > 0,
        "sample plan should dispatch at least one agent call; report = {report:#}"
    );

    let efficiency_path = workdir.join(".roko").join("learn").join("efficiency.jsonl");
    assert!(
        efficiency_path.exists(),
        "efficiency log should be written at {}",
        efficiency_path.display()
    );

    let events = read_efficiency_events(&efficiency_path)
        .await
        .expect("read efficiency events");

    let plan_events: Vec<_> = events
        .into_iter()
        .filter(|event| event.plan_id == common::SAMPLE_PLAN_ID)
        .collect();

    assert_eq!(
        plan_events.len() as u64,
        total_agent_calls,
        "expected one efficiency event per agent call for plan {}, got {} events for {} agent calls",
        common::SAMPLE_PLAN_ID,
        plan_events.len(),
        total_agent_calls
    );

    let mut seen_attempts: HashSet<(String, String, String)> = HashSet::new();
    let mut total_cost_usd = 0.0;
    let mut saw_failure_event = false;

    for event in &plan_events {
        assert!(
            !event.plan_id.is_empty(),
            "efficiency event must have a non-empty plan_id"
        );
        assert!(
            !event.task_id.is_empty(),
            "efficiency event must have a non-empty task_id"
        );
        assert!(
            !event.attempt_id.is_empty(),
            "efficiency event must have a non-empty attempt_id"
        );
        assert!(
            event.cost_usd >= 0.0,
            "cost_usd must be non-negative, got {} for attempt {}",
            event.cost_usd,
            event.attempt_id
        );
        if !event.gate_passed {
            saw_failure_event = true;
        }

        let key = (
            event.plan_id.clone(),
            event.task_id.clone(),
            event.attempt_id.clone(),
        );
        assert!(
            seen_attempts.insert(key),
            "duplicate efficiency event for ({}, {}, {}) would double-count learning cost",
            event.plan_id,
            event.task_id,
            event.attempt_id
        );

        total_cost_usd += event.cost_usd;
    }

    assert!(
        saw_failure_event,
        "the forced gate failure should be reflected in at least one efficiency event"
    );
    assert!(
        total_cost_usd >= 0.0,
        "total efficiency cost must be non-negative, got {total_cost_usd}"
    );
}
