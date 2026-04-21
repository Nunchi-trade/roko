//! Isolated knowledge spaces with ACL enforcement.
//!
//! Each [`CognitiveNamespace`] provides an isolated scope for agent data
//! and knowledge. Cross-namespace access requires explicit [`Channel`]
//! declarations. All read/write operations check the namespace ACL
//! before proceeding.
//!
//! # Design
//!
//! - **ACL**: Each namespace has readers, writers, and admins.
//! - **Channels**: Typed, rate-limited pathways for cross-namespace data flow.
//! - **Rate limiting**: Optional per-namespace rate limits.
//!
//! This module provides the data model. Enforcement is handled at call
//! sites (e.g. `FileSubstrate`, `KnowledgeStore`) by consulting the
//! namespace ACL before I/O.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Unique identifier for an agent (typically the agent name or ID).
pub type AgentId = String;

/// Direction of a cross-namespace channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDirection {
    /// Data flows from source to target only.
    Unidirectional,
    /// Data flows in both directions.
    Bidirectional,
}

/// A typed channel for cross-namespace data flow.
///
/// Channels provide explicit, auditable pathways for data to cross
/// namespace boundaries. Without a channel declaration, cross-namespace
/// reads are denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    /// Human-readable channel name.
    pub name: String,
    /// Source namespace ID.
    pub source_ns: String,
    /// Target namespace ID.
    pub target_ns: String,
    /// Direction of data flow.
    pub direction: ChannelDirection,
    /// Optional JSON schema for data validation.
    pub schema: Option<serde_json::Value>,
}

/// Access control list for a namespace.
///
/// Agents in `admins` have full access. Agents in `readers` can read.
/// Agents in `writers` can write. All sets are independent (an admin
/// has read+write implicitly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NamespaceAcl {
    /// Agents allowed to read from this namespace.
    pub readers: HashSet<AgentId>,
    /// Agents allowed to write to this namespace.
    pub writers: HashSet<AgentId>,
    /// Agents with full access (read + write + admin).
    pub admins: HashSet<AgentId>,
}

impl NamespaceAcl {
    /// Returns `true` if `agent` is allowed to read from this namespace.
    #[must_use]
    pub fn can_read(&self, agent: &str) -> bool {
        self.readers.contains(agent) || self.admins.contains(agent)
    }

    /// Returns `true` if `agent` is allowed to write to this namespace.
    #[must_use]
    pub fn can_write(&self, agent: &str) -> bool {
        self.writers.contains(agent) || self.admins.contains(agent)
    }

    /// Returns `true` if `agent` has admin access.
    #[must_use]
    pub fn is_admin(&self, agent: &str) -> bool {
        self.admins.contains(agent)
    }

    /// Grant read access to an agent.
    pub fn grant_read(&mut self, agent: impl Into<AgentId>) {
        self.readers.insert(agent.into());
    }

    /// Grant write access to an agent.
    pub fn grant_write(&mut self, agent: impl Into<AgentId>) {
        self.writers.insert(agent.into());
    }

    /// Grant admin access to an agent.
    pub fn grant_admin(&mut self, agent: impl Into<AgentId>) {
        self.admins.insert(agent.into());
    }
}

/// Optional per-namespace rate limit configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum reads per window.
    pub max_reads_per_window: u64,
    /// Maximum writes per window.
    pub max_writes_per_window: u64,
    /// Window duration in seconds.
    pub window_secs: u64,
}

/// An isolated knowledge space with ACL enforcement.
///
/// Each namespace scopes data access to authorized agents. Channels
/// declare explicit cross-namespace pathways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveNamespace {
    /// Unique namespace identifier.
    pub id: String,
    /// Agent that owns this namespace.
    pub owner: AgentId,
    /// Access control list.
    pub acl: NamespaceAcl,
    /// Cross-namespace channels.
    pub channels: Vec<Channel>,
    /// Optional rate limit.
    pub rate_limit: Option<RateLimitConfig>,
}

