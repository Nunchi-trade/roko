//! Emergent goal structures -- goals that emerge from behavior patterns.
//!
//! Rather than being explicitly programmed, goals are discovered by observing
//! recurring patterns in agent behavior. A `GoalSeed` captures a nascent pattern;
//! when enough evidence accumulates the seed is promoted into the `GoalTree`.
//!
//! # Lifecycle
//!
//! ```text
//! observation → GoalSeed → (evidence accumulates) → GoalNode in GoalTree
//!                             ↓ pruned if score < threshold
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A nascent behavioral pattern that may become a goal.
///
/// Seeds are created when a recurring pattern is detected (e.g., the agent
/// repeatedly fixes compile errors before running tests). Each seed tracks
/// how often the pattern is observed and accumulates a score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalSeed {
    /// Unique identifier for this seed.
    pub id: String,
    /// Human-readable description of the observed pattern.
    pub pattern: String,
    /// How many times the pattern has been observed.
    pub observation_count: u64,
    /// Accumulated score (higher = more likely to become a real goal).
    pub score: f64,
    /// When the pattern was first observed.
    pub first_seen: DateTime<Utc>,
    /// When the pattern was most recently observed.
    pub last_seen: DateTime<Utc>,
    /// Tags / categories extracted from the pattern context.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl GoalSeed {
    /// Create a new seed from an observed pattern.
    #[must_use]
    pub fn new(id: impl Into<String>, pattern: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: id.into(),
            pattern: pattern.into(),
            observation_count: 1,
            score: 1.0,
            first_seen: now,
            last_seen: now,
            tags: Vec::new(),
        }
    }

    /// Record another observation of this pattern, incrementing count and score.
    pub fn observe(&mut self, weight: f64) {
        self.observation_count += 1;
        self.score += weight;
        self.last_seen = Utc::now();
    }

    /// Whether this seed has accumulated enough evidence to be promoted.
    #[must_use]
    pub fn is_promotable(&self, min_observations: u64, min_score: f64) -> bool {
        self.observation_count >= min_observations && self.score >= min_score
    }

    /// Whether this seed should be pruned (score below threshold).
    #[must_use]
    pub fn should_prune(&self, prune_threshold: f64) -> bool {
        self.score < prune_threshold
    }

    /// Apply time-based decay to the seed's score.
    pub fn decay(&mut self, decay_factor: f64) {
        self.score *= decay_factor;
    }
}

/// A node in the goal tree, representing a promoted goal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GoalNode {
    /// Unique identifier (may match the originating GoalSeed id).
    pub id: String,
    /// Human-readable description of the goal.
    pub description: String,
    /// Priority weight in [0, 1]. Higher = more important.
    pub priority: f64,
    /// Estimated progress toward completion in [0, 1].
    pub progress: f64,
    /// Status of the goal.
    pub status: GoalStatus,
    /// Child goal IDs forming a sub-goal hierarchy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
    /// Parent goal ID, if this is a sub-goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// When the goal was created (promoted from seed).
    pub created_at: DateTime<Utc>,
    /// When the goal was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Status of a goal in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Active and being pursued.
    Active,
    /// Completed (all sub-goals met or progress >= threshold).
    Completed,
    /// Suspended temporarily.
    Suspended,
    /// Pruned due to low priority or irrelevance.
    Pruned,
}

