//! Multi-dimensional resource-aware scheduling (ORCH-08).
//!
//! Extends the basic [`ResourceBudget`](super::ResourceBudget) with the full
//! five-resource model from the spec: agent slots, API rate limits, LLM token
//! budget, worktree slots, and USD cost budget. The executor tick loop can use
//! [`FullResourceBudget::can_schedule`] before dispatching a task and
//! [`FullResourceBudget::reserve`] / [`FullResourceBudget::release`] to track
//! live resource consumption.

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Supporting types
// ---------------------------------------------------------------------------

/// A pool of identical bounded resources (agent slots, worktree slots, etc.).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourcePool {
    /// Total capacity of the pool.
    pub capacity: usize,
    /// Number of units currently in use.
    pub in_use: usize,
}

impl ResourcePool {
    /// Create a pool with the given capacity.
    #[must_use]
    pub const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            in_use: 0,
        }
    }

    /// Whether at least one unit is available.
    #[must_use]
    pub const fn available(&self) -> bool {
        self.in_use < self.capacity
    }

    /// How many units are free.
    #[must_use]
    pub const fn free(&self) -> usize {
        self.capacity.saturating_sub(self.in_use)
    }

    /// Acquire one unit. Returns `false` if none available.
    pub fn acquire(&mut self) -> bool {
        if self.available() {
            self.in_use += 1;
            true
        } else {
            false
        }
    }

    /// Release one unit.
    pub fn release(&mut self) {
        self.in_use = self.in_use.saturating_sub(1);
    }
}

impl Default for ResourcePool {
    fn default() -> Self {
        Self::new(8)
    }
}

/// Token-bucket rate limiter for API calls.
///
/// Models a bucket that refills at a constant rate and allows bursts up
/// to `capacity`. Each API call costs one token.
#[derive(Clone, Debug)]
pub struct RateLimitResource {
    /// Maximum burst capacity.
    pub capacity: u32,
    /// Tokens per second refill rate.
    pub refill_rate: f64,
    /// Current available tokens (fractional to allow smooth refill).
    pub current_tokens: f64,
    /// When the bucket was last updated.
    pub last_update: Instant,
}

impl RateLimitResource {
    /// Create a new rate limiter with the given capacity and refill rate.
    #[must_use]
    pub fn new(capacity: u32, refill_rate: f64) -> Self {
        Self {
            capacity,
            refill_rate,
            current_tokens: f64::from(capacity),
            last_update: Instant::now(),
        }
    }

    /// Refill the bucket based on elapsed time.
    pub fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.current_tokens =
            (self.current_tokens + elapsed * self.refill_rate).min(f64::from(self.capacity));
        self.last_update = now;
    }

    /// Try to consume one token. Returns `true` if allowed.
    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.current_tokens >= 1.0 {
            self.current_tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Whether at least one token is available (after refill).
    #[must_use]
    pub fn available(&mut self) -> bool {
        self.refill();
        self.current_tokens >= 1.0
    }
}

impl Default for RateLimitResource {
    fn default() -> Self {
        // 60 calls/min = 1 call/sec with burst of 10.
        Self::new(10, 1.0)
    }
}

/// Depletable LLM token budget.
///
/// Tracks total token allocation and per-task defaults with
/// complexity-based multipliers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenBudget {
    /// Total tokens available for the session.
    pub total: u64,
    /// Tokens spent so far.
    pub spent: u64,
    /// Default tokens per task.
    pub per_task_default: u64,
    /// Hard cap per task.
    pub per_task_max: u64,
    /// Per-complexity multipliers (e.g., "complex" -> 2.0, "trivial" -> 0.5).
    #[serde(default)]
    pub complexity_multiplier: HashMap<String, f64>,
}

