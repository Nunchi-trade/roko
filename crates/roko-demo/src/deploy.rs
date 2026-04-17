//! Alloy-backed contract deployer.
//!
//! Reads forge artifacts (`contracts/out/<Name>.sol/<Name>.json`), ABI-encodes
//! constructor args per the scenario manifest, and deploys contracts in order
//! using a single named deployer wallet. Emits a `deployments.json` address
//! registry for downstream fixtures + agents to consume.
//!
//! We shell out to `forge` only for compilation (see [`ensure_artifacts_built`]);
//! all on-chain interaction goes through alloy so mirage-rs doesn't have to
//! satisfy forge's broadcast-watcher model.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use alloy::dyn_abi::DynSolValue;
use alloy::hex;
use alloy::network::{EthereumWallet, TransactionBuilder};
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::eth::TransactionRequest;
use alloy::signers::local::PrivateKeySigner;
use serde::Deserialize;

use crate::manifest::{
    ConstructorArg, ContractDeploy, DeployStep, LoadedManifest, TypedArg, WalletEntry, Wallets,
};

/// Raw forge artifact shape.
#[derive(Deserialize)]
struct ForgeArtifact {
    abi: serde_json::Value,
    bytecode: ForgeBytecode,
}

#[derive(Deserialize)]
struct ForgeBytecode {
    object: String,
}

/// Loaded + decoded forge artifact.
#[derive(Clone)]
pub struct ContractArtifact {
    /// Contract name.
    pub name: String,
    /// Raw ABI JSON.
    pub abi: serde_json::Value,
    /// Deploy bytecode (includes init code, no constructor args).
    pub init_code: Bytes,
}

impl ContractArtifact {
    /// Load `contracts/out/<Name>.sol/<Name>.json`.
    ///
    /// # Errors
    ///
    /// Returns an error if the artifact cannot be read, parsed, decoded, or if
    /// the bytecode is empty.
    pub fn load(contracts_dir: &Path, name: &str) -> anyhow::Result<Self> {
        let path = contracts_dir
            .join("out")
            .join(format!("{name}.sol"))
            .join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read artifact {}: {e}", path.display()))?;
        let raw: ForgeArtifact = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parse artifact {}: {e}", path.display()))?;
        let hex_str = raw.bytecode.object.trim_start_matches("0x");
        let bytes =
            hex::decode(hex_str).map_err(|e| anyhow::anyhow!("decode bytecode for {name}: {e}"))?;
        if bytes.is_empty() {
            return Err(anyhow::anyhow!(
                "empty bytecode for {name} (did you run `forge build`?)"
            ));
        }
        Ok(Self {
            name: name.to_string(),
            abi: raw.abi,
            init_code: Bytes::from(bytes),
        })
    }

    /// Find the constructor inputs (types) from the ABI.
    pub fn constructor_inputs(&self) -> Vec<String> {
        let Some(arr) = self.abi.as_array() else {
            return Vec::new();
        };
        for item in arr {
            if item.get("type").and_then(|v| v.as_str()) == Some("constructor") {
                if let Some(inputs) = item.get("inputs").and_then(|v| v.as_array()) {
                    return inputs
                        .iter()
                        .filter_map(|i| i.get("type").and_then(|t| t.as_str()).map(String::from))
                        .collect();
                }
            }
        }
        Vec::new()
    }
}

/// Ensure `forge build` has produced artifacts; run it if out/ is empty.
///
/// # Errors
///
/// Returns an error if `forge build` cannot be spawned or exits unsuccessfully.
pub fn ensure_artifacts_built(contracts_dir: &Path) -> anyhow::Result<()> {
    let out = contracts_dir.join("out");
    let needs_build =
        !out.exists() || std::fs::read_dir(&out).is_ok_and(|mut d| d.next().is_none());
    if !needs_build {
        return Ok(());
    }
    tracing::info!("running `forge build` in {}", contracts_dir.display());
    let status = std::process::Command::new("forge")
        .arg("build")
        .current_dir(contracts_dir)
        .status()
        .map_err(|e| anyhow::anyhow!("spawn forge build: {e}"))?;
    if !status.success() {
        return Err(anyhow::anyhow!("forge build failed: {status}"));
    }
    Ok(())
}

