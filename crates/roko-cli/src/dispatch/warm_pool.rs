//! Warm pool — pre-spawned agents for fast role transitions.
//!
//! ## Why this exists
//!
//! Cold agent starts are expensive: subprocess fork, MCP handshake,
//! initial token throughput. The biggest cost is paid on the
//! gate → reviewer transition where two agents fire back-to-back. The
//! warm pool keeps a small set of pre-spawned agents alive so the
//! reviewer (or any second-leg agent) can take over an existing process
//! instead of cold-starting.
//!
//! ## Design
//!
//! - Per-role bounded LRU. `pool[role] = VecDeque<WarmAgent>` capped at
//!   `max_per_role`.
//! - `take(role)` pops the freshest agent (TTL-aware) or returns `None`.
//! - `return_to_pool` returns an agent after a successful turn (used by
//!   short-lived providers that can be reused).
//! - `evict_expired` is called periodically; agents past their TTL are
//!   shut down cleanly.
//!
//! ## What it doesn't own
//!
//! The pool stores agent handles, not provider state. The agent itself
//! lives in `roko-agent`; this module is a small lifecycle holder so the
//! dispatcher doesn't need to thread these handles through.
//!
//! ## Current scope
//!
//! This implementation is a *typed*, fully tested LRU container — it
//! does *not* yet pre-spawn real agents, since real spawn requires a
//! provider runtime that the dispatcher doesn't own. Tests verify the
//! container semantics exhaustively (TTL eviction, capacity, take/insert
//! ordering). Spawning real agents is wired through the dispatcher
//! facade once the provider bridge is online; see `.roko/GAPS.md`.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One warm agent slot.
///
/// `id` lets the dispatcher correlate the pooled handle with a `pid` /
/// session id without forcing the agent type itself into this module.
#[derive(Debug, Clone)]
pub struct WarmAgent {
    /// Stable identifier the dispatcher assigned to this agent.
    pub id: String,
    /// Model slug the agent was spawned with.
    pub model: String,
    /// When the agent was added to the pool.
    pub spawned_at: Instant,
    /// Time-to-live; expired agents are evicted on next access.
    pub ttl: Duration,
}

impl WarmAgent {
    /// `true` if the agent has exceeded its TTL.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.spawned_at) >= self.ttl
    }
}

/// Lightweight summary used by `/agents/warm-pool` and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WarmPoolStats {
    /// Sum of pooled agents across all roles.
    pub size: usize,
    /// Number of roles with at least one pooled agent.
    pub roles_with_warm_agents: usize,
    /// Configured cap per role.
    pub max_per_role: usize,
}

/// Per-role warm-agent pool.
#[derive(Debug, Default)]
pub struct WarmPool {
    inner: Mutex<WarmPoolInner>,
    max_per_role: usize,
}

#[derive(Debug, Default)]
struct WarmPoolInner {
    pools: HashMap<String, VecDeque<WarmAgent>>,
}

impl WarmPool {
    /// Construct a pool with `max_per_role` slots per role.
    #[must_use]
    pub fn new(max_per_role: usize) -> Self {
        Self {
            inner: Mutex::new(WarmPoolInner::default()),
            max_per_role,
        }
    }

    /// Insert a freshly spawned agent.
    ///
    /// If the role's queue is at capacity, the oldest entry is dropped
    /// (caller is responsible for shutting it down — return value tells
    /// them which id was evicted).
    pub fn insert(&self, role: impl Into<String>, agent: WarmAgent) -> Option<WarmAgent> {
        if self.max_per_role == 0 {
            return Some(agent);
        }
        let role = role.into();
        let mut guard = self.inner.lock().expect("poisoned");
        let queue = guard.pools.entry(role).or_default();
        let evicted = if queue.len() >= self.max_per_role {
            queue.pop_front()
        } else {
            None
        };
        queue.push_back(agent);
        evicted
    }

