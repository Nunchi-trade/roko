//! Crash-recovery snapshot for the executor.
//!
//! [`ExecutorSnapshot`] captures the full mutable state of a
//! [`ParallelExecutor`](super::ParallelExecutor) so it can be serialized
//! to disk and restored after a crash or restart. The snapshot is designed
//! to be written atomically (write-to-temp + rename) by the persistence
//! layer.
//!
//! # Delta snapshots (ORCH-03)
//!
//! [`DeltaSnapshot`] records only the fields that changed between two full
//! snapshots. A chain of deltas can be replayed on top of a base to
//! reconstruct any intermediate state without storing a full copy each time.
//!
//! # Cryptographic verification (ORCH-04)
//!
//! [`SnapshotVerifier`] wraps serialized snapshots in a binary envelope
//! (`ROKO` magic + length + payload + BLAKE3 hash + `END!` trailer) and
//! verifies integrity on load.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use roko_core::PlanPhase;

use super::SpeculativeExecution;
use super::plan_state::PlanState;

/// Serializable per-plan failure record captured from the conductor circuit breaker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCircuitBreakerFailureRecord {
    /// Number of failures recorded.
    pub count: u32,
    /// Unix milliseconds of the last failure.
    pub last_failure_ms: Option<i64>,
    /// Descriptions of each failure (most recent last).
    #[serde(default)]
    pub reasons: Vec<String>,
}

/// Serializable conductor circuit-breaker snapshot carried alongside executor state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedCircuitBreakerState {
    /// Maximum failures threshold active when the snapshot was taken.
    pub max_failures: u32,
    /// Per-plan failure records keyed by `plan_id`.
    #[serde(default)]
    pub records: HashMap<String, PersistedCircuitBreakerFailureRecord>,
}

/// Serializable snapshot of the entire executor state.
///
/// The runtime writes this periodically (or on every significant event)
/// to `.roko/state/executor.json`. On startup, if the file exists, the
/// executor restores from it and resumes.
///
/// Missing `schema_version` implies a legacy v0 snapshot. Callers that load
/// persisted snapshots across releases should route through a migration shim
/// before deserializing into this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorSnapshot {
    /// Version of the persisted snapshot schema.
    #[serde(default)]
    pub schema_version: u32,
    /// Per-plan mutable state, keyed by `plan_id`.
    #[serde(default)]
    pub plan_states: HashMap<String, PlanState>,
    /// Queue order: `plan_id`s in execution priority order.
    #[serde(default)]
    pub queue_order: Vec<String>,
    /// Optional conductor circuit-breaker state captured with the executor snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conductor_circuit_breaker: Option<PersistedCircuitBreakerState>,
    /// Live speculative execution branches, keyed by `plan:task`.
    #[serde(default)]
    pub speculative_executions: HashMap<String, SpeculativeExecution>,
    /// Unix millisecond timestamp when the snapshot was taken.
    #[serde(default)]
    pub timestamp_ms: u64,
}

/// Current on-disk schema version for [`ExecutorSnapshot`].
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Return the current on-disk schema version for [`ExecutorSnapshot`].
#[must_use]
pub const fn current_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

impl ExecutorSnapshot {
    /// Create an empty snapshot at the given timestamp.
    #[must_use]
    pub fn new(timestamp_ms: u64) -> Self {
        Self {
            schema_version: current_schema_version(),
            plan_states: HashMap::new(),
            queue_order: Vec::new(),
            conductor_circuit_breaker: None,
            speculative_executions: HashMap::new(),
            timestamp_ms,
        }
    }

