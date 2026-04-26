//! File-conflict-aware merge queue for serializing plan merges (§14.7).
//!
//! The [`MergeQueue`] accepts [`MergeRequest`]s from plans that are ready
//! to merge, and hands them out one at a time — but only if the next
//! request does not touch files that an in-progress merge is already
//! modifying.
//!
//! This prevents two plans that both modify `src/lib.rs` from racing to
//! merge concurrently. Plans that touch completely disjoint file sets
//! *can* merge in parallel.
//!
//! Thread-safe: all state is behind a `parking_lot::Mutex`.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Default number of failed merge attempts before a request is considered
/// permanently failed (callers can still manually re-enqueue).
pub const DEFAULT_MAX_MERGE_RETRIES: u32 = 5;

/// Maximum number of retries before a request is considered permanently
/// failed (callers can still manually re-enqueue).
const MAX_RETRIES: u32 = DEFAULT_MAX_MERGE_RETRIES;

// ─── MergeRequest ──────────────────────────────────────────────────────

/// A request to merge a plan branch into the batch branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeRequest {
    /// The plan identifier (e.g. `"46-reputation-engine"`).
    pub plan_id: String,
    /// Branch name to merge from.
    pub branch_name: String,
    /// Files that this plan has modified (used for conflict detection).
    pub files_changed: Vec<String>,
    /// Priority — higher values merge first when there is no conflict.
    pub priority: u32,
    /// Number of times this request has been retried after failure.
    pub retry_count: u32,
}

impl MergeRequest {
    /// Construct a new merge request.
    #[must_use]
    pub fn new(
        plan_id: impl Into<String>,
        branch_name: impl Into<String>,
        files_changed: Vec<String>,
        priority: u32,
    ) -> Self {
        Self {
            plan_id: plan_id.into(),
            branch_name: branch_name.into(),
            files_changed,
            priority,
            retry_count: 0,
        }
    }

    /// Effective priority, reduced by retry count to let fresh requests
    /// go first when priorities are otherwise equal.
    #[must_use]
    pub const fn effective_priority(&self) -> u32 {
        self.priority.saturating_sub(self.retry_count)
    }
}

// ─── MergeStatus ───────────────────────────────────────────────────────

/// Status of a request inside the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    /// Waiting in the queue.
    Queued,
    /// Currently being merged (files are locked).
    Merging,
}

// ─── Serializable queue state ──────────────────────────────────────────

/// Serializable entry in a [`MergeQueueSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueueEntrySnapshot {
    /// The merge request.
    pub request: MergeRequest,
    /// The request's current queue status.
    pub status: MergeStatus,
}

/// Serializable snapshot of the merge queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueueSnapshot {
    /// Entries in deterministic `plan_id` order.
    pub entries: Vec<MergeQueueEntrySnapshot>,
    /// File locks held by in-progress merges, `path -> plan_id`.
    pub locked_files: BTreeMap<String, String>,
    /// Permanently failed plan IDs with their last failure reason.
    pub failed: BTreeMap<String, String>,
    /// Queued plan IDs in effective-priority order.
    pub queued_order: Vec<String>,
    /// Retry limit used by this queue implementation.
    pub max_retries: u32,
}

/// Point-in-time merge queue metrics for dashboards and recovery checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeQueueMetrics {
    /// Requests waiting to merge.
    pub queued: usize,
    /// Requests currently merging.
    pub merging: usize,
    /// Requests that exhausted retry budget.
    pub failed: usize,
    /// Number of files currently locked by in-progress merges.
    pub locked_files: usize,
    /// Number of queued requests mergeable under the current locks.
    pub ready: usize,
}

/// A queued merge blocked by an in-progress merge that owns one or more files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeConflict {
    /// The queued plan that is currently blocked.
    pub waiting_plan_id: String,
    /// The in-progress plan holding the conflicting file lock.
    pub blocking_plan_id: String,
    /// Locked files shared by both plans.
    pub files: Vec<String>,
}

