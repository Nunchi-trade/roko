//! Taint propagation for Roko safety (MORI-PARITY-CHECKLIST §28.9).
//!
//! `TaintTracker` records how tainted information flows through signal
//! lineage. When a signal with `Provenance::tainted == true` is used as an
//! input, any derived signal becomes tainted too. Sinks that need to refuse
//! tainted data (git commits, network egress, signal emits) consult
//! [`TaintTracker::is_tainted`] before proceeding.
//!
//! The tracker stores one [`TaintReason`] per [`ContentHash`] keyed in a
//! `HashMap` behind a `parking_lot::Mutex`, so multiple executor tasks may
//! consult/update it concurrently without deadlock risk.
//!
//! # Example
//!
//! ```
//! use roko_core::ContentHash;
//! use roko_orchestrator::safety::taint_propagation::{TaintTracker, TaintReason};
//!
//! let tracker = TaintTracker::new();
//! let source = ContentHash::of(b"user input");
//! let derived = ContentHash::of(b"parsed user input");
//!
//! tracker.mark_tainted(source, TaintReason::external("webhook"));
//! tracker.propagate(&[source], derived);
//!
//! assert!(tracker.is_tainted(&derived));
//! ```

use parking_lot::Mutex;
use roko_core::{ContentHash, Engram, TaintInfo};
use std::collections::HashMap;

/// Why a particular [`ContentHash`] is considered tainted.
///
/// Reasons are informational — they travel with the taint flag so that
/// downstream audits can explain refusals ("refused: came from untrusted
/// webhook"). They are not interpreted semantically by the tracker itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaintReason {
    /// Short machine-readable category (e.g. `"external"`, `"user_input"`,
    /// `"propagated"`).
    pub category: String,
    /// Human-readable explanation; kept brief to stay loggable.
    pub detail: String,
    /// Upstream tainted hash that caused propagation, when known.
    pub inherited_from: Option<ContentHash>,
}

impl TaintReason {
    /// Build a new [`TaintReason`] from a category tag and an explanation.
    pub fn new(category: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            detail: detail.into(),
            inherited_from: None,
        }
    }

    /// Convenience constructor for taint coming from an external source.
    pub fn external(detail: impl Into<String>) -> Self {
        Self::new("external", detail)
    }

    /// Convenience constructor for taint coming from user input.
    pub fn user_input(detail: impl Into<String>) -> Self {
        Self::new("user_input", detail)
    }

    /// Convenience constructor for taint that was inherited from a parent
    /// signal during propagation.
    pub fn propagated(detail: impl Into<String>) -> Self {
        Self::new("propagated", detail)
    }

    /// Convert this tracker-local reason into the shared provenance metadata shape.
    #[must_use]
    pub fn to_taint_info(&self) -> TaintInfo {
        let mut info = TaintInfo::new(self.category.clone(), self.detail.clone());
        info.inherited_from = self.inherited_from;
        info
    }

    fn from_taint_info(info: &TaintInfo) -> Self {
        Self {
            category: info.category.clone(),
            detail: info.detail.clone(),
            inherited_from: info.inherited_from,
        }
    }
}

/// Tracks taint status across a signal DAG.
///
/// A [`TaintTracker`] is cheap to create and safe to share across threads
/// (`Arc<TaintTracker>` is the expected sharing pattern). All mutating
/// methods take `&self` because internal state is protected by a
/// `parking_lot::Mutex`.
///
/// # Semantics
///
/// * [`mark_tainted`](Self::mark_tainted) stamps a hash with a reason.
///   Calling it twice overwrites the reason (last writer wins), which is
///   fine — taint is a boolean-with-annotation, not a vote.
/// * [`propagate`](Self::propagate) marks `child` tainted if **any** parent
///   is already tainted. If no parent is tainted, the child is left alone
///   (a clean child must not become tainted by being combined with other
///   clean signals).
/// * [`is_tainted`](Self::is_tainted) is a pure read.
/// * [`reason`](Self::reason) returns the stored reason, if any.
#[derive(Debug, Default)]
pub struct TaintTracker {
    inner: Mutex<HashMap<ContentHash, TaintReason>>,
}

