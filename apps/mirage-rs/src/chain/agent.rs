//! Agent identity, trace, and stats tracking.
//!
//! Provides first-class agent entities with cognitive trace recording,
//! heartbeat-based liveness detection, and accumulated stats. The
//! [`AgentRegistry`] is wired into [`super::super::chain_rpc::ChainContext`]
//! and exposed via HTTP (`/api/agents/*`) and JSON-RPC (`chain_*Agent*`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Cognitive phase in the CoALA-style Retrieve→Reason→Act→Verify loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitivePhase {
    /// Information retrieval from memory or environment.
    Retrieve,
    /// Reasoning over retrieved context.
    Reason,
    /// Taking an action in the environment.
    Act,
    /// Verifying the outcome of an action.
    Verify,
}

/// Per-agent accumulated statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStats {
    /// Number of insight confirmations issued.
    pub confirmations_given: u64,
    /// Number of insight challenges issued.
    pub challenges_given: u64,
    /// Number of warnings posted.
    pub warnings_posted: u64,
    /// Number of insights posted.
    pub insights_posted: u64,
    /// Number of tasks completed successfully.
    pub tasks_completed: u64,
    /// Number of tasks that failed.
    pub tasks_failed: u64,
    /// Number of cognitive cycles completed.
    pub delta_cycles: u64,
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
}

/// Per-skill configuration persisted for an agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillConfig {
    /// Whether the skill is currently enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Arbitrary per-skill parameters.
    #[serde(default, rename = "config", alias = "parameters")]
    pub config: HashMap<String, Value>,
}

/// A single cognitive trace entry for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTrace {
    /// Cognitive cycle number.
    pub cycle: u64,
    /// Phase of the cognitive loop.
    pub phase: CognitivePhase,
    /// Resources read during this phase.
    pub reads: Vec<String>,
    /// Reasoning text.
    pub reasoning: String,
    /// Action taken.
    pub action: String,
    /// Unique action identifier.
    pub action_id: String,
    /// Unix timestamp in seconds.
    pub timestamp: u64,
}

/// A registered agent entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    /// Unique agent identifier.
    pub id: String,
    /// On-chain address bytes.
    pub address: Vec<u8>,
    /// Agent role (e.g. "researcher", "coder").
    pub role: String,
    /// Wallet or account that owns this agent.
    #[serde(default)]
    pub owner: String,
    /// Registration timestamp (Unix seconds).
    pub registered_at: u64,
    /// Block number of last heartbeat.
    pub last_heartbeat_block: u64,
    /// Timestamp of last heartbeat.
    pub last_heartbeat_ts: u64,
    /// Accumulated statistics.
    pub stats: AgentStats,
    /// Per-agent skill configuration.
    #[serde(default)]
    pub skills: HashMap<String, SkillConfig>,
}

/// Events broadcast on the agent bus for WebSocket streaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A cognitive trace was recorded.
    Trace {
        /// Agent that produced the trace.
        agent_id: String,
        /// The trace entry.
        trace: AgentTrace,
    },
    /// An agent sent a heartbeat.
    Heartbeat {
        /// Agent that sent the heartbeat.
        agent_id: String,
        /// Block number.
        block: u64,
        /// Unix timestamp.
        timestamp: u64,
    },
    /// Agent stats were updated.
    Stats {
        /// Agent whose stats changed.
        agent_id: String,
        /// The delta applied.
        delta: AgentStats,
    },
    /// A new agent was registered.
    Registered {
        /// The newly registered agent's ID.
        agent_id: String,
        /// The agent's role.
        role: String,
    },
}

/// Registry tracking all known agents, their traces, and stats.
#[derive(Debug, Default)]
pub struct AgentRegistry {
    agents: HashMap<String, AgentEntry>,
    traces: HashMap<String, Vec<AgentTrace>>,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new agent. Returns `false` if already registered.
    pub fn register(
        &mut self,
        id: String,
        address: Vec<u8>,
        role: String,
        owner: String,
        timestamp: u64,
    ) -> bool {
        if self.agents.contains_key(&id) {
            return false;
        }
        self.agents.insert(
            id.clone(),
            AgentEntry {
                id: id.clone(),
                address,
                role,
                owner,
                registered_at: timestamp,
                last_heartbeat_block: 0,
                last_heartbeat_ts: timestamp,
                stats: AgentStats::default(),
                skills: HashMap::new(),
            },
        );
        self.traces.insert(id, Vec::new());
        true
    }

