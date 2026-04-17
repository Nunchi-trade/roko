//! Snapshot schema migration helpers.
//!
//! This module upgrades raw executor snapshot JSON into the current
//! [`ExecutorSnapshot`] shape by applying forward-only migrations in
//! version order.

use anyhow::{Context as _, Result, anyhow, bail};
use roko_orchestrator::{CURRENT_SCHEMA_VERSION, ExecutorSnapshot, current_schema_version};
use serde_json::{Value, json};

fn snapshot_schema_version(raw: &Value) -> Result<u32> {
    match raw.get("schema_version") {
        None => Ok(0),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow!("snapshot schema_version must be an unsigned integer"))
            .and_then(|version| {
                u32::try_from(version)
                    .map_err(|_| anyhow!("snapshot schema_version {version} does not fit in u32"))
            }),
    }
}

/// Parse and upgrade a persisted executor snapshot JSON string.
///
/// # Errors
///
/// Returns an error if the input is not valid JSON, the schema version is
/// unsupported, or the upgraded snapshot cannot be deserialized.
pub fn load_executor_snapshot(json: &str) -> Result<ExecutorSnapshot> {
    let raw: Value = serde_json::from_str(json).context("parse executor snapshot JSON")?;
    upgrade(raw)
}

/// Upgrade raw snapshot JSON to the current executor snapshot schema.
///
/// # Errors
///
/// Returns an error if the snapshot is newer than this build supports,
/// contains an unknown intermediate schema version, or cannot be parsed
/// after migration.
pub fn upgrade(raw: Value) -> Result<ExecutorSnapshot> {
    let mut migrated = raw;
    let mut version = snapshot_schema_version(&migrated)?;

    if version > CURRENT_SCHEMA_VERSION {
        bail!(
            "snapshot schema v{} is newer than this build supports (v{}); upgrade roko-cli or restore an older snapshot",
            version,
            CURRENT_SCHEMA_VERSION,
        );
    }

    while version < CURRENT_SCHEMA_VERSION {
        migrated = match version {
            0 => migrate_v0_to_v1(migrated)?,
            other => bail!("unknown snapshot schema_version: {other}"),
        };

        let next_version = snapshot_schema_version(&migrated)?;
        if next_version <= version {
            bail!(
                "snapshot migration did not advance schema_version: v{} -> v{}",
                version,
                next_version
            );
        }
        version = next_version;
    }

    serde_json::from_value(migrated).context("deserialize upgraded executor snapshot")
}

fn migrate_v0_to_v1(mut raw: Value) -> Result<Value> {
    let Some(obj) = raw.as_object_mut() else {
        bail!("snapshot schema v0 must be a JSON object");
    };

    obj.insert(
        "schema_version".to_string(),
        json!(current_schema_version()),
    );
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::PlanPhase;
    use roko_orchestrator::{ExecutorSnapshot, PlanState};

    #[test]
    fn upgrade_v0_to_v1_succeeds() {
        let raw = serde_json::json!({
            "timestamp_ms": 42,
            "queue_order": ["plan-1"],
            "plan_states": {
                "plan-1": {
                    "plan_id": "plan-1",
                    "current_phase": {"kind": "queued"}
                }
            }
        });

        let snapshot = upgrade(raw).expect("upgrade snapshot");

        assert_eq!(snapshot.timestamp_ms, 42);
        assert_eq!(snapshot.queue_order, vec!["plan-1"]);
        assert_eq!(
            snapshot.plan_states["plan-1"].current_phase,
            PlanPhase::Queued
        );
    }

    #[test]
    fn upgrade_future_version_errors() {
        let raw = serde_json::json!({
            "schema_version": 2,
            "timestamp_ms": 42
        });

        let err = upgrade(raw).expect_err("future schema should fail");

        assert!(err.to_string().contains("newer than this build supports"));
    }

    #[test]
    fn upgrade_current_is_noop() {
        let mut snapshot = ExecutorSnapshot::new(7);
        let mut plan_state = PlanState::new("plan-1");
        plan_state.current_phase = PlanPhase::Implementing;
        snapshot
            .plan_states
            .insert("plan-1".to_string(), plan_state);

        let raw = serde_json::json!({
            "schema_version": 1,
            "plan_states": snapshot.plan_states,
            "queue_order": ["plan-1"],
            "speculative_executions": {},
            "timestamp_ms": 7
        });

        let restored = upgrade(raw).expect("upgrade current schema");

        assert_eq!(restored.timestamp_ms, 7);
        assert_eq!(restored.queue_order, vec!["plan-1"]);
        assert_eq!(
            restored.plan_states["plan-1"].current_phase,
            PlanPhase::Implementing
        );
    }

    #[test]
    fn upgrade_v0_missing_schema_version_defaults_to_zero() {
        let raw = serde_json::json!({
            "timestamp_ms": 5
        });

        let snapshot = upgrade(raw).expect("upgrade v0 snapshot");

        assert_eq!(snapshot.timestamp_ms, 5);
        assert!(snapshot.plan_states.is_empty());
        assert!(snapshot.queue_order.is_empty());
    }
}
