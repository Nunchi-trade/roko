//! `MultiAgentPool` — parallel agent execution across multiple roles.
//!
//! Manages multiple [`AgentPool`](super::pool::AgentPool) instances for
//! concurrent execution, with warm-pool pre-spawning so agents are ready
//! to accept work without cold-start latency.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use roko_core::AgentRole;

use crate::agent::{Agent, AgentResult};
use crate::pool::{AgentInstanceId, AgentTask, InstanceStatus, TaskOutcome};
use crate::session::{WarmReusePolicy, WarmReuseRequest};

// ─── WarmEntry ───────────────────────────────────────────────────────────

/// A pre-spawned agent waiting in the warm pool.
pub struct WarmEntry {
    /// The agent implementation, ready to run.
    pub agent: Arc<dyn Agent>,
    /// When this entry was added to the warm pool.
    pub spawned_at: Instant,
    /// Reuse policy and scope for this warm entry.
    pub reuse_policy: WarmReusePolicy,
}

// ─── ActiveEntry ─────────────────────────────────────────────────────────

/// An active agent instance with its current state.
struct ActiveEntry {
    /// The agent implementation.
    agent: Arc<dyn Agent>,
    /// Current lifecycle status.
    status: InstanceStatus,
    /// The last result (if completed).
    last_result: Option<AgentResult>,
}

// ─── MultiAgentPool ──────────────────────────────────────────────────────

/// Parallel agent pool: manages multiple agent instances across roles.
///
/// Supports:
/// - Pre-spawning warm agents that are ready to accept work immediately.
/// - Promoting warm agents to active when work arrives.
/// - Evicting idle warm agents after a timeout.
/// - Concurrency limits per role.
/// - Bulk kill operations (all, by plan, by role).
pub struct MultiAgentPool {
    /// Active agent instances.
    active: HashMap<AgentInstanceId, ActiveEntry>,
    /// Warm pool: pre-spawned agents keyed by `(role, instance_name)`.
    warm: HashMap<(AgentRole, String), WarmEntry>,
    /// Optional fallback agents per role (used when primary fails).
    fallbacks: HashMap<AgentRole, Arc<dyn Agent>>,
    /// Maximum concurrent active instances per role.
    concurrency_limits: HashMap<AgentRole, usize>,
    /// Default concurrency limit when no per-role limit is set.
    default_concurrency: usize,
}