impl TaintTracker {
    /// Construct a fresh, empty tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark `hash` tainted with the given `reason`. Overwrites any prior
    /// reason for the same hash.
    pub fn mark_tainted(&self, hash: ContentHash, reason: TaintReason) {
        self.inner.lock().insert(hash, reason);
    }

    /// Returns `true` if `hash` has been marked tainted at any point.
    #[must_use]
    pub fn is_tainted(&self, hash: &ContentHash) -> bool {
        self.inner.lock().contains_key(hash)
    }

    /// Retrieve the [`TaintReason`] stored for `hash`, if any.
    #[must_use]
    pub fn reason(&self, hash: &ContentHash) -> Option<TaintReason> {
        self.inner.lock().get(hash).cloned()
    }

    /// Retrieve structured taint metadata suitable for storing in provenance.
    #[must_use]
    pub fn taint_info(&self, hash: &ContentHash) -> Option<TaintInfo> {
        self.reason(hash).map(|reason| reason.to_taint_info())
    }

    /// Propagate taint from parents to `child`.
    ///
    /// If any parent in `parents` is currently tainted, `child` is marked
    /// tainted with a `"propagated"` reason that names the offending
    /// parent. If multiple parents are tainted, the first-encountered
    /// parent is cited for the reason, but all of them would have caused
    /// the propagation.
    ///
    /// If `child` was already tainted with a more specific (non-
    /// propagated) reason, that reason is preserved — we never weaken a
    /// concrete `external` / `user_input` explanation with a generic
    /// `propagated` one.
    ///
    /// Returns `true` if taint was actually propagated; `false` if no
    /// parent was tainted and `child` was left untouched.
    pub fn propagate(&self, parents: &[ContentHash], child: ContentHash) -> bool {
        let mut guard = self.inner.lock();
        let first_tainted_parent = parents.iter().find(|p| guard.contains_key(p)).copied();
        first_tainted_parent.is_some_and(|parent| {
            // Preserve any pre-existing, stronger reason for `child`
            // (anything that isn't itself just "propagated").
            let already_specific = guard
                .get(&child)
                .is_some_and(|r| r.category != "propagated");
            if !already_specific {
                let mut reason =
                    TaintReason::propagated(format!("inherited from {}", parent.short()));
                reason.inherited_from = Some(parent);
                guard.insert(child, reason);
            }
            true
        })
    }

    /// Inspect a [`Engram`] and, if its provenance is tainted, mark it in
    /// the tracker with an `"external"` reason naming the signal's author.
    ///
    /// Returns `true` if the signal was (or already was) tainted, `false`
    /// if the signal's provenance is clean.
    pub fn observe_signal(&self, signal: &Engram) -> bool {
        if signal.provenance.tainted {
            let reason = signal
                .provenance
                .taint_info
                .as_ref()
                .map(TaintReason::from_taint_info)
                .unwrap_or_else(|| {
                    TaintReason::external(format!("signal author {}", signal.provenance.author))
                });
            self.mark_tainted(signal.id, reason);
            true
        } else {
            false
        }
    }

    /// Forget all recorded taint. Useful between isolated runs/tests.
    pub fn clear(&self) {
        self.inner.lock().clear();
    }

