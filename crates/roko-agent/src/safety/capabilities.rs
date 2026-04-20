//! OCaps-style capability tokens for agent execution.

use std::path::{Path, PathBuf};

use rand::random;
use serde::{Deserialize, Serialize};
use url::Url;

// ─── Plugin trust tiers ──────────────────────────────────────────────

/// Trust tier assigned to plugins and MCP servers.
///
/// Each tier grants a specific set of capabilities. Lower tiers are
/// more restricted: tiers 1-2 are denied secrets and network egress by
/// default. The tier is assigned in `.mcp.json` or at registration time;
/// absent an explicit tier, plugins default to [`PluginTier::Sandboxed`].
///
/// # Tier summary
///
/// | Tier | Label | FS | Network | Secrets |
/// |------|-------------|-----------|---------|---------|
/// | 1 | Untrusted | none | no | no |
/// | 2 | Sandboxed | read-only | no | no |
/// | 3 | Standard | worktree | allow | no |
/// | 4 | Trusted | full | full | yes |
/// | 5 | Kernel | full | full | yes |
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTier {
    /// Tier 1: untrusted WASM — no filesystem, no network, no secrets.
    Untrusted = 1,
    /// Tier 2: sandboxed native — read-only filesystem, no network.
    Sandboxed = 2,
    /// Tier 3: standard plugin — worktree-scoped filesystem, allowlisted network.
    Standard = 3,
    /// Tier 4: trusted native — full filesystem, full network, secrets allowed.
    Trusted = 4,
    /// Tier 5: kernel extension — same trust as core.
    Kernel = 5,
}

impl Default for PluginTier {
    /// Plugins default to Sandboxed (tier 2) when no explicit tier is set.
    fn default() -> Self {
        Self::Sandboxed
    }
}

impl PluginTier {
    /// Return the set of capabilities that this tier permits.
    ///
    /// Higher tiers are strict supersets of lower tiers. The returned
    /// capabilities are used by [`check_plugin_tier`] to gate tool calls
    /// originating from plugins.
    #[must_use]
    pub fn allowed_capabilities(&self) -> Vec<Capability> {
        match self {
            Self::Untrusted => {
                // Tier 1: only named tool invocation, nothing else.
                vec![]
            }
            Self::Sandboxed => {
                // Tier 2: read-only paths.
                vec![Capability::ReadPath(PathBuf::from("/"))]
            }
            Self::Standard => {
                // Tier 3: read + write (worktree-scoped via PathPolicy) + exec.
                vec![
                    Capability::ReadPath(PathBuf::from("/")),
                    Capability::WritePath(PathBuf::from("/")),
                    Capability::Exec("*".into()),
                ]
            }
            Self::Trusted | Self::Kernel => {
                // Tier 4-5: full capabilities including network.
                vec![
                    Capability::ReadPath(PathBuf::from("/")),
                    Capability::WritePath(PathBuf::from("/")),
                    Capability::Exec("*".into()),
                    Capability::Network {
                        host: "*".into(),
                        port: 0,
                    },
                ]
            }
        }
    }

    /// Return `true` if this tier permits network egress.
    #[must_use]
    pub const fn allows_network(&self) -> bool {
        matches!(self, Self::Standard | Self::Trusted | Self::Kernel)
    }

    /// Return `true` if this tier permits access to secrets.
    #[must_use]
    pub const fn allows_secrets(&self) -> bool {
        matches!(self, Self::Trusted | Self::Kernel)
    }

    /// Return `true` if this tier permits filesystem writes.
    #[must_use]
    pub const fn allows_writes(&self) -> bool {
        matches!(self, Self::Standard | Self::Trusted | Self::Kernel)
    }
}

