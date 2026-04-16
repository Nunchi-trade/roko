//! Integration tests for executor snapshot migration and persistence.

use std::fs;
use std::path::{Path, PathBuf};

use roko_cli::orchestrate::save_snapshot_atomic;
use roko_cli::snapshot_migrate;
use roko_orchestrator::{CURRENT_SCHEMA_VERSION, ExecutorSnapshot};
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
