//! Append-only event log with hash-chain integrity (event sourcing).
//!
//! Every significant orchestration event (plan started, task assigned, gate
//! result, merge attempted, etc.) is recorded as an [`EventEntry`] on an
//! [`EventLog`]. Entries are hash-chained: each entry's content hash
//! includes the hash of the previous entry, making any in-place mutation,
//! deletion, or reordering detectable via [`EventLog::verify_integrity`].
//!
//! The log is append-only by design: there is no public API to modify or
//! remove entries after they have been recorded. The [`snapshot`] /
//! [`restore`] pair supports crash recovery without breaking the
//! hash-chain invariant.
//!
//! [`snapshot`]: EventLog::snapshot
//! [`restore`]: EventLog::restore

use std::sync::Arc;

use parking_lot::Mutex;
use roko_core::ContentHash;
use serde::{Deserialize, Serialize};

// ─── EventKind ──────────────────────────────────────────────────────────

/// Classification of orchestration events stored in the log.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventKind {
    /// A plan has started execution.
    PlanStarted,
    /// A task has been assigned to an agent.
    TaskAssigned,
    /// An agent process has been spawned.
    AgentSpawned,
    /// A gate (compile, test, clippy, etc.) produced a result.
    GateResult,
    /// A merge was attempted.
    MergeAttempted,
    /// A plan completed successfully.
    PlanCompleted,
    /// A plan failed terminally.
    PlanFailed,
    /// An error occurred (fail-loud event).
    ErrorOccurred,
    /// An intervention was fired by a policy.
    InterventionFired,
    /// A phase transition occurred.
    PhaseTransition,
    /// Enrichment data was validated.
    EnrichmentValidated,
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlanStarted => write!(f, "plan.started"),
            Self::TaskAssigned => write!(f, "task.assigned"),
            Self::AgentSpawned => write!(f, "agent.spawned"),
            Self::GateResult => write!(f, "gate.result"),
            Self::MergeAttempted => write!(f, "merge.attempted"),
            Self::PlanCompleted => write!(f, "plan.completed"),
            Self::PlanFailed => write!(f, "plan.failed"),
            Self::ErrorOccurred => write!(f, "error.occurred"),
            Self::InterventionFired => write!(f, "intervention.fired"),
            Self::PhaseTransition => write!(f, "phase.transition"),
            Self::EnrichmentValidated => write!(f, "enrichment.validated"),
        }
    }
}

// ─── EventEntry ─────────────────────────────────────────────────────────

/// A single entry in the event log. Entries are append-only and
/// hash-chained for tamper detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEntry {
    /// Monotonically increasing sequence number (0-based).
    pub sequence_number: u64,
    /// Unix millisecond timestamp of when the event was recorded.
    pub timestamp_ms: i64,
    /// The kind of event.
    pub event_kind: EventKind,
    /// Structured payload (event-specific data).
    pub payload: serde_json::Value,
    /// BLAKE3 content hash of this entry (includes the previous hash).
    pub content_hash: [u8; 32],
}

impl EventEntry {
    /// Compute the content hash for an entry given its fields and the
    /// previous entry's hash. Uses a deterministic canonical encoding.
    fn compute_hash(
        seq: u64,
        ts_ms: i64,
        kind: &EventKind,
        payload: &serde_json::Value,
        prev_hash: &[u8; 32],
    ) -> [u8; 32] {
        let kind_str = kind.to_string();
        // Canonical JSON for payload: serde_json::to_vec is deterministic
        // for Value (keys are sorted in BTreeMap-backed maps, but we use
        // the default HashMap-backed Value whose key order is
        // insertion-order; callers should use consistent construction).
        let payload_bytes = serde_json::to_vec(payload).unwrap_or_default();

        let mut buf: Vec<u8> =
            Vec::with_capacity(8 + 8 + 32 + kind_str.len() + payload_bytes.len() + 16);
        buf.extend_from_slice(b"eventv1|");
        buf.extend_from_slice(&seq.to_be_bytes());
        buf.extend_from_slice(&ts_ms.to_be_bytes());
        buf.extend_from_slice(prev_hash);
        push_lp(&mut buf, kind_str.as_bytes());
        push_lp(&mut buf, &payload_bytes);
        ContentHash::of(&buf).0
    }
}

/// Length-prefixed field push (avoids field-body collisions).
fn push_lp(buf: &mut Vec<u8>, data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(data);
}

