//! Service integration architecture (TOOL-10).
//!
//! Three-layer model for connecting roko to external platforms:
//!
//! 1. **Event Reception** — webhook endpoints, polling adapters, WebSocket
//!    streams that accept external events.
//! 2. **Agent Execution** — received events become [`Engram`]s, matched to
//!    [`Subscription`]s, and dispatched to agent templates.
//! 3. **MCP Tool Adapters** — agents interact with external platforms via
//!    MCP servers (github.*, slack.*, scripts.*).
//!
//! # Built-in integrations
//!
//! | Integration | Event Reception | Tool Adapter |
//! |---|---|---|
//! | GitHub | `POST /webhooks/github` | `roko-mcp-github` |
//! | Slack | `POST /webhooks/slack` | `roko-mcp-slack` |
//! | Generic | `POST /webhooks/generic` | N/A |
//! | Cron | `CronEventSource` | N/A |
//! | FileWatch | `FileWatchEventSource` | N/A |
//!
//! # Configuration
//!
//! Integrations are configured via `roko.toml`:
//! ```toml
//! [webhooks.github]
//! secret = "my-secret"
//!
//! [webhooks.slack]
//! signing_secret = "slack-signing-secret"
//!
//! [[subscriptions]]
//! template = "pr-review"
//! trigger = "github:pr:opened"
//! ```

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

// ─── Integration catalog ─────────────────────────────────────────────

/// Describes which layer an integration component belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationLayer {
    /// Layer 1: receives external events (webhooks, polling, WebSocket).
    EventReception,
    /// Layer 2: dispatches events to agent templates.
    AgentExecution,
    /// Layer 3: provides tools to agents via MCP protocol.
    McpToolAdapter,
}

impl fmt::Display for IntegrationLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventReception => write!(f, "event_reception"),
            Self::AgentExecution => write!(f, "agent_execution"),
            Self::McpToolAdapter => write!(f, "mcp_tool_adapter"),
        }
    }
}

/// Classification of an integration's importance to core functionality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    /// Required for core functionality (e.g., GitHub for code review).
    Structural,
    /// Optional enhancement (e.g., social integrations).
    Decorative,
}

/// The transport mechanism used by an event reception integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceptionTransport {
    /// HTTP webhook endpoint.
    Webhook,
    /// Periodic polling adapter.
    Polling,
    /// WebSocket / SSE stream.
    Stream,
    /// Cron-scheduled trigger.
    Cron,
    /// Filesystem watcher.
    FileWatch,
}

/// Describes a single service integration across all three layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceIntegration {
    /// Integration identifier (e.g., "github", "slack", "linear").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Structural vs decorative classification.
    pub kind: IntegrationKind,
    /// Layer 1: how events are received.
    pub reception: Option<ReceptionDescriptor>,
    /// Layer 2: how events are dispatched to agents.
    pub execution: Option<ExecutionDescriptor>,
    /// Layer 3: MCP server providing tools for this integration.
    pub mcp_adapter: Option<McpAdapterDescriptor>,
}

/// Layer 1 descriptor: how this integration receives events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceptionDescriptor {
    /// Transport mechanism.
    pub transport: ReceptionTransport,
    /// Endpoint path (for webhooks) or source identifier.
    pub endpoint: String,
    /// Whether HMAC signature verification is supported.
    pub hmac_verified: bool,
    /// Signal kinds emitted by this integration.
    pub signal_kinds: Vec<String>,
}

/// Layer 2 descriptor: how events map to agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionDescriptor {
    /// Subscription trigger patterns that match events from this integration.
    pub trigger_patterns: Vec<String>,
    /// Default agent templates associated with this integration.
    pub default_templates: Vec<String>,
}

/// Layer 3 descriptor: MCP tool adapter for this integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAdapterDescriptor {
    /// MCP server binary or crate name.
    pub server: String,
    /// Tool name prefixes provided by this adapter (e.g., "github.*").
    pub tool_prefixes: Vec<String>,
    /// Number of tools provided.
    pub tool_count: usize,
}

// ─── Registry ────────────────────────────────────────────────────────

/// Registry of all known service integrations.
#[derive(Debug, Clone, Default)]
pub struct IntegrationRegistry {
    integrations: HashMap<String, ServiceIntegration>,
}