    /// Record a heartbeat at the given block and timestamp.
    pub fn heartbeat(&mut self, id: &str, block: u64, timestamp: u64) -> bool {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.last_heartbeat_block = block;
            agent.last_heartbeat_ts = timestamp;
            true
        } else {
            false
        }
    }

    /// Append a cognitive trace entry.
    pub fn add_trace(&mut self, id: &str, trace: AgentTrace) -> bool {
        if let Some(traces) = self.traces.get_mut(id) {
            traces.push(trace);
            true
        } else {
            false
        }
    }

    /// Get stats for an agent.
    pub fn get_stats(&self, id: &str) -> Option<&AgentStats> {
        self.agents.get(id).map(|a| &a.stats)
    }

    /// Get traces with pagination.
    pub fn get_traces(
        &self,
        id: &str,
        limit: usize,
        offset: usize,
    ) -> Option<(&[AgentTrace], usize)> {
        self.traces.get(id).map(|traces| {
            let total = traces.len();
            let start = offset.min(total);
            let end = (start + limit).min(total);
            (&traces[start..end], total)
        })
    }

    /// Get a specific agent entry.
    pub fn get_agent(&self, id: &str) -> Option<&AgentEntry> {
        self.agents.get(id)
    }

    /// List all registered agents.
    pub fn list_agents(&self) -> Vec<&AgentEntry> {
        self.agents.values().collect()
    }

    /// List agents owned by a specific wallet or account.
    pub fn list_agents_by_owner(&self, owner: &str) -> Vec<&AgentEntry> {
        self.agents
            .values()
            .filter(|agent| agent.owner == owner)
            .collect()
    }

    /// Check if an agent is alive (heartbeat within `timeout_blocks` of `current_block`).
    pub fn is_alive(&self, id: &str, current_block: u64, timeout_blocks: u64) -> Option<bool> {
        self.agents
            .get(id)
            .map(|a| current_block.saturating_sub(a.last_heartbeat_block) < timeout_blocks)
    }

    /// Increment stats counters by a delta.
    pub fn add_stats_delta(&mut self, id: &str, delta: &AgentStats) -> bool {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.stats.confirmations_given += delta.confirmations_given;
            agent.stats.challenges_given += delta.challenges_given;
            agent.stats.warnings_posted += delta.warnings_posted;
            agent.stats.insights_posted += delta.insights_posted;
            agent.stats.tasks_completed += delta.tasks_completed;
            agent.stats.tasks_failed += delta.tasks_failed;
            agent.stats.delta_cycles += delta.delta_cycles;
            agent.stats.total_cost_usd += delta.total_cost_usd;
            agent.stats.total_tokens += delta.total_tokens;
            true
        } else {
            false
        }
    }

    /// Get the skills configured for an agent.
    pub fn get_skills(&self, id: &str) -> Option<&HashMap<String, SkillConfig>> {
        self.agents.get(id).map(|agent| &agent.skills)
    }

    /// Replace all skills for an agent.
    pub fn set_skills(&mut self, id: &str, skills: HashMap<String, SkillConfig>) -> bool {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.skills = skills;
            true
        } else {
            false
        }
    }

    /// Update or insert a single skill for an agent.
    pub fn set_skill(&mut self, id: &str, skill_name: &str, config: SkillConfig) -> bool {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.skills.insert(skill_name.to_owned(), config);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_agent() {
        let mut reg = AgentRegistry::new();
        assert!(reg.register(
            "agent-1".into(),
            vec![1, 2, 3],
            "researcher".into(),
            "0xabc".into(),
            1000,
        ));
        assert!(!reg.register(
            "agent-1".into(),
            vec![1, 2, 3],
            "researcher".into(),
            "0xabc".into(),
            1000,
        ));
        assert_eq!(reg.list_agents().len(), 1);
        let agent = reg.get_agent("agent-1").unwrap();
        assert_eq!(agent.role, "researcher");
        assert_eq!(agent.owner, "0xabc");
        assert_eq!(agent.registered_at, 1000);
    }

    #[test]
    fn heartbeat_updates_block() {
        let mut reg = AgentRegistry::new();
        reg.register("agent-1".into(), vec![], "worker".into(), "0x1".into(), 100);
        assert!(reg.heartbeat("agent-1", 50, 200));
        assert!(!reg.heartbeat("nonexistent", 50, 200));
        let agent = reg.get_agent("agent-1").unwrap();
        assert_eq!(agent.last_heartbeat_block, 50);
        assert_eq!(agent.last_heartbeat_ts, 200);
    }

    #[test]
    fn add_and_get_traces() {
        let mut reg = AgentRegistry::new();
        reg.register("agent-1".into(), vec![], "coder".into(), "0x1".into(), 0);
        for i in 0..5 {
            reg.add_trace(
                "agent-1",
                AgentTrace {
                    cycle: i,
                    phase: CognitivePhase::Reason,
                    reads: vec![format!("file-{i}")],
                    reasoning: format!("thought-{i}"),
                    action: format!("action-{i}"),
                    action_id: format!("id-{i}"),
                    timestamp: i * 10,
                },
            );
        }
        let (traces, total) = reg.get_traces("agent-1", 3, 0).unwrap();
        assert_eq!(total, 5);
        assert_eq!(traces.len(), 3);
        let (traces, total) = reg.get_traces("agent-1", 3, 3).unwrap();
        assert_eq!(total, 5);
        assert_eq!(traces.len(), 2);
        assert!(reg.get_traces("nonexistent", 3, 0).is_none());
    }

    #[test]
    fn stats_delta_accumulates() {
        let mut reg = AgentRegistry::new();
        reg.register("agent-1".into(), vec![], "analyst".into(), "0x1".into(), 0);
        let delta = AgentStats {
            confirmations_given: 5,
            challenges_given: 2,
            warnings_posted: 1,
            insights_posted: 3,
            tasks_completed: 4,
            tasks_failed: 1,
            delta_cycles: 10,
            total_cost_usd: 0.5,
            total_tokens: 1000,
        };
        assert!(reg.add_stats_delta("agent-1", &delta));
        assert!(reg.add_stats_delta("agent-1", &delta));
        assert!(!reg.add_stats_delta("nonexistent", &delta));
        let stats = reg.get_stats("agent-1").unwrap();
        assert_eq!(stats.confirmations_given, 10);
        assert_eq!(stats.total_tokens, 2000);
    }

    #[test]
    fn liveness_check() {
        let mut reg = AgentRegistry::new();
        reg.register("agent-1".into(), vec![], "watcher".into(), "0x1".into(), 0);
        reg.heartbeat("agent-1", 100, 500);
        assert_eq!(reg.is_alive("agent-1", 150, 200), Some(true));
        assert_eq!(reg.is_alive("agent-1", 350, 200), Some(false));
        assert_eq!(reg.is_alive("nonexistent", 150, 200), None);
    }

    #[test]
    fn list_agents_by_owner_filters() {
        let mut reg = AgentRegistry::new();
        reg.register(
            "agent-1".into(),
            vec![],
            "watcher".into(),
            "0xabc".into(),
            0,
        );
        reg.register(
            "agent-2".into(),
            vec![],
            "watcher".into(),
            "0xdef".into(),
            0,
        );
        reg.register(
            "agent-3".into(),
            vec![],
            "watcher".into(),
            "0xabc".into(),
            0,
        );

        let owned = reg.list_agents_by_owner("0xabc");
        assert_eq!(owned.len(), 2);
        assert!(owned.iter().all(|agent| agent.owner == "0xabc"));
    }

    #[test]
    fn skills_round_trip() {
        let mut reg = AgentRegistry::new();
        reg.register(
            "agent-1".into(),
            vec![],
            "watcher".into(),
            "0xabc".into(),
            0,
        );

        let config = SkillConfig {
            enabled: true,
            config: HashMap::from([("confidence_threshold".into(), Value::from(80))]),
        };
        assert!(reg.set_skill("agent-1", "risk-sentinel", config.clone()));
        assert_eq!(
            reg.get_skills("agent-1")
                .and_then(|skills| skills.get("risk-sentinel"))
                .cloned(),
            Some(config.clone())
        );

        let updated = HashMap::from([("hedge-agent".into(), config)]);
        assert!(reg.set_skills("agent-1", updated.clone()));
        assert_eq!(reg.get_skills("agent-1").cloned(), Some(updated));
        assert!(!reg.set_skill("missing", "x", SkillConfig::default()));
    }
}