impl GoalNode {
    /// Create a new active goal from a promoted seed.
    #[must_use]
    pub fn from_seed(seed: &GoalSeed, priority: f64) -> Self {
        let now = Utc::now();
        Self {
            id: seed.id.clone(),
            description: seed.pattern.clone(),
            priority: priority.clamp(0.0, 1.0),
            progress: 0.0,
            status: GoalStatus::Active,
            children: Vec::new(),
            parent: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Update progress and check for completion.
    pub fn update_progress(&mut self, new_progress: f64, completion_threshold: f64) {
        self.progress = new_progress.clamp(0.0, 1.0);
        self.updated_at = Utc::now();
        if self.progress >= completion_threshold {
            self.status = GoalStatus::Completed;
        }
    }

    /// Add a child sub-goal.
    pub fn add_child(&mut self, child_id: impl Into<String>) {
        self.children.push(child_id.into());
        self.updated_at = Utc::now();
    }

    /// Whether the goal is still active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == GoalStatus::Active
    }

    /// Whether the goal should be pruned (priority below threshold).
    #[must_use]
    pub fn should_prune(&self, prune_threshold: f64) -> bool {
        self.priority < prune_threshold && self.status == GoalStatus::Active
    }
}

/// Hierarchical goal structure managing seeds, active goals, and completed goals.
pub struct GoalTree {
    /// Seeds awaiting promotion.
    seeds: Vec<GoalSeed>,
    /// Active goal nodes by ID.
    nodes: HashMap<String, GoalNode>,
    /// Promotion thresholds.
    min_observations: u64,
    min_score: f64,
    /// Completion threshold.
    completion_threshold: f64,
    /// Prune threshold.
    prune_threshold: f64,
}

impl GoalTree {
    /// Create a new goal tree with the given thresholds.
    #[must_use]
    pub fn new(
        min_observations: u64,
        min_score: f64,
        completion_threshold: f64,
        prune_threshold: f64,
    ) -> Self {
        Self {
            seeds: Vec::new(),
            nodes: HashMap::new(),
            min_observations,
            min_score,
            completion_threshold,
            prune_threshold,
        }
    }

    /// Create a goal tree with default thresholds from config values.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(3, 5.0, 0.95, 0.1)
    }

    /// Add or update a seed based on a behavioral observation.
    /// If a seed with the same id exists, it's updated; otherwise a new seed is created.
    pub fn observe(&mut self, id: impl Into<String>, pattern: impl Into<String>, weight: f64) {
        let id = id.into();
        if let Some(seed) = self.seeds.iter_mut().find(|s| s.id == id) {
            seed.observe(weight);
        } else {
            let mut seed = GoalSeed::new(id, pattern);
            seed.score = weight;
            self.seeds.push(seed);
        }
    }

    /// Promote all eligible seeds into goal nodes. Returns the IDs of promoted goals.
    pub fn promote_seeds(&mut self) -> Vec<String> {
        let mut promoted = Vec::new();
        let mut remaining_seeds = Vec::new();

        for seed in self.seeds.drain(..) {
            if seed.is_promotable(self.min_observations, self.min_score) {
                let priority = (seed.score / (self.min_score * 3.0)).clamp(0.0, 1.0);
                let node = GoalNode::from_seed(&seed, priority);
                promoted.push(node.id.clone());
                self.nodes.insert(node.id.clone(), node);
            } else {
                remaining_seeds.push(seed);
            }
        }

        self.seeds = remaining_seeds;
        promoted
    }

    /// Prune low-priority goals and seeds. Returns count of pruned items.
    pub fn prune(&mut self) -> usize {
        let mut pruned = 0;

        // Prune seeds.
        let before = self.seeds.len();
        self.seeds.retain(|s| !s.should_prune(self.prune_threshold));
        pruned += before - self.seeds.len();

        // Prune goals.
        for node in self.nodes.values_mut() {
            if node.should_prune(self.prune_threshold) {
                node.status = GoalStatus::Pruned;
                pruned += 1;
            }
        }

        pruned
    }

    /// Apply decay to all seeds. Returns count of decayed seeds.
    pub fn decay_seeds(&mut self, factor: f64) -> usize {
        let count = self.seeds.len();
        for seed in &mut self.seeds {
            seed.decay(factor);
        }
        count
    }

