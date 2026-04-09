//! TOML schema for the demo environment + scenarios.
//!
//! Layout (see `roko/demo/`):
//!
//! ```text
//! demo/
//!   manifest.toml                 # lists scenarios by name → path
//!   wallets.toml                  # named (address, private_key) entries
//!   scenarios/
//!     <name>.toml                 # per-scenario: deploy + fixtures + agents
//!   prompts/
//!     <role>.md                   # prompt templates for agent roles
//! ```
//!
//! Extending the demo is strictly a matter of adding files here — no Rust glue
//! change required for most cases.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Top-level registry of scenarios.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    /// Schema version for forward/backward compat.
    pub schema_version: u32,
    /// Defaults inherited by every scenario.
    #[serde(default)]
    pub defaults: Defaults,
    /// All registered scenarios.
    pub scenarios: Vec<ScenarioEntry>,
}

/// Defaults shared across scenarios.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Defaults {
    /// JSON-RPC endpoint; overridden by env `ROKO_MIRAGE_URL`.
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// Chain id.
    #[serde(default)]
    pub chain_id: Option<u64>,
    /// Path to wallets.toml (relative to demo dir).
    #[serde(default)]
    pub wallet_file: Option<String>,
    /// Path to the contracts foundry project (relative to roko/).
    #[serde(default)]
    pub contracts_dir: Option<String>,
}

/// One entry in the top-level scenarios table.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScenarioEntry {
    /// Unique scenario name (used as CLI selector).
    pub name: String,
    /// Path to the scenario TOML file, relative to the demo dir.
    pub path: String,
    /// Short human-readable description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Full scenario definition (from `demo/scenarios/<name>.toml`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Scenario {
    /// Schema version.
    pub schema_version: u32,
    /// Scenario name (should match its entry in manifest.toml).
    pub name: String,
    /// Deployment step.
    pub deploy: DeployStep,
    /// Fixtures run in order post-deploy.
    #[serde(default)]
    pub fixtures: Vec<FixtureStep>,
    /// Agents spawned post-fixtures.
    #[serde(default)]
    pub agents: Vec<AgentSpec>,
    /// Success criteria for exit 0.
    #[serde(default)]
    pub success: SuccessCriteria,
}

/// Declares which contracts to deploy and in what order.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeployStep {
    /// Named wallet (in wallets.toml) that deploys everything.
    pub sender_wallet: String,
    /// Contracts to deploy in order. Each entry references a forge artifact by
    /// name (e.g. "MockERC20") and optionally supplies constructor args.
    pub contracts: Vec<ContractDeploy>,
}

/// Single contract in the deployment sequence.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContractDeploy {
    /// Artifact name — corresponds to `contracts/out/<Name>.sol/<Name>.json`.
    pub name: String,
    /// Constructor args. Each arg is either a literal, a named reference
    /// (`"$deployer"`, `"$contract(MockERC20)"`), or a typed scalar.
    #[serde(default)]
    pub args: Vec<ConstructorArg>,
}

/// A single constructor argument.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ConstructorArg {
    /// Uint scalar (`{"type": "uint256", "value": 18}`) or typed via adjacent struct.
    Typed(TypedArg),
    /// Bare literal passed directly.
    Literal(toml::Value),
}

/// Explicitly typed constructor argument (for disambiguation).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TypedArg {
    /// Solidity ABI type ("address", "uint256", "string", "bytes32", "bool").
    #[serde(rename = "type")]
    pub ty: String,
    /// Value as TOML — coerced per `ty` at encode time.
    pub value: toml::Value,
}

/// Fixture step: anything that mutates chain state after deploy.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FixtureStep {
    /// Human-readable name.
    pub name: String,
    /// Discriminator.
    pub kind: FixtureKind,
    /// Kind-specific config (raw TOML passed to handler).
    #[serde(flatten)]
    pub config: toml::Value,
}

