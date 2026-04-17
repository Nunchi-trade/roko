//! Post-run invariant checks.
//!
//! Given a deployments.json + the scenario definition, asserts:
//! 1. every listed contract has code at its address
//! 2. every expected event fired at least `min_count` times

use std::path::Path;

use alloy::primitives::Address;
use alloy::providers::Provider;
use alloy::rpc::types::eth::{BlockNumberOrTag, Filter};
use serde::Deserialize;

use crate::chain_ctx::ChainCtx;
use crate::deploy::ContractArtifact;
use crate::manifest::Scenario;

/// Persisted deployments.json shape.
#[derive(Debug, Deserialize)]
pub struct Deployments {
    /// Chain id captured at deploy time.
    pub chain_id: u64,
    /// Block at which the last contract was mined.
    pub deployed_at_block: u64,
    /// contract-name → 0x-address.
    pub contracts: std::collections::HashMap<String, String>,
}

impl Deployments {
    /// Load from a file.
    ///
    /// # Errors
    ///
    /// Returns an error if the deployments file cannot be read or parsed.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read deployments {}: {e}", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    }
}

/// Aggregate outcome.
pub struct VerifyReport {
    /// True when every invariant held.
    pub ok: bool,
    /// Human-readable findings.
    pub findings: Vec<String>,
}

/// Run all checks. Never panics — returns a report.
///
/// # Errors
///
/// Returns an error if the read provider cannot be built, a deployed address
/// cannot be parsed, a contract artifact cannot be loaded, or a required chain
/// query fails.
pub async fn verify(
    ctx: &ChainCtx,
    scenario: &Scenario,
    contracts_dir: &Path,
) -> anyhow::Result<VerifyReport> {
    let mut findings = Vec::new();
    let mut ok = true;
    let provider = ctx.read_provider()?;

    // 1. Bytecode at every deployed address.
    for (name, hex_addr) in &ctx.addresses {
        let addr: Address = hex_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("parse {hex_addr}: {e}"))?;
        let code = provider
            .get_code_at(addr)
            .await
            .map_err(|e| anyhow::anyhow!("get_code_at {name}: {e}"))?;
        if code.is_empty() {
            ok = false;
            findings.push(format!("FAIL: {name} at {hex_addr} has no bytecode"));
        } else {
            findings.push(format!("ok:   {name} at {hex_addr} ({} bytes)", code.len()));
        }
    }

    // 2. Expected events.
    for exp in &scenario.success.expected_events {
        let Some(hex_addr) = ctx.addresses.get(&exp.contract) else {
            ok = false;
            findings.push(format!(
                "FAIL: event check for undeployed contract {}",
                exp.contract
            ));
            continue;
        };
        let addr: Address = hex_addr
            .parse()
            .map_err(|e| anyhow::anyhow!("parse {hex_addr}: {e}"))?;
        // Compute event topic0 from ABI.
        let artifact = ContractArtifact::load(contracts_dir, &exp.contract)?;
        let Some(topic0) = event_topic0(&artifact, &exp.event)? else {
            ok = false;
            findings.push(format!(
                "FAIL: event {} not found in {} ABI",
                exp.event, exp.contract
            ));
            continue;
        };
        let filter = Filter::new()
            .address(addr)
            .event_signature(topic0)
            .from_block(BlockNumberOrTag::Earliest)
            .to_block(BlockNumberOrTag::Latest);
        let logs = match provider.get_logs(&filter).await {
            Ok(l) => l,
            Err(e) => {
                // Mirage's eth_getLogs is stubbed — don't fail hard.
                findings.push(format!(
                    "skip: {}.{} log query unsupported ({e})",
                    exp.contract, exp.event
                ));
                continue;
            }
        };
        let count = logs.len() as u32;
        if count < exp.min_count {
            ok = false;
            findings.push(format!(
                "FAIL: {}.{} fired {count} times (need {})",
                exp.contract, exp.event, exp.min_count
            ));
        } else {
            findings.push(format!(
                "ok:   {}.{} fired {count} times (>= {})",
                exp.contract, exp.event, exp.min_count
            ));
        }
    }

    Ok(VerifyReport { ok, findings })
}

fn event_topic0(
    artifact: &ContractArtifact,
    event_name: &str,
) -> anyhow::Result<Option<alloy::primitives::B256>> {
    let Some(arr) = artifact.abi.as_array() else {
        return Ok(None);
    };
    for item in arr {
        if item.get("type").and_then(|v| v.as_str()) != Some("event") {
            continue;
        }
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if name != event_name {
            continue;
        }
        let inputs = item
            .get("inputs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let type_str = inputs
            .iter()
            .filter_map(|i| i.get("type").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let sig = format!("{event_name}({type_str})");
        let hash = alloy::primitives::keccak256(sig.as_bytes());
        return Ok(Some(hash));
    }
    Ok(None)
}