/// Wrap everything needed to deploy a scenario.
pub struct DeployCtx {
    /// HTTP RPC URL.
    pub rpc_url: String,
    /// Chain id.
    pub chain_id: u64,
    /// Contracts project directory (foundry root).
    pub contracts_dir: PathBuf,
    /// Wallet file.
    pub wallets: Wallets,
}

/// Result of a deployment run.
pub struct DeployedSuite {
    /// Map contract-name → 0x-prefixed address.
    pub addresses: HashMap<String, String>,
    /// Block number at which the last contract was mined.
    pub last_block: u64,
}

/// Advance the chain by one block (via `evm_mine`) if the tip is still at
/// genesis. Works around mirage-rs's lazy local block initialization so
/// alloy's `eth_getBlockByNumber("latest")` sees a real block.
///
/// # Errors
///
/// Returns an error if the JSON-RPC probe request fails or if the probe
/// response body cannot be decoded.
pub async fn warmup_chain(rpc_url: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_blockNumber",
            "params": [],
            "id": 1
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("block_number probe: {e}"))?
        .json::<serde_json::Value>()
        .await?;
    let hex = resp.get("result").and_then(|v| v.as_str()).unwrap_or("0x0");
    let tip = u64::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap_or(0);
    if tip == 0 {
        let _ = client
            .post(rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "evm_mine",
                "params": [],
                "id": 1
            }))
            .send()
            .await;
    }
    Ok(())
}

/// Deploy the full suite in order.
///
/// # Errors
///
/// Returns an error if warming up the chain, building artifacts, loading an
/// artifact, submitting a deployment transaction, waiting for the receipt, or
/// decoding the deployed contract address fails.
pub async fn deploy_suite(ctx: &DeployCtx, deploy: &DeployStep) -> anyhow::Result<DeployedSuite> {
    warmup_chain(&ctx.rpc_url).await?;
    ensure_artifacts_built(&ctx.contracts_dir)?;

    let wallet_entry = ctx.wallets.get(&deploy.sender_wallet).ok_or_else(|| {
        anyhow::anyhow!("deploy sender wallet not found: {}", deploy.sender_wallet)
    })?;
    let provider = wallet_provider(&ctx.rpc_url, wallet_entry)?;
    let signer_address = parse_signer_address(wallet_entry)?;

    let mut addresses: HashMap<String, String> = HashMap::new();
    let mut last_block: u64 = 0;

    for step in &deploy.contracts {
        let artifact = ContractArtifact::load(&ctx.contracts_dir, &step.name)?;
        let init_code = encode_init_code(&artifact, step, &addresses, signer_address)?;
        let tx = TransactionRequest::default()
            .with_from(signer_address)
            .with_deploy_code(init_code)
            .with_chain_id(ctx.chain_id);
        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| anyhow::anyhow!("submit deploy of {}: {e}", step.name))?;
        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| anyhow::anyhow!("receipt for {}: {e}", step.name))?;
        if !receipt.status() {
            return Err(anyhow::anyhow!(
                "deploy of {} reverted (tx={}, block={})",
                step.name,
                receipt.transaction_hash,
                receipt.block_number.unwrap_or(0)
            ));
        }
        let address = receipt
            .contract_address
            .ok_or_else(|| anyhow::anyhow!("missing contract_address for {}", step.name))?;
        last_block = receipt.block_number.unwrap_or(last_block);
        tracing::info!(
            contract = %step.name,
            address = %format_addr(address),
            block = last_block,
            "deployed"
        );
        addresses.insert(step.name.clone(), format_addr(address));
    }

    Ok(DeployedSuite {
        addresses,
        last_block,
    })
}

fn wallet_provider(rpc_url: &str, entry: &WalletEntry) -> anyhow::Result<Arc<DynProvider>> {
    let url = reqwest::Url::parse(rpc_url)?;
    let trimmed = entry.private_key.trim_start_matches("0x");
    let signer: PrivateKeySigner = trimmed
        .parse()
        .map_err(|e| anyhow::anyhow!("parse {}.private_key: {e}", entry.name))?;
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .connect_http(url)
        .erased();
    Ok(Arc::new(provider))
}

