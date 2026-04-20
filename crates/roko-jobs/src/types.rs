//! Shared types for roko-jobs. Ports `cli/jobs/registry.py` +
//! `cli/jobs/strategy_interfaces.py`.

use std::collections::HashMap;

use roko_custody::CustodyPolicy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobCategory {
    Keeper,
    Operator,
    Cooperative,
    Managed,
}

impl JobCategory {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Keeper => "keeper",
            Self::Operator => "operator",
            Self::Cooperative => "cooperative",
            Self::Managed => "managed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    NewBlock,
    OracleUpdate,
    Event,
    ClearingRound,
    Timer,
}

impl TriggerType {
    /// Event-type strings the subscriber should listen for.
    #[must_use]
    pub fn event_names(&self, trigger_config: &HashMap<String, serde_json::Value>) -> Vec<String> {
        match self {
            Self::NewBlock | Self::Timer => vec!["NewBlock".into()],
            Self::OracleUpdate => vec!["OracleUpdate".into()],
            Self::ClearingRound => vec!["ClearingRound".into()],
            Self::Event => {
                let name = trigger_config
                    .get("event_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("ContractEvent");
                vec![name.to_string()]
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobDefinition {
    pub job_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub category: JobCategory,
    pub trigger: TriggerType,
    #[serde(default)]
    pub trigger_config: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub required_role: Option<String>,
    #[serde(default)]
    pub requires_tee: bool,
    #[serde(default)]
    pub min_stake_eth: f64,
    #[serde(default = "default_stake_token")]
    pub stake_token: String,
    #[serde(default)]
    pub custody: CustodyPolicy,
    #[serde(default)]
    pub strategy_interface: String,
    #[serde(default)]
    pub context_template: String,
    #[serde(default)]
    pub engine_type: String,
    #[serde(default)]
    pub default_strategy: Option<String>,
}

fn default_stake_token() -> String {
    "HYPE".into()
}

impl JobDefinition {
    #[must_use]
    pub fn min_stake_display(&self) -> String {
        if self.min_stake_eth <= 0.0 {
            "none".into()
        } else if self.min_stake_eth.fract() == 0.0 {
            format!("{} {}", self.min_stake_eth as u64, self.stake_token)
        } else {
            format!("{} {}", self.min_stake_eth, self.stake_token)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobConfig {
    pub job_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub mainnet: bool,
    #[serde(default)]
    pub chain_rpc: String,
    #[serde(default)]
    pub event_bus_ws: String,
    #[serde(default)]
    pub relay_url: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub strategy_params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub stake_amount: f64,
    #[serde(default)]
    pub tee_enabled: bool,
    #[serde(default)]
    pub pcr_whitelist: Vec<String>,
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_s: u64,
}

fn default_data_dir() -> String {
    "data/jobs".into()
}
fn default_heartbeat_interval() -> u64 {
    60
}

/// An on-chain event received from the event bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainEvent {
    pub event_type: String,
    pub block_number: u64,
    #[serde(default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub data: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub timestamp_ms: i64,
}

/// Per-event context supplied to keeper strategies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeeperContext {
    pub event: ChainEvent,
    #[serde(default)]
    pub chain_state: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub gas_price_gwei: f64,
    #[serde(default)]
    pub agent_balance_eth: f64,
}

/// A transaction to submit. Mirror of `cli/jobs/strategy_interfaces.Transaction`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub to: String,
    pub data: String,
    #[serde(default)]
    pub value_wei: u128,
    #[serde(default)]
    pub gas_limit: u64,
    #[serde(default)]
    pub chain_id: u64,
}

impl From<&Transaction> for roko_custody::Transaction {
    fn from(tx: &Transaction) -> Self {
        Self {
            to: tx.to.clone(),
            data: tx.data.clone(),
            value_wei: tx.value_wei,
            gas_limit: (tx.gas_limit != 0).then_some(tx.gas_limit),
            chain_id: (tx.chain_id != 0).then_some(tx.chain_id),
        }
    }
}