/// Check whether a plugin at the given `tier` is allowed to invoke the
/// requested `capability`. Returns `Ok(())` on success; returns a
/// human-readable error on denial.
pub fn check_plugin_tier(tier: PluginTier, capability: &Capability) -> Result<(), String> {
    match capability {
        Capability::Network { .. } if !tier.allows_network() => Err(format!(
            "plugin tier {:?} does not permit network access",
            tier
        )),
        Capability::WritePath(_) if !tier.allows_writes() => Err(format!(
            "plugin tier {:?} does not permit filesystem writes",
            tier
        )),
        Capability::ReadPath(_) if matches!(tier, PluginTier::Untrusted) => Err(format!(
            "plugin tier {:?} does not permit filesystem reads",
            tier
        )),
        Capability::Exec(_) if matches!(tier, PluginTier::Untrusted | PluginTier::Sandboxed) => {
            Err(format!(
                "plugin tier {:?} does not permit subprocess execution",
                tier
            ))
        }
        _ => Ok(()),
    }
}

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

    if subset
        .iter()
        .any(|required| !check_capability(warrant, required))
    {
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
        (
            Capability::Network { host: ah, port: ap },
            Capability::Network { host: bh, port: bp },
        ) => ah == bh && ap == bp,
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
        assert!(!check_capability(
            &warrant,
            &Capability::Tool("grep".into())
        ));
    }

    #[test]
    fn delegate_reduces_scope() {
        let warrant = AgentWarrant::new(
            "issuer",
            vec![
                Capability::Tool("bash".into()),
                Capability::Exec("bash".into()),
            ],
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

    // ─── PluginTier tests ────────────────────────────────────────────

    #[test]
    fn plugin_tier_default_is_sandboxed() {
        assert_eq!(PluginTier::default(), PluginTier::Sandboxed);
    }

    #[test]
    fn plugin_tier_ordering_is_ascending() {
        assert!(PluginTier::Untrusted < PluginTier::Sandboxed);
        assert!(PluginTier::Sandboxed < PluginTier::Standard);
        assert!(PluginTier::Standard < PluginTier::Trusted);
        assert!(PluginTier::Trusted < PluginTier::Kernel);
    }

    #[test]
    fn untrusted_tier_blocks_everything() {
        let tier = PluginTier::Untrusted;
        assert!(!tier.allows_network());
        assert!(!tier.allows_secrets());
        assert!(!tier.allows_writes());
        assert!(check_plugin_tier(tier, &Capability::ReadPath(PathBuf::from("/tmp"))).is_err());
        assert!(check_plugin_tier(tier, &Capability::WritePath(PathBuf::from("/tmp"))).is_err());
        assert!(check_plugin_tier(tier, &Capability::Exec("ls".into())).is_err());
        assert!(
            check_plugin_tier(
                tier,
                &Capability::Network {
                    host: "example.com".into(),
                    port: 443
                }
            )
            .is_err()
        );
    }

    #[test]
    fn sandboxed_tier_allows_reads_only() {
        let tier = PluginTier::Sandboxed;
        assert!(!tier.allows_network());
        assert!(!tier.allows_secrets());
        assert!(!tier.allows_writes());
        assert!(check_plugin_tier(tier, &Capability::ReadPath(PathBuf::from("/tmp"))).is_ok());
        assert!(check_plugin_tier(tier, &Capability::WritePath(PathBuf::from("/tmp"))).is_err());
        assert!(check_plugin_tier(tier, &Capability::Exec("ls".into())).is_err());
    }

    #[test]
    fn standard_tier_allows_reads_writes_exec_no_network() {
        let tier = PluginTier::Standard;
        assert!(tier.allows_network());
        assert!(!tier.allows_secrets());
        assert!(tier.allows_writes());
        assert!(check_plugin_tier(tier, &Capability::ReadPath(PathBuf::from("/tmp"))).is_ok());
        assert!(check_plugin_tier(tier, &Capability::WritePath(PathBuf::from("/tmp"))).is_ok());
        assert!(check_plugin_tier(tier, &Capability::Exec("ls".into())).is_ok());
    }

    #[test]
    fn trusted_tier_allows_everything() {
        let tier = PluginTier::Trusted;
        assert!(tier.allows_network());
        assert!(tier.allows_secrets());
        assert!(tier.allows_writes());
        assert!(
            check_plugin_tier(
                tier,
                &Capability::Network {
                    host: "example.com".into(),
                    port: 443
                }
            )
            .is_ok()
        );
    }

    #[test]
    fn plugin_tier_round_trips_through_serde() {
        for tier in [
            PluginTier::Untrusted,
            PluginTier::Sandboxed,
            PluginTier::Standard,
            PluginTier::Trusted,
            PluginTier::Kernel,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let decoded: PluginTier = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, tier);
        }
    }
}