fn parse_signer_address(entry: &WalletEntry) -> anyhow::Result<Address> {
    let trimmed = entry.private_key.trim_start_matches("0x");
    let signer: PrivateKeySigner = trimmed
        .parse()
        .map_err(|e| anyhow::anyhow!("derive address: {e}"))?;
    Ok(signer.address())
}

fn format_addr(a: Address) -> String {
    format!("{a:#x}")
}

/// init_code = contract_bytecode ++ abi.encode(constructor_args).
fn encode_init_code(
    artifact: &ContractArtifact,
    step: &ContractDeploy,
    known_addresses: &HashMap<String, String>,
    deployer: Address,
) -> anyhow::Result<Bytes> {
    let input_types = artifact.constructor_inputs();
    if input_types.len() != step.args.len() {
        return Err(anyhow::anyhow!(
            "{}: constructor expects {} args, scenario supplies {}",
            step.name,
            input_types.len(),
            step.args.len()
        ));
    }
    let mut values = Vec::with_capacity(step.args.len());
    for (ty, arg) in input_types.iter().zip(step.args.iter()) {
        values.push(coerce_arg(ty, arg, known_addresses, deployer)?);
    }
    let tuple = DynSolValue::Tuple(values);
    let encoded = tuple.abi_encode_params();
    let mut out = artifact.init_code.to_vec();
    out.extend_from_slice(&encoded);
    Ok(Bytes::from(out))
}

fn coerce_arg(
    sol_type: &str,
    arg: &ConstructorArg,
    known: &HashMap<String, String>,
    deployer: Address,
) -> anyhow::Result<DynSolValue> {
    let (declared, raw) = match arg {
        ConstructorArg::Typed(TypedArg { ty, value }) => (Some(ty.as_str()), value.clone()),
        ConstructorArg::Literal(v) => (None, v.clone()),
    };
    let effective_type = declared.unwrap_or(sol_type);
    coerce_toml(effective_type, &raw, known, deployer)
}

/// Public wrapper for fixture use (uses zero deployer — `$deployer` refs not supported here).
///
/// # Errors
///
/// Returns an error if the Solidity type is unsupported, the TOML value is of
/// the wrong shape, a reference cannot be resolved, or a value cannot be
/// parsed into the requested ABI type.
pub fn coerce_toml_public(
    sol_type: &str,
    value: &toml::Value,
    known: &HashMap<String, String>,
) -> anyhow::Result<DynSolValue> {
    coerce_toml(sol_type, value, known, Address::ZERO)
}

fn coerce_toml(
    sol_type: &str,
    value: &toml::Value,
    known: &HashMap<String, String>,
    deployer: Address,
) -> anyhow::Result<DynSolValue> {
    // Resolve `$` refs if value is a string.
    let resolved = if let toml::Value::String(s) = value {
        resolve_ref(s, known, deployer)?
    } else {
        value.clone()
    };
    match sol_type {
        t if t.starts_with("uint") || t.starts_with("int") => {
            let n = match &resolved {
                toml::Value::Integer(i) => U256::from(*i as i128 as u128),
                toml::Value::String(s) => parse_u256(s)?,
                _ => return Err(anyhow::anyhow!("{sol_type}: expected integer")),
            };
            Ok(DynSolValue::Uint(n, type_bits(t).unwrap_or(256)))
        }
        "address" => {
            let s = resolved
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("address: expected string"))?;
            let addr: Address = s
                .parse()
                .map_err(|e| anyhow::anyhow!("parse address {s}: {e}"))?;
            Ok(DynSolValue::Address(addr))
        }
        "string" => {
            let s = resolved
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("string: expected string"))?;
            Ok(DynSolValue::String(s.to_string()))
        }
        "bool" => {
            let b = resolved
                .as_bool()
                .ok_or_else(|| anyhow::anyhow!("bool: expected bool"))?;
            Ok(DynSolValue::Bool(b))
        }
        t if t.starts_with("bytes") && t != "bytes" => {
            // Fixed-size bytesN.
            let s = resolved
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{t}: expected 0x-hex string"))?;
            let bytes = hex::decode(s.trim_start_matches("0x"))
                .map_err(|e| anyhow::anyhow!("decode {t}: {e}"))?;
            let n = t.trim_start_matches("bytes").parse::<usize>().unwrap_or(32);
            let mut padded = vec![0u8; n];
            let len = bytes.len().min(n);
            padded[..len].copy_from_slice(&bytes[..len]);
            Ok(DynSolValue::FixedBytes(
                alloy::primitives::FixedBytes::from_slice(&padded),
                n,
            ))
        }
        "bytes" => {
            let s = resolved
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("bytes: expected 0x-hex string"))?;
            let bytes = hex::decode(s.trim_start_matches("0x"))
                .map_err(|e| anyhow::anyhow!("decode bytes: {e}"))?;
            Ok(DynSolValue::Bytes(bytes))
        }
        other => Err(anyhow::anyhow!("unsupported abi type: {other}")),
    }
}

