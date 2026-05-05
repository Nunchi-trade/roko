//! Chain and relay configuration sections.

use serde::{Deserialize, Serialize};

/// Chain connection settings used by the `chain.*` tool domain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ChainConfig {
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

// ---------------------------------------------------------------------------
// ISFRSection (E1)
// ---------------------------------------------------------------------------

/// ISFR (Intersubjective Fact Rate) keeper configuration.
///
/// Controls the DeFi rate aggregation background task that computes
/// composite lending/funding/staking rates from multiple on-chain sources.
///
/// ```toml
/// [isfr]
/// enabled = true
/// poll_interval_secs = 60
/// epoch_duration_secs = 28800
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ISFRSection {
    /// Whether the ISFR keeper is enabled. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// How often (in seconds) the keeper polls all sources. Default: 60.
    #[serde(default = "default_isfr_poll_interval")]
    pub poll_interval_secs: u64,
    /// Epoch duration in seconds for rate aggregation. Default: 28800 (8 h).
    #[serde(default = "default_isfr_epoch_duration")]
    pub epoch_duration_secs: u64,
}

impl Default for ISFRSection {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_secs: default_isfr_poll_interval(),
            epoch_duration_secs: default_isfr_epoch_duration(),
        }
    }
}

const fn default_isfr_poll_interval() -> u64 {
    60
}

const fn default_isfr_epoch_duration() -> u64 {
    28_800 // 8 hours
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isfr_section_deserializes_with_defaults() {
        let section: ISFRSection = toml::from_str("").unwrap();
        assert!(!section.enabled);
        assert_eq!(section.poll_interval_secs, 60);
        assert_eq!(section.epoch_duration_secs, 28_800);
    }

    #[test]
    fn isfr_section_deserializes_explicit_values() {
        let toml_str = r#"
            enabled = true
            poll_interval_secs = 30
            epoch_duration_secs = 14400
        "#;
        let section: ISFRSection = toml::from_str(toml_str).unwrap();
        assert!(section.enabled);
        assert_eq!(section.poll_interval_secs, 30);
        assert_eq!(section.epoch_duration_secs, 14400);
    }
}