    /// Serialize to JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (should not happen for these types).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON bytes.
    ///
    /// # Errors
    ///
    /// Falls back to a legacy `tasks`-based schema if the current
    /// `plan_states` layout is unavailable.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        // Peek at the raw value: if it has a `tasks` key but no
        // `plan_states`, it is a legacy snapshot and should use the
        // compat loader even though the primary path would succeed
        // (all fields are `#[serde(default)]`).
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
            if value.get("tasks").is_some() && value.get("plan_states").is_none() {
                return Self::from_legacy_json(json);
            }
        }
        match serde_json::from_str(json) {
            Ok(snapshot) => Ok(snapshot),
            Err(primary) => Self::from_legacy_json(json).or(Err(primary)),
        }
    }

    /// Number of plans in the snapshot.
    #[must_use]
    pub fn plan_count(&self) -> usize {
        self.plan_states.len()
    }

    /// Compute the BLAKE3 hash of this snapshot's canonical JSON.
    ///
    /// Uses `serde_json::Value` as an intermediary to ensure deterministic
    /// key ordering (alphabetical) regardless of `HashMap` iteration order.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn compute_hash(&self) -> Result<[u8; 32], serde_json::Error> {
        let value = serde_json::to_value(self)?;
        let canonical = serde_json::to_vec(&value)?;
        Ok(SnapshotVerifier::compute_hash(&canonical))
    }

    /// Compute an incremental delta from `base` to `self`.
    ///
    /// The delta records which plan IDs were added, removed, or changed,
    /// and stores the full JSON of changed plan states so the delta can
    /// be applied to the base to reconstruct `self`.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (used internally for hashing).
    pub fn delta_from(&self, base: &Self) -> Result<DeltaSnapshot, serde_json::Error> {
        let base_hash = base.compute_hash()?;
        let expected_hash = self.compute_hash()?;

        let mut added_plan_ids: Vec<String> = self
            .plan_states
            .keys()
            .filter(|id| !base.plan_states.contains_key(*id))
            .cloned()
            .collect();

        let mut removed_plan_ids: Vec<String> = base
            .plan_states
            .keys()
            .filter(|id| !self.plan_states.contains_key(*id))
            .cloned()
            .collect();

        let changed = self.build_delta_changed(base)?;

        added_plan_ids.sort();
        removed_plan_ids.sort();

        Ok(DeltaSnapshot {
            base_hash,
            expected_hash,
            changed,
            removed_plan_ids,
            added_plan_ids,
            sequence: 0,
        })
    }

    /// Build a JSON object of only the fields that differ from `base`.
    fn build_delta_changed(
        &self,
        base: &Self,
    ) -> Result<serde_json::Value, serde_json::Error> {
        let mut changed_plans = serde_json::Map::new();
        for (id, state) in &self.plan_states {
            let same_as_base = base
                .plan_states
                .get(id)
                .and_then(|base_state| {
                    let a = serde_json::to_value(base_state).ok()?;
                    let b = serde_json::to_value(state).ok()?;
                    Some(a == b)
                })
                .unwrap_or(false);
            if !same_as_base {
                changed_plans.insert(id.clone(), serde_json::to_value(state)?);
            }
        }

        let mut changed = serde_json::Map::new();
        if !changed_plans.is_empty() {
            changed.insert(
                "plan_states".into(),
                serde_json::Value::Object(changed_plans),
            );
        }
        if self.queue_order != base.queue_order {
            changed.insert(
                "queue_order".into(),
                serde_json::to_value(&self.queue_order)?,
            );
        }
        if self.timestamp_ms != base.timestamp_ms {
            changed.insert(
                "timestamp_ms".into(),
                serde_json::Value::Number(self.timestamp_ms.into()),
            );
        }
        if self.schema_version != base.schema_version {
            changed.insert(
                "schema_version".into(),
                serde_json::Value::Number(self.schema_version.into()),
            );
        }
        Ok(serde_json::Value::Object(changed))
    }

    /// Apply a delta on top of this snapshot to produce a new snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotIntegrityError::BaseHashMismatch`] if this snapshot's
    /// hash does not match the delta's `base_hash`.
    /// Returns [`SnapshotIntegrityError::ResultHashMismatch`] if the
    /// reconstructed snapshot's hash does not match `expected_hash`.
    pub fn apply_delta(&self, delta: &DeltaSnapshot) -> Result<Self, SnapshotIntegrityError> {
        let our_hash = self
            .compute_hash()
            .map_err(|e| SnapshotIntegrityError::SerializationFailed(e.to_string()))?;
        if our_hash != delta.base_hash {
            return Err(SnapshotIntegrityError::BaseHashMismatch {
                expected: delta.base_hash,
                actual: our_hash,
            });
        }

        let mut result = self.clone();

        for id in &delta.removed_plan_ids {
            result.plan_states.remove(id);
        }

        if let Some(obj) = delta.changed.as_object() {
            apply_changed_fields(&mut result, obj)?;
        }

        let result_hash = result
            .compute_hash()
            .map_err(|e| SnapshotIntegrityError::SerializationFailed(e.to_string()))?;
        if result_hash != delta.expected_hash {
            return Err(SnapshotIntegrityError::ResultHashMismatch {
                expected: delta.expected_hash,
                actual: result_hash,
            });
        }

        Ok(result)
    }

    fn from_legacy_json(json: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let Some(tasks) = value.get("tasks").and_then(|tasks| tasks.as_array()) else {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing plan_states or tasks",
            )));
        };

        let timestamp_ms = value
            .get("timestamp_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        let mut plan_states: HashMap<String, PlanState> = HashMap::new();
        let mut queue_order: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        let mut plan_progress: HashMap<String, (usize, usize, bool)> = HashMap::new();

        for task in tasks {
            let plan_id = task
                .get("plan")
                .or_else(|| task.get("plan_id"))
                .and_then(|plan| plan.as_str())
                .unwrap_or_default();
            if plan_id.is_empty() {
                continue;
            }

            if seen.insert(plan_id.to_string()) {
                queue_order.push(plan_id.to_string());
            }

            let status = task
                .get("status")
                .and_then(|status| status.as_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();

            let entry = plan_progress
                .entry(plan_id.to_string())
                .or_insert((0usize, 0usize, false));
            entry.0 += 1;
            if matches!(status.as_str(), "done" | "complete" | "completed") {
                entry.1 += 1;
            } else {
                entry.2 = true;
            }
        }

        for (plan_id, (total, done, has_active)) in plan_progress {
            let mut plan_state = PlanState::new(plan_id.clone());
            if total > 0 && done == total {
                plan_state.current_phase = PlanPhase::Complete;
            } else if done > 0 || has_active {
                plan_state.current_phase = PlanPhase::Implementing;
            }
            plan_states.insert(plan_id, plan_state);
        }

        if let Some(order) = value.get("queue_order").and_then(|order| order.as_array()) {
            let legacy_order = order
                .iter()
                .filter_map(|entry| entry.as_str().map(String::from))
                .collect::<Vec<_>>();
            if !legacy_order.is_empty() {
                queue_order = legacy_order;
            }
        }

        Ok(Self {
            schema_version: current_schema_version(),
            plan_states,
            queue_order,
            conductor_circuit_breaker: None,
            speculative_executions: HashMap::new(),
            timestamp_ms,
        })
    }
}