impl TokenBudget {
    /// Remaining tokens.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.total.saturating_sub(self.spent)
    }

    /// Compute the token allocation for a task with the given complexity.
    #[must_use]
    pub fn allocation_for(&self, complexity: &str) -> u64 {
        let multiplier = self
            .complexity_multiplier
            .get(complexity)
            .copied()
            .unwrap_or(1.0);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let raw = (self.per_task_default as f64 * multiplier) as u64;
        raw.min(self.per_task_max).min(self.remaining())
    }

    /// Whether there are enough tokens for at least one default-complexity task.
    #[must_use]
    pub fn has_budget(&self) -> bool {
        self.remaining() >= self.per_task_default
    }

    /// Record token usage.
    pub fn spend(&mut self, tokens: u64) {
        self.spent = self.spent.saturating_add(tokens);
    }
}

impl Default for TokenBudget {
    fn default() -> Self {
        Self {
            total: 10_000_000, // 10M tokens
            spent: 0,
            per_task_default: 100_000, // 100K per task
            per_task_max: 500_000,     // 500K cap per task
            complexity_multiplier: HashMap::new(),
        }
    }
}

/// USD cost budget with warning and hard-stop thresholds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostBudget {
    /// Total USD budget for the session.
    pub total_usd: f64,
    /// USD spent so far.
    pub spent_usd: f64,
    /// Fraction of total at which to emit a warning (default 0.8).
    pub warn_threshold: f64,
    /// Fraction of total at which to hard-stop (default 1.0).
    pub stop_threshold: f64,
}

impl CostBudget {
    /// Remaining USD budget.
    #[must_use]
    pub fn remaining(&self) -> f64 {
        (self.total_usd - self.spent_usd).max(0.0)
    }

    /// Whether the budget is exhausted (at or past the stop threshold).
    #[must_use]
    pub fn is_exhausted(&self) -> bool {
        self.spent_usd >= self.total_usd * self.stop_threshold
    }

    /// Whether the budget is in the warning zone.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.spent_usd >= self.total_usd * self.warn_threshold && !self.is_exhausted()
    }

    /// Record a cost expenditure.
    pub fn spend(&mut self, usd: f64) {
        self.spent_usd += usd;
    }
}