    /// Number of tainted hashes currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// `true` if no taint has been recorded (yet).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    fn h(tag: &[u8]) -> ContentHash {
        ContentHash::of(tag)
    }

    #[test]
    fn mark_tainted_records_hash() {
        let tracker = TaintTracker::new();
        let id = h(b"one");
        assert!(!tracker.is_tainted(&id));
        tracker.mark_tainted(id, TaintReason::user_input("CLI flag"));
        assert!(tracker.is_tainted(&id));
    }

    #[test]
    fn is_tainted_on_clean_returns_false() {
        let tracker = TaintTracker::new();
        let id = h(b"never-touched");
        assert!(!tracker.is_tainted(&id));
        assert!(tracker.is_empty());
        assert_eq!(tracker.len(), 0);
    }

    #[test]
    fn propagate_via_single_parent() {
        let tracker = TaintTracker::new();
        let parent = h(b"p");
        let child = h(b"c");
        tracker.mark_tainted(parent, TaintReason::external("api"));
        let propagated = tracker.propagate(&[parent], child);
        assert!(propagated);
        assert!(tracker.is_tainted(&child));
    }

    #[test]
    fn propagate_via_multiple_parents_when_one_tainted() {
        let tracker = TaintTracker::new();
        let clean_parent = h(b"clean");
        let dirty_parent = h(b"dirty");
        let child = h(b"child");
        tracker.mark_tainted(dirty_parent, TaintReason::user_input("stdin"));
        let propagated = tracker.propagate(&[clean_parent, dirty_parent], child);
        assert!(propagated);
        assert!(tracker.is_tainted(&child));
        assert!(!tracker.is_tainted(&clean_parent));
    }

    #[test]
    fn propagate_no_ops_when_all_parents_clean() {
        let tracker = TaintTracker::new();
        let p1 = h(b"p1");
        let p2 = h(b"p2");
        let child = h(b"child");
        let propagated = tracker.propagate(&[p1, p2], child);
        assert!(!propagated);
        assert!(!tracker.is_tainted(&child));
    }

    #[test]
    fn propagate_with_empty_parents_is_noop() {
        let tracker = TaintTracker::new();
        let child = h(b"lonely");
        let propagated = tracker.propagate(&[], child);
        assert!(!propagated);
        assert!(!tracker.is_tainted(&child));
    }

    #[test]
    fn reason_retrieval_returns_stored_reason() {
        let tracker = TaintTracker::new();
        let id = h(b"r");
        tracker.mark_tainted(id, TaintReason::external("webhook"));
        let reason = tracker.reason(&id).expect("reason should be present");
        assert_eq!(reason.category, "external");
        assert_eq!(reason.detail, "webhook");
    }

    #[test]
    fn taint_info_roundtrips_from_reason() {
        let tracker = TaintTracker::new();
        let id = h(b"info");
        tracker.mark_tainted(id, TaintReason::external("webhook"));
        let info = tracker
            .taint_info(&id)
            .expect("taint info should be present");
        assert_eq!(info.category, "external");
        assert_eq!(info.detail, "webhook");
        assert_eq!(info.inherited_from, None);
    }

    #[test]
    fn reason_on_clean_returns_none() {
        let tracker = TaintTracker::new();
        assert!(tracker.reason(&h(b"absent")).is_none());
    }

    #[test]
    fn mark_twice_overwrites_reason() {
        let tracker = TaintTracker::new();
        let id = h(b"twice");
        tracker.mark_tainted(id, TaintReason::external("first"));
        tracker.mark_tainted(id, TaintReason::user_input("second"));
        let reason = tracker.reason(&id).expect("reason present");
        assert_eq!(reason.category, "user_input");
        assert_eq!(reason.detail, "second");
    }

    #[test]
    fn propagate_reason_cites_parent() {
        let tracker = TaintTracker::new();
        let parent = h(b"parent");
        let child = h(b"child");
        tracker.mark_tainted(parent, TaintReason::external("api"));
        tracker.propagate(&[parent], child);
        let reason = tracker.reason(&child).expect("child reason");
        assert_eq!(reason.category, "propagated");
        assert!(reason.detail.contains(&parent.short()));
        assert_eq!(reason.inherited_from, Some(parent));
    }

    #[test]
    fn transitive_propagation_spreads_taint() {
        // a -> b -> c: taint at `a` reaches `c` via two propagate calls.
        let tracker = TaintTracker::new();
        let a = h(b"a");
        let b = h(b"b");
        let c = h(b"c");
        tracker.mark_tainted(a, TaintReason::external("root"));
        assert!(tracker.propagate(&[a], b));
        assert!(tracker.propagate(&[b], c));
        assert!(tracker.is_tainted(&a));
        assert!(tracker.is_tainted(&b));
        assert!(tracker.is_tainted(&c));
    }

    #[test]
    fn propagate_preserves_stronger_preexisting_reason() {
        // Child is already marked with a specific reason; propagate must
        // not overwrite it with a weaker "propagated" reason.
        let tracker = TaintTracker::new();
        let parent = h(b"parent");
        let child = h(b"child");
        tracker.mark_tainted(parent, TaintReason::external("api"));
        tracker.mark_tainted(child, TaintReason::user_input("kept"));
        assert!(tracker.propagate(&[parent], child));
        let r = tracker.reason(&child).expect("reason");
        assert_eq!(r.category, "user_input");
        assert_eq!(r.detail, "kept");
    }

    #[test]
    fn propagate_upgrades_from_propagated_reason() {
        // If child's current reason is already "propagated", a new
        // propagate call may refresh the parent citation.
        let tracker = TaintTracker::new();
        let p1 = h(b"p1");
        let p2 = h(b"p2");
        let child = h(b"c");
        tracker.mark_tainted(p1, TaintReason::external("a"));
        tracker.mark_tainted(p2, TaintReason::external("b"));
        tracker.propagate(&[p1], child);
        let r1 = tracker.reason(&child).expect("reason");
        assert_eq!(r1.category, "propagated");
        tracker.propagate(&[p2], child);
        let r2 = tracker.reason(&child).expect("reason");
        assert_eq!(r2.category, "propagated");
        assert!(r2.detail.contains(&p2.short()));
    }

    #[test]
    fn clear_drops_all_state() {
        let tracker = TaintTracker::new();
        tracker.mark_tainted(h(b"x"), TaintReason::external("x"));
        tracker.mark_tainted(h(b"y"), TaintReason::external("y"));
        assert_eq!(tracker.len(), 2);
        tracker.clear();
        assert!(tracker.is_empty());
        assert!(!tracker.is_tainted(&h(b"x")));
        assert!(!tracker.is_tainted(&h(b"y")));
    }

    #[test]
    fn observe_signal_marks_tainted_provenance() {
        use roko_core::{Body, Engram, Kind};

        let tainted_signal = Engram::builder(Kind::AgentOutput)
            .body(Body::text("external payload"))
            .provenance(roko_core::Provenance::external("webhook"))
            .build();
        let clean_signal = Engram::builder(Kind::AgentOutput)
            .body(Body::text("internal payload"))
            .provenance(roko_core::Provenance::trusted("orchestrator"))
            .build();

        let tracker = TaintTracker::new();
        assert!(tracker.observe_signal(&tainted_signal));
        assert!(!tracker.observe_signal(&clean_signal));

        assert!(tracker.is_tainted(&tainted_signal.id));
        assert!(!tracker.is_tainted(&clean_signal.id));
        let reason = tracker.reason(&tainted_signal.id).expect("has reason");
        assert_eq!(reason.category, "external");
    }

    #[test]
    fn observe_signal_prefers_provenance_taint_info() {
        use roko_core::{Body, Engram, Kind, Provenance, TaintInfo};

        let tainted_signal = Engram::builder(Kind::AgentOutput)
            .body(Body::text("external payload"))
            .provenance(
                Provenance::trusted("gateway")
                    .with_taint_info(TaintInfo::external("webhook payload")),
            )
            .build();

        let tracker = TaintTracker::new();
        assert!(tracker.observe_signal(&tainted_signal));
        let reason = tracker.reason(&tainted_signal.id).expect("has reason");
        assert_eq!(reason.category, "external");
        assert_eq!(reason.detail, "webhook payload");
    }

    #[test]
    fn concurrent_marks_are_safe() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(TaintTracker::new());
        let mut handles = Vec::new();
        for i in 0u8..16 {
            let t = Arc::clone(&tracker);
            handles.push(thread::spawn(move || {
                let id = h(&[i]);
                t.mark_tainted(id, TaintReason::external("thread"));
            }));
        }
        for handle in handles {
            handle.join().expect("thread join");
        }
        assert_eq!(tracker.len(), 16);
    }

    #[test]
    fn taint_reason_constructors_set_category() {
        assert_eq!(TaintReason::external("x").category, "external");
        assert_eq!(TaintReason::user_input("x").category, "user_input");
        assert_eq!(TaintReason::propagated("x").category, "propagated");
        assert_eq!(TaintReason::new("custom", "x").category, "custom");
    }
}