/// Apply changed fields from a delta JSON object onto a snapshot.
fn apply_changed_fields(
    result: &mut ExecutorSnapshot,
    obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), SnapshotIntegrityError> {
    if let Some(serde_json::Value::Object(plan_states)) = obj.get("plan_states") {
        for (id, value) in plan_states {
            let state: PlanState = serde_json::from_value(value.clone())
                .map_err(|e| SnapshotIntegrityError::SerializationFailed(e.to_string()))?;
            result.plan_states.insert(id.clone(), state);
        }
    }
    if let Some(queue_order) = obj.get("queue_order") {
        result.queue_order = serde_json::from_value(queue_order.clone())
            .map_err(|e| SnapshotIntegrityError::SerializationFailed(e.to_string()))?;
    }
    if let Some(ts) = obj.get("timestamp_ms").and_then(serde_json::Value::as_u64) {
        result.timestamp_ms = ts;
    }
    if let Some(sv) = obj
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
    {
        // schema_version is u32; truncation is intentional (only valid values fit).
        #[allow(clippy::cast_possible_truncation)]
        {
            result.schema_version = sv as u32;
        }
    }
    Ok(())
}

// ─── Delta snapshots (ORCH-03) ──────────────────────────────────────────

/// An incremental delta between two [`ExecutorSnapshot`]s.
///
/// Instead of persisting a full snapshot every time, the runtime can
/// store a chain of deltas. Each delta captures only the changed
/// plan states, added/removed plan IDs, and modified top-level fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeltaSnapshot {
    /// BLAKE3 hash of the base snapshot this delta was computed against.
    pub base_hash: [u8; 32],
    /// BLAKE3 hash of the expected result after applying this delta.
    pub expected_hash: [u8; 32],
    /// JSON object containing only the changed top-level fields.
    pub changed: serde_json::Value,
    /// Plan IDs that were removed since the base.
    pub removed_plan_ids: Vec<String>,
    /// Plan IDs that were added since the base.
    pub added_plan_ids: Vec<String>,
    /// Position of this delta in the chain (0-indexed from the base).
    pub sequence: u64,
}

