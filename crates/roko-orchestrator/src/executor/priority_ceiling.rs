//! Immediate Ceiling Priority Protocol (ICPP) for resource scheduling.
//!
//! Prevents priority inversion when high-priority plans are blocked by
//! low-priority plans holding shared resources (merge queue slots,
//! worktrees, etc.).
//!
//! # Algorithm (Sha et al. 1990)
//!
//! Each shared resource is assigned a **ceiling** equal to the highest
//! priority of any plan that may use it. When a plan acquires a resource,
//! its effective priority is immediately raised to the resource's ceiling.
//! This guarantees:
//!
//! - **Bounded blocking**: a high-priority plan is blocked by at most one
//!   lower-priority critical section.
//! - **Deadlock freedom**: no circular wait is possible.
//! - **No chained blocking**: a plan never waits behind a chain of
//!   lower-priority resource holders.
//!
//! # Usage
//!
//! ```rust,ignore
//! use roko_orchestrator::executor::priority_ceiling::*;
//!
//! let plans = vec![
//!     PlanResourceInfo { plan_id: "a".into(), priority: 10, resources: vec![ResourceId::MergeQueueSlot] },
//!     PlanResourceInfo { plan_id: "b".into(), priority: 5, resources: vec![ResourceId::MergeQueueSlot, ResourceId::Worktree("wt-1".into())] },
//! ];
//! let ceiling = PriorityCeiling::compute(&plans);
//! assert_eq!(ceiling.ceiling(&ResourceId::MergeQueueSlot), Some(10));
//!
//! let mut tracker = EffectivePriorityTracker::new(&ceiling);
//! tracker.acquire("b", &ResourceId::MergeQueueSlot);
//! // Plan "b" is now boosted to priority 10 while holding the merge queue slot.
//! assert_eq!(tracker.effective_priority("b", 5), 10);
//! tracker.release("b", &ResourceId::MergeQueueSlot);
//! assert_eq!(tracker.effective_priority("b", 5), 5);
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ─── Resource identifier ────────────────────────────────────────────────

/// A shared resource that plans contend for.
///
/// The executor manages several types of shared resources; this enum
/// gives them a common identity so the priority ceiling protocol can
/// track them uniformly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceId {
    /// A slot in the merge queue (only one plan can merge at a time per
    /// file set).
    MergeQueueSlot,
    /// A named git worktree used for isolated execution.
    Worktree(String),
    /// One of the bounded agent-dispatch slots.
    AgentSlot,
    /// A custom resource defined by configuration.
    Custom(String),
}

impl std::fmt::Display for ResourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MergeQueueSlot => write!(f, "merge_queue_slot"),
            Self::Worktree(name) => write!(f, "worktree:{name}"),
            Self::AgentSlot => write!(f, "agent_slot"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

// ─── Plan resource metadata ─────────────────────────────────────────────

/// Metadata about a plan's priority and resource requirements, used to
/// compute resource ceilings.
#[derive(Debug, Clone)]
pub struct PlanResourceInfo {
    /// The plan identifier.
    pub plan_id: String,
    /// The plan's declared (base) priority.
    pub priority: u32,
    /// Resources this plan may acquire during execution.
    pub resources: Vec<ResourceId>,
}

// ─── PriorityCeiling ────────────────────────────────────────────────────

/// Precomputed priority ceilings for every shared resource.
///
/// Each resource's ceiling is the maximum priority of any plan that
/// declares it as a required resource. This is computed once at executor
/// startup (or whenever plans are added/removed) and remains constant
/// until the plan set changes.
///
/// Serializes the ceiling map as a list of `(resource, priority)` pairs
/// because JSON requires string keys and [`ResourceId`] is a tagged enum.
#[derive(Debug, Clone, Default)]
pub struct PriorityCeiling {
    /// Resource → ceiling priority.
    ceilings: HashMap<ResourceId, u32>,
}

// Custom Serialize / Deserialize to avoid JSON string-key constraint.
impl Serialize for PriorityCeiling {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let entries: Vec<(&ResourceId, &u32)> = self.ceilings.iter().collect();
        entries.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PriorityCeiling {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries: Vec<(ResourceId, u32)> = Vec::deserialize(deserializer)?;
        Ok(Self {
            ceilings: entries.into_iter().collect(),
        })
    }
}

impl PriorityCeiling {
    /// Compute ceilings from a set of plan resource declarations.
    ///
    /// For each resource, the ceiling is `max(priority)` over all plans
    /// that list that resource.
    #[must_use]
    pub fn compute(plans: &[PlanResourceInfo]) -> Self {
        let mut ceilings: HashMap<ResourceId, u32> = HashMap::new();
        for plan in plans {
            for resource in &plan.resources {
                let entry = ceilings.entry(resource.clone()).or_insert(0);
                *entry = (*entry).max(plan.priority);
            }
        }
        Self { ceilings }
    }

    /// Look up the ceiling for a specific resource.
    ///
    /// Returns `None` if the resource was not declared by any plan.
    #[must_use]
    pub fn ceiling(&self, resource: &ResourceId) -> Option<u32> {
        self.ceilings.get(resource).copied()
    }

    /// Iterate over all (resource, ceiling) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&ResourceId, &u32)> {
        self.ceilings.iter()
    }

    /// Number of tracked resources.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ceilings.len()
    }

    /// Whether no resources are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ceilings.is_empty()
    }
}

