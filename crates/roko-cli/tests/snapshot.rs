//! Integration tests for executor snapshot migration and persistence.

use std::fs;
use std::path::{Path, PathBuf};

use roko_cli::orchestrate::save_snapshot_atomic;
use roko_cli::snapshot_migrate;
use roko_cli::snapshot_reconcile::{SnapshotReconcileError, reconcile_snapshot_vs_plans};
use roko_orchestrator::{CURRENT_SCHEMA_VERSION, ExecutorSnapshot, PlanInfo, PlanState};
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("snapshots")
        .join(name)
}

fn read_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let body = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("read fixture {}: {err}", path.display());
    });
    serde_json::from_str(&body).unwrap_or_else(|err| {
        panic!("parse fixture {}: {err}", path.display());
    })
}

fn plan_info(id: &str) -> PlanInfo {
    let num = id.split_once('-').map_or(id, |(prefix, _)| prefix);
    PlanInfo {
        base: id.to_owned(),
        num: num.to_owned(),
        path: Path::new("plans").join(id).join("plan.md"),
        frontmatter: None,
    }
}

fn snapshot_with_plan_ids(plan_ids: &[&str]) -> ExecutorSnapshot {
    let mut snapshot = ExecutorSnapshot::new(123);
    snapshot.queue_order = plan_ids.iter().map(|id| (*id).to_owned()).collect();
    for plan_id in plan_ids {
        snapshot
            .plan_states
            .insert((*plan_id).to_owned(), PlanState::new(*plan_id));
    }
    snapshot
}

#[test]
fn upgrade_v0_to_v1_succeeds() {
    let snapshot = snapshot_migrate::upgrade(read_fixture("v0-sample.json")).expect("upgrade v0");

    assert_eq!(snapshot.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(snapshot.timestamp_ms, 123);
    assert_eq!(snapshot.queue_order, vec!["plan-a"]);
    assert!(snapshot.plan_states.contains_key("plan-a"));
}

#[test]
fn upgrade_future_version_errors() {
    let err = snapshot_migrate::upgrade(serde_json::json!({
        "schema_version": CURRENT_SCHEMA_VERSION + 1,
        "timestamp_ms": 123
    }))
    .expect_err("future schema should fail");

    assert!(err.to_string().contains("newer than this build supports"));
}

#[test]
fn upgrade_current_is_noop() {
    let raw = read_fixture("v1-sample.json");
    let snapshot = snapshot_migrate::upgrade(raw).expect("upgrade current schema");

    assert_eq!(snapshot.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(snapshot.timestamp_ms, 123);
    assert_eq!(snapshot.queue_order, vec!["plan-a"]);
    assert!(snapshot.plan_states.contains_key("plan-a"));
}

#[test]
fn save_writes_current_version() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("executor.json");
    let snapshot = ExecutorSnapshot::new(77);

    save_snapshot_atomic(&snapshot, &path).expect("save snapshot");

    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read snapshot"))
            .expect("parse snapshot");
    assert_eq!(
        saved
            .get("schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(u64::from(CURRENT_SCHEMA_VERSION))
    );
}

#[test]
fn reconcile_noop_on_match() {
    let snapshot = snapshot_with_plan_ids(&["foo", "bar"]);
    let discovered = vec![plan_info("foo"), plan_info("bar")];

    reconcile_snapshot_vs_plans(
        &snapshot,
        &discovered,
        Path::new(".roko/state/executor.json"),
        Path::new("plans"),
    )
    .expect("matching plans should reconcile");
}

#[test]
fn reconcile_reports_missing() {
    let snapshot = snapshot_with_plan_ids(&["foo", "bar"]);
    let discovered = vec![plan_info("baz"), plan_info("qux")];

    let err = reconcile_snapshot_vs_plans(
        &snapshot,
        &discovered,
        Path::new(".roko/state/executor.json"),
        Path::new("plans"),
    )
    .expect_err("missing plans should fail");

    let message = err.to_string();

    match err {
        SnapshotReconcileError::PlanIdsMissing {
            missing,
            discovered,
            snapshot_path,
            plans_root,
        } => {
            assert_eq!(missing, vec!["foo".to_owned(), "bar".to_owned()]);
            assert_eq!(discovered, vec!["baz".to_owned(), "qux".to_owned()]);
            assert_eq!(snapshot_path, PathBuf::from(".roko/state/executor.json"));
            assert_eq!(plans_root, PathBuf::from("plans"));
        }
    }

    assert!(message.contains("resume snapshot references plans [foo, bar]"));
    assert!(message.contains("plans/ at plans has [baz, qux]"));
    assert!(message.contains("Rename or prune the snapshot before resuming."));
    assert!(message.contains("Snapshot path: .roko/state/executor.json"));
    assert!(message.contains("Plans root: plans"));
    assert!(message.contains("Missing: foo, bar"));
}

#[test]
fn reconcile_noop_on_superset() {
    let snapshot = snapshot_with_plan_ids(&["foo"]);
    let discovered = vec![plan_info("foo"), plan_info("bar"), plan_info("baz")];

    reconcile_snapshot_vs_plans(
        &snapshot,
        &discovered,
        Path::new(".roko/state/executor.json"),
        Path::new("plans"),
    )
    .expect("additional discovered plans should be allowed");
}