// ─── Inner state ───────────────────────────────────────────────────────

/// An entry in the queue — request + current status.
#[derive(Debug, Clone)]
struct QueueEntry {
    request: MergeRequest,
    status: MergeStatus,
}

/// Protected inner state.
#[derive(Debug, Default)]
struct Inner {
    /// Entries keyed by `plan_id` for O(1) lookup. Insertion order is
    /// preserved by the `order` vector.
    entries: BTreeMap<String, QueueEntry>,
    /// Plan IDs in priority order (rebuilt on mutation).
    order: Vec<String>,
    /// Files currently locked by in-progress merges — maps each file
    /// path to the `plan_id` that owns it.
    locked_files: BTreeMap<String, String>,
    /// Permanently failed plan IDs with reason.
    failed: BTreeMap<String, String>,
}

impl Inner {
    /// Rebuild the sorted order vector. Higher effective priority first;
    /// ties broken by `plan_id` (lexicographic ascending) for determinism.
    fn rebuild_order(&mut self) {
        let mut ids: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.status == MergeStatus::Queued)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort_by(|a, b| {
            let ea = self.entries[a].request.effective_priority();
            let eb = self.entries[b].request.effective_priority();
            // Higher priority first, then lexicographic plan_id.
            eb.cmp(&ea).then_with(|| a.cmp(b))
        });
        self.order = ids;
    }
}

fn locked_conflicts(
    locked_files: &BTreeMap<String, String>,
    files_changed: &[String],
) -> Vec<(String, String)> {
    let files: HashSet<&str> = files_changed.iter().map(String::as_str).collect();
    locked_files
        .iter()
        .filter_map(|(file, owner)| {
            files
                .contains(file.as_str())
                .then(|| (file.clone(), owner.clone()))
        })
        .collect()
}

fn build_conflicts_for_request(
    locked_files: &BTreeMap<String, String>,
    request: &MergeRequest,
) -> Vec<MergeConflict> {
    let mut by_owner: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (file, owner) in locked_conflicts(locked_files, &request.files_changed) {
        if owner != request.plan_id {
            by_owner.entry(owner).or_default().push(file);
        }
    }

    by_owner
        .into_iter()
        .map(|(blocking_plan_id, mut files)| {
            files.sort();
            files.dedup();
            MergeConflict {
                waiting_plan_id: request.plan_id.clone(),
                blocking_plan_id,
                files,
            }
        })
        .collect()
}

// ─── MergeQueue ────────────────────────────────────────────────────────

/// File-conflict-aware merge queue.
///
/// Merge requests enter via [`enqueue`](Self::enqueue) and are handed
/// out in priority order via [`next_mergeable`](Self::next_mergeable),
/// skipping any request whose file set overlaps with an in-progress
/// merge.
#[derive(Debug, Clone, Default)]
pub struct MergeQueue {
    inner: Arc<Mutex<Inner>>,
}

impl MergeQueue {
    /// Create an empty merge queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a merge request to the queue.
    ///
    /// If a request for the same `plan_id` already exists, it is
    /// replaced (useful for re-queuing after a plan re-gates).
    pub fn enqueue(&self, request: MergeRequest) {
        let mut guard = self.inner.lock();
        let plan_id = request.plan_id.clone();
        guard.entries.insert(
            plan_id,
            QueueEntry {
                request,
                status: MergeStatus::Queued,
            },
        );
        guard.rebuild_order();
    }

    /// Return the next request that does not conflict with any
    /// in-progress merge, or `None` if no such request exists.
    #[must_use]
    #[allow(clippy::significant_drop_tightening)]
    pub fn next_mergeable(&self) -> Option<MergeRequest> {
        let guard = self.inner.lock();
        for plan_id in &guard.order {
            let entry = &guard.entries[plan_id];
            if entry.status != MergeStatus::Queued {
                continue;
            }
            if locked_conflicts(&guard.locked_files, &entry.request.files_changed).is_empty() {
                return Some(entry.request.clone());
            }
        }
        None
    }