fn type_bits(t: &str) -> Option<usize> {
    let stripped = t.trim_start_matches("uint").trim_start_matches("int");
    if stripped.is_empty() {
        return Some(256);
    }
    stripped.parse::<usize>().ok()
}

fn parse_u256(s: &str) -> anyhow::Result<U256> {
    if let Some(hex) = s.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).map_err(|e| anyhow::anyhow!("invalid u256 hex: {e}"))
    } else {
        U256::from_str_radix(s, 10).map_err(|e| anyhow::anyhow!("invalid u256 decimal: {e}"))
    }
}

fn resolve_ref(
    value: &str,
    known: &HashMap<String, String>,
    deployer: Address,
) -> anyhow::Result<toml::Value> {
    if value == "$deployer" {
        return Ok(toml::Value::String(format!("{deployer:#x}")));
    }
    if let Some(name) = value
        .strip_prefix("$contract(")
        .and_then(|s| s.strip_suffix(")"))
    {
        let addr = known
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown contract ref: {name}"))?;
        return Ok(toml::Value::String(addr.clone()));
    }
    Ok(toml::Value::String(value.to_string()))
}

impl LoadedManifest {
    /// Build a `DeployCtx` from the manifest + env overrides.
    /// Build a deployment context from the loaded manifest.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest is missing required defaults, the RPC
    /// endpoint cannot be determined, or the contracts directory is invalid.
    pub fn build_deploy_ctx(&self, override_rpc: Option<String>) -> anyhow::Result<DeployCtx> {
        let defaults = &self.manifest.defaults;
        let rpc_url = override_rpc
            .or_else(|| std::env::var("ROKO_MIRAGE_URL").ok())
            .or_else(|| defaults.rpc_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:8545".to_string());
        let chain_id = defaults.chain_id.unwrap_or(31337);
        let contracts_rel = defaults
            .contracts_dir
            .clone()
            .unwrap_or_else(|| "../contracts".into());
        let contracts_dir = self.demo_dir.join(contracts_rel);
        let contracts_dir = contracts_dir.canonicalize().unwrap_or(contracts_dir);
        let wallets = self.load_wallets()?;
        Ok(DeployCtx {
            rpc_url,
            chain_id,
            contracts_dir,
            wallets,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_artifact() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("contracts");
        if !root.join("out/MockERC20.sol/MockERC20.json").exists() {
            eprintln!("skip: forge artifacts missing");
            return;
        }
        let art = ContractArtifact::load(&root, "MockERC20").unwrap();
        assert_eq!(art.name, "MockERC20");
        let inputs = art.constructor_inputs();
        assert_eq!(inputs, vec!["string", "string", "uint8"]);
    }

    #[test]
    fn type_bits_parsing() {
        assert_eq!(type_bits("uint256"), Some(256));
        assert_eq!(type_bits("uint8"), Some(8));
        assert_eq!(type_bits("int128"), Some(128));
        assert_eq!(type_bits("uint"), Some(256));
    }
}