impl IntegrationRegistry {
    /// Create a new registry pre-populated with built-in integrations.
    #[must_use]
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        for integration in builtin_integrations() {
            registry.register(integration);
        }
        registry
    }

    /// Register a service integration.
    pub fn register(&mut self, integration: ServiceIntegration) {
        self.integrations
            .insert(integration.name.clone(), integration);
    }

    /// Look up an integration by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ServiceIntegration> {
        self.integrations.get(name)
    }

    /// List all registered integrations.
    #[must_use]
    pub fn list(&self) -> Vec<&ServiceIntegration> {
        let mut integrations: Vec<_> = self.integrations.values().collect();
        integrations.sort_by_key(|i| &i.name);
        integrations
    }

    /// List integrations filtered by layer capability.
    #[must_use]
    pub fn by_layer(&self, layer: IntegrationLayer) -> Vec<&ServiceIntegration> {
        self.integrations
            .values()
            .filter(|i| match layer {
                IntegrationLayer::EventReception => i.reception.is_some(),
                IntegrationLayer::AgentExecution => i.execution.is_some(),
                IntegrationLayer::McpToolAdapter => i.mcp_adapter.is_some(),
            })
            .collect()
    }

    /// List integrations filtered by kind.
    #[must_use]
    pub fn by_kind(&self, kind: IntegrationKind) -> Vec<&ServiceIntegration> {
        self.integrations
            .values()
            .filter(|i| i.kind == kind)
            .collect()
    }

    /// Total number of registered integrations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.integrations.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.integrations.is_empty()
    }
}

// ─── Built-in integrations ──────────────────────────────────────────