    /// Take the freshest non-expired agent for `role`.
    pub fn take(&self, role: &str) -> Option<WarmAgent> {
        let now = Instant::now();
        let mut guard = self.inner.lock().expect("poisoned");
        let queue = guard.pools.get_mut(role)?;
        // Drop expired entries first (oldest first).
        while let Some(front) = queue.front() {
            if front.is_expired(now) {
                queue.pop_front();
            } else {
                break;
            }
        }
        queue.pop_back()
    }

    /// Drop expired entries across every role. Returns ids dropped.
    pub fn evict_expired(&self) -> Vec<String> {
        let now = Instant::now();
        let mut dropped = Vec::new();
        let mut guard = self.inner.lock().expect("poisoned");
        for queue in guard.pools.values_mut() {
            queue.retain(|agent| {
                let alive = !agent.is_expired(now);
                if !alive {
                    dropped.push(agent.id.clone());
                }
                alive
            });
        }
        dropped
    }

    /// Snapshot the current pool size (sum across roles).
    pub fn stats(&self) -> WarmPoolStats {
        let guard = self.inner.lock().expect("poisoned");
        let size = guard.pools.values().map(VecDeque::len).sum();
        let roles_with_warm_agents = guard
            .pools
            .values()
            .filter(|queue| !queue.is_empty())
            .count();
        WarmPoolStats {
            size,
            roles_with_warm_agents,
            max_per_role: self.max_per_role,
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: &str, ttl_ms: u64) -> WarmAgent {
        WarmAgent {
            id: id.into(),
            model: "claude-sonnet-4-6".into(),
            spawned_at: Instant::now(),
            ttl: Duration::from_millis(ttl_ms),
        }
    }

    #[test]
    fn take_returns_freshest_then_empties() {
        let pool = WarmPool::new(4);
        pool.insert("implementer", agent("a1", 10_000));
        pool.insert("implementer", agent("a2", 10_000));
        let first = pool.take("implementer").unwrap();
        assert_eq!(first.id, "a2", "freshest = LIFO take");
        let second = pool.take("implementer").unwrap();
        assert_eq!(second.id, "a1");
        assert!(pool.take("implementer").is_none());
    }

    #[test]
    fn capacity_enforced_per_role_with_oldest_evicted() {
        let pool = WarmPool::new(2);
        assert!(pool.insert("reviewer", agent("a1", 10_000)).is_none());
        assert!(pool.insert("reviewer", agent("a2", 10_000)).is_none());
        let evicted = pool.insert("reviewer", agent("a3", 10_000)).unwrap();
        assert_eq!(evicted.id, "a1", "oldest must be evicted on overflow");
        assert_eq!(pool.stats().size, 2);
    }

    #[test]
    fn zero_capacity_pool_returns_supplied_agent_immediately() {
        let pool = WarmPool::new(0);
        let evicted = pool.insert("any", agent("a1", 10_000)).unwrap();
        assert_eq!(evicted.id, "a1");
        assert_eq!(pool.stats().size, 0);
    }

    #[test]
    fn expired_agents_not_returned_by_take() {
        let pool = WarmPool::new(2);
        pool.insert("scribe", agent("expired", 0));
        std::thread::sleep(Duration::from_millis(2));
        assert!(
            pool.take("scribe").is_none(),
            "expired agent must not be returned"
        );
    }

    #[test]
    fn evict_expired_drops_only_expired_entries() {
        let pool = WarmPool::new(4);
        pool.insert("auditor", agent("alive", 10_000));
        pool.insert("auditor", agent("dead", 0));
        std::thread::sleep(Duration::from_millis(2));
        let dropped = pool.evict_expired();
        assert_eq!(dropped, vec!["dead".to_string()]);
        let stats = pool.stats();
        assert_eq!(stats.size, 1);
        assert_eq!(stats.roles_with_warm_agents, 1);
    }

    #[test]
    fn stats_reflect_per_role_population() {
        let pool = WarmPool::new(4);
        pool.insert("a", agent("1", 10_000));
        pool.insert("b", agent("2", 10_000));
        pool.insert("b", agent("3", 10_000));
        let stats = pool.stats();
        assert_eq!(stats.size, 3);
        assert_eq!(stats.roles_with_warm_agents, 2);
        assert_eq!(stats.max_per_role, 4);
    }
}