/// Configuration for snapshot frequency and delta chain limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotConfig {
    /// How often (in ticks/events) to write a full snapshot.
    pub full_interval: u64,
    /// How often (in ticks/events) to write a delta snapshot.
    pub delta_interval: u64,
    /// Maximum number of deltas before forcing a full snapshot.
    pub max_delta_chain: usize,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            full_interval: 100,
            delta_interval: 10,
            max_delta_chain: 10,
        }
    }
}

// ─── Cryptographic verification (ORCH-04) ───────────────────────────────

/// Binary envelope magic bytes: `ROKO`.
const MAGIC: &[u8; 4] = b"ROKO";
/// Binary envelope trailer bytes: `END!`.
const TRAILER: &[u8; 4] = b"END!";

/// Errors from snapshot integrity verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotIntegrityError {
    /// The file does not start with the expected magic bytes.
    BadMagic,
    /// The file does not end with the expected trailer.
    BadTrailer,
    /// The file is too short to contain a valid envelope.
    TooShort,
    /// The declared payload length exceeds the available data.
    LengthMismatch {
        /// Declared payload length.
        declared: u64,
        /// Actual available bytes.
        available: usize,
    },
    /// The BLAKE3 checksum does not match the payload.
    ChecksumMismatch {
        /// Expected hash from the envelope.
        expected: [u8; 32],
        /// Computed hash of the payload.
        actual: [u8; 32],
    },
    /// The base snapshot hash did not match when applying a delta.
    BaseHashMismatch {
        /// Hash stored in the delta.
        expected: [u8; 32],
        /// Computed hash of the base snapshot.
        actual: [u8; 32],
    },
    /// The result snapshot hash did not match after applying a delta.
    ResultHashMismatch {
        /// Hash stored in the delta.
        expected: [u8; 32],
        /// Computed hash of the reconstructed snapshot.
        actual: [u8; 32],
    },
    /// JSON serialization/deserialization failed during delta application.
    SerializationFailed(String),
}

impl std::fmt::Display for SnapshotIntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "invalid magic bytes (expected ROKO)"),
            Self::BadTrailer => write!(f, "invalid trailer (expected END!)"),
            Self::TooShort => write!(f, "data too short for snapshot envelope"),
            Self::LengthMismatch {
                declared,
                available,
            } => write!(
                f,
                "payload length mismatch: declared {declared}, available {available}"
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "BLAKE3 checksum mismatch: expected {}, got {}",
                hex_prefix(expected),
                hex_prefix(actual)
            ),
            Self::BaseHashMismatch { expected, actual } => write!(
                f,
                "base hash mismatch: expected {}, got {}",
                hex_prefix(expected),
                hex_prefix(actual)
            ),
            Self::ResultHashMismatch { expected, actual } => write!(
                f,
                "result hash mismatch: expected {}, got {}",
                hex_prefix(expected),
                hex_prefix(actual)
            ),
            Self::SerializationFailed(msg) => write!(f, "serialization failed: {msg}"),
        }
    }
}

impl std::error::Error for SnapshotIntegrityError {}

/// Format a 32-byte hash as a hex string (first 8 bytes for brevity).
fn hex_prefix(hash: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(19); // 16 hex chars + "..."
    for b in &hash[..8] {
        let _ = write!(s, "{b:02x}");
    }
    s.push_str("...");
    s
}

/// BLAKE3-based snapshot verifier.
///
/// Wraps serialized data in a binary envelope:
/// `[4B magic "ROKO"] [8B little-endian payload length] [payload] [32B BLAKE3 hash] [4B trailer "END!"]`
pub struct SnapshotVerifier;

impl SnapshotVerifier {
    /// Compute the BLAKE3 hash of arbitrary data.
    #[must_use]
    pub fn compute_hash(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }

    /// Verify that `data` hashes to `expected`.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotIntegrityError::ChecksumMismatch`] on mismatch.
    pub fn verify_checksum(
        data: &[u8],
        expected: &[u8; 32],
    ) -> Result<(), SnapshotIntegrityError> {
        let actual = Self::compute_hash(data);
        if actual == *expected {
            Ok(())
        } else {
            Err(SnapshotIntegrityError::ChecksumMismatch {
                expected: *expected,
                actual,
            })
        }
    }