/// Return the set of built-in integrations shipped with roko.
fn builtin_integrations() -> Vec<ServiceIntegration> {
    vec![
        ServiceIntegration {
            name: "github".into(),
            description: "GitHub code hosting: PRs, issues, pushes, reviews".into(),
            kind: IntegrationKind::Structural,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::Webhook,
                endpoint: "/webhooks/github".into(),
                hmac_verified: true,
                signal_kinds: vec![
                    "github:push".into(),
                    "github:pr:opened".into(),
                    "github:pr:review".into(),
                    "github:issue:opened".into(),
                    "prd:plan:approved".into(),
                ],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec![
                    "github:*".into(),
                    "prd:plan:approved".into(),
                ],
                default_templates: vec![
                    "pr-review".into(),
                    "code-implementer".into(),
                ],
            }),
            mcp_adapter: Some(McpAdapterDescriptor {
                server: "roko-mcp-github".into(),
                tool_prefixes: vec!["github.*".into()],
                tool_count: 19,
            }),
        },
        ServiceIntegration {
            name: "slack".into(),
            description: "Slack messaging: messages, reactions, threads".into(),
            kind: IntegrationKind::Structural,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::Webhook,
                endpoint: "/webhooks/slack".into(),
                hmac_verified: true,
                signal_kinds: vec![
                    "slack:message".into(),
                    "slack:reaction".into(),
                ],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec!["slack:*".into()],
                default_templates: vec!["slack-notify".into()],
            }),
            mcp_adapter: Some(McpAdapterDescriptor {
                server: "roko-mcp-slack".into(),
                tool_prefixes: vec!["slack.*".into()],
                tool_count: 9,
            }),
        },
        ServiceIntegration {
            name: "generic-webhook".into(),
            description: "Generic JSON webhook for custom integrations".into(),
            kind: IntegrationKind::Decorative,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::Webhook,
                endpoint: "/webhooks/generic".into(),
                hmac_verified: false,
                signal_kinds: vec!["webhook:generic".into()],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec!["webhook:*".into()],
                default_templates: vec![],
            }),
            mcp_adapter: None,
        },
        ServiceIntegration {
            name: "scripts".into(),
            description: "Config-driven script wrappers exposed as tools".into(),
            kind: IntegrationKind::Decorative,
            reception: None,
            execution: None,
            mcp_adapter: Some(McpAdapterDescriptor {
                server: "roko-mcp-scripts".into(),
                tool_prefixes: vec!["scripts.*".into()],
                tool_count: 0, // dynamic, depends on scripts.toml
            }),
        },
        ServiceIntegration {
            name: "cron".into(),
            description: "Scheduled event triggers via cron expressions".into(),
            kind: IntegrationKind::Structural,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::Cron,
                endpoint: "[scheduler] config section".into(),
                hmac_verified: false,
                signal_kinds: vec!["cron:tick".into()],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec!["cron:*".into()],
                default_templates: vec![],
            }),
            mcp_adapter: None,
        },
        ServiceIntegration {
            name: "file-watch".into(),
            description: "Filesystem change detection for watched paths".into(),
            kind: IntegrationKind::Structural,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::FileWatch,
                endpoint: "[watcher] config section".into(),
                hmac_verified: false,
                signal_kinds: vec!["file:changed".into()],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec!["file:*".into()],
                default_templates: vec![],
            }),
            mcp_adapter: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_contains_expected_integrations() {
        let registry = IntegrationRegistry::with_builtins();
        assert!(registry.len() >= 6);
        assert!(registry.get("github").is_some());
        assert!(registry.get("slack").is_some());
        assert!(registry.get("generic-webhook").is_some());
        assert!(registry.get("scripts").is_some());
        assert!(registry.get("cron").is_some());
        assert!(registry.get("file-watch").is_some());
    }

    #[test]
    fn github_integration_has_all_three_layers() {
        let registry = IntegrationRegistry::with_builtins();
        let github = registry.get("github").unwrap();
        assert!(github.reception.is_some());
        assert!(github.execution.is_some());
        assert!(github.mcp_adapter.is_some());
        assert_eq!(github.kind, IntegrationKind::Structural);
    }

    #[test]
    fn scripts_integration_is_mcp_only() {
        let registry = IntegrationRegistry::with_builtins();
        let scripts = registry.get("scripts").unwrap();
        assert!(scripts.reception.is_none());
        assert!(scripts.execution.is_none());
        assert!(scripts.mcp_adapter.is_some());
    }

    #[test]
    fn filter_by_layer_returns_correct_subsets() {
        let registry = IntegrationRegistry::with_builtins();
        let event_reception = registry.by_layer(IntegrationLayer::EventReception);
        let mcp_adapters = registry.by_layer(IntegrationLayer::McpToolAdapter);

        // github, slack, generic-webhook, cron, file-watch have event reception
        assert!(event_reception.len() >= 5);
        // github, slack, scripts have MCP adapters
        assert!(mcp_adapters.len() >= 3);
    }

    #[test]
    fn filter_by_kind_separates_structural_and_decorative() {
        let registry = IntegrationRegistry::with_builtins();
        let structural = registry.by_kind(IntegrationKind::Structural);
        let decorative = registry.by_kind(IntegrationKind::Decorative);

        assert!(structural.iter().any(|i| i.name == "github"));
        assert!(structural.iter().any(|i| i.name == "slack"));
        assert!(decorative.iter().any(|i| i.name == "generic-webhook"));
    }

    #[test]
    fn custom_integration_can_be_registered() {
        let mut registry = IntegrationRegistry::with_builtins();
        let initial_count = registry.len();

        registry.register(ServiceIntegration {
            name: "linear".into(),
            description: "Linear project management polling adapter".into(),
            kind: IntegrationKind::Structural,
            reception: Some(ReceptionDescriptor {
                transport: ReceptionTransport::Polling,
                endpoint: "https://api.linear.app/graphql".into(),
                hmac_verified: false,
                signal_kinds: vec!["linear:issue:created".into()],
            }),
            execution: Some(ExecutionDescriptor {
                trigger_patterns: vec!["linear:*".into()],
                default_templates: vec!["triage".into()],
            }),
            mcp_adapter: None,
        });

        assert_eq!(registry.len(), initial_count + 1);
        assert!(registry.get("linear").is_some());
    }

    #[test]
    fn github_reception_has_hmac_verification() {
        let registry = IntegrationRegistry::with_builtins();
        let github = registry.get("github").unwrap();
        let reception = github.reception.as_ref().unwrap();
        assert!(reception.hmac_verified);
        assert_eq!(reception.endpoint, "/webhooks/github");
        assert!(!reception.signal_kinds.is_empty());
    }
}