    /// Atomically reserve the next mergeable request.
    ///
    /// This combines [`next_mergeable`](Self::next_mergeable) and
    /// [`mark_merging`](Self::mark_merging) under a single lock so multiple
    /// queue-draining workers cannot race to claim the same request.
    pub fn reserve_next_mergeable(&self) -> Option<MergeRequest> {
        let mut guard = self.inner.lock();
        let plan_id = guard
            .order
            .iter()
            .find(|plan_id| {
                let entry = &guard.entries[*plan_id];
                entry.status == MergeStatus::Queued
                    && locked_conflicts(&guard.locked_files, &entry.request.files_changed)
                        .is_empty()
            })
            .cloned()?;

        let request = guard.entries[&plan_id].request.clone();
        if let Some(entry) = guard.entries.get_mut(&plan_id) {
            entry.status = MergeStatus::Merging;
        }
        for file in &request.files_changed {
            guard.locked_files.insert(file.clone(), plan_id.clone());
        }
        guard.rebuild_order();
        Some(request)
    }

    /// Return a non-mutating batch of mutually non-conflicting requests
    /// that could be reserved now.
    ///
    /// `limit == 0` means unbounded. The method accounts for current file
    /// locks and for locks that would be acquired by earlier requests in the
    /// returned batch.
    #[must_use]
    pub fn ready_batch(&self, limit: usize) -> Vec<MergeRequest> {
        let guard = self.inner.lock();
        let mut virtual_locks = guard.locked_files.clone();
        let mut out = Vec::new();

        for plan_id in &guard.order {
            if limit > 0 && out.len() >= limit {
                break;
            }
            let entry = &guard.entries[plan_id];
            if entry.status != MergeStatus::Queued {
                continue;
            }
            if !locked_conflicts(&virtual_locks, &entry.request.files_changed).is_empty() {
                continue;
            }
            for file in &entry.request.files_changed {
                virtual_locks.insert(file.clone(), plan_id.clone());
            }
            out.push(entry.request.clone());
        }

        out
    }

    /// Check whether two merge requests have overlapping file sets.
    #[must_use]
    pub fn file_conflicts(a: &MergeRequest, b: &MergeRequest) -> bool {
        let set_a: HashSet<&str> = a.files_changed.iter().map(String::as_str).collect();
        b.files_changed.iter().any(|f| set_a.contains(f.as_str()))
    }

    /// Mark a plan as currently merging, locking its files.
    ///
    /// Returns `false` if the plan is not in the queue or is already
    /// merging.
    pub fn mark_merging(&self, plan_id: &str) -> bool {
        let mut guard = self.inner.lock();
        // Check preconditions via an immutable borrow.
        let can_merge = guard
            .entries
            .get(plan_id)
            .is_some_and(|e| e.status == MergeStatus::Queued);
        if !can_merge {
            return false;
        }
        // Collect files before mutating.
        let files: Vec<String> = guard.entries[plan_id].request.files_changed.clone();
        // Now mutate.
        if let Some(entry) = guard.entries.get_mut(plan_id) {
            entry.status = MergeStatus::Merging;
        }
        for file in files {
            guard.locked_files.insert(file, plan_id.to_string());
        }
        guard.rebuild_order();
        true
    }

    /// Mark a plan's merge as complete, removing it from the queue and
    /// releasing its file locks.
    pub fn mark_complete(&self, plan_id: &str) {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.entries.remove(plan_id) {
            for file in &entry.request.files_changed {
                guard.locked_files.remove(file);
            }
        }
        guard.rebuild_order();
    }