    /// Serialize a snapshot and wrap it in the verified binary envelope.
    ///
    /// # Errors
    ///
    /// Returns a serde error if JSON serialization fails.
    pub fn save_verified(snapshot: &ExecutorSnapshot) -> Result<Vec<u8>, serde_json::Error> {
        // Canonical form: serialize via Value to get deterministic key ordering.
        let value = serde_json::to_value(snapshot)?;
        let payload = serde_json::to_vec(&value)?;
        let hash = Self::compute_hash(&payload);
        let len = payload.len() as u64;

        // 4 (magic) + 8 (length) + payload + 32 (hash) + 4 (trailer)
        let mut buf = Vec::with_capacity(4 + 8 + payload.len() + 32 + 4);
        buf.extend_from_slice(MAGIC);
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&payload);
        buf.extend_from_slice(&hash);
        buf.extend_from_slice(TRAILER);
        Ok(buf)
    }

    /// Load a snapshot from the verified binary envelope.
    ///
    /// Checks magic, length, BLAKE3 hash, and trailer before deserializing.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotIntegrityError`] if any structural or hash check
    /// fails, or if JSON deserialization fails.
    pub fn load_verified(data: &[u8]) -> Result<ExecutorSnapshot, SnapshotIntegrityError> {
        // Minimum size: 4 + 8 + 0 + 32 + 4 = 48 (empty payload)
        const MIN_SIZE: usize = 4 + 8 + 32 + 4;
        if data.len() < MIN_SIZE {
            return Err(SnapshotIntegrityError::TooShort);
        }

        if &data[..4] != MAGIC {
            return Err(SnapshotIntegrityError::BadMagic);
        }

        if &data[data.len() - 4..] != TRAILER {
            return Err(SnapshotIntegrityError::BadTrailer);
        }

        let mut len_bytes = [0u8; 8];
        len_bytes.copy_from_slice(&data[4..12]);
        let payload_len = u64::from_le_bytes(len_bytes);

        let available = data.len() - 4 - 8 - 32 - 4;
        if payload_len != available as u64 {
            return Err(SnapshotIntegrityError::LengthMismatch {
                declared: payload_len,
                available,
            });
        }

        let payload_len = available; // now use as usize
        let payload = &data[12..12 + payload_len];

        let hash_start = 12 + payload_len;
        let mut expected_hash = [0u8; 32];
        expected_hash.copy_from_slice(&data[hash_start..hash_start + 32]);
        Self::verify_checksum(payload, &expected_hash)?;

        serde_json::from_slice(payload)
            .map_err(|e| SnapshotIntegrityError::SerializationFailed(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::PlanPhase;

    #[test]
    fn empty_snapshot_roundtrips() {
        let snap = ExecutorSnapshot::new(1000);
        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(restored.timestamp_ms, 1000);
        assert!(restored.plan_states.is_empty());
        assert!(restored.queue_order.is_empty());
        assert!(restored.conductor_circuit_breaker.is_none());
    }

    #[test]
    fn snapshot_with_plans_roundtrips() {
        let mut snap = ExecutorSnapshot::new(42_000);
        let mut ps = PlanState::new("plan-1");
        ps.current_phase = PlanPhase::Implementing;
        ps.iteration = 2;
        snap.plan_states.insert("plan-1".into(), ps);

        let mut ps2 = PlanState::new("plan-2");
        ps2.current_phase = PlanPhase::Gating;
        snap.plan_states.insert("plan-2".into(), ps2);
        snap.queue_order = vec!["plan-1".into(), "plan-2".into()];

        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.plan_count(), 2);
        assert_eq!(restored.queue_order.len(), 2);
        assert_eq!(
            restored.plan_states["plan-1"].current_phase,
            PlanPhase::Implementing
        );
        assert_eq!(restored.plan_states["plan-1"].iteration, 2);
        assert_eq!(
            restored.plan_states["plan-2"].current_phase,
            PlanPhase::Gating
        );
        assert!(restored.conductor_circuit_breaker.is_none());
    }

    #[test]
    fn snapshot_preserves_queue_order() {
        let mut snap = ExecutorSnapshot::new(0);
        snap.queue_order = vec!["c".into(), "a".into(), "b".into()];
        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert_eq!(restored.queue_order, vec!["c", "a", "b"]);
    }

    #[test]
    fn plan_count_matches_states() {
        let mut snap = ExecutorSnapshot::new(0);
        assert_eq!(snap.plan_count(), 0);
        snap.plan_states.insert("a".into(), PlanState::new("a"));
        snap.plan_states.insert("b".into(), PlanState::new("b"));
        assert_eq!(snap.plan_count(), 2);
    }

    #[test]
    fn from_json_rejects_garbage() {
        assert!(ExecutorSnapshot::from_json("not json").is_err());
    }

    #[test]
    fn snapshot_with_partial_plan_state_uses_defaults() {
        // PlanPhase uses `#[serde(tag = "kind", rename_all = "kebab-case")]`,
        // so it is internally tagged: `{"kind": "queued"}` not `"Queued"`.
        let json = r#"
        {
            "plan_states": {
                "plan-1": {
                    "plan_id": "plan-1",
                    "current_phase": {"kind": "queued"}
                }
            },
            "queue_order": ["plan-1"]
        }
        "#;

        let restored = ExecutorSnapshot::from_json(json).unwrap();
        assert_eq!(restored.schema_version, 0);
        let ps = &restored.plan_states["plan-1"];
        assert_eq!(ps.plan_id, "plan-1");
        assert_eq!(ps.current_phase, PlanPhase::Queued);
        assert!(ps.assigned_agents.is_empty());
        assert!(ps.gate_results.is_empty());
        assert_eq!(ps.iteration, 1);
        assert_eq!(ps.started_at_ms, 0);
        assert!(ps.files_changed.is_empty());
        assert_eq!(ps.merge_attempts, 0);
        assert!(ps.last_error.is_none());
        assert!(!ps.paused);
        assert_eq!(ps.priority, 0);
        assert!(restored.conductor_circuit_breaker.is_none());
    }

    #[test]
    fn legacy_task_snapshot_falls_back_to_compat_loader() {
        let json = r#"
        {
            "tasks": [
                { "id": "task-1", "status": "done", "plan": "plan-a" },
                { "id": "task-2", "status": "running", "plan": "plan-a" },
                { "id": "task-3", "status": "complete", "plan": "plan-b" }
            ],
            "queue_order": ["plan-b", "plan-a"],
            "timestamp_ms": 42
        }
        "#;

        let restored = ExecutorSnapshot::from_json(json).unwrap();
        assert_eq!(restored.schema_version, current_schema_version());
        assert_eq!(restored.timestamp_ms, 42);
        assert_eq!(restored.queue_order, vec!["plan-b", "plan-a"]);
        assert_eq!(restored.plan_states.len(), 2);
        assert_eq!(
            restored.plan_states["plan-a"].current_phase,
            PlanPhase::Implementing
        );
        assert_eq!(
            restored.plan_states["plan-b"].current_phase,
            PlanPhase::Complete
        );
        assert!(restored.conductor_circuit_breaker.is_none());
    }

    #[test]
    fn snapshot_with_terminal_plan() {
        let mut snap = ExecutorSnapshot::new(99);
        let mut ps = PlanState::new("done-plan");
        ps.current_phase = PlanPhase::Complete;
        snap.plan_states.insert("done-plan".into(), ps);
        snap.queue_order = vec!["done-plan".into()];

        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        assert!(restored.plan_states["done-plan"].is_terminal());
    }

    #[test]
    fn snapshot_with_circuit_breaker_roundtrips() {
        let mut snap = ExecutorSnapshot::new(21);
        let mut circuit_breaker = PersistedCircuitBreakerState {
            max_failures: 2,
            ..PersistedCircuitBreakerState::default()
        };
        circuit_breaker.records.insert(
            "plan-1".into(),
            PersistedCircuitBreakerFailureRecord {
                count: 2,
                last_failure_ms: Some(200),
                reasons: vec!["compile".into(), "tests".into()],
            },
        );
        snap.conductor_circuit_breaker = Some(circuit_breaker);

        let json = snap.to_json().unwrap();
        let restored = ExecutorSnapshot::from_json(&json).unwrap();
        let restored_breaker = restored
            .conductor_circuit_breaker
            .expect("circuit breaker state should roundtrip");

        assert_eq!(restored_breaker.max_failures, 2);
        assert_eq!(restored_breaker.records["plan-1"].count, 2);
        assert_eq!(
            restored_breaker.records["plan-1"].reasons,
            vec!["compile", "tests"]
        );
    }

    // ─── ORCH-03: Delta snapshot tests ──────────────────────────────────

    #[test]
    fn delta_from_identical_snapshots_is_empty() {
        let snap = ExecutorSnapshot::new(1000);
        let delta = snap.delta_from(&snap).unwrap();
        assert!(delta.added_plan_ids.is_empty());
        assert!(delta.removed_plan_ids.is_empty());
        assert_eq!(delta.base_hash, delta.expected_hash);
        let obj = delta.changed.as_object().unwrap();
        assert!(obj.is_empty(), "no fields should change: {obj:?}");
    }

    #[test]
    fn delta_captures_added_plan() {
        let base = ExecutorSnapshot::new(1000);
        let mut current = base.clone();
        current
            .plan_states
            .insert("new-plan".into(), PlanState::new("new-plan"));
        current.queue_order.push("new-plan".into());
        current.timestamp_ms = 2000;

        let delta = current.delta_from(&base).unwrap();
        assert!(delta.added_plan_ids.contains(&"new-plan".to_string()));
        assert!(delta.removed_plan_ids.is_empty());
    }

    #[test]
    fn delta_captures_removed_plan() {
        let mut base = ExecutorSnapshot::new(1000);
        base.plan_states
            .insert("old-plan".into(), PlanState::new("old-plan"));
        base.queue_order.push("old-plan".into());

        let current = ExecutorSnapshot::new(2000);

        let delta = current.delta_from(&base).unwrap();
        assert!(delta.removed_plan_ids.contains(&"old-plan".to_string()));
        assert!(delta.added_plan_ids.is_empty());
    }

    #[test]
    fn delta_roundtrip_add_modify_remove() {
        let mut base = ExecutorSnapshot::new(1000);
        base.plan_states
            .insert("plan-a".into(), PlanState::new("plan-a"));
        let mut ps_b = PlanState::new("plan-b");
        ps_b.current_phase = PlanPhase::Implementing;
        base.plan_states.insert("plan-b".into(), ps_b);
        base.queue_order = vec!["plan-a".into(), "plan-b".into()];

        let mut current = ExecutorSnapshot::new(2000);
        let mut ps_a = PlanState::new("plan-a");
        ps_a.current_phase = PlanPhase::Gating;
        ps_a.iteration = 3;
        current.plan_states.insert("plan-a".into(), ps_a);
        current
            .plan_states
            .insert("plan-c".into(), PlanState::new("plan-c"));
        current.queue_order = vec!["plan-c".into(), "plan-a".into()];

        let delta = current.delta_from(&base).unwrap();
        let reconstructed = base.apply_delta(&delta).unwrap();

        assert_eq!(reconstructed.timestamp_ms, 2000);
        assert_eq!(reconstructed.plan_count(), 2);
        assert!(reconstructed.plan_states.contains_key("plan-a"));
        assert!(reconstructed.plan_states.contains_key("plan-c"));
        assert!(!reconstructed.plan_states.contains_key("plan-b"));
        assert_eq!(
            reconstructed.plan_states["plan-a"].current_phase,
            PlanPhase::Gating
        );
        assert_eq!(reconstructed.plan_states["plan-a"].iteration, 3);
        assert_eq!(reconstructed.queue_order, vec!["plan-c", "plan-a"]);
    }

    #[test]
    fn apply_delta_rejects_wrong_base() {
        let base = ExecutorSnapshot::new(1000);
        let different_base = ExecutorSnapshot::new(9999);
        let current = ExecutorSnapshot::new(2000);

        let delta = current.delta_from(&base).unwrap();
        let err = different_base.apply_delta(&delta).unwrap_err();
        assert!(
            matches!(err, SnapshotIntegrityError::BaseHashMismatch { .. }),
            "expected BaseHashMismatch, got: {err:?}"
        );
    }

    #[test]
    fn snapshot_config_defaults() {
        let config = SnapshotConfig::default();
        assert_eq!(config.full_interval, 100);
        assert_eq!(config.delta_interval, 10);
        assert_eq!(config.max_delta_chain, 10);
    }

    // ─── ORCH-04: Cryptographic verification tests ──────────────────────

    #[test]
    fn verified_roundtrip() {
        let mut snap = ExecutorSnapshot::new(42_000);
        snap.plan_states
            .insert("plan-1".into(), PlanState::new("plan-1"));
        snap.queue_order = vec!["plan-1".into()];

        let envelope = SnapshotVerifier::save_verified(&snap).unwrap();
        let restored = SnapshotVerifier::load_verified(&envelope).unwrap();

        assert_eq!(restored.timestamp_ms, 42_000);
        assert_eq!(restored.plan_count(), 1);
        assert!(restored.plan_states.contains_key("plan-1"));
        assert_eq!(restored.queue_order, vec!["plan-1"]);
    }

    #[test]
    fn verified_detects_corruption() {
        let snap = ExecutorSnapshot::new(1000);
        let mut envelope = SnapshotVerifier::save_verified(&snap).unwrap();

        // Flip a byte in the payload area
        envelope[20] ^= 0xFF;

        let err = SnapshotVerifier::load_verified(&envelope).unwrap_err();
        assert!(
            matches!(err, SnapshotIntegrityError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got: {err:?}"
        );
    }

    #[test]
    fn verified_rejects_bad_magic() {
        let snap = ExecutorSnapshot::new(1000);
        let mut envelope = SnapshotVerifier::save_verified(&snap).unwrap();
        envelope[0] = b'X';

        let err = SnapshotVerifier::load_verified(&envelope).unwrap_err();
        assert_eq!(err, SnapshotIntegrityError::BadMagic);
    }

    #[test]
    fn verified_rejects_bad_trailer() {
        let snap = ExecutorSnapshot::new(1000);
        let mut envelope = SnapshotVerifier::save_verified(&snap).unwrap();
        let last = envelope.len() - 1;
        envelope[last] = b'X';

        let err = SnapshotVerifier::load_verified(&envelope).unwrap_err();
        assert_eq!(err, SnapshotIntegrityError::BadTrailer);
    }

    #[test]
    fn verified_rejects_truncated_data() {
        let err = SnapshotVerifier::load_verified(&[0u8; 10]).unwrap_err();
        assert_eq!(err, SnapshotIntegrityError::TooShort);
    }

    #[test]
    fn verified_rejects_length_mismatch() {
        let snap = ExecutorSnapshot::new(1000);
        let mut envelope = SnapshotVerifier::save_verified(&snap).unwrap();
        let bad_len: u64 = 999_999;
        envelope[4..12].copy_from_slice(&bad_len.to_le_bytes());

        let err = SnapshotVerifier::load_verified(&envelope).unwrap_err();
        assert!(
            matches!(err, SnapshotIntegrityError::LengthMismatch { .. }),
            "expected LengthMismatch, got: {err:?}"
        );
    }

    #[test]
    fn compute_hash_is_deterministic() {
        let data = b"hello roko";
        let h1 = SnapshotVerifier::compute_hash(data);
        let h2 = SnapshotVerifier::compute_hash(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn verify_checksum_ok() {
        let data = b"some payload";
        let hash = SnapshotVerifier::compute_hash(data);
        assert!(SnapshotVerifier::verify_checksum(data, &hash).is_ok());
    }

    #[test]
    fn verify_checksum_mismatch() {
        let data = b"some payload";
        let wrong_hash = [0u8; 32];
        let err = SnapshotVerifier::verify_checksum(data, &wrong_hash).unwrap_err();
        assert!(matches!(
            err,
            SnapshotIntegrityError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn snapshot_integrity_error_display() {
        let err = SnapshotIntegrityError::BadMagic;
        assert_eq!(err.to_string(), "invalid magic bytes (expected ROKO)");

        let err = SnapshotIntegrityError::TooShort;
        assert_eq!(err.to_string(), "data too short for snapshot envelope");
    }

    #[test]
    fn empty_envelope_roundtrips() {
        let snap = ExecutorSnapshot::new(0);
        let envelope = SnapshotVerifier::save_verified(&snap).unwrap();
        let restored = SnapshotVerifier::load_verified(&envelope).unwrap();
        assert_eq!(restored.timestamp_ms, 0);
        assert!(restored.plan_states.is_empty());
    }

    // ─── Combined: delta + verified envelope ────────────────────────────

    #[test]
    fn delta_through_verified_envelope() {
        let mut base = ExecutorSnapshot::new(100);
        base.plan_states
            .insert("p1".into(), PlanState::new("p1"));

        let mut current = ExecutorSnapshot::new(200);
        current
            .plan_states
            .insert("p1".into(), PlanState::new("p1"));
        current
            .plan_states
            .insert("p2".into(), PlanState::new("p2"));
        current.queue_order = vec!["p2".into()];

        let delta = current.delta_from(&base).unwrap();

        let base_envelope = SnapshotVerifier::save_verified(&base).unwrap();
        let loaded_base = SnapshotVerifier::load_verified(&base_envelope).unwrap();
        let reconstructed = loaded_base.apply_delta(&delta).unwrap();

        assert_eq!(reconstructed.plan_count(), 2);
        assert_eq!(reconstructed.timestamp_ms, 200);
    }
}