impl MultiAgentPool {
    /// Create an empty pool with a default concurrency limit of 4.
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            warm: HashMap::new(),
            fallbacks: HashMap::new(),
            concurrency_limits: HashMap::new(),
            default_concurrency: 4,
        }
    }

    /// Set the default concurrency limit for roles without a specific limit.
    #[must_use]
    pub const fn with_default_concurrency(mut self, limit: usize) -> Self {
        self.default_concurrency = limit;
        self
    }

    /// Set the concurrency limit for a specific role.
    pub fn set_concurrency_limit(&mut self, role: AgentRole, limit: usize) {
        self.concurrency_limits.insert(role, limit);
    }

    /// Set a fallback agent for a role.
    pub fn set_fallback(&mut self, role: AgentRole, agent: Arc<dyn Agent>) {
        self.fallbacks.insert(role, agent);
    }

    /// Get the concurrency limit for a role.
    #[must_use]
    pub fn concurrency_limit(&self, role: AgentRole) -> usize {
        self.concurrency_limits
            .get(&role)
            .copied()
            .unwrap_or(self.default_concurrency)
    }

    // ── Warm pool ────────────────────────────────────────────────────────

    /// Pre-spawn `count` warm agents for a role. Each gets a unique instance
    /// name of the form `"warm-{i}"`.
    ///
    /// Agents added to the warm pool are ready to be promoted to active
    /// immediately when work arrives, avoiding cold-start latency.
    pub fn pre_spawn_warm(
        &mut self,
        role: AgentRole,
        count: usize,
        agent_fn: &dyn Fn() -> Arc<dyn Agent>,
    ) {
        let now = Instant::now();
        for i in 0..count {
            let instance = format!("warm-{i}");
            let key = (role, instance);
            self.warm.entry(key).or_insert_with(|| WarmEntry {
                agent: agent_fn(),
                spawned_at: now,
                reuse_policy: WarmReusePolicy::stateless("legacy-warm"),
            });
        }
    }

    /// Pre-spawn a single warm agent with a specific instance name.
    pub fn pre_spawn_warm_named(
        &mut self,
        role: AgentRole,
        instance: impl Into<String>,
        agent: Arc<dyn Agent>,
    ) {
        let instance = instance.into();
        let key = (role, instance);
        self.warm.entry(key).or_insert(WarmEntry {
            agent,
            spawned_at: Instant::now(),
            reuse_policy: WarmReusePolicy::stateless("legacy-warm"),
        });
    }

    /// Pre-spawn a named warm agent with an explicit checked reuse policy.
    pub fn pre_spawn_warm_named_with_policy(
        &mut self,
        role: AgentRole,
        instance: impl Into<String>,
        agent: Arc<dyn Agent>,
        reuse_policy: WarmReusePolicy,
    ) {
        let instance = instance.into();
        let key = (role, instance);
        self.warm.entry(key).or_insert(WarmEntry {
            agent,
            spawned_at: Instant::now(),
            reuse_policy,
        });
    }

    /// Promote a warm agent to active status. Returns the `AgentInstanceId`
    /// if a warm agent was available for the role, or `None` if the warm pool
    /// is empty for that role.
    ///
    /// The promoted agent is removed from the warm pool and added to the
    /// active set with `InstanceStatus::Active`.
    pub fn promote_warm(&mut self, role: AgentRole) -> Option<AgentInstanceId> {
        // Find the first warm entry for this role.
        let key = self.warm.keys().find(|(r, _)| *r == role)?.clone();

        let entry = self.warm.remove(&key)?;
        let id = AgentInstanceId::new(role, key.1);
        self.active.insert(
            id.clone(),
            ActiveEntry {
                agent: entry.agent,
                status: InstanceStatus::Active,
                last_result: None,
            },
        );
        Some(id)
    }

    /// Promote a warm agent with a specific instance name.
    pub fn promote_warm_named(
        &mut self,
        role: AgentRole,
        instance: &str,
    ) -> Option<AgentInstanceId> {
        let key = (role, instance.to_string());
        let entry = self.warm.remove(&key)?;
        let id = AgentInstanceId::new(role, instance);
        self.active.insert(
            id.clone(),
            ActiveEntry {
                agent: entry.agent,
                status: InstanceStatus::Active,
                last_result: None,
            },
        );
        Some(id)
    }

    /// Promote a warm agent only when its reuse policy matches the request.
    ///
    /// This is the production-safe warm reuse path: callers must supply the
    /// desired scope and fingerprints, so a session warmed with old context
    /// cannot bleed into a new task silently.
    pub fn promote_warm_for(
        &mut self,
        role: AgentRole,
        request: &WarmReuseRequest,
    ) -> Option<AgentInstanceId> {
        let now = Instant::now();
        let key = self
            .warm
            .iter()
            .find(|((warm_role, _), entry)| {
                *warm_role == role && entry.reuse_policy.allows(request, entry.spawned_at, now)
            })
            .map(|(key, _)| key.clone())?;

        let entry = self.warm.remove(&key)?;
        let id = AgentInstanceId::new(role, key.1);
        self.active.insert(
            id.clone(),
            ActiveEntry {
                agent: entry.agent,
                status: InstanceStatus::Active,
                last_result: None,
            },
        );
        Some(id)
    }

    /// Promote a warm agent to active status only when role capacity allows it.
    ///
    /// Returns `None` when no warm entry is available or the role is already at
    /// capacity.
    pub fn promote_warm_if_capacity(&mut self, role: AgentRole) -> Option<AgentInstanceId> {
        if self.at_capacity(role) {
            return None;
        }
        self.promote_warm(role)
    }

    /// Promote a named warm agent to active status only when role capacity
    /// allows it.
    pub fn promote_warm_named_if_capacity(
        &mut self,
        role: AgentRole,
        instance: &str,
    ) -> Option<AgentInstanceId> {
        if self.at_capacity(role) {
            return None;
        }
        self.promote_warm_named(role, instance)
    }

    /// Ensure that an instance is active for the given role/instance name.
    ///
    /// Activation strategy:
    /// 1. Reuse an already-active matching instance.
    /// 2. Promote a named warm instance.
    /// 3. Promote any warm instance for the role.
    /// 4. Spawn and add a fresh active instance.
    ///
    /// Returns `None` when no activation path is possible (usually capacity).
    pub fn ensure_active_instance<F>(
        &mut self,
        role: AgentRole,
        instance: &str,
        agent_fn: &F,
    ) -> Option<AgentInstanceId>
    where
        F: Fn(AgentRole, &str) -> Arc<dyn Agent> + Send + Sync + ?Sized,
    {
        let id = AgentInstanceId::new(role, instance);
        if self.active.contains_key(&id) {
            return Some(id);
        }

        if let Some(promoted) = self.promote_warm_named_if_capacity(role, instance) {
            return Some(promoted);
        }
        if let Some(promoted) = self.promote_warm_if_capacity(role) {
            return Some(promoted);
        }

        if self.at_capacity(role) {
            return None;
        }

        let spawned = agent_fn(role, instance);
        if self.add_active(id.clone(), spawned) {
            Some(id)
        } else {
            None
        }
    }

    /// Evict warm agents for a role that have been idle longer than `max_idle`.
    ///
    /// Returns the number of agents evicted.
    pub fn evict_warm(&mut self, role: AgentRole, max_idle: Duration) -> usize {
        let now = Instant::now();
        let keys_to_remove: Vec<(AgentRole, String)> = self
            .warm
            .iter()
            .filter(|((r, _), entry)| {
                *r == role && now.duration_since(entry.spawned_at) >= max_idle
            })
            .map(|(k, _)| k.clone())
            .collect();
        let count = keys_to_remove.len();
        for key in keys_to_remove {
            self.warm.remove(&key);
        }
        count
    }

    /// Evict all warm agents for a role regardless of idle time.
    pub fn evict_warm_all(&mut self, role: AgentRole) -> usize {
        let keys_to_remove: Vec<(AgentRole, String)> = self
            .warm
            .keys()
            .filter(|(r, _)| *r == role)
            .cloned()
            .collect();
        let count = keys_to_remove.len();
        for key in keys_to_remove {
            self.warm.remove(&key);
        }
        count
    }

    /// Number of warm agents for a role.
    #[must_use]
    pub fn warm_count(&self, role: AgentRole) -> usize {
        self.warm.keys().filter(|(r, _)| *r == role).count()
    }

    /// Total number of warm agents across all roles.
    #[must_use]
    pub fn total_warm_count(&self) -> usize {
        self.warm.len()
    }

    /// Compatibility alias for docs that name the stale-entry eviction path explicitly.
    pub fn evict_stale_warm(&mut self, role: AgentRole, max_idle: Duration) -> usize {
        self.evict_warm(role, max_idle)
    }

    // ── Active instances ─────────────────────────────────────────────────

    /// Add an active agent instance directly (bypass warm pool).
    ///
    /// Returns `false` if the concurrency limit for the role would be
    /// exceeded, or if an instance with the same ID already exists.
    pub fn add_active(&mut self, id: AgentInstanceId, agent: Arc<dyn Agent>) -> bool {
        if self.active.contains_key(&id) {
            return false;
        }
        let current = self.active_count_for_role(id.role);
        if current >= self.concurrency_limit(id.role) {
            return false;
        }
        self.active.insert(
            id,
            ActiveEntry {
                agent,
                status: InstanceStatus::Active,
                last_result: None,
            },
        );
        true
    }

    /// Run a task on a specific active instance. Returns the outcome.
    ///
    /// If the primary run fails and a fallback is configured for the role,
    /// retries with the fallback agent.
    pub async fn run_task(&mut self, task: AgentTask) -> TaskOutcome {
        let id = task.id.clone();

        // Get agent for this instance.
        let agent = match self.active.get(&id) {
            Some(entry) => entry.agent.clone(),
            None => {
                return TaskOutcome {
                    id,
                    result: None,
                    status: InstanceStatus::Failed,
                    used_fallback: false,
                };
            }
        };

        // Update status to active.
        if let Some(entry) = self.active.get_mut(&id) {
            entry.status = InstanceStatus::Active;
        }

        // Run primary.
        let result = agent.run(&task.input, &task.ctx).await;

        if result.success {
            if let Some(entry) = self.active.get_mut(&id) {
                entry.status = InstanceStatus::Done;
                entry.last_result = Some(result.clone());
            }
            return TaskOutcome {
                id,
                result: Some(result),
                status: InstanceStatus::Done,
                used_fallback: false,
            };
        }

        // Primary failed — try fallback.
        if let Some(fallback) = self.fallbacks.get(&id.role).cloned() {
            let fb_result = fallback.run(&task.input, &task.ctx).await;
            let status = if fb_result.success {
                InstanceStatus::Done
            } else {
                InstanceStatus::Failed
            };
            if let Some(entry) = self.active.get_mut(&id) {
                entry.status = status;
                entry.last_result = Some(fb_result.clone());
            }
            TaskOutcome {
                id,
                result: Some(fb_result),
                status,
                used_fallback: true,
            }
        } else {
            if let Some(entry) = self.active.get_mut(&id) {
                entry.status = InstanceStatus::Failed;
                entry.last_result = Some(result.clone());
            }
            TaskOutcome {
                id,
                result: Some(result),
                status: InstanceStatus::Failed,
                used_fallback: false,
            }
        }
    }

    /// Ensure an active instance exists for `task.id`, then run the task.
    ///
    /// When activation fails (for example because concurrency limits are
    /// saturated), this returns a failed outcome with no result payload.
    pub async fn run_task_with_auto_activation<F>(
        &mut self,
        task: AgentTask,
        agent_fn: &F,
    ) -> TaskOutcome
    where
        F: Fn(AgentRole, &str) -> Arc<dyn Agent> + Send + Sync + ?Sized,
    {
        let id = task.id.clone();
        let role = id.role;
        let instance = id.instance.clone();

        if self
            .ensure_active_instance(role, &instance, agent_fn)
            .is_none()
        {
            return TaskOutcome {
                id,
                result: None,
                status: InstanceStatus::Failed,
                used_fallback: false,
            };
        }

        self.run_task(task).await
    }

    /// Move a terminal active instance (`done`, `failed`, `cancelled`) back
    /// into the warm pool for potential reuse.
    ///
    /// Returns `true` when an instance was recycled.
    pub fn recycle_terminal_to_warm(&mut self, id: &AgentInstanceId) -> bool {
        let Some(entry) = self.active.get(id) else {
            return false;
        };
        if !matches!(
            entry.status,
            InstanceStatus::Done | InstanceStatus::Failed | InstanceStatus::Cancelled
        ) {
            return false;
        }

        let Some(entry) = self.active.remove(id) else {
            return false;
        };
        self.warm.insert(
            (id.role, id.instance.clone()),
            WarmEntry {
                agent: entry.agent,
                spawned_at: Instant::now(),
                reuse_policy: WarmReusePolicy::disabled(),
            },
        );
        true
    }

    /// Remove active instances that are in a terminal state.
    ///
    /// Returns the number of instances removed.
    pub fn reap_terminal_active(&mut self) -> usize {
        let ids: Vec<AgentInstanceId> = self
            .active
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.status,
                    InstanceStatus::Done | InstanceStatus::Failed | InstanceStatus::Cancelled
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        let removed = ids.len();
        for id in ids {
            self.active.remove(&id);
        }
        removed
    }

    // ── Kill operations ──────────────────────────────────────────────────

    /// Kill all agents (active + warm) with a deadline. Agents that haven't
    /// finished being cleaned up within `deadline` are forcibly dropped.
    ///
    /// This is a synchronous operation since our agents are trait objects
    /// without async teardown; the deadline applies to the cleanup loop.
    pub fn kill_all(&mut self, deadline: Duration) -> KillReport {
        let start = Instant::now();
        let mut killed_active = 0usize;
        let mut killed_warm = 0usize;
        let mut aborted = 0usize;

        // Kill active instances.
        let active_ids: Vec<AgentInstanceId> = self.active.keys().cloned().collect();
        for id in active_ids {
            if start.elapsed() >= deadline {
                aborted += 1;
                self.active.remove(&id);
                continue;
            }
            self.active.remove(&id);
            killed_active += 1;
        }

        // Kill warm instances.
        let warm_keys: Vec<(AgentRole, String)> = self.warm.keys().cloned().collect();
        for key in warm_keys {
            if start.elapsed() >= deadline {
                aborted += 1;
                self.warm.remove(&key);
                continue;
            }
            self.warm.remove(&key);
            killed_warm += 1;
        }

        KillReport {
            killed_active,
            killed_warm,
            aborted,
        }
    }

    /// Kill all agents associated with a specific plan, matched by instance
    /// ID substring.
    ///
    /// This kills both active and warm agents whose instance ID key contains
    /// `plan_id`.
    pub fn kill_plan_agents(&mut self, plan_id: &str) -> usize {
        let active_ids: Vec<AgentInstanceId> = self
            .active
            .keys()
            .filter(|id| id.matches(plan_id))
            .cloned()
            .collect();
        let active_count = active_ids.len();
        for id in active_ids {
            self.active.remove(&id);
        }

        let warm_keys: Vec<(AgentRole, String)> = self
            .warm
            .keys()
            .filter(|(_, instance)| instance.contains(plan_id))
            .cloned()
            .collect();
        let warm_count = warm_keys.len();
        for key in warm_keys {
            self.warm.remove(&key);
        }

        active_count + warm_count
    }

    /// Kill all instances of a specific role (active + warm).
    pub fn kill_role(&mut self, role: AgentRole) -> usize {
        let active_ids: Vec<AgentInstanceId> = self
            .active
            .keys()
            .filter(|id| id.role == role)
            .cloned()
            .collect();
        let active_count = active_ids.len();
        for id in active_ids {
            self.active.remove(&id);
        }

        let warm_keys: Vec<(AgentRole, String)> = self
            .warm
            .keys()
            .filter(|(r, _)| *r == role)
            .cloned()
            .collect();
        let warm_count = warm_keys.len();
        for key in warm_keys {
            self.warm.remove(&key);
        }

        active_count + warm_count
    }

    /// Compatibility alias for plan-scoped kill operations.
    pub fn kill_by_plan(&mut self, plan_id: &str) -> usize {
        self.kill_plan_agents(plan_id)
    }

    /// Compatibility alias for role-scoped kill operations.
    pub fn kill_by_role(&mut self, role: AgentRole) -> usize {
        self.kill_role(role)
    }

    // ── Queries ──────────────────────────────────────────────────────────

    /// Total number of active instances across all roles.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Number of active instances for a specific role.
    #[must_use]
    pub fn active_count_for_role(&self, role: AgentRole) -> usize {
        self.active.keys().filter(|id| id.role == role).count()
    }

    /// Get the status of a specific instance.
    #[must_use]
    pub fn status(&self, id: &AgentInstanceId) -> Option<InstanceStatus> {
        self.active.get(id).map(|e| e.status)
    }

    /// Get all active instance IDs for a role.
    #[must_use]
    pub fn instances_for_role(&self, role: AgentRole) -> Vec<AgentInstanceId> {
        self.active
            .keys()
            .filter(|id| id.role == role)
            .cloned()
            .collect()
    }

    /// Check whether an instance is active.
    #[must_use]
    pub fn is_active(&self, id: &AgentInstanceId) -> bool {
        self.active.contains_key(id)
    }

    /// Check whether a warm agent exists for a role (any instance).
    #[must_use]
    pub fn has_warm(&self, role: AgentRole) -> bool {
        self.warm.keys().any(|(r, _)| *r == role)
    }

    /// Whether adding another instance for the role would exceed the limit.
    #[must_use]
    pub fn at_capacity(&self, role: AgentRole) -> bool {
        self.active_count_for_role(role) >= self.concurrency_limit(role)
    }
}

