//! Hash-linked append-only audit log (parity §28.3).
//!
//! Every privileged operation (capability consumption, permit issuance,
//! sandbox boundary crossing, loop-guard trip) records an [`AuditEntry`]
//! on the chain. Each entry carries the hash of the *previous* entry in
//! its `prev_hash` field; the entry's own content hash is derived from
//! that link plus the entry body. Any mutation of a historical entry
//! invalidates every subsequent hash — [`AuditChain::verify`] detects
//! tampering by walking the chain and recomputing hashes.
//!
//! # Design
//!
//! - Hash function: BLAKE3 via [`roko_core::ContentHash`]. The content
//!   hashed for each entry is a canonical serialization of
//!   `(prev_hash, kind, actor, resource, ts_ms, signature)`.
//! - Storage: in-memory `Vec<AuditEntry>` guarded by a
//!   [`parking_lot::Mutex`]. Persistence is a concern for a separate
//!   module — this type provides tamper-evident semantics first.
//! - Genesis: the first entry's `prev_hash` is all-zero (`[0u8; 32]`).
//!
//! # Tamper model
//!
//! `verify()` returns `false` if any entry's `prev_hash` does not match
//! the recomputed hash of its predecessor, or if the genesis entry does
//! not start from the zero hash. This catches in-place mutation,
//! reordering, insertion, and deletion in the middle of the chain.

use std::sync::Arc;

use parking_lot::Mutex;
use roko_core::ContentHash;
use serde::{Deserialize, Serialize};

/// A single entry on the audit chain. Entries are **append-only** —
/// there is no public API to modify one after it has been recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Hash of the preceding entry. Zeroed for the genesis entry.
    pub prev_hash: [u8; 32],
    /// Operation kind, e.g. `"capability.issued"` or
    /// `"sandbox.violation"`.
    pub kind: String,
    /// Actor that triggered the operation (agent id, role, service).
    pub actor: String,
    /// Resource the operation targeted (worktree path, permit id,
    /// capability id).
    pub resource: String,
    /// Unix millisecond timestamp of when the entry was recorded.
    pub ts_ms: i64,
    /// Optional detached signature over the entry body. Not interpreted
    /// by the chain itself — it is hashed like any other field so it
    /// becomes part of the tamper-evident envelope.
    pub signature: Option<String>,
}

impl AuditEntry {
    /// Build a new entry with the current wall-clock timestamp. The
    /// `prev_hash` should be the output of [`AuditChain::tip`] (or
    /// `[0; 32]` for the genesis entry) — callers normally reach this
    /// through [`AuditChain::append`] which wires the link automatically.
    #[must_use]
    pub fn new(
        prev_hash: [u8; 32],
        kind: impl Into<String>,
        actor: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            prev_hash,
            kind: kind.into(),
            actor: actor.into(),
            resource: resource.into(),
            ts_ms: chrono::Utc::now().timestamp_millis(),
            signature: None,
        }
    }

    /// Attach a detached signature string. The signature is part of the
    /// hashed envelope, so changing it after append invalidates the
    /// chain.
    #[must_use]
    pub fn with_signature(mut self, signature: impl Into<String>) -> Self {
        self.signature = Some(signature.into());
        self
    }

    /// Compute the content hash of this entry. The hash is taken over a
    /// deterministic byte serialization of every field, so any mutation
    /// yields a different hash.
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        // Hand-rolled canonical encoding: field tags + length-prefixed
        // bodies. We avoid `serde_json` here so the format is stable
        // across serde versions and cannot be perturbed by map ordering.
        let mut buf: Vec<u8> = Vec::with_capacity(
            32 + 8
                + self.kind.len()
                + self.actor.len()
                + self.resource.len()
                + self.signature.as_ref().map_or(0, String::len)
                + 48,
        );
        buf.extend_from_slice(b"auditv1|");
        buf.extend_from_slice(&self.prev_hash);
        push_field(&mut buf, b"kind", self.kind.as_bytes());
        push_field(&mut buf, b"actor", self.actor.as_bytes());
        push_field(&mut buf, b"resource", self.resource.as_bytes());
        push_field(&mut buf, b"ts_ms", &self.ts_ms.to_be_bytes());
        match &self.signature {
            Some(sig) => push_field(&mut buf, b"sig+", sig.as_bytes()),
            None => push_field(&mut buf, b"sig-", b""),
        }
        ContentHash::of(&buf).0
    }
}

fn push_field(buf: &mut Vec<u8>, tag: &[u8], body: &[u8]) {
    buf.push(b'|');
    buf.extend_from_slice(tag);
    buf.push(b'=');
    // 4-byte big-endian length prefix prevents field-body collisions.
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(body);
}