// ─── EffectivePriorityTracker ───────────────────────────────────────────

/// Tracks which resources each plan currently holds and computes
/// effective (boosted) priorities.
///
/// When a plan acquires a resource, its effective priority is raised to
/// the resource's ceiling. When it releases the resource, the boost is
/// removed (the effective priority falls back to the plan's base priority
/// or the ceiling of any other resource it still holds).
#[derive(Debug, Clone, Default)]
pub struct EffectivePriorityTracker {
    /// plan_id → set of resources currently held.
    held: HashMap<String, Vec<ResourceId>>,
    /// Precomputed ceilings (shared reference; cloned for ownership).
    ceilings: PriorityCeiling,
}

impl EffectivePriorityTracker {
    /// Create a new tracker backed by the given ceilings.
    #[must_use]
    pub fn new(ceilings: &PriorityCeiling) -> Self {
        Self {
            held: HashMap::new(),
            ceilings: ceilings.clone(),
        }
    }

    /// Record that `plan_id` has acquired `resource`.
    ///
    /// After this call, [`effective_priority`](Self::effective_priority)
    /// for the plan will be at least the resource's ceiling.
    pub fn acquire(&mut self, plan_id: &str, resource: &ResourceId) {
        let held = self.held.entry(plan_id.to_string()).or_default();
        if !held.contains(resource) {
            held.push(resource.clone());
        }
    }

    /// Record that `plan_id` has released `resource`.
    pub fn release(&mut self, plan_id: &str, resource: &ResourceId) {
        if let Some(held) = self.held.get_mut(plan_id) {
            held.retain(|r| r != resource);
            if held.is_empty() {
                self.held.remove(plan_id);
            }
        }
    }

    /// Release all resources held by `plan_id`.
    pub fn release_all(&mut self, plan_id: &str) {
        self.held.remove(plan_id);
    }

    /// Compute the effective priority for a plan.
    ///
    /// The effective priority is `max(base_priority, ceiling_of_each_held_resource)`.
    /// If the plan holds no resources, the effective priority equals the base.
    #[must_use]
    pub fn effective_priority(&self, plan_id: &str, base_priority: u32) -> u32 {
        let Some(held) = self.held.get(plan_id) else {
            return base_priority;
        };
        let max_ceiling = held
            .iter()
            .filter_map(|r| self.ceilings.ceiling(r))
            .max()
            .unwrap_or(0);
        base_priority.max(max_ceiling)
    }

    /// Whether the plan currently holds any resources.
    #[must_use]
    pub fn is_holding(&self, plan_id: &str) -> bool {
        self.held
            .get(plan_id)
            .is_some_and(|held| !held.is_empty())
    }

    /// List resources currently held by a plan.
    #[must_use]
    pub fn resources_held(&self, plan_id: &str) -> &[ResourceId] {
        self.held
            .get(plan_id)
            .map_or(&[] as &[ResourceId], Vec::as_slice)
    }

    /// Sort a list of plan IDs by effective priority (highest first).
    ///
    /// Plans with equal effective priority retain their original order
    /// (stable sort).
    pub fn sort_by_effective_priority(&self, plan_ids: &mut [String], base_priorities: &HashMap<String, u32>) {
        plan_ids.sort_by(|a, b| {
            let pa = self.effective_priority(a, base_priorities.get(a).copied().unwrap_or(0));
            let pb = self.effective_priority(b, base_priorities.get(b).copied().unwrap_or(0));
            pb.cmp(&pa) // descending
        });
    }
}