impl Default for MultiAgentPool {
    fn default() -> Self {
        Self::new()
    }
}

// ─── KillReport ──────────────────────────────────────────────────────────

/// Summary of a `kill_all` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillReport {
    /// Active agents that were killed within the deadline.
    pub killed_active: usize,
    /// Warm agents that were killed within the deadline.
    pub killed_warm: usize,
    /// Agents that were forcibly dropped after the deadline expired.
    pub aborted: usize,
}

impl KillReport {
    /// Total agents cleaned up (killed + aborted).
    #[must_use]
    pub const fn total(&self) -> usize {
        self.killed_active + self.killed_warm + self.aborted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockAgent;
    use roko_core::{Body, Context, Engram, Kind};

    fn prompt(text: &str) -> Engram {
        Engram::builder(Kind::Prompt).body(Body::text(text)).build()
    }

    fn ctx() -> Context {
        Context::at(0)
    }

    fn mock_ok() -> Arc<dyn Agent> {
        Arc::new(MockAgent::reply("ok"))
    }

    fn mock_fail() -> Arc<dyn Agent> {
        Arc::new(MockAgent::fail_with("fail"))
    }

    // ── Warm pool tests ──────────────────────────────────────────────────

    #[test]
    fn multi_pool_pre_spawn_warm_creates_entries() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 3, &mock_ok);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 3);
        assert_eq!(pool.warm_count(AgentRole::Auditor), 0);
        assert_eq!(pool.total_warm_count(), 3);
    }

    #[test]
    fn multi_pool_pre_spawn_warm_idempotent() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 2);
    }

    #[test]
    fn multi_pool_promote_warm_moves_to_active() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);

        let id = pool.promote_warm(AgentRole::Implementer).unwrap();
        assert_eq!(id.role, AgentRole::Implementer);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 1);
        assert_eq!(pool.active_count(), 1);
        assert!(pool.is_active(&id));
        assert_eq!(pool.status(&id), Some(InstanceStatus::Active));
    }

    #[test]
    fn multi_pool_promote_warm_returns_none_when_empty() {
        let mut pool = MultiAgentPool::new();
        assert!(pool.promote_warm(AgentRole::Implementer).is_none());
    }

    #[test]
    fn multi_pool_evict_warm_by_idle_time() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);

        // With zero duration, everything is "old enough" to evict.
        let evicted = pool.evict_warm(AgentRole::Implementer, Duration::from_secs(0));
        assert_eq!(evicted, 2);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 0);
    }

    #[test]
    fn multi_pool_evict_warm_skips_recent() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);

        // With a very long max_idle, nothing should be evicted.
        let evicted = pool.evict_warm(AgentRole::Implementer, Duration::from_secs(3600));
        assert_eq!(evicted, 0);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 2);
    }

    // ── Active instance tests ────────────────────────────────────────────

    #[test]
    fn multi_pool_add_active_respects_concurrency_limit() {
        let mut pool = MultiAgentPool::new();
        pool.set_concurrency_limit(AgentRole::Implementer, 2);

        let id1 = AgentInstanceId::new(AgentRole::Implementer, "t1");
        let id2 = AgentInstanceId::new(AgentRole::Implementer, "t2");
        let id3 = AgentInstanceId::new(AgentRole::Implementer, "t3");

        assert!(pool.add_active(id1, mock_ok()));
        assert!(pool.add_active(id2, mock_ok()));
        assert!(!pool.add_active(id3, mock_ok())); // Exceeds limit.
        assert_eq!(pool.active_count_for_role(AgentRole::Implementer), 2);
        assert!(pool.at_capacity(AgentRole::Implementer));
    }

    #[test]
    fn multi_pool_add_active_rejects_duplicate_id() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "t1");
        assert!(pool.add_active(id.clone(), mock_ok()));
        assert!(!pool.add_active(id, mock_ok()));
    }

    #[tokio::test]
    async fn multi_pool_run_task_success() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "t1");
        pool.add_active(id.clone(), mock_ok());

        let task = AgentTask::new(id.clone(), prompt("do it"), ctx());
        let outcome = pool.run_task(task).await;
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(outcome.result.unwrap().success);
        assert!(!outcome.used_fallback);
    }

    #[tokio::test]
    async fn multi_pool_run_task_fallback_on_failure() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "t1");
        pool.add_active(id.clone(), mock_fail());
        pool.set_fallback(AgentRole::Implementer, mock_ok());

        let task = AgentTask::new(id.clone(), prompt("fix"), ctx());
        let outcome = pool.run_task(task).await;
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(outcome.used_fallback);
    }

    #[tokio::test]
    async fn multi_pool_run_task_on_missing_instance() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "ghost");
        let task = AgentTask::new(id, prompt("?"), ctx());
        let outcome = pool.run_task(task).await;
        assert_eq!(outcome.status, InstanceStatus::Failed);
        assert!(outcome.result.is_none());
    }

    // ── Kill operations ──────────────────────────────────────────────────

    #[test]
    fn multi_pool_kill_all_with_deadline() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 2, &mock_ok);
        let id = AgentInstanceId::new(AgentRole::Auditor, "a1");
        pool.add_active(id, mock_ok());

        let report = pool.kill_all(Duration::from_secs(3));
        assert_eq!(report.killed_active, 1);
        assert_eq!(report.killed_warm, 2);
        assert_eq!(report.aborted, 0);
        assert_eq!(report.total(), 3);
        assert_eq!(pool.active_count(), 0);
        assert_eq!(pool.total_warm_count(), 0);
    }

    #[test]
    fn multi_pool_kill_plan_agents_by_substring() {
        let mut pool = MultiAgentPool::new();
        let id1 = AgentInstanceId::new(AgentRole::Implementer, "plan42-task1");
        let id2 = AgentInstanceId::new(AgentRole::Implementer, "plan42-task2");
        let id3 = AgentInstanceId::new(AgentRole::Implementer, "plan99-task1");
        pool.add_active(id1, mock_ok());
        pool.add_active(id2, mock_ok());
        pool.add_active(id3, mock_ok());

        // Also add a warm agent for plan42.
        pool.pre_spawn_warm_named(AgentRole::Auditor, "plan42-warm", mock_ok());

        let killed = pool.kill_plan_agents("plan42");
        assert_eq!(killed, 3); // 2 active + 1 warm
        assert_eq!(pool.active_count(), 1); // plan99 survives
    }

    #[test]
    fn multi_pool_kill_role() {
        let mut pool = MultiAgentPool::new();
        let id1 = AgentInstanceId::new(AgentRole::Implementer, "t1");
        let id2 = AgentInstanceId::new(AgentRole::Implementer, "t2");
        let id3 = AgentInstanceId::new(AgentRole::Auditor, "t1");
        pool.add_active(id1, mock_ok());
        pool.add_active(id2, mock_ok());
        pool.add_active(id3, mock_ok());
        pool.pre_spawn_warm(AgentRole::Implementer, 1, &mock_ok);

        let killed = pool.kill_role(AgentRole::Implementer);
        assert_eq!(killed, 3); // 2 active + 1 warm
        assert_eq!(pool.active_count(), 1); // auditor survives
        assert_eq!(pool.active_count_for_role(AgentRole::Implementer), 0);
    }

    // ── Query tests ──────────────────────────────────────────────────────

    #[test]
    fn multi_pool_instances_for_role() {
        let mut pool = MultiAgentPool::new();
        let id1 = AgentInstanceId::new(AgentRole::Implementer, "t1");
        let id2 = AgentInstanceId::new(AgentRole::Implementer, "t2");
        let id3 = AgentInstanceId::new(AgentRole::Auditor, "a1");
        pool.add_active(id1.clone(), mock_ok());
        pool.add_active(id2.clone(), mock_ok());
        pool.add_active(id3, mock_ok());

        let impls = pool.instances_for_role(AgentRole::Implementer);
        assert_eq!(impls.len(), 2);
        assert!(impls.contains(&id1));
        assert!(impls.contains(&id2));
    }

    #[test]
    fn multi_pool_has_warm() {
        let mut pool = MultiAgentPool::new();
        assert!(!pool.has_warm(AgentRole::Implementer));
        pool.pre_spawn_warm(AgentRole::Implementer, 1, &mock_ok);
        assert!(pool.has_warm(AgentRole::Implementer));
    }

    #[test]
    fn multi_pool_default_concurrency() {
        let pool = MultiAgentPool::new().with_default_concurrency(8);
        assert_eq!(pool.concurrency_limit(AgentRole::Implementer), 8);
    }

    #[test]
    fn multi_pool_promote_warm_named() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm_named(AgentRole::Auditor, "reviewer-plan7", mock_ok());

        let id = pool
            .promote_warm_named(AgentRole::Auditor, "reviewer-plan7")
            .unwrap();
        assert_eq!(id.instance, "reviewer-plan7");
        assert!(pool.is_active(&id));
        assert_eq!(pool.warm_count(AgentRole::Auditor), 0);
    }

    #[test]
    fn multi_pool_checked_warm_reuse_requires_matching_fingerprints() {
        let mut pool = MultiAgentPool::new();
        let policy = WarmReusePolicy::stateless("task-warm")
            .for_session("session-a")
            .with_fingerprints(Some("prompt-v1".into()), Some("ctx-v1".into()))
            .allow_context_carryover(true);
        pool.pre_spawn_warm_named_with_policy(
            AgentRole::Implementer,
            "warm-ctx",
            mock_ok(),
            policy,
        );

        let wrong_context = WarmReuseRequest::session("session-a")
            .with_fingerprints(Some("prompt-v1".into()), Some("ctx-v2".into()));
        assert!(
            pool.promote_warm_for(AgentRole::Implementer, &wrong_context)
                .is_none(),
            "old context fingerprint must not match a new task"
        );
        assert_eq!(pool.warm_count(AgentRole::Implementer), 1);

        let matching = WarmReuseRequest::session("session-a")
            .with_fingerprints(Some("prompt-v1".into()), Some("ctx-v1".into()));
        let id = pool
            .promote_warm_for(AgentRole::Implementer, &matching)
            .expect("matching scoped reuse should promote");
        assert_eq!(id.instance, "warm-ctx");
        assert_eq!(pool.warm_count(AgentRole::Implementer), 0);
    }

    #[test]
    fn multi_pool_recycled_terminal_instance_is_not_implicitly_reusable() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "task-a");
        assert!(pool.add_active(id.clone(), mock_ok()));
        let task = AgentTask::new(id.clone(), prompt("x"), ctx());

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let outcome = rt.block_on(pool.run_task(task));
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(pool.recycle_terminal_to_warm(&id));

        let request = WarmReuseRequest::session("anything");
        assert!(
            pool.promote_warm_for(AgentRole::Implementer, &request)
                .is_none(),
            "recycled agents require an explicit fresh policy before checked reuse"
        );
    }

    #[test]
    fn multi_pool_promote_warm_if_capacity_blocks_when_full() {
        let mut pool = MultiAgentPool::new();
        pool.set_concurrency_limit(AgentRole::Implementer, 1);
        assert!(pool.add_active(
            AgentInstanceId::new(AgentRole::Implementer, "active"),
            mock_ok()
        ));
        pool.pre_spawn_warm_named(AgentRole::Implementer, "warm-a", mock_ok());

        assert!(
            pool.promote_warm_if_capacity(AgentRole::Implementer)
                .is_none()
        );
        assert_eq!(pool.warm_count(AgentRole::Implementer), 1);
    }

    #[test]
    fn multi_pool_evict_warm_all() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm(AgentRole::Implementer, 5, &mock_ok);
        let evicted = pool.evict_warm_all(AgentRole::Implementer);
        assert_eq!(evicted, 5);
        assert_eq!(pool.warm_count(AgentRole::Implementer), 0);
    }

    #[test]
    fn multi_pool_ensure_active_instance_reuses_named_warm() {
        let mut pool = MultiAgentPool::new();
        pool.pre_spawn_warm_named(AgentRole::Implementer, "plan-7-task-2", mock_ok());

        let id = pool
            .ensure_active_instance(AgentRole::Implementer, "plan-7-task-2", &|_, _| mock_fail())
            .unwrap();
        assert_eq!(id.instance, "plan-7-task-2");
        assert!(pool.is_active(&id));
        assert_eq!(pool.warm_count(AgentRole::Implementer), 0);
    }

    #[test]
    fn multi_pool_ensure_active_instance_spawns_when_no_warm_available() {
        let mut pool = MultiAgentPool::new();
        let id = pool
            .ensure_active_instance(AgentRole::Auditor, "fresh", &|_, _| mock_ok())
            .unwrap();
        assert_eq!(id.role, AgentRole::Auditor);
        assert!(pool.is_active(&id));
    }

    #[tokio::test]
    async fn multi_pool_run_task_with_auto_activation_runs_without_pre_registration() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "auto-1");
        let task = AgentTask::new(id.clone(), prompt("auto"), ctx());

        let outcome = pool
            .run_task_with_auto_activation(task, &|_, _| mock_ok())
            .await;
        assert_eq!(outcome.id, id);
        assert_eq!(outcome.status, InstanceStatus::Done);
        assert!(outcome.result.is_some());
    }

    #[tokio::test]
    async fn multi_pool_run_task_with_auto_activation_fails_when_capacity_blocked() {
        let mut pool = MultiAgentPool::new();
        pool.set_concurrency_limit(AgentRole::Implementer, 0);
        let id = AgentInstanceId::new(AgentRole::Implementer, "blocked");
        let task = AgentTask::new(id.clone(), prompt("auto"), ctx());

        let outcome = pool
            .run_task_with_auto_activation(task, &|_, _| mock_ok())
            .await;
        assert_eq!(outcome.id, id);
        assert_eq!(outcome.status, InstanceStatus::Failed);
        assert!(outcome.result.is_none());
    }

    #[tokio::test]
    async fn multi_pool_recycle_terminal_to_warm_moves_done_instance() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "recycle-me");
        pool.add_active(id.clone(), mock_ok());
        let task = AgentTask::new(id.clone(), prompt("x"), ctx());
        let _ = pool.run_task(task).await;

        assert!(pool.recycle_terminal_to_warm(&id));
        assert!(!pool.is_active(&id));
        assert_eq!(pool.warm_count(AgentRole::Implementer), 1);
    }

    #[tokio::test]
    async fn multi_pool_reap_terminal_active_removes_done_entries() {
        let mut pool = MultiAgentPool::new();
        let id = AgentInstanceId::new(AgentRole::Implementer, "reap-me");
        pool.add_active(id.clone(), mock_ok());
        let task = AgentTask::new(id.clone(), prompt("x"), ctx());
        let _ = pool.run_task(task).await;

        let removed = pool.reap_terminal_active();
        assert_eq!(removed, 1);
        assert!(!pool.is_active(&id));
    }

    #[test]
    fn multi_pool_kill_report_total() {
        let report = KillReport {
            killed_active: 3,
            killed_warm: 2,
            aborted: 1,
        };
        assert_eq!(report.total(), 6);
    }
}
