//! OCaps-style capability tokens for agent execution.

use std::path::{Path, PathBuf};

use rand::random;
use serde::{Deserialize, Serialize};
use url::Url;

/// A concrete capability required to execute a tool or resource access.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Capability {
    /// Named tool invocation.
    Tool(String),
    /// Read access to a path.
    ReadPath(PathBuf),
    /// Write access to a path.
    WritePath(PathBuf),
    /// Shell execution capability.
    Exec(String),
    /// Network capability for a host/port pair.
    Network { host: String, port: u16 },
}

/// Unforgeable warrant token carrying a reduced capability set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWarrant {
    /// Token identifier.
    pub id: [u8; 32],
    /// Granted capabilities.
    pub capabilities: Vec<Capability>,
    /// Authority that issued the warrant.
    pub issuer: String,
    /// Optional expiry timestamp in unix seconds.
    pub expires_at: Option<u64>,
    /// Remaining delegation depth.
    pub delegate_depth: u8,
}

impl AgentWarrant {
    /// Create a warrant with a random identifier.
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        capabilities: Vec<Capability>,
        delegate_depth: u8,
    ) -> Self {
        Self {
            id: random(),
            capabilities,
            issuer: issuer.into(),
            expires_at: None,
            delegate_depth,
        }
    }

    /// Attach an expiry timestamp.
    #[must_use]
    pub fn with_expiry(mut self, expires_at: Option<u64>) -> Self {
        self.expires_at = expires_at;
        self
    }
}

/// Errors raised when a warrant cannot be delegated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// The requested capability is not covered by the parent warrant.
    #[error("subset capability is not covered by parent warrant")]
    NotCovered,
    /// The warrant cannot be delegated further.
    #[error("delegation depth exhausted")]
    DepthExhausted,
}

/// Check whether `warrant` covers `required`.
#[must_use]
pub fn check_capability(warrant: &AgentWarrant, required: &Capability) -> bool {
    warrant
        .capabilities
        .iter()
        .any(|granted| capability_covers(granted, required))
}

/// Delegate a warrant to a strict subset of its capabilities.
pub fn delegate(
    warrant: &AgentWarrant,
    subset: &[Capability],
) -> Result<AgentWarrant, CapabilityError> {
    if warrant.delegate_depth == 0 {
        return Err(CapabilityError::DepthExhausted);
    }

    if subset.iter().any(|required| !check_capability(warrant, required)) {
        return Err(CapabilityError::NotCovered);
    }

    Ok(AgentWarrant {
        id: random(),
        capabilities: subset.to_vec(),
        issuer: warrant.issuer.clone(),
        expires_at: warrant.expires_at,
        delegate_depth: warrant.delegate_depth.saturating_sub(1),
    })
}

fn capability_covers(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        (Capability::Tool(a), Capability::Tool(b)) => a == b,
        (Capability::Exec(a), Capability::Exec(b)) => a == b,
        (Capability::Network { host: ah, port: ap }, Capability::Network { host: bh, port: bp }) => {
            ah == bh && ap == bp
        }
        (Capability::ReadPath(granted), Capability::ReadPath(required))
        | (Capability::WritePath(granted), Capability::WritePath(required)) => {
            path_contains(granted, required)
        }
        _ => false,
    }
}

fn path_contains(granted: &Path, required: &Path) -> bool {
    required.starts_with(granted)
}

/// Build a capability requirement from a network URL.
#[must_use]
pub fn network_capability_from_url(url: &str) -> Option<Capability> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed
        .host_str()?
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    Some(Capability::Network {
        host,
        port: parsed.port_or_known_default().unwrap_or(0),
    })
}

/// Extract the first shell token for an exec capability requirement.
#[must_use]
pub fn exec_capability_from_command(command: &str) -> Option<Capability> {
    command
        .split_whitespace()
        .next()
        .filter(|token| !token.is_empty())
        .map(|token| Capability::Exec(token.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_capability_matches_exact_tool() {
        let warrant = AgentWarrant::new("issuer", vec![Capability::Tool("bash".into())], 1);
        assert!(check_capability(&warrant, &Capability::Tool("bash".into())));
        assert!(!check_capability(&warrant, &Capability::Tool("grep".into())));
    }

    #[test]
    fn delegate_reduces_scope() {
        let warrant = AgentWarrant::new(
            "issuer",
            vec![Capability::Tool("bash".into()), Capability::Exec("bash".into())],
            1,
        );
        let child = delegate(&warrant, &[Capability::Tool("bash".into())]).unwrap();
        assert_eq!(child.delegate_depth, 0);
        assert_eq!(child.capabilities.len(), 1);
    }

    #[test]
    fn network_capability_parses_host_and_port() {
        let cap = network_capability_from_url("https://api.example.com:443/path").unwrap();
        assert!(matches!(cap, Capability::Network { .. }));
    }
}
