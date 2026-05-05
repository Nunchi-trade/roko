//! Chain and relay configuration sections.

use serde::{Deserialize, Serialize};

/// Chain connection settings used by the `chain.*` tool domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ChainConfig {
    /// Chain profile name: "mirage" (local dev), "daeji" (testnet), or custom.
    /// Resolves into a ChainProfile at runtime via ChainProfile::from_roko_config().
    #[serde(default = "default_chain_profile")]
    pub profile: String,
    /// HTTP JSON-RPC endpoint (e.g. `https://mirage-devnet.up.railway.app`).
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// Chain ID. Must match the endpoint. Mirage uses 1.
    #[serde(default)]
    pub chain_id: Option<u64>,
    /// Hex-encoded private key (0x-prefixed or bare). Used to sign txs.
    #[serde(default)]
    pub wallet_key: Option<String>,
    /// ERC-8004 IdentityRegistry contract address.
    #[serde(default)]
    pub identity_registry: Option<String>,
    /// ERC-8004 ReputationRegistry contract address.
    #[serde(default)]
    pub reputation_registry: Option<String>,
    /// ERC-8004 ValidationRegistry contract address.
    #[serde(default)]
    pub validation_registry: Option<String>,
    /// AgentRegistry contract address. Required for on-chain agent features.
    #[serde(default)]
    pub agent_registry: Option<String>,
    /// BountyMarket contract address. Required for on-chain bounty features.
    #[serde(default)]
    pub bounty_market: Option<String>,
    /// Deployer / funder address.
    #[serde(default)]
    pub deployer: Option<String>,
}

/// Relay registration and workspace discovery settings.
///
/// When enabled, `roko serve` registers itself with the relay on startup so
/// that dashboards can auto-discover the workspace without manual URL entry.
///
/// ```toml
/// [relay]
/// url = "wss://relay.nunchi.dev"
/// workspace_name = "will-dev"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RelayConfig {
    /// Relay WebSocket URL (e.g. `wss://relay.nunchi.dev`).
    /// If unset, workspace registration is disabled.
    #[serde(default)]
    pub url: Option<String>,
    /// Human-readable workspace name shown in dashboard discovery.
    /// Defaults to hostname.
    #[serde(default)]
    pub workspace_name: Option<String>,
    /// Public URL of this roko instance (e.g. `https://my-roko.up.railway.app`).
    /// Auto-detected from RAILWAY_PUBLIC_DOMAIN or FLY_APP_NAME if not set.
    #[serde(default)]
    pub public_url: Option<String>,
    /// Heartbeat interval in seconds for workspace presence. Default: 30.
    #[serde(default = "default_relay_heartbeat")]
    pub heartbeat_interval_secs: u64,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            url: None,
            workspace_name: None,
            public_url: None,
            heartbeat_interval_secs: 30,
        }
    }
}

const fn default_relay_heartbeat() -> u64 {
    30
}

fn default_chain_profile() -> String {
    "mirage".to_string()
}

/// `[isfr]` section in roko.toml — ISFR keeper configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ISFRSection {
    /// Whether ISFR features are enabled (default: false).
    pub enabled: bool,
    /// Epoch duration in seconds (default: 28800 = 8 hours).
    pub epoch_duration_secs: u64,
    /// Source poll interval in seconds (default: 10).
    pub poll_interval_secs: u64,
    /// Minimum live source readings required to publish a composite (default: 2).
    pub min_submissions: u32,
    /// Outlier rejection sigma threshold (default: 3.0).
    pub outlier_sigma: f64,
    /// Rate source definitions.
    pub sources: Vec<ISFRSourceConfig>,
}

impl Default for ISFRSection {
    fn default() -> Self {
        Self {
            enabled: false,
            epoch_duration_secs: 28_800,
            poll_interval_secs: 10,
            min_submissions: 2,
            outlier_sigma: 3.0,
            sources: Vec::new(),
        }
    }
}

/// `[[isfr.sources]]` entry in roko.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ISFRSourceConfig {
    /// Human-readable source name (e.g. "mock-aave-v3").
    pub name: String,
    /// Source kind: "mock", "aave_v3", "compound_v3", "ethena", "eth_staking".
    pub kind: String,
    /// Composite weight (0.0–1.0, default: 0.25).
    #[serde(default = "default_isfr_weight")]
    pub weight: f64,
    /// Rate class: "lending", "structured", "funding", "staking".
    pub class: String,
    /// Base rate in bps — mock sources only (e.g. 620 = 6.20%).
    #[serde(default)]
    pub rate_bps: u64,
    /// Rate jitter in bps — mock sources only.
    #[serde(default)]
    pub jitter_bps: u64,
    /// JSON-RPC endpoint — live sources only.
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// Protocol pool/contract address — live sources only.
    #[serde(default)]
    pub pool_address: Option<String>,
}

fn default_isfr_weight() -> f64 {
    0.25
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_isfr_section() {
        let toml_str = r#"
enabled = true
poll_interval_secs = 5

[[sources]]
name = "test"
kind = "mock"
weight = 1.0
class = "lending"
rate_bps = 500
"#;
        let section: ISFRSection = toml::from_str(toml_str).unwrap();
        assert!(section.enabled);
        assert_eq!(section.poll_interval_secs, 5);
        assert_eq!(section.sources.len(), 1);
        assert_eq!(section.sources[0].name, "test");
    }

    #[test]
    fn defaults_when_missing() {
        let section: ISFRSection = toml::from_str("").unwrap();
        assert!(!section.enabled);
        assert_eq!(section.epoch_duration_secs, 28_800);
        assert_eq!(section.poll_interval_secs, 10);
    }

    #[test]
    fn chain_config_profile_default() {
        let config: ChainConfig = toml::from_str("").unwrap();
        assert_eq!(config.profile, "mirage");
    }
}
