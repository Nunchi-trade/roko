//! Connector trait for external system I/O.
//!
//! 6 connector kinds: MCP, API, Database, Blockchain, Feed, Custom.
//! The [`ConnectorRegistry`] provides an in-memory catalog of configured
//! connectors with health tracking.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Enums ─────────────────────────────────────────────────────────

/// The kind of external system a connector integrates with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ConnectorKind {
    /// Model Context Protocol server.
    Mcp,
    /// Generic REST / gRPC API.
    Api,
    /// Relational or document database.
    Database,
    /// On-chain RPC endpoint.
    Blockchain,
    /// Streaming data feed.
    Feed,
    /// User-defined connector.
    Custom,
}

/// Liveness status reported by a connector health check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    /// The connector is reachable and responding normally.
    Connected,
    /// The connector is not reachable.
    Disconnected,
    /// The connector is reachable but responding slowly or with errors.
    Degraded,
}

// ── Structs ───────────────────────────────────────────────────────

/// Static configuration for a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    /// Human-readable connector name (unique within a registry).
    pub name: String,
    /// What kind of system this connects to.
    pub kind: ConnectorKind,
    /// Target endpoint URL or address.
    pub endpoint: String,
    /// Optional authentication token / credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
    /// Optional HTTP headers to attach to every request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Request timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    5000
}

/// Live health snapshot for a connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorHealth {
    /// Current liveness status.
    pub status: ConnectorStatus,
    /// Round-trip latency of the most recent health check (milliseconds).
    pub latency_ms: u64,
    /// When the last health check was performed.
    pub last_check: DateTime<Utc>,
}

/// Full descriptor for a registered connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorInfo {
    /// Unique connector name.
    pub name: String,
    /// Connector kind.
    pub kind: ConnectorKind,
    /// Latest health snapshot.
    pub health: ConnectorHealth,
    /// When the connector was first registered.
    pub created_at: DateTime<Utc>,
    /// Arbitrary metadata attached by the registrant.
    #[serde(default)]
    pub metadata: Value,
}

// ── Registry ──────────────────────────────────────────────────────

/// In-memory registry of [`ConnectorInfo`] entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectorRegistry {
    connectors: Vec<ConnectorInfo>,
}

impl ConnectorRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            connectors: Vec::new(),
        }
    }

    /// Register a new connector (or replace an existing one with the same name).
    pub fn register(&mut self, info: ConnectorInfo) {
        // Replace if a connector with this name already exists.
        if let Some(existing) = self.connectors.iter_mut().find(|c| c.name == info.name) {
            *existing = info;
        } else {
            self.connectors.push(info);
        }
    }

    /// Remove a connector by name. Returns `true` if it was present.
    pub fn unregister(&mut self, name: &str) -> bool {
        let before = self.connectors.len();
        self.connectors.retain(|c| c.name != name);
        self.connectors.len() != before
    }

    /// Look up a connector by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ConnectorInfo> {
        self.connectors.iter().find(|c| c.name == name)
    }

    /// List all registered connectors.
    #[must_use]
    pub fn list(&self) -> &[ConnectorInfo] {
        &self.connectors
    }

    /// Count connectors whose health status is [`ConnectorStatus::Connected`].
    #[must_use]
    pub fn healthy_count(&self) -> usize {
        self.connectors
            .iter()
            .filter(|c| c.health.status == ConnectorStatus::Connected)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_info(name: &str, status: ConnectorStatus) -> ConnectorInfo {
        ConnectorInfo {
            name: name.to_string(),
            kind: ConnectorKind::Api,
            health: ConnectorHealth {
                status,
                latency_ms: 42,
                last_check: Utc::now(),
            },
            created_at: Utc::now(),
            metadata: Value::Null,
        }
    }

    #[test]
    fn register_and_list() {
        let mut reg = ConnectorRegistry::new();
        assert!(reg.list().is_empty());

        reg.register(sample_info("alpha", ConnectorStatus::Connected));
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].name, "alpha");
    }

    #[test]
    fn register_replaces_existing() {
        let mut reg = ConnectorRegistry::new();
        reg.register(sample_info("alpha", ConnectorStatus::Disconnected));
        reg.register(sample_info("alpha", ConnectorStatus::Connected));
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].health.status, ConnectorStatus::Connected);
    }

    #[test]
    fn unregister_returns_true_when_present() {
        let mut reg = ConnectorRegistry::new();
        reg.register(sample_info("alpha", ConnectorStatus::Connected));
        assert!(reg.unregister("alpha"));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn unregister_returns_false_when_absent() {
        let mut reg = ConnectorRegistry::new();
        assert!(!reg.unregister("ghost"));
    }

    #[test]
    fn get_returns_entry() {
        let mut reg = ConnectorRegistry::new();
        reg.register(sample_info("beta", ConnectorStatus::Degraded));
        let entry = reg.get("beta").expect("should find beta");
        assert_eq!(entry.health.status, ConnectorStatus::Degraded);
        assert!(reg.get("nope").is_none());
    }

    #[test]
    fn healthy_count_filters_connected() {
        let mut reg = ConnectorRegistry::new();
        reg.register(sample_info("a", ConnectorStatus::Connected));
        reg.register(sample_info("b", ConnectorStatus::Disconnected));
        reg.register(sample_info("c", ConnectorStatus::Connected));
        reg.register(sample_info("d", ConnectorStatus::Degraded));
        assert_eq!(reg.healthy_count(), 2);
    }

    #[test]
    fn serde_roundtrip() {
        let mut reg = ConnectorRegistry::new();
        reg.register(sample_info("x", ConnectorStatus::Connected));
        let json = serde_json::to_string(&reg).expect("serialize");
        let restored: ConnectorRegistry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.list().len(), 1);
        assert_eq!(restored.list()[0].name, "x");
    }
}