fn verify_entry_sequences(entries: &[EventEntry]) -> Result<(), IntegrityError> {
    for (expected, entry) in entries.iter().enumerate() {
        let expected = expected as u64;
        if entry.sequence_number != expected {
            let reason = if expected == 0 {
                format!("sequence starts at {} (expected 0)", entry.sequence_number)
            } else {
                format!(
                    "sequence {} follows {} (expected {})",
                    entry.sequence_number,
                    expected - 1,
                    expected
                )
            };
            return Err(IntegrityError {
                at_sequence: entry.sequence_number,
                reason,
            });
        }
    }

    Ok(())
}

// ─── Integrity error ────────────────────────────────────────────────────

/// Error returned by [`EventLog::verify_integrity`] when tampering is
/// detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityError {
    /// The sequence number at which the chain broke.
    pub at_sequence: u64,
    /// Human-readable explanation.
    pub reason: String,
}

impl std::fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "integrity violation at seq {}: {}",
            self.at_sequence, self.reason
        )
    }
}

impl std::error::Error for IntegrityError {}

// ─── EventLogSnapshot ───────────────────────────────────────────────────

/// Serializable snapshot of the entire event log for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogSnapshot {
    /// All entries in insertion order.
    pub entries: Vec<EventEntry>,
    /// The hash of the most recently appended entry (or zero for empty).
    pub tip: [u8; 32],
}

// ─── Zero hash ──────────────────────────────────────────────────────────

/// The hash used as the previous hash for the first entry.
const ZERO_HASH: [u8; 32] = [0u8; 32];

// ─── EventLog ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct LogInner {
    entries: Vec<EventEntry>,
    tip: [u8; 32],
}

/// Append-only, tamper-evident event log for orchestration events.
///
/// Thread-safe: the internal state is protected by a `parking_lot::Mutex`.
/// The log can be cloned (clones share the same underlying state via
/// `Arc`).
#[derive(Debug, Default, Clone)]
pub struct EventLog {
    inner: Arc<Mutex<LogInner>>,
}