    /// Mark a merge as failed. If the retry count has not exceeded the
    /// limit, the request is moved back to `Queued` status with an
    /// incremented retry count. Otherwise, the request is removed from
    /// the queue and added to the failed set.
    ///
    /// Returns `true` if the request will be retried.
    pub fn mark_failed(&self, plan_id: &str, reason: &str) -> bool {
        let mut guard = self.inner.lock();
        if !guard.entries.contains_key(plan_id) {
            return false;
        }
        // Collect files to unlock before mutating entries.
        let files: Vec<String> = guard.entries[plan_id].request.files_changed.clone();
        for file in &files {
            guard.locked_files.remove(file);
        }
        // Now mutate the entry.
        if let Some(entry) = guard.entries.get_mut(plan_id) {
            entry.request.retry_count += 1;
            if entry.request.retry_count >= MAX_RETRIES {
                guard.failed.insert(plan_id.to_string(), reason.to_string());
                guard.entries.remove(plan_id);
                guard.rebuild_order();
                return false;
            }
            entry.status = MergeStatus::Queued;
        }
        guard.rebuild_order();
        true
    }

    /// Number of requests currently in the queue (both queued and
    /// in-progress).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// Whether the queue is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().entries.is_empty()
    }

    /// Number of merges currently in progress.
    #[must_use]
    pub fn in_progress_count(&self) -> usize {
        self.inner
            .lock()
            .entries
            .values()
            .filter(|e| e.status == MergeStatus::Merging)
            .count()
    }

    /// Set of file paths currently locked by in-progress merges.
    #[must_use]
    pub fn locked_files(&self) -> BTreeSet<String> {
        self.inner.lock().locked_files.keys().cloned().collect()
    }

    /// Returns the set of permanently failed plan IDs with their reasons.
    #[must_use]
    pub fn failed_plans(&self) -> BTreeMap<String, String> {
        self.inner.lock().failed.clone()
    }

    /// Requests currently in queued state, in merge priority order.
    #[must_use]
    pub fn queued_requests(&self) -> Vec<MergeRequest> {
        let guard = self.inner.lock();
        guard
            .order
            .iter()
            .filter_map(|id| guard.entries.get(id))
            .filter(|entry| entry.status == MergeStatus::Queued)
            .map(|entry| entry.request.clone())
            .collect()
    }

    /// Requests currently marked as in-progress.
    #[must_use]
    pub fn in_progress_requests(&self) -> Vec<MergeRequest> {
        let guard = self.inner.lock();
        guard
            .entries
            .values()
            .filter(|entry| entry.status == MergeStatus::Merging)
            .map(|entry| entry.request.clone())
            .collect()
    }

    /// Returns plan IDs in current priority order (queued only).
    #[must_use]
    pub fn queued_order(&self) -> Vec<String> {
        self.inner.lock().order.clone()
    }

    /// Peek at a request by `plan_id` without removing it.
    #[must_use]
    pub fn get(&self, plan_id: &str) -> Option<MergeRequest> {
        self.inner
            .lock()
            .entries
            .get(plan_id)
            .map(|e| e.request.clone())
    }

    /// Return the status of a queued or in-progress request.
    #[must_use]
    pub fn status_of(&self, plan_id: &str) -> Option<MergeStatus> {
        self.inner.lock().entries.get(plan_id).map(|e| e.status)
    }

    /// Return current lock conflicts for `plan_id`.
    #[must_use]
    pub fn lock_conflicts_for(&self, plan_id: &str) -> Vec<MergeConflict> {
        let guard = self.inner.lock();
        let Some(entry) = guard.entries.get(plan_id) else {
            return Vec::new();
        };
        build_conflicts_for_request(&guard.locked_files, &entry.request)
    }

    /// Return all queued requests currently blocked by in-progress file locks.
    #[must_use]
    pub fn blocked_conflicts(&self) -> Vec<MergeConflict> {
        let guard = self.inner.lock();
        let mut out = Vec::new();
        for plan_id in &guard.order {
            let entry = &guard.entries[plan_id];
            if entry.status == MergeStatus::Queued {
                out.extend(build_conflicts_for_request(
                    &guard.locked_files,
                    &entry.request,
                ));
            }
        }
        out
    }

    /// Return queue metrics for dashboards or recovery assertions.
    #[must_use]
    pub fn metrics(&self) -> MergeQueueMetrics {
        let guard = self.inner.lock();
        let queued = guard
            .entries
            .values()
            .filter(|entry| entry.status == MergeStatus::Queued)
            .count();
        let merging = guard
            .entries
            .values()
            .filter(|entry| entry.status == MergeStatus::Merging)
            .count();
        let ready = guard
            .order
            .iter()
            .filter(|plan_id| {
                let entry = &guard.entries[*plan_id];
                entry.status == MergeStatus::Queued
                    && locked_conflicts(&guard.locked_files, &entry.request.files_changed)
                        .is_empty()
            })
            .count();

        MergeQueueMetrics {
            queued,
            merging,
            failed: guard.failed.len(),
            locked_files: guard.locked_files.len(),
            ready,
        }
    }

    /// Serialize the queue state for crash recovery.
    #[must_use]
    pub fn snapshot(&self) -> MergeQueueSnapshot {
        let guard = self.inner.lock();
        let entries = guard
            .entries
            .values()
            .map(|entry| MergeQueueEntrySnapshot {
                request: entry.request.clone(),
                status: entry.status,
            })
            .collect();

        MergeQueueSnapshot {
            entries,
            locked_files: guard.locked_files.clone(),
            failed: guard.failed.clone(),
            queued_order: guard.order.clone(),
            max_retries: MAX_RETRIES,
        }
    }

    /// Restore a merge queue from a previously captured snapshot.
    ///
    /// File locks are rebuilt from `Merging` entries instead of trusting the
    /// snapshot's lock map, preventing stale locks from becoming durable.
    #[must_use]
    pub fn from_snapshot(snapshot: MergeQueueSnapshot) -> Self {
        let mut inner = Inner {
            failed: snapshot.failed,
            ..Inner::default()
        };

        for entry in snapshot.entries {
            let plan_id = entry.request.plan_id.clone();
            inner.entries.insert(
                plan_id.clone(),
                QueueEntry {
                    request: entry.request,
                    status: entry.status,
                },
            );
            if entry.status == MergeStatus::Merging {
                if let Some(stored) = inner.entries.get(&plan_id) {
                    for file in &stored.request.files_changed {
                        inner.locked_files.insert(file.clone(), plan_id.clone());
                    }
                }
            }
        }

        inner.rebuild_order();
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn req(plan: &str, files: &[&str], priority: u32) -> MergeRequest {
        MergeRequest::new(
            plan,
            format!("branch/{plan}"),
            files.iter().map(|s| (*s).to_string()).collect(),
            priority,
        )
    }

    // ── 1. Basic enqueue and dequeue ─────────────────────────────────

    #[test]
    fn enqueue_and_next_mergeable() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        q.enqueue(req("plan-b", &["src/b.rs"], 20));
        // plan-b has higher priority.
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-b");
    }

    // ── 2. Empty queue returns None ──────────────────────────────────

    #[test]
    fn empty_queue_returns_none() {
        let q = MergeQueue::new();
        assert!(q.next_mergeable().is_none());
        assert!(q.is_empty());
        assert_eq!(q.len(), 0);
    }

    // ── 3. File conflict detection ───────────────────────────────────

    #[test]
    fn file_conflicts_detects_overlap() {
        let a = req("a", &["src/lib.rs", "src/a.rs"], 1);
        let b = req("b", &["src/lib.rs", "src/b.rs"], 1);
        assert!(MergeQueue::file_conflicts(&a, &b));
    }

    #[test]
    fn file_conflicts_no_overlap() {
        let a = req("a", &["src/a.rs"], 1);
        let b = req("b", &["src/b.rs"], 1);
        assert!(!MergeQueue::file_conflicts(&a, &b));
    }

    // ── 4. Serialization: conflicting merges are blocked ─────────────

    #[test]
    fn conflicting_merge_is_blocked() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/lib.rs"], 10));
        q.enqueue(req("plan-b", &["src/lib.rs"], 5));

        // Take plan-a (higher priority).
        q.mark_merging("plan-a");
        // plan-b conflicts, so next_mergeable should return None.
        assert!(q.next_mergeable().is_none());
    }

    // ── 5. Non-conflicting merge proceeds ────────────────────────────

    #[test]
    fn non_conflicting_merge_proceeds() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        q.enqueue(req("plan-b", &["src/b.rs"], 5));

        q.mark_merging("plan-a");
        // plan-b touches different files, should be available.
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-b");
    }

    // ── 6. mark_complete releases locks ──────────────────────────────

    #[test]
    fn mark_complete_releases_locks() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/lib.rs"], 10));
        q.enqueue(req("plan-b", &["src/lib.rs"], 5));

        q.mark_merging("plan-a");
        assert!(q.next_mergeable().is_none());

        q.mark_complete("plan-a");
        // Locks released, plan-b can now proceed.
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-b");
        assert_eq!(q.len(), 1);
    }

    // ── 7. mark_failed retries with increased count ──────────────────

    #[test]
    fn mark_failed_retries() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        q.mark_merging("plan-a");

        let will_retry = q.mark_failed("plan-a", "merge conflict");
        assert!(will_retry);
        assert_eq!(q.len(), 1);

        let r = q.get("plan-a").unwrap();
        assert_eq!(r.retry_count, 1);
    }

    // ── 8. mark_failed exhausts retries ──────────────────────────────

    #[test]
    fn mark_failed_exhausts_retries() {
        let q = MergeQueue::new();
        let mut r = req("plan-a", &["src/a.rs"], 10);
        r.retry_count = MAX_RETRIES - 1;
        q.enqueue(r);
        q.mark_merging("plan-a");

        let will_retry = q.mark_failed("plan-a", "still broken");
        assert!(!will_retry);
        assert!(q.is_empty());
        assert!(q.failed_plans().contains_key("plan-a"));
    }

    // ── 9. Priority ordering is deterministic ────────────────────────

    #[test]
    fn priority_ordering_is_deterministic() {
        let q = MergeQueue::new();
        q.enqueue(req("z-plan", &["src/z.rs"], 10));
        q.enqueue(req("a-plan", &["src/a.rs"], 10));
        q.enqueue(req("m-plan", &["src/m.rs"], 10));

        // Same priority -> sorted by plan_id lexicographically.
        let order = q.queued_order();
        assert_eq!(order, vec!["a-plan", "m-plan", "z-plan"]);
    }

    // ── 10. Effective priority decreases with retries ────────────────

    #[test]
    fn effective_priority_decreases_with_retries() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));

        let mut r = req("plan-b", &["src/b.rs"], 12);
        r.retry_count = 3;
        q.enqueue(r);

        // plan-b has priority 12 but retried 3 times -> effective 9.
        // plan-a has effective 10.
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-a");
    }

    // ── 11. Replace existing request on re-enqueue ───────────────────

    #[test]
    fn re_enqueue_replaces_existing() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 5));
        assert_eq!(q.get("plan-a").unwrap().priority, 5);

        // Re-enqueue with different priority.
        q.enqueue(req("plan-a", &["src/a.rs", "src/b.rs"], 20));
        assert_eq!(q.len(), 1);
        assert_eq!(q.get("plan-a").unwrap().priority, 20);
        assert_eq!(q.get("plan-a").unwrap().files_changed.len(), 2);
    }

    // ── 12. mark_merging returns false for unknown plan ──────────────

    #[test]
    fn mark_merging_unknown_returns_false() {
        let q = MergeQueue::new();
        assert!(!q.mark_merging("nonexistent"));
    }

    // ── 13. locked_files tracks in-progress file locks ───────────────

    #[test]
    fn locked_files_tracks_merging() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs", "src/lib.rs"], 10));
        assert!(q.locked_files().is_empty());

        q.mark_merging("plan-a");
        let locked = q.locked_files();
        assert!(locked.contains("src/a.rs"));
        assert!(locked.contains("src/lib.rs"));
        assert_eq!(locked.len(), 2);

        q.mark_complete("plan-a");
        assert!(q.locked_files().is_empty());
    }

    // ── 14. in_progress_count is accurate ────────────────────────────

    #[test]
    fn in_progress_count() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        q.enqueue(req("plan-b", &["src/b.rs"], 5));
        assert_eq!(q.in_progress_count(), 0);

        q.mark_merging("plan-a");
        assert_eq!(q.in_progress_count(), 1);

        q.mark_merging("plan-b");
        assert_eq!(q.in_progress_count(), 2);

        q.mark_complete("plan-a");
        assert_eq!(q.in_progress_count(), 1);
    }

    // ── 15. Multiple non-conflicting merges can run in parallel ──────

    #[test]
    fn multiple_non_conflicting_parallel() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        q.enqueue(req("plan-b", &["src/b.rs"], 8));
        q.enqueue(req("plan-c", &["src/c.rs"], 6));

        q.mark_merging("plan-a");
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-b");

        q.mark_merging("plan-b");
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-c");

        q.mark_merging("plan-c");
        assert!(q.next_mergeable().is_none());
    }

    // ── 16. Complex conflict graph: partial blocking ─────────────────

    #[test]
    fn complex_conflict_graph() {
        let q = MergeQueue::new();
        // plan-a and plan-b share lib.rs; plan-c is independent.
        q.enqueue(req("plan-a", &["src/lib.rs", "src/a.rs"], 10));
        q.enqueue(req("plan-b", &["src/lib.rs", "src/b.rs"], 8));
        q.enqueue(req("plan-c", &["src/c.rs"], 6));

        q.mark_merging("plan-a");
        // plan-b blocked (shares lib.rs), but plan-c should be available.
        let next = q.next_mergeable().unwrap();
        assert_eq!(next.plan_id, "plan-c");
    }

    // ── 17. mark_failed releases file locks ──────────────────────────

    #[test]
    fn mark_failed_releases_locks() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/lib.rs"], 10));
        q.enqueue(req("plan-b", &["src/lib.rs"], 5));

        q.mark_merging("plan-a");
        assert!(q.next_mergeable().is_none());

        q.mark_failed("plan-a", "merge conflict");
        // plan-a's lock on src/lib.rs should be released.
        // plan-a is back in queue but plan-b should now also be eligible
        // (plan-a still has higher effective priority though, so plan-a
        // comes out first).
        let next = q.next_mergeable().unwrap();
        // plan-a has effective priority 10-1=9, plan-b has 5.
        assert_eq!(next.plan_id, "plan-a");
    }

    // ── 18. Empty files list never conflicts ─────────────────────────

    #[test]
    fn empty_files_never_conflict() {
        let a = req("a", &[], 1);
        let b = req("b", &["src/lib.rs"], 1);
        assert!(!MergeQueue::file_conflicts(&a, &b));
        assert!(!MergeQueue::file_conflicts(&b, &a));
    }

    // ── 19. Thread-safety: clone shares state ────────────────────────

    #[test]
    fn clone_shares_state() {
        let q1 = MergeQueue::new();
        let q2 = q1.clone();
        q1.enqueue(req("plan-a", &["src/a.rs"], 10));
        assert_eq!(q2.len(), 1);
    }

    // ── 20. mark_merging on already-merging returns false ────────────

    #[test]
    fn double_mark_merging_returns_false() {
        let q = MergeQueue::new();
        q.enqueue(req("plan-a", &["src/a.rs"], 10));
        assert!(q.mark_merging("plan-a"));
        assert!(!q.mark_merging("plan-a"));
    }
}