/// Fixture dispatch tag.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixtureKind {
    /// Shell out to forge-script. Kept for scenarios whose setup is cleaner in Solidity.
    ForgeScript,
    /// Direct JSON-RPC call(s) against the chain (e.g. `chain_postInsight` precompiles).
    Jsonrpc,
    /// Rust handler registered at compile time by a scenario module.
    Rust,
    /// ABI-encoded call to a deployed contract via alloy (e.g. post-deploy wiring).
    ContractCall,
}

/// Agent role to spawn (templated via `{i}` for `count > 1`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentSpec {
    /// Role name (e.g. "worker-bidder").
    pub role: String,
    /// Wallet name in wallets.toml (templated with `{i}` if count > 1).
    pub wallet: String,
    /// Path (relative to demo dir) to the prompt template file.
    pub prompt_template: String,
    /// Number of instances to spawn.
    #[serde(default = "default_count")]
    pub count: u32,
    /// Names of scripted-spine actions to run in this agent.
    #[serde(default)]
    pub scripted_actions: Vec<String>,
    /// LLM output slots this role's prompts must fill.
    #[serde(default)]
    pub llm_slots: Vec<String>,
}

fn default_count() -> u32 {
    1
}

/// Exit criteria — roko-demo exits 0 if all satisfied within `max_duration_secs`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SuccessCriteria {
    /// Kill switch: abort after this many seconds if criteria unmet.
    #[serde(default = "default_max_secs")]
    pub max_duration_secs: u64,
    /// Events that must each fire at least `min_count` times.
    #[serde(default)]
    pub expected_events: Vec<ExpectedEvent>,
}

fn default_max_secs() -> u64 {
    300
}

/// One event invariant.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExpectedEvent {
    /// Contract artifact name.
    pub contract: String,
    /// Event name (e.g. "JobResolved").
    pub event: String,
    /// Minimum number of times the event must fire.
    pub min_count: u32,
}

/// Named wallet entry.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WalletEntry {
    /// Wallet alias (used from manifest + scenarios).
    pub name: String,
    /// Hex private key (`0x…`).
    pub private_key: String,
    /// Address derived from the key — optional, checked if present.
    #[serde(default)]
    pub address: Option<String>,
}

/// Wallets file.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Wallets {
    /// All named wallets.
    pub wallets: Vec<WalletEntry>,
}

impl Wallets {
    /// Look up by name.
    pub fn get(&self, name: &str) -> Option<&WalletEntry> {
        self.wallets.iter().find(|w| w.name == name)
    }
}

/// A loaded manifest + its resolved base dir.
pub struct LoadedManifest {
    /// Parsed top-level manifest.
    pub manifest: Manifest,
    /// Directory that contained manifest.toml (used to resolve relative paths).
    pub demo_dir: PathBuf,
}

impl LoadedManifest {
    /// Load the manifest from `demo/manifest.toml`.
    pub fn load(demo_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let demo_dir = demo_dir.as_ref().to_path_buf();
        let path = demo_dir.join("manifest.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read manifest at {}: {e}", path.display()))?;
        let manifest: Manifest = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse manifest at {}: {e}", path.display()))?;
        if manifest.schema_version != 1 {
            return Err(anyhow::anyhow!(
                "unsupported manifest schema_version: {}",
                manifest.schema_version
            ));
        }
        Ok(Self { manifest, demo_dir })
    }

    /// Resolve a scenario by name and load its full definition.
    pub fn load_scenario(&self, name: &str) -> anyhow::Result<Scenario> {
        let entry = self
            .manifest
            .scenarios
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown scenario: {name}"))?;
        let path = self.demo_dir.join(&entry.path);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read scenario at {}: {e}", path.display()))?;
        let scenario: Scenario = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse scenario at {}: {e}", path.display()))?;
        if scenario.name != name {
            return Err(anyhow::anyhow!(
                "scenario name mismatch: manifest={name}, file={}",
                scenario.name
            ));
        }
        Ok(scenario)
    }

    /// Load wallets.toml (path defaults to `<demo>/wallets.toml`).
    pub fn load_wallets(&self) -> anyhow::Result<Wallets> {
        let rel = self
            .manifest
            .defaults
            .wallet_file
            .clone()
            .unwrap_or_else(|| "wallets.toml".into());
        let path = self.demo_dir.join(rel);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read wallets at {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| anyhow::anyhow!("parse wallets: {e}"))
    }

    /// Expand templated agent specs (count > 1 → `{i}` substitution).
    pub fn expand_agents(agents: &[AgentSpec]) -> Vec<ExpandedAgent> {
        let mut out = Vec::new();
        for spec in agents {
            for i in 0..spec.count {
                out.push(ExpandedAgent {
                    role: spec.role.clone(),
                    wallet: spec.wallet.replace("{i}", &i.to_string()),
                    prompt_template: spec.prompt_template.clone(),
                    index: i,
                    scripted_actions: spec.scripted_actions.clone(),
                    llm_slots: spec.llm_slots.clone(),
                });
            }
        }
        out
    }
}