    /// Get a goal node by ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&GoalNode> {
        self.nodes.get(id)
    }

    /// Get a mutable goal node by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut GoalNode> {
        self.nodes.get_mut(id)
    }

    /// All active goals sorted by priority (highest first).
    #[must_use]
    pub fn active_goals(&self) -> Vec<&GoalNode> {
        let mut goals: Vec<&GoalNode> = self
            .nodes
            .values()
            .filter(|n| n.is_active())
            .collect();
        goals.sort_by(|a, b| b.priority.partial_cmp(&a.priority).unwrap_or(std::cmp::Ordering::Equal));
        goals
    }

    /// All root goals (no parent).
    #[must_use]
    pub fn roots(&self) -> Vec<&GoalNode> {
        self.nodes
            .values()
            .filter(|n| n.parent.is_none() && n.is_active())
            .collect()
    }

    /// Number of pending seeds.
    #[must_use]
    pub fn seed_count(&self) -> usize {
        self.seeds.len()
    }

    /// Total number of goal nodes (all statuses).
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Update progress on a goal, checking for completion.
    pub fn update_progress(&mut self, id: &str, progress: f64) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.update_progress(progress, self.completion_threshold);
            true
        } else {
            false
        }
    }

    /// Set one goal as a child of another. Returns false if either doesn't exist.
    pub fn set_parent(&mut self, child_id: &str, parent_id: &str) -> bool {
        if !self.nodes.contains_key(child_id) || !self.nodes.contains_key(parent_id) {
            return false;
        }
        // Borrow workaround: split the mutation.
        let child_id_owned = child_id.to_string();
        let parent_id_owned = parent_id.to_string();
        if let Some(parent) = self.nodes.get_mut(&parent_id_owned) {
            if !parent.children.contains(&child_id_owned) {
                parent.add_child(&child_id_owned);
            }
        }
        if let Some(child) = self.nodes.get_mut(&child_id_owned) {
            child.parent = Some(parent_id_owned);
        }
        true
    }
}

impl Default for GoalTree {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_creation_and_observation() {
        let mut seed = GoalSeed::new("fix-first", "fix compile errors before testing");
        assert_eq!(seed.observation_count, 1);
        assert_eq!(seed.score, 1.0);

        seed.observe(2.0);
        assert_eq!(seed.observation_count, 2);
        assert_eq!(seed.score, 3.0);
    }

    #[test]
    fn seed_promotable() {
        let mut seed = GoalSeed::new("pattern", "desc");
        assert!(!seed.is_promotable(3, 5.0));

        seed.observe(2.0);
        seed.observe(2.0);
        assert_eq!(seed.observation_count, 3);
        assert_eq!(seed.score, 5.0);
        assert!(seed.is_promotable(3, 5.0));
    }

    #[test]
    fn seed_decay_and_prune() {
        let mut seed = GoalSeed::new("pattern", "desc");
        seed.score = 1.0;
        seed.decay(0.5);
        assert_eq!(seed.score, 0.5);
        seed.decay(0.1);
        assert!(seed.should_prune(0.1));
    }

    #[test]
    fn goal_node_from_seed() {
        let seed = GoalSeed::new("g1", "pattern");
        let node = GoalNode::from_seed(&seed, 0.8);
        assert_eq!(node.id, "g1");
        assert_eq!(node.priority, 0.8);
        assert!(node.is_active());
        assert_eq!(node.progress, 0.0);
    }

    #[test]
    fn goal_node_completion() {
        let seed = GoalSeed::new("g1", "pattern");
        let mut node = GoalNode::from_seed(&seed, 0.8);
        node.update_progress(0.5, 0.95);
        assert!(node.is_active());

        node.update_progress(0.96, 0.95);
        assert_eq!(node.status, GoalStatus::Completed);
    }

    #[test]
    fn goal_node_pruning() {
        let seed = GoalSeed::new("g1", "pattern");
        let node = GoalNode::from_seed(&seed, 0.05);
        assert!(node.should_prune(0.1));

        let node2 = GoalNode::from_seed(&seed, 0.5);
        assert!(!node2.should_prune(0.1));
    }