/// Zero-hash used as the `prev_hash` of the genesis entry.
pub const GENESIS_PREV_HASH: [u8; 32] = [0u8; 32];

/// Append-only, tamper-evident log of privileged operations.
///
/// The chain is safe to share across threads: internally it uses a
/// `parking_lot::Mutex` to serialize access to the underlying entry
/// vector.
#[derive(Debug, Default, Clone)]
pub struct AuditChain {
    inner: Arc<Mutex<ChainInner>>,
}

#[derive(Debug, Default)]
struct ChainInner {
    entries: Vec<AuditEntry>,
    tip: [u8; 32],
}

impl AuditChain {
    /// Create an empty chain.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of entries recorded on the chain.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// `true` if no entries have been appended yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().entries.is_empty()
    }

    /// Current chain tip (hash of the most recent entry). Equal to
    /// [`GENESIS_PREV_HASH`] when the chain is empty.
    #[must_use]
    pub fn tip(&self) -> [u8; 32] {
        self.inner.lock().tip
    }

    /// Append an entry to the chain. The caller supplies a partially
    /// constructed entry — this method overwrites `prev_hash` to point
    /// at the current tip, records the entry, and returns a clone of
    /// the fully linked entry.
    ///
    /// The returned entry's `content_hash()` matches the new tip.
    pub fn append(&self, mut entry: AuditEntry) -> AuditEntry {
        let mut guard = self.inner.lock();
        entry.prev_hash = guard.tip;
        let hash = entry.content_hash();
        guard.tip = hash;
        guard.entries.push(entry.clone());
        entry
    }

    /// Record a new operation in one call. Equivalent to
    /// `append(AuditEntry::new(...))` but doesn't require the caller to
    /// know the current tip.
    pub fn record(
        &self,
        kind: impl Into<String>,
        actor: impl Into<String>,
        resource: impl Into<String>,
    ) -> AuditEntry {
        let mut guard = self.inner.lock();
        let prev = guard.tip;
        let entry = AuditEntry::new(prev, kind, actor, resource);
        let hash = entry.content_hash();
        guard.tip = hash;
        guard.entries.push(entry.clone());
        entry
    }

    /// Verify the chain has not been tampered with.
    ///
    /// Returns `true` when:
    /// - the genesis entry's `prev_hash` equals [`GENESIS_PREV_HASH`],
    /// - every other entry's `prev_hash` equals the preceding entry's
    ///   content hash,
    /// - the cached tip equals the final entry's content hash (or the
    ///   genesis hash if the chain is empty).
    #[must_use]
    pub fn verify(&self) -> bool {
        let guard = self.inner.lock();
        if guard.entries.is_empty() {
            return guard.tip == GENESIS_PREV_HASH;
        }
        let mut expected_prev = GENESIS_PREV_HASH;
        for entry in &guard.entries {
            if entry.prev_hash != expected_prev {
                return false;
            }
            expected_prev = entry.content_hash();
        }
        guard.tip == expected_prev
    }

    /// Snapshot the current entries. Returns a cloned `Vec`; the
    /// chain's internal buffer is not exposed directly so that entries
    /// remain append-only.
    ///
    /// Named `iter` for API symmetry with the parity spec; the chain
    /// intentionally exposes a snapshot `Vec` rather than a borrowed
    /// iterator so that the internal mutex is released immediately.
    #[allow(clippy::iter_not_returning_iterator)]
    #[must_use]
    pub fn iter(&self) -> Vec<AuditEntry> {
        self.inner.lock().entries.clone()
    }

    /// Return only entries whose `kind` field starts with the given
    /// prefix. Useful for filtering, e.g. all `capability.*` events.
    #[must_use]
    pub fn entries_with_kind_prefix(&self, prefix: &str) -> Vec<AuditEntry> {
        self.inner
            .lock()
            .entries
            .iter()
            .filter(|e| e.kind.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Corruption hook used **only by tests in this crate**: returns a
    /// clone of the raw chain for verification of the hash strategy.
    /// Gated behind `cfg(test)` so production code cannot observe the
    /// internal vector directly.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn test_inner_clone(&self) -> Vec<AuditEntry> {
        self.inner.lock().entries.clone()
    }

    /// Corruption hook used **only by tests in this crate**: mutate the
    /// entry at `index`. The chain's cached tip is left unchanged so
    /// `verify()` detects the mutation as tampering. Gated behind
    /// `cfg(test)` so production code can never rewrite history.
    #[cfg(test)]
    pub(crate) fn test_mutate<F>(&self, index: usize, f: F) -> bool
    where
        F: FnOnce(&mut AuditEntry),
    {
        let mut guard = self.inner.lock();
        guard.entries.get_mut(index).is_some_and(|entry| {
            f(entry);
            true
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::missing_panics_doc)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_verifies() {
        let chain = AuditChain::new();
        assert_eq!(chain.len(), 0);
        assert!(chain.is_empty());
        assert_eq!(chain.tip(), GENESIS_PREV_HASH);
        assert!(chain.verify());
        assert!(chain.iter().is_empty());
    }

    #[test]
    fn single_entry_links_to_genesis_and_verifies() {
        let chain = AuditChain::new();
        let entry = chain.record("capability.issued", "agent-42", "worktree:plan-7");
        assert_eq!(chain.len(), 1);
        assert!(!chain.is_empty());
        assert_eq!(entry.prev_hash, GENESIS_PREV_HASH);
        assert_eq!(chain.tip(), entry.content_hash());
        assert!(chain.verify());
    }

    #[test]
    fn multiple_entries_form_a_contiguous_chain() {
        let chain = AuditChain::new();
        let a = chain.record("capability.issued", "agent-a", "r1");
        let b = chain.record("capability.consumed", "agent-a", "r1");
        let c = chain.record("permit.committed", "agent-b", "r2");

        assert_eq!(chain.len(), 3);
        assert_eq!(a.prev_hash, GENESIS_PREV_HASH);
        assert_eq!(b.prev_hash, a.content_hash());
        assert_eq!(c.prev_hash, b.content_hash());
        assert_eq!(chain.tip(), c.content_hash());
        assert!(chain.verify());
    }

    #[test]
    fn tamper_with_middle_entry_breaks_verification() {
        let chain = AuditChain::new();
        chain.record("capability.issued", "agent-a", "r1");
        chain.record("capability.consumed", "agent-a", "r1");
        chain.record("permit.committed", "agent-b", "r2");
        assert!(chain.verify());

        let mutated = chain.test_mutate(1, |entry| {
            entry.resource = "r-MODIFIED".to_string();
        });
        assert!(mutated);
        assert!(
            !chain.verify(),
            "tampering with middle entry must be detected"
        );
    }

    #[test]
    fn tamper_with_first_entry_breaks_verification() {
        let chain = AuditChain::new();
        chain.record("sandbox.entered", "agent-a", "job-1");
        chain.record("sandbox.exited", "agent-a", "job-1");
        assert!(chain.verify());

        chain.test_mutate(0, |entry| {
            entry.actor = "agent-HIJACK".to_string();
        });
        assert!(!chain.verify());
    }

    #[test]
    fn tamper_with_prev_hash_link_breaks_verification() {
        let chain = AuditChain::new();
        chain.record("loop.tripped", "guard", "scope-x");
        chain.record("loop.tripped", "guard", "scope-y");
        assert!(chain.verify());

        chain.test_mutate(1, |entry| {
            // Flip a byte in the prev_hash so the link is broken.
            entry.prev_hash[0] ^= 0xff;
        });
        assert!(!chain.verify());
    }

    #[test]
    fn tamper_with_signature_is_detected() {
        let chain = AuditChain::new();
        chain.append(
            AuditEntry::new(GENESIS_PREV_HASH, "permit.issued", "agent-a", "permit-1")
                .with_signature("sig-ok"),
        );
        chain.record("permit.committed", "agent-a", "permit-1");
        assert!(chain.verify());

        chain.test_mutate(0, |entry| {
            entry.signature = Some("sig-FORGED".to_string());
        });
        assert!(!chain.verify());
    }

    #[test]
    fn reordering_entries_breaks_verification() {
        let chain = AuditChain::new();
        chain.record("sandbox.entered", "a", "j1");
        chain.record("sandbox.exited", "a", "j1");
        chain.record("sandbox.entered", "b", "j2");
        assert!(chain.verify());

        // Swap two entries; prev_hash links no longer align.
        let mut snapshot = chain.test_inner_clone();
        snapshot.swap(0, 2);
        for (i, entry) in snapshot.into_iter().enumerate() {
            chain.test_mutate(i, |slot| *slot = entry);
        }
        assert!(!chain.verify());
    }

    #[test]
    fn iter_returns_entries_in_insertion_order() {
        let chain = AuditChain::new();
        let a = chain.record("k1", "actor", "r");
        let b = chain.record("k2", "actor", "r");
        let c = chain.record("k3", "actor", "r");
        let entries = chain.iter();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], a);
        assert_eq!(entries[1], b);
        assert_eq!(entries[2], c);
    }

    #[test]
    fn entries_with_kind_prefix_filters_correctly() {
        let chain = AuditChain::new();
        chain.record("capability.issued", "a", "r1");
        chain.record("capability.consumed", "a", "r1");
        chain.record("permit.committed", "b", "r2");
        chain.record("sandbox.entered", "b", "j1");

        let caps = chain.entries_with_kind_prefix("capability.");
        assert_eq!(caps.len(), 2);
        assert!(caps.iter().all(|e| e.kind.starts_with("capability.")));

        let perms = chain.entries_with_kind_prefix("permit.");
        assert_eq!(perms.len(), 1);
        assert_eq!(perms[0].kind, "permit.committed");

        assert!(chain.entries_with_kind_prefix("nope").is_empty());
    }

    #[test]
    fn append_overwrites_caller_prev_hash_with_current_tip() {
        let chain = AuditChain::new();
        chain.record("first", "a", "r");
        // Caller passes a garbage prev_hash; append() must overwrite it.
        let garbage = [0xAA_u8; 32];
        let entry = chain.append(AuditEntry::new(garbage, "second", "a", "r"));
        assert_ne!(entry.prev_hash, garbage);
        assert_eq!(entry.prev_hash, chain.iter()[0].content_hash());
        assert!(chain.verify());
    }

    #[test]
    fn content_hash_is_deterministic() {
        // Two entries with identical fields must hash identically.
        let a = AuditEntry {
            prev_hash: [7u8; 32],
            kind: "k".into(),
            actor: "a".into(),
            resource: "r".into(),
            ts_ms: 1_700_000_000_000,
            signature: Some("sig".into()),
        };
        let b = a.clone();
        assert_eq!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn content_hash_distinguishes_field_changes() {
        let base = AuditEntry {
            prev_hash: [0u8; 32],
            kind: "k".into(),
            actor: "a".into(),
            resource: "r".into(),
            ts_ms: 42,
            signature: None,
        };
        let h0 = base.content_hash();

        let mut m_kind = base.clone();
        m_kind.kind = "kk".into();
        assert_ne!(m_kind.content_hash(), h0);

        let mut m_actor = base.clone();
        m_actor.actor = "aa".into();
        assert_ne!(m_actor.content_hash(), h0);

        let mut m_resource = base.clone();
        m_resource.resource = "rr".into();
        assert_ne!(m_resource.content_hash(), h0);

        let mut m_ts = base.clone();
        m_ts.ts_ms = 43;
        assert_ne!(m_ts.content_hash(), h0);

        let mut m_sig = base.clone();
        m_sig.signature = Some(String::new());
        assert_ne!(m_sig.content_hash(), h0);

        let mut m_prev = base;
        m_prev.prev_hash[0] = 1;
        assert_ne!(m_prev.content_hash(), h0);
    }

    #[test]
    fn thousand_entries_verify_fast() {
        let chain = AuditChain::new();
        for i in 0..1_000 {
            chain.record("bulk", "agent", format!("resource-{i}"));
        }
        assert_eq!(chain.len(), 1_000);
        assert!(chain.verify());
    }

    #[test]
    fn chain_clone_shares_state() {
        let chain = AuditChain::new();
        let handle = chain.clone();
        chain.record("k", "a", "r");
        assert_eq!(handle.len(), 1);
        assert_eq!(handle.tip(), chain.tip());
    }

    #[test]
    fn concurrent_append_from_multiple_threads_keeps_chain_valid() {
        use std::thread;

        let chain = AuditChain::new();
        let mut handles = Vec::new();
        for t in 0..8 {
            let c = chain.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    c.record("concurrent", format!("t{t}"), format!("i{i}"));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(chain.len(), 8 * 50);
        assert!(chain.verify());
    }

    #[test]
    fn with_signature_is_hashed() {
        let chain = AuditChain::new();
        let a =
            chain.append(AuditEntry::new(GENESIS_PREV_HASH, "k", "a", "r").with_signature("s1"));
        assert_eq!(a.signature.as_deref(), Some("s1"));
        assert!(chain.verify());

        // Tip reflects the content hash of the entry (including its signature).
        assert_eq!(chain.tip(), a.content_hash());
    }

    #[test]
    fn deletion_of_last_entry_leaves_tip_mismatch() {
        let chain = AuditChain::new();
        chain.record("k1", "a", "r");
        chain.record("k2", "a", "r");
        assert!(chain.verify());

        // Simulate a drop of the last entry WITHOUT updating the tip.
        // Access inner via test hook: we can't pop directly, but we can
        // overwrite the last entry with a synthetic one whose content
        // hash differs from the cached tip.
        chain.test_mutate(1, |e| {
            e.kind = "k2-MUTATED".to_string();
        });
        assert!(!chain.verify());
    }
}