impl CognitiveNamespace {
    /// Create a new namespace owned by the given agent.
    ///
    /// The owner is automatically granted admin access.
    #[must_use]
    pub fn new(id: impl Into<String>, owner: impl Into<AgentId>) -> Self {
        let owner = owner.into();
        let mut acl = NamespaceAcl::default();
        acl.grant_admin(owner.clone());
        Self {
            id: id.into(),
            owner,
            acl,
            channels: Vec::new(),
            rate_limit: None,
        }
    }

    /// Add a cross-namespace channel.
    #[must_use]
    pub fn with_channel(mut self, channel: Channel) -> Self {
        self.channels.push(channel);
        self
    }

    /// Set a rate limit for this namespace.
    #[must_use]
    pub fn with_rate_limit(mut self, limit: RateLimitConfig) -> Self {
        self.rate_limit = Some(limit);
        self
    }

    /// Check whether the given agent can read from this namespace.
    #[must_use]
    pub fn check_read(&self, agent: &str) -> bool {
        self.acl.can_read(agent)
    }

    /// Check whether the given agent can write to this namespace.
    #[must_use]
    pub fn check_write(&self, agent: &str) -> bool {
        self.acl.can_write(agent)
    }

    /// Check whether there is a channel from `source_ns` to this namespace.
    #[must_use]
    pub fn has_channel_from(&self, source_ns: &str) -> bool {
        self.channels.iter().any(|ch| {
            (ch.source_ns == source_ns && ch.target_ns == self.id)
                || (ch.direction == ChannelDirection::Bidirectional
                    && ch.target_ns == source_ns
                    && ch.source_ns == self.id)
        })
    }
}

/// Registry of cognitive namespaces.
///
/// The registry provides lookup and cross-namespace access checks.
/// It is typically loaded from `roko.toml` at startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NamespaceRegistry {
    namespaces: HashMap<String, CognitiveNamespace>,
}