    #[test]
    fn goal_tree_full_lifecycle() {
        let mut tree = GoalTree::new(3, 5.0, 0.95, 0.1);

        // Observe patterns
        tree.observe("fix-first", "fix compile errors before testing", 2.0);
        tree.observe("fix-first", "fix compile errors before testing", 2.0);
        tree.observe("fix-first", "fix compile errors before testing", 2.0);

        // Not enough observations yet (3 observations, but first has score 2.0 not 1.0)
        // Actually: observe creates with score=weight, then observe adds weight
        // First: new with score 2.0, observe(2.0) => 4.0, observe(2.0) => 6.0
        assert_eq!(tree.seed_count(), 1);

        // Promote eligible seeds
        let promoted = tree.promote_seeds();
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0], "fix-first");
        assert_eq!(tree.seed_count(), 0);
        assert_eq!(tree.node_count(), 1);

        // Check the promoted goal
        let goal = tree.get("fix-first").unwrap();
        assert!(goal.is_active());
        assert_eq!(goal.progress, 0.0);

        // Update progress
        tree.update_progress("fix-first", 0.5);
        assert!(tree.get("fix-first").unwrap().is_active());

        tree.update_progress("fix-first", 0.96);
        assert_eq!(tree.get("fix-first").unwrap().status, GoalStatus::Completed);
    }

    #[test]
    fn goal_tree_hierarchy() {
        let mut tree = GoalTree::with_defaults();

        // Manually insert nodes for hierarchy testing
        tree.observe("parent", "high-level goal", 10.0);
        tree.observe("parent", "high-level goal", 10.0);
        tree.observe("parent", "high-level goal", 10.0);
        tree.observe("child", "sub-goal", 10.0);
        tree.observe("child", "sub-goal", 10.0);
        tree.observe("child", "sub-goal", 10.0);
        tree.promote_seeds();

        assert!(tree.set_parent("child", "parent"));
        let parent = tree.get("parent").unwrap();
        assert!(parent.children.contains(&"child".to_string()));
        let child = tree.get("child").unwrap();
        assert_eq!(child.parent.as_deref(), Some("parent"));

        // Roots should only include parent
        let roots = tree.roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].id, "parent");
    }

    #[test]
    fn goal_tree_prune() {
        let mut tree = GoalTree::new(1, 0.5, 0.95, 0.1);
        tree.observe("strong", "pattern a", 5.0);
        tree.observe("weak", "pattern b", 0.05);
        assert_eq!(tree.seed_count(), 2);

        let pruned = tree.prune();
        assert_eq!(pruned, 1); // weak seed pruned
        assert_eq!(tree.seed_count(), 1);
    }

    #[test]
    fn goal_tree_decay_seeds() {
        let mut tree = GoalTree::with_defaults();
        tree.observe("a", "pattern", 1.0);
        tree.observe("b", "pattern", 2.0);
        tree.decay_seeds(0.5);

        // Seeds' scores should be halved
        assert_eq!(tree.seed_count(), 2);
    }

    #[test]
    fn active_goals_sorted_by_priority() {
        let mut tree = GoalTree::new(1, 1.0, 0.95, 0.0);
        tree.observe("low", "low priority", 1.0);
        tree.observe("high", "high priority", 10.0);
        tree.promote_seeds();

        let active = tree.active_goals();
        assert_eq!(active.len(), 2);
        assert!(active[0].priority >= active[1].priority);
    }

    #[test]
    fn serde_roundtrip_goal_seed() {
        let seed = GoalSeed::new("test", "a pattern");
        let json = serde_json::to_string(&seed).unwrap();
        let back: GoalSeed = serde_json::from_str(&json).unwrap();
        assert_eq!(seed, back);
    }

    #[test]
    fn serde_roundtrip_goal_node() {
        let seed = GoalSeed::new("test", "a pattern");
        let node = GoalNode::from_seed(&seed, 0.7);
        let json = serde_json::to_string(&node).unwrap();
        let back: GoalNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, back);
    }
}