impl EventLog {
    /// Create an empty event log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of events recorded.
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().entries.is_empty()
    }

    /// Append a new event to the log. Returns the fully constructed entry.
    pub fn append(&self, event_kind: EventKind, payload: serde_json::Value) -> EventEntry {
        let mut guard = self.inner.lock();
        let seq = guard.entries.len() as u64;
        let ts_ms = chrono::Utc::now().timestamp_millis();
        let prev_hash = guard.tip;
        let content_hash = EventEntry::compute_hash(seq, ts_ms, &event_kind, &payload, &prev_hash);

        let entry = EventEntry {
            sequence_number: seq,
            timestamp_ms: ts_ms,
            event_kind,
            payload,
            content_hash,
        };

        guard.tip = content_hash;
        guard.entries.push(entry.clone());
        entry
    }

    /// Replay all events in insertion order.
    pub fn replay(&self) -> Vec<EventEntry> {
        self.inner.lock().entries.clone()
    }

    /// Replay events starting from a given sequence number (inclusive).
    pub fn replay_from(&self, seq: u64) -> Vec<EventEntry> {
        let guard = self.inner.lock();
        guard
            .entries
            .iter()
            .filter(|e| e.sequence_number >= seq)
            .cloned()
            .collect()
    }

    /// Verify the hash chain is intact. Returns `Ok(())` if every entry's
    /// content hash matches a recomputation from scratch, or an
    /// [`IntegrityError`] at the first broken link.
    ///
    /// # Errors
    ///
    /// Returns [`IntegrityError`] when an entry hash or the stored tail
    /// hash does not match a recomputation from scratch.
    #[allow(clippy::significant_drop_tightening)]
    pub fn verify_integrity(&self) -> Result<(), IntegrityError> {
        let guard = self.inner.lock();
        verify_entry_sequences(&guard.entries)?;
        let mut prev_hash = ZERO_HASH;

        for entry in &guard.entries {
            let expected = EventEntry::compute_hash(
                entry.sequence_number,
                entry.timestamp_ms,
                &entry.event_kind,
                &entry.payload,
                &prev_hash,
            );
            if entry.content_hash != expected {
                return Err(IntegrityError {
                    at_sequence: entry.sequence_number,
                    reason: format!(
                        "hash mismatch: expected {}, got {}",
                        ContentHash(expected).short(),
                        ContentHash(entry.content_hash).short(),
                    ),
                });
            }
            prev_hash = entry.content_hash;
        }

        if guard.tip != prev_hash {
            return Err(IntegrityError {
                at_sequence: guard.entries.len() as u64,
                reason: "tip hash does not match final entry".into(),
            });
        }

        Ok(())
    }

    /// Serialize the current state as a snapshot for crash recovery.
    pub fn snapshot(&self) -> EventLogSnapshot {
        let guard = self.inner.lock();
        EventLogSnapshot {
            entries: guard.entries.clone(),
            tip: guard.tip,
        }
    }

    /// Restore a log from a previously created snapshot without validation.
    pub fn restore(snapshot: EventLogSnapshot) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LogInner {
                entries: snapshot.entries,
                tip: snapshot.tip,
            })),
        }
    }

    /// Restore a log from a previously created snapshot and verify integrity.
    ///
    /// # Errors
    ///
    /// Returns [`IntegrityError`] if the snapshot contains a broken hash
    /// chain, a mismatched tip hash, or non-contiguous sequence numbers.
    pub fn restore_verified(snapshot: EventLogSnapshot) -> Result<Self, IntegrityError> {
        let log = Self::restore(snapshot);
        log.verify_integrity()?;
        Ok(log)
    }

    /// Return entries filtered by kind.
    pub fn entries_by_kind(&self, kind: &EventKind) -> Vec<EventEntry> {
        self.inner
            .lock()
            .entries
            .iter()
            .filter(|e| &e.event_kind == kind)
            .cloned()
            .collect()
    }

    /// Current tip hash.
    pub fn tip(&self) -> [u8; 32] {
        self.inner.lock().tip
    }

    /// Test-only: mutate an entry to simulate tampering.
    #[cfg(test)]
    pub(crate) fn test_mutate<F>(&self, index: usize, f: F)
    where
        F: FnOnce(&mut EventEntry),
    {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.entries.get_mut(index) {
            f(entry);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_log_verifies() {
        let log = EventLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        assert!(log.verify_integrity().is_ok());
    }

    #[test]
    fn append_increments_sequence() {
        let log = EventLog::new();
        let e0 = log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        let e1 = log.append(EventKind::TaskAssigned, json!({"task": "t1"}));
        let e2 = log.append(EventKind::GateResult, json!({"pass": true}));

        assert_eq!(e0.sequence_number, 0);
        assert_eq!(e1.sequence_number, 1);
        assert_eq!(e2.sequence_number, 2);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn hash_chain_links_entries() {
        let log = EventLog::new();
        let e0 = log.append(EventKind::PlanStarted, json!({}));
        let e1 = log.append(EventKind::AgentSpawned, json!({}));

        // e1's hash must depend on e0's hash (via prev_hash in compute_hash)
        // Verify by recomputing
        let recomputed = EventEntry::compute_hash(
            e1.sequence_number,
            e1.timestamp_ms,
            &e1.event_kind,
            &e1.payload,
            &e0.content_hash,
        );
        assert_eq!(e1.content_hash, recomputed);
    }

    #[test]
    fn verify_integrity_passes_on_valid_chain() {
        let log = EventLog::new();
        for i in 0..100 {
            log.append(EventKind::TaskAssigned, json!({"i": i}));
        }
        assert!(log.verify_integrity().is_ok());
    }

    #[test]
    fn tamper_with_payload_detected() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        log.append(EventKind::TaskAssigned, json!({"task": "t1"}));
        log.append(EventKind::GateResult, json!({"pass": true}));
        assert!(log.verify_integrity().is_ok());

        // Tamper with the middle entry's payload
        log.test_mutate(1, |e| {
            e.payload = json!({"task": "HIJACKED"});
        });
        let err = log.verify_integrity().unwrap_err();
        assert_eq!(err.at_sequence, 1);
    }

    #[test]
    fn tamper_with_content_hash_detected() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({}));
        log.append(EventKind::PlanCompleted, json!({}));
        assert!(log.verify_integrity().is_ok());

        // Flip a byte in the first entry's content hash
        log.test_mutate(0, |e| {
            e.content_hash[0] ^= 0xFF;
        });
        assert!(log.verify_integrity().is_err());
    }

    #[test]
    fn replay_returns_all_events_in_order() {
        let log = EventLog::new();
        let e0 = log.append(EventKind::PlanStarted, json!({"a": 1}));
        let e1 = log.append(EventKind::PlanCompleted, json!({"a": 2}));

        let all = log.replay();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], e0);
        assert_eq!(all[1], e1);
    }

    #[test]
    fn replay_from_filters_by_sequence() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({}));
        log.append(EventKind::TaskAssigned, json!({}));
        let e2 = log.append(EventKind::GateResult, json!({}));
        let e3 = log.append(EventKind::PlanCompleted, json!({}));

        let from_2 = log.replay_from(2);
        assert_eq!(from_2.len(), 2);
        assert_eq!(from_2[0], e2);
        assert_eq!(from_2[1], e3);

        let from_10 = log.replay_from(10);
        assert!(from_10.is_empty());
    }

    #[test]
    fn snapshot_and_restore_roundtrip() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        log.append(EventKind::AgentSpawned, json!({"agent": "a1"}));
        log.append(EventKind::GateResult, json!({"pass": false}));

        let snap = log.snapshot();
        let restored = EventLog::restore_verified(snap).unwrap();

        assert_eq!(restored.len(), 3);
        assert_eq!(restored.tip(), log.tip());
        assert!(restored.verify_integrity().is_ok());

        // The replays should be identical
        assert_eq!(restored.replay(), log.replay());
    }

    #[test]
    fn snapshot_restore_preserves_integrity_after_more_appends() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({}));
        let snap = log.snapshot();

        let restored = EventLog::restore_verified(snap).unwrap();
        restored.append(EventKind::PlanCompleted, json!({}));

        assert_eq!(restored.len(), 2);
        assert!(restored.verify_integrity().is_ok());
    }

    #[test]
    fn verify_integrity_rejects_non_zero_start() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        log.test_mutate(0, |entry| {
            entry.sequence_number = 1;
        });

        let err = log.verify_integrity().unwrap_err();
        assert_eq!(err.at_sequence, 1);
        assert!(err.reason.contains("expected 0"));
    }

    #[test]
    fn verify_integrity_rejects_sequence_gap() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        log.append(EventKind::TaskAssigned, json!({"task": "t1"}));
        log.test_mutate(1, |entry| {
            entry.sequence_number = 2;
        });

        let err = log.verify_integrity().unwrap_err();
        assert_eq!(err.at_sequence, 2);
        assert!(err.reason.contains("expected 1"));
    }

    #[test]
    fn restore_verified_rejects_invalid_snapshot_sequences() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({"plan": "p1"}));
        log.append(EventKind::TaskAssigned, json!({"task": "t1"}));

        let mut snapshot = log.snapshot();
        snapshot.entries[1].sequence_number = 3;

        let err = EventLog::restore_verified(snapshot).unwrap_err();
        assert_eq!(err.at_sequence, 3);
        assert!(err.reason.contains("expected 1"));
    }

    #[test]
    fn entries_by_kind_filters() {
        let log = EventLog::new();
        log.append(EventKind::PlanStarted, json!({}));
        log.append(EventKind::ErrorOccurred, json!({"err": "boom"}));
        log.append(EventKind::ErrorOccurred, json!({"err": "bang"}));
        log.append(EventKind::PlanFailed, json!({}));

        let errors = log.entries_by_kind(&EventKind::ErrorOccurred);
        assert_eq!(errors.len(), 2);
        assert!(errors
            .iter()
            .all(|e| e.event_kind == EventKind::ErrorOccurred));
    }

    #[test]
    fn event_kind_display() {
        assert_eq!(EventKind::PlanStarted.to_string(), "plan.started");
        assert_eq!(EventKind::ErrorOccurred.to_string(), "error.occurred");
        assert_eq!(
            EventKind::InterventionFired.to_string(),
            "intervention.fired"
        );
    }

    #[test]
    fn concurrent_appends_preserve_integrity() {
        use std::thread;

        let log = EventLog::new();
        let mut handles = Vec::new();
        for t in 0..4 {
            let l = log.clone();
            handles.push(thread::spawn(move || {
                for i in 0..25 {
                    l.append(EventKind::TaskAssigned, json!({"thread": t, "iter": i}));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(log.len(), 100);
        assert!(log.verify_integrity().is_ok());
    }

    #[test]
    fn integrity_error_display() {
        let err = IntegrityError {
            at_sequence: 42,
            reason: "hash mismatch".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("42"));
        assert!(msg.contains("hash mismatch"));
    }
}
