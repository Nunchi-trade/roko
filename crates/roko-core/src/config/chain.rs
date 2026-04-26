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
