//! Single-file, checksummed orchestration state snapshot.
//!
//! All four mutable state groups (executor, orchestrator, run counters, gate thresholds)
//! are serialized into this struct and written atomically in one `atomic_write` call.
//! The `checksum` field is a SHA-256 hex digest of the JSON-serialized inner payloads
//! (computed before they are embedded, not over the outer document).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bump this constant whenever the shape of `StateSnapshot` changes in an incompatible way.
/// Resume code must reject snapshots with a different version.
pub const STATE_SNAPSHOT_VERSION: u32 = 1;

/// All runtime state groups bundled for a single atomic write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// Schema version -- compared against `STATE_SNAPSHOT_VERSION` on load.
    pub version: u32,
    /// Wall-clock timestamp of this snapshot (milliseconds since Unix epoch).
    pub timestamp_ms: u64,
    /// Executor snapshot JSON (opaque to roko-runtime; owned by roko-orchestrator).
    pub executor_json: String,
    /// Orchestrator snapshot JSON (opaque; includes merge queue).
    pub orchestrator_json: String,
    /// Run-state counters JSON.
    pub run_state_json: String,
    /// Gate threshold EMA state JSON.
    pub gate_thresholds_json: String,
    /// SHA-256 hex digest of the concatenation
    /// `executor_json || orchestrator_json || run_state_json || gate_thresholds_json`
    /// computed at save time. Validated at load time before any field is consumed.
    pub checksum: String,
}

impl StateSnapshot {
    /// Construct and checksum a new snapshot from its constituent serialized pieces.
    pub fn new(
        timestamp_ms: u64,
        executor_json: String,
        orchestrator_json: String,
        run_state_json: String,
        gate_thresholds_json: String,
    ) -> Self {
        let checksum = compute_checksum(
            &executor_json,
            &orchestrator_json,
            &run_state_json,
            &gate_thresholds_json,
        );
        Self {
            version: STATE_SNAPSHOT_VERSION,
            timestamp_ms,
            executor_json,
            orchestrator_json,
            run_state_json,
            gate_thresholds_json,
            checksum,
        }
    }

    /// Validate the embedded checksum. Returns `Err` with a descriptive message on mismatch.
    pub fn verify(&self) -> Result<(), String> {
        if self.version != STATE_SNAPSHOT_VERSION {
            return Err(format!(
                "state snapshot version mismatch: file has {}, code expects {}",
                self.version, STATE_SNAPSHOT_VERSION
            ));
        }
        let expected = compute_checksum(
            &self.executor_json,
            &self.orchestrator_json,
            &self.run_state_json,
            &self.gate_thresholds_json,
        );
        if expected != self.checksum {
            return Err(format!(
                "state snapshot checksum mismatch: stored {}, computed {expected}",
                self.checksum
            ));
        }
        Ok(())
    }
}

fn compute_checksum(
    executor: &str,
    orchestrator: &str,
    run_state: &str,
    gate_thresholds: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(executor.as_bytes());
    hasher.update(orchestrator.as_bytes());
    hasher.update(run_state.as_bytes());
    hasher.update(gate_thresholds.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_snapshot_verifies_cleanly() {
        let snap = StateSnapshot::new(
            1_000_000,
            r#"{"tasks":[]}"#.to_string(),
            r#"{"merge_queue":[]}"#.to_string(),
            r#"{"run_id":"run-1"}"#.to_string(),
            r#"{"rungs":{}}"#.to_string(),
        );
        assert_eq!(snap.version, STATE_SNAPSHOT_VERSION);
        assert!(snap.verify().is_ok());
    }

    #[test]
    fn corrupted_checksum_fails_verification() {
        let mut snap = StateSnapshot::new(
            1_000_000,
            r#"{"tasks":[]}"#.to_string(),
            r#"{"merge_queue":[]}"#.to_string(),
            r#"{"run_id":"run-1"}"#.to_string(),
            r#"{"rungs":{}}"#.to_string(),
        );
        snap.checksum =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        let err = snap.verify().unwrap_err();
        assert!(err.contains("checksum mismatch"), "got: {err}");
    }

    #[test]
    fn mutated_payload_fails_verification() {
        let mut snap = StateSnapshot::new(
            1_000_000,
            r#"{"tasks":[]}"#.to_string(),
            r#"{"merge_queue":[]}"#.to_string(),
            r#"{"run_id":"run-1"}"#.to_string(),
            r#"{"rungs":{}}"#.to_string(),
        );
        // Mutate one of the inner payloads after construction.
        snap.executor_json = r#"{"tasks":["tampered"]}"#.to_string();
        let err = snap.verify().unwrap_err();
        assert!(err.contains("checksum mismatch"), "got: {err}");
    }

    #[test]
    fn wrong_version_fails_verification() {
        let mut snap = StateSnapshot::new(
            1_000_000,
            r#"{"tasks":[]}"#.to_string(),
            r#"{"merge_queue":[]}"#.to_string(),
            r#"{"run_id":"run-1"}"#.to_string(),
            r#"{"rungs":{}}"#.to_string(),
        );
        snap.version = STATE_SNAPSHOT_VERSION + 1;
        let err = snap.verify().unwrap_err();
        assert!(err.contains("version mismatch"), "got: {err}");
    }

    #[test]
    fn checksum_is_64_char_hex() {
        let snap = StateSnapshot::new(
            1_000_000,
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        );
        assert_eq!(snap.checksum.len(), 64);
        assert!(snap.checksum.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn round_trip_through_json() {
        let snap = StateSnapshot::new(
            42,
            r#"{"x":1}"#.to_string(),
            r#"{"y":2}"#.to_string(),
            r#"{"z":3}"#.to_string(),
            r#"{"w":4}"#.to_string(),
        );
        let json = serde_json::to_string_pretty(&snap).unwrap();
        let loaded: StateSnapshot = serde_json::from_str(&json).unwrap();
        assert!(loaded.verify().is_ok());
        assert_eq!(loaded.checksum, snap.checksum);
        assert_eq!(loaded.timestamp_ms, 42);
    }
}