// ─── Tests ────���───────────────────────────────���─────────────────────────

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sample_plans() -> Vec<PlanResourceInfo> {
        vec![
            PlanResourceInfo {
                plan_id: "high".into(),
                priority: 100,
                resources: vec![ResourceId::MergeQueueSlot],
            },
            PlanResourceInfo {
                plan_id: "medium".into(),
                priority: 50,
                resources: vec![
                    ResourceId::MergeQueueSlot,
                    ResourceId::Worktree("wt-1".into()),
                ],
            },
            PlanResourceInfo {
                plan_id: "low".into(),
                priority: 10,
                resources: vec![ResourceId::Worktree("wt-1".into()), ResourceId::AgentSlot],
            },
        ]
    }

    #[test]
    fn ceiling_is_max_priority_per_resource() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        assert_eq!(ceiling.ceiling(&ResourceId::MergeQueueSlot), Some(100));
        assert_eq!(
            ceiling.ceiling(&ResourceId::Worktree("wt-1".into())),
            Some(50)
        );
        assert_eq!(ceiling.ceiling(&ResourceId::AgentSlot), Some(10));
        assert_eq!(ceiling.ceiling(&ResourceId::Custom("x".into())), None);
        assert_eq!(ceiling.len(), 3);
    }

    #[test]
    fn empty_plans_produce_empty_ceilings() {
        let ceiling = PriorityCeiling::compute(&[]);
        assert!(ceiling.is_empty());
        assert_eq!(ceiling.ceiling(&ResourceId::MergeQueueSlot), None);
    }

    #[test]
    fn effective_priority_without_resources_is_base() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let tracker = EffectivePriorityTracker::new(&ceiling);
        assert_eq!(tracker.effective_priority("low", 10), 10);
    }

    #[test]
    fn effective_priority_boosted_by_resource_ceiling() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        // Low-priority plan acquires merge queue (ceiling=100).
        tracker.acquire("low", &ResourceId::MergeQueueSlot);
        assert_eq!(tracker.effective_priority("low", 10), 100);
        assert!(tracker.is_holding("low"));

        // Release brings it back down.
        tracker.release("low", &ResourceId::MergeQueueSlot);
        assert_eq!(tracker.effective_priority("low", 10), 10);
        assert!(!tracker.is_holding("low"));
    }

    #[test]
    fn multiple_resources_take_max_ceiling() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        tracker.acquire("low", &ResourceId::Worktree("wt-1".into()));
        assert_eq!(tracker.effective_priority("low", 10), 50);

        tracker.acquire("low", &ResourceId::MergeQueueSlot);
        assert_eq!(tracker.effective_priority("low", 10), 100);

        // Release the higher one; still boosted by worktree ceiling.
        tracker.release("low", &ResourceId::MergeQueueSlot);
        assert_eq!(tracker.effective_priority("low", 10), 50);

        tracker.release("low", &ResourceId::Worktree("wt-1".into()));
        assert_eq!(tracker.effective_priority("low", 10), 10);
    }

    #[test]
    fn release_all_removes_all_held_resources() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        tracker.acquire("low", &ResourceId::MergeQueueSlot);
        tracker.acquire("low", &ResourceId::AgentSlot);
        assert_eq!(tracker.resources_held("low").len(), 2);

        tracker.release_all("low");
        assert_eq!(tracker.effective_priority("low", 10), 10);
        assert!(!tracker.is_holding("low"));
    }

    #[test]
    fn sort_by_effective_priority_respects_boosted_plans() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        // Low holds merge queue, boosting to 100.
        tracker.acquire("low", &ResourceId::MergeQueueSlot);

        let mut plan_ids = vec!["high".into(), "medium".into(), "low".into()];
        let base_priorities: HashMap<String, u32> = [
            ("high".into(), 100),
            ("medium".into(), 50),
            ("low".into(), 10),
        ]
        .into_iter()
        .collect();

        tracker.sort_by_effective_priority(&mut plan_ids, &base_priorities);
        // "low" (boosted to 100) ties with "high" (100); stable sort preserves order.
        // "medium" (50) is last.
        assert_eq!(plan_ids[2], "medium");
        // Both "high" and "low" have effective priority 100.
        let top_two: Vec<&str> = plan_ids[..2].iter().map(String::as_str).collect();
        assert!(top_two.contains(&"high"));
        assert!(top_two.contains(&"low"));
    }

    #[test]
    fn duplicate_acquire_is_idempotent() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        tracker.acquire("low", &ResourceId::MergeQueueSlot);
        tracker.acquire("low", &ResourceId::MergeQueueSlot);
        assert_eq!(tracker.resources_held("low").len(), 1);
    }

    #[test]
    fn release_nonexistent_resource_is_noop() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let mut tracker = EffectivePriorityTracker::new(&ceiling);

        // Release something never acquired.
        tracker.release("low", &ResourceId::AgentSlot);
        assert!(!tracker.is_holding("low"));
    }

    #[test]
    fn resource_id_display() {
        assert_eq!(ResourceId::MergeQueueSlot.to_string(), "merge_queue_slot");
        assert_eq!(
            ResourceId::Worktree("wt-42".into()).to_string(),
            "worktree:wt-42"
        );
        assert_eq!(ResourceId::AgentSlot.to_string(), "agent_slot");
        assert_eq!(
            ResourceId::Custom("gpu".into()).to_string(),
            "custom:gpu"
        );
    }

    #[test]
    fn priority_ceiling_serde_roundtrip() {
        let ceiling = PriorityCeiling::compute(&sample_plans());
        let json = serde_json::to_string(&ceiling).unwrap();
        let restored: PriorityCeiling = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.ceiling(&ResourceId::MergeQueueSlot),
            Some(100)
        );
        assert_eq!(restored.len(), ceiling.len());
    }
}