impl NamespaceRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a namespace.
    pub fn register(&mut self, ns: CognitiveNamespace) {
        self.namespaces.insert(ns.id.clone(), ns);
    }

    /// Look up a namespace by ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&CognitiveNamespace> {
        self.namespaces.get(id)
    }

    /// Return the number of registered namespaces.
    #[must_use]
    pub fn len(&self) -> usize {
        self.namespaces.len()
    }

    /// Return whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.namespaces.is_empty()
    }

    /// Check whether `agent` can read from `namespace_id`.
    ///
    /// Returns `false` if the namespace does not exist.
    #[must_use]
    pub fn check_read(&self, namespace_id: &str, agent: &str) -> bool {
        self.namespaces
            .get(namespace_id)
            .is_some_and(|ns| ns.check_read(agent))
    }

    /// Check whether `agent` can write to `namespace_id`.
    ///
    /// Returns `false` if the namespace does not exist.
    #[must_use]
    pub fn check_write(&self, namespace_id: &str, agent: &str) -> bool {
        self.namespaces
            .get(namespace_id)
            .is_some_and(|ns| ns.check_write(agent))
    }

    /// Check whether there is a channel from `source_ns` to `target_ns`.
    ///
    /// Returns `false` if the target namespace does not exist.
    #[must_use]
    pub fn check_cross_namespace(&self, source_ns: &str, target_ns: &str) -> bool {
        self.namespaces
            .get(target_ns)
            .is_some_and(|ns| ns.has_channel_from(source_ns))
    }

    /// Return all namespace IDs.
    pub fn namespace_ids(&self) -> impl Iterator<Item = &str> {
        self.namespaces.keys().map(String::as_str)
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_gets_admin_access() {
        let ns = CognitiveNamespace::new("ns-1", "agent-owner");
        assert!(ns.check_read("agent-owner"));
        assert!(ns.check_write("agent-owner"));
        assert!(ns.acl.is_admin("agent-owner"));
    }

    #[test]
    fn reader_can_read_but_not_write() {
        let mut ns = CognitiveNamespace::new("ns-1", "owner");
        ns.acl.grant_read("reader-agent");
        assert!(ns.check_read("reader-agent"));
        assert!(!ns.check_write("reader-agent"));
    }

    #[test]
    fn writer_can_write_but_not_read() {
        let mut ns = CognitiveNamespace::new("ns-1", "owner");
        ns.acl.grant_write("writer-agent");
        assert!(!ns.check_read("writer-agent"));
        assert!(ns.check_write("writer-agent"));
    }

    #[test]
    fn unknown_agent_denied() {
        let ns = CognitiveNamespace::new("ns-1", "owner");
        assert!(!ns.check_read("stranger"));
        assert!(!ns.check_write("stranger"));
    }

    #[test]
    fn unidirectional_channel_one_way() {
        let ns = CognitiveNamespace::new("target-ns", "owner").with_channel(Channel {
            name: "data-feed".into(),
            source_ns: "source-ns".into(),
            target_ns: "target-ns".into(),
            direction: ChannelDirection::Unidirectional,
            schema: None,
        });
        assert!(ns.has_channel_from("source-ns"));
        assert!(!ns.has_channel_from("other-ns"));
    }

    #[test]
    fn bidirectional_channel_both_ways() {
        let ns_a = CognitiveNamespace::new("ns-a", "owner").with_channel(Channel {
            name: "sync".into(),
            source_ns: "ns-a".into(),
            target_ns: "ns-b".into(),
            direction: ChannelDirection::Bidirectional,
            schema: None,
        });
        // ns-a has a bidirectional channel with ns-b, so ns-b can reach ns-a.
        assert!(ns_a.has_channel_from("ns-b"));
    }

    #[test]
    fn registry_register_and_lookup() {
        let mut reg = NamespaceRegistry::new();
        reg.register(CognitiveNamespace::new("ns-1", "agent-1"));
        reg.register(CognitiveNamespace::new("ns-2", "agent-2"));
        assert_eq!(reg.len(), 2);
        assert!(reg.get("ns-1").is_some());
        assert!(reg.get("ns-3").is_none());
    }

    #[test]
    fn registry_check_read_write() {
        let mut reg = NamespaceRegistry::new();
        let mut ns = CognitiveNamespace::new("data", "admin");
        ns.acl.grant_read("reader");
        reg.register(ns);

        assert!(reg.check_read("data", "admin"));
        assert!(reg.check_read("data", "reader"));
        assert!(!reg.check_read("data", "stranger"));
        assert!(reg.check_write("data", "admin"));
        assert!(!reg.check_write("data", "reader"));
    }

    #[test]
    fn registry_check_cross_namespace() {
        let mut reg = NamespaceRegistry::new();
        reg.register(
            CognitiveNamespace::new("target", "owner").with_channel(Channel {
                name: "feed".into(),
                source_ns: "source".into(),
                target_ns: "target".into(),
                direction: ChannelDirection::Unidirectional,
                schema: None,
            }),
        );

        assert!(reg.check_cross_namespace("source", "target"));
        assert!(!reg.check_cross_namespace("other", "target"));
        assert!(!reg.check_cross_namespace("source", "nonexistent"));
    }

    #[test]
    fn namespace_round_trips_through_serde() {
        let ns = CognitiveNamespace::new("test-ns", "agent-1")
            .with_channel(Channel {
                name: "ch".into(),
                source_ns: "a".into(),
                target_ns: "test-ns".into(),
                direction: ChannelDirection::Bidirectional,
                schema: Some(serde_json::json!({"type": "object"})),
            })
            .with_rate_limit(RateLimitConfig {
                max_reads_per_window: 100,
                max_writes_per_window: 50,
                window_secs: 60,
            });

        let json = serde_json::to_string(&ns).unwrap();
        let decoded: CognitiveNamespace = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "test-ns");
        assert_eq!(decoded.owner, "agent-1");
        assert_eq!(decoded.channels.len(), 1);
        assert!(decoded.rate_limit.is_some());
        assert!(decoded.acl.is_admin("agent-1"));
    }
}