/// One concrete (un-templated) agent instance.
#[derive(Clone, Debug)]
pub struct ExpandedAgent {
    /// Role name (shared across instances).
    pub role: String,
    /// Concrete wallet name.
    pub wallet: String,
    /// Path to prompt template.
    pub prompt_template: String,
    /// Instance index within the role (0-based).
    pub index: u32,
    /// Scripted-spine action identifiers.
    pub scripted_actions: Vec<String>,
    /// LLM output slots.
    pub llm_slots: Vec<String>,
}

/// Write the post-deploy address registry.
pub fn write_deployments(
    runtime_dir: impl AsRef<Path>,
    scenario: &str,
    addresses: &HashMap<String, String>,
    chain_id: u64,
    block_number: u64,
) -> anyhow::Result<PathBuf> {
    let dir = runtime_dir.as_ref().join(scenario);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("deployments.json");
    let doc = serde_json::json!({
        "chain_id": chain_id,
        "deployed_at_block": block_number,
        "contracts": addresses,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
    Ok(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_manifest() {
        let toml_src = r#"
schema_version = 1

[[scenarios]]
name = "smoke"
path = "scenarios/smoke.toml"
"#;
        let m: Manifest = toml::from_str(toml_src).unwrap();
        assert_eq!(m.schema_version, 1);
        assert_eq!(m.scenarios.len(), 1);
        assert_eq!(m.scenarios[0].name, "smoke");
    }

    #[test]
    fn parses_full_scenario() {
        let toml_src = r#"
schema_version = 1
name = "job-board"

[deploy]
sender_wallet = "deployer"
contracts = [
  { name = "MockERC20", args = [
    { type = "string", value = "DAEJI" },
    { type = "string", value = "DAEJI" },
    { type = "uint8", value = 18 },
  ] },
  { name = "WorkerRegistry", args = [
    { type = "address", value = "$contract(MockERC20)" },
  ] },
]

[[fixtures]]
name = "mint"
kind = "jsonrpc"
method = "eth_blockNumber"

[[agents]]
role = "worker-bidder"
wallet = "worker{i}"
prompt_template = "prompts/worker-bidder.md"
count = 3
scripted_actions = ["watch_and_bid"]
llm_slots = ["bid_amount"]

[success]
max_duration_secs = 60
expected_events = [
  { contract = "MockERC20", event = "Transfer", min_count = 1 },
]
"#;
        let s: Scenario = toml::from_str(toml_src).unwrap();
        assert_eq!(s.name, "job-board");
        assert_eq!(s.deploy.contracts.len(), 2);
        assert_eq!(s.agents[0].count, 3);
        assert_eq!(s.success.expected_events.len(), 1);

        let expanded = LoadedManifest::expand_agents(&s.agents);
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].wallet, "worker0");
        assert_eq!(expanded[2].wallet, "worker2");
    }
}