impl Default for CostBudget {
    fn default() -> Self {
        Self {
            total_usd: 100.0,
            spent_usd: 0.0,
            warn_threshold: 0.80,
            stop_threshold: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Resource check and reservation
// ---------------------------------------------------------------------------

/// Result of checking whether a task can be scheduled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceCheck {
    /// All resources available; dispatch is allowed.
    Available,
    /// One or more resources are blocked. The `Vec` lists the blocked
    /// resource names.
    Blocked(Vec<String>),
}

impl ResourceCheck {
    /// Whether scheduling is allowed.
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// A reservation handle returned from [`FullResourceBudget::reserve`].
///
/// Holds the resources claimed for one task. The caller must pass this
/// back to [`FullResourceBudget::release`] when the task completes.
#[derive(Clone, Debug)]
pub struct ResourceReservation {
    /// Which agent slot pool was claimed (always 1 slot).
    pub agent_slot: bool,
    /// Which worktree slot pool was claimed (always 1 slot if true).
    pub worktree_slot: bool,
    /// Token allocation for this task.
    pub token_allocation: u64,
    /// Complexity label used for token allocation.
    pub complexity: String,
}

// ---------------------------------------------------------------------------
// FullResourceBudget
// ---------------------------------------------------------------------------

/// Five-dimensional resource budget for the executor.
///
/// Wraps agent slots, API rate limiter, LLM token budget, worktree slots,
/// and cost budget into a single struct with `can_schedule` / `reserve` /
/// `release` methods for the tick loop.
#[derive(Clone, Debug)]
pub struct FullResourceBudget {
    /// Bounded pool of concurrent agent slots.
    pub agent_slots: ResourcePool,
    /// Token-bucket rate limiter for API calls.
    pub api_tokens: RateLimitResource,
    /// Depletable LLM token budget.
    pub token_budget: TokenBudget,
    /// Bounded pool of git worktree slots.
    pub worktree_slots: ResourcePool,
    /// USD cost budget with warning/stop thresholds.
    pub cost_budget: CostBudget,
}

impl Default for FullResourceBudget {
    fn default() -> Self {
        Self {
            agent_slots: ResourcePool::new(8),
            api_tokens: RateLimitResource::default(),
            token_budget: TokenBudget::default(),
            worktree_slots: ResourcePool::new(4),
            cost_budget: CostBudget::default(),
        }
    }
}

/// Task descriptor for scheduling decisions.
///
/// Contains just enough information for the resource budget to decide
/// whether a task can be scheduled and how many resources to reserve.
#[derive(Clone, Debug, Default)]
pub struct TaskResourceRequest {
    /// Complexity label (e.g., "trivial", "standard", "complex").
    pub complexity: String,
    /// Whether the task needs an isolated git worktree.
    pub needs_worktree: bool,
    /// Estimated USD cost for this task.
    pub estimated_cost_usd: f64,
}

impl FullResourceBudget {
    /// Check whether the given task can be scheduled without blocking.
    ///
    /// This is a read-only check that does not modify any state (except
    /// for the rate-limiter refill which is idempotent).
    #[must_use]
    pub fn can_schedule(&self, task: &TaskResourceRequest) -> ResourceCheck {
        let mut blocked = Vec::new();

        if !self.agent_slots.available() {
            blocked.push("agent_slots".into());
        }
        // Rate limiter: clone to avoid mutating self.
        if !self.api_tokens.clone().available() {
            blocked.push("api_tokens".into());
        }
        if !self.token_budget.has_budget() {
            blocked.push("token_budget".into());
        }
        if task.needs_worktree && !self.worktree_slots.available() {
            blocked.push("worktree_slots".into());
        }
        if self.cost_budget.is_exhausted() {
            blocked.push("cost_budget".into());
        }

        if blocked.is_empty() {
            ResourceCheck::Available
        } else {
            ResourceCheck::Blocked(blocked)
        }
    }

    /// Reserve resources for a task.
    ///
    /// Returns a [`ResourceReservation`] on success, or the list of
    /// blocked resources on failure.
    ///
    /// # Errors
    ///
    /// Returns `Err(Vec<String>)` if any required resource is unavailable.
    pub fn reserve(
        &mut self,
        task: &TaskResourceRequest,
    ) -> Result<ResourceReservation, Vec<String>> {
        // Pre-check before mutating.
        let check = self.can_schedule(task);
        if let ResourceCheck::Blocked(reasons) = check {
            return Err(reasons);
        }

        // Acquire resources.
        self.agent_slots.acquire();
        self.api_tokens.try_consume();
        let token_allocation = self.token_budget.allocation_for(&task.complexity);
        let worktree = if task.needs_worktree {
            self.worktree_slots.acquire();
            true
        } else {
            false
        };

        Ok(ResourceReservation {
            agent_slot: true,
            worktree_slot: worktree,
            token_allocation,
            complexity: task.complexity.clone(),
        })
    }

    /// Release resources held by a reservation.
    ///
    /// Call this when a task completes (success or failure) to return
    /// resources to their pools.
    pub fn release(&mut self, reservation: &ResourceReservation) {
        if reservation.agent_slot {
            self.agent_slots.release();
        }
        if reservation.worktree_slot {
            self.worktree_slots.release();
        }
    }

    /// Record actual token and cost expenditure after a task completes.
    pub fn record_usage(&mut self, tokens_spent: u64, cost_usd: f64) {
        self.token_budget.spend(tokens_spent);
        self.cost_budget.spend(cost_usd);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn resource_pool_acquire_release() {
        let mut pool = ResourcePool::new(2);
        assert!(pool.available());
        assert_eq!(pool.free(), 2);

        assert!(pool.acquire());
        assert!(pool.acquire());
        assert!(!pool.acquire()); // full
        assert_eq!(pool.free(), 0);

        pool.release();
        assert_eq!(pool.free(), 1);
        assert!(pool.acquire());
    }

    #[test]
    fn rate_limit_basic() {
        let mut rl = RateLimitResource::new(2, 100.0);
        // Should have full capacity initially.
        assert!(rl.try_consume());
        assert!(rl.try_consume());
        // Third should fail (no time passed for refill).
        assert!(!rl.try_consume());
    }

    #[test]
    fn token_budget_allocation() {
        let mut budget = TokenBudget {
            total: 1_000_000,
            spent: 0,
            per_task_default: 100_000,
            per_task_max: 500_000,
            complexity_multiplier: HashMap::from([
                ("trivial".into(), 0.5),
                ("complex".into(), 2.0),
            ]),
        };

        assert_eq!(budget.allocation_for("standard"), 100_000);
        assert_eq!(budget.allocation_for("trivial"), 50_000);
        assert_eq!(budget.allocation_for("complex"), 200_000);
        assert!(budget.has_budget());

        budget.spend(950_000);
        // Only 50K left, so complex task (200K) gets capped to 50K.
        assert_eq!(budget.allocation_for("complex"), 50_000);
    }

    #[test]
    fn cost_budget_thresholds() {
        let mut cost = CostBudget {
            total_usd: 100.0,
            spent_usd: 0.0,
            warn_threshold: 0.80,
            stop_threshold: 1.0,
        };

        assert!(!cost.is_warning());
        assert!(!cost.is_exhausted());

        cost.spend(85.0);
        assert!(cost.is_warning());
        assert!(!cost.is_exhausted());

        cost.spend(15.0);
        assert!(cost.is_exhausted());
        assert!(!cost.is_warning()); // past warning zone
    }

    #[test]
    fn full_budget_can_schedule_and_reserve() {
        let mut budget = FullResourceBudget::default();
        let task = TaskResourceRequest {
            complexity: "standard".into(),
            needs_worktree: true,
            estimated_cost_usd: 1.0,
        };

        assert!(budget.can_schedule(&task).is_available());

        let reservation = budget.reserve(&task).unwrap();
        assert!(reservation.agent_slot);
        assert!(reservation.worktree_slot);
        assert!(reservation.token_allocation > 0);

        // Agent slots have capacity - 1 now.
        assert_eq!(budget.agent_slots.free(), 7);
        assert_eq!(budget.worktree_slots.free(), 3);

        budget.release(&reservation);
        assert_eq!(budget.agent_slots.free(), 8);
        assert_eq!(budget.worktree_slots.free(), 4);
    }

    #[test]
    fn full_budget_blocks_when_exhausted() {
        let mut budget = FullResourceBudget {
            agent_slots: ResourcePool::new(1),
            cost_budget: CostBudget {
                total_usd: 10.0,
                spent_usd: 10.0,
                warn_threshold: 0.8,
                stop_threshold: 1.0,
            },
            ..FullResourceBudget::default()
        };

        let task = TaskResourceRequest::default();
        let check = budget.can_schedule(&task);
        assert!(
            matches!(check, ResourceCheck::Blocked(ref reasons) if reasons.contains(&"cost_budget".to_string()))
        );
    }

    #[test]
    fn full_budget_blocks_when_no_agent_slots() {
        let mut budget = FullResourceBudget {
            agent_slots: ResourcePool::new(0),
            ..FullResourceBudget::default()
        };

        let task = TaskResourceRequest::default();
        assert!(!budget.can_schedule(&task).is_available());
    }

    #[test]
    fn record_usage_depletes_budgets() {
        let mut budget = FullResourceBudget::default();
        budget.record_usage(500_000, 25.0);
        assert_eq!(budget.token_budget.spent, 500_000);
        assert_eq!(budget.cost_budget.spent_usd, 25.0);
    }

    #[test]
    fn reserve_fails_with_blocked_resources() {
        let mut budget = FullResourceBudget {
            agent_slots: ResourcePool::new(0),
            ..FullResourceBudget::default()
        };

        let task = TaskResourceRequest::default();
        let err = budget.reserve(&task).unwrap_err();
        assert!(err.contains(&"agent_slots".to_string()));
    }
}
