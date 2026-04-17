//! Fixture runner — applies post-deploy chain state changes declared in the scenario.
//!
//! Three kinds:
//! - `forge-script`: shell out to `forge script <path> --broadcast`.
//! - `jsonrpc`: POST one or more JSON-RPC calls to the chain (custom mirage
//!   precompiles live here, e.g. `chain_postInsight`).
//! - `rust`: dispatch to a scenario-registered Rust handler; used when fixture
//!   logic needs typed contract calls via alloy bindings.

use std::collections::HashMap;
use std::path::Path;

use alloy::dyn_abi::DynSolValue;
use alloy::network::TransactionBuilder;
use alloy::primitives::{Bytes, keccak256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::TransactionRequest;
use async_trait::async_trait;
use serde::Deserialize;

use crate::chain_ctx::ChainCtx;
use crate::manifest::{FixtureKind, FixtureStep};

/// Rust fixture handler signature.
#[async_trait]
pub trait RustFixture: Send + Sync {
    /// Apply this fixture against the chain.
    async fn apply(&self, ctx: &ChainCtx, args: toml::Value) -> anyhow::Result<()>;
}

/// Registry of named Rust fixture handlers (populated by scenario modules).
#[derive(Default)]
pub struct FixtureRegistry {
    handlers: HashMap<String, Box<dyn RustFixture>>,
}

impl FixtureRegistry {
    /// New empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a Rust fixture under `name`.
    pub fn register(&mut self, name: impl Into<String>, handler: Box<dyn RustFixture>) {
        self.handlers.insert(name.into(), handler);
    }

    /// Look up a handler.
    pub fn get(&self, name: &str) -> Option<&dyn RustFixture> {
        self.handlers.get(name).map(|b| b.as_ref())
    }
}

/// Forge-script fixture config.
#[derive(Clone, Debug, Deserialize)]
struct ForgeScriptCfg {
    forge_script: String,
    #[serde(default)]
    sender_wallet: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

/// Simple single-call JSON-RPC fixture.
#[derive(Clone, Debug, Deserialize)]
struct JsonrpcCfg {
    method: String,
    #[serde(default)]
    params: Vec<serde_json::Value>,
    #[serde(default)]
    iterations: Option<u32>,
}

/// Rust handler fixture config.
#[derive(Clone, Debug, Deserialize)]
struct RustCfg {
    handler: String,
    #[serde(flatten, default)]
    rest: HashMap<String, toml::Value>,
}

/// Execute all fixtures in sequence.
///
/// # Errors
///
/// Returns an error if any fixture step fails, including configuration
/// decoding, wallet lookup, spawned processes, or RPC calls.
pub async fn run_fixtures(
    ctx: &ChainCtx,
    registry: &FixtureRegistry,
    steps: &[FixtureStep],
    contracts_dir: &Path,
) -> anyhow::Result<()> {
    for step in steps {
        tracing::info!(fixture = %step.name, kind = ?step.kind, "running fixture");
        match step.kind {
            FixtureKind::ForgeScript => run_forge_script(ctx, step, contracts_dir).await?,
            FixtureKind::Jsonrpc => run_jsonrpc(ctx, step).await?,
            FixtureKind::Rust => run_rust(ctx, registry, step).await?,
            FixtureKind::ContractCall => run_contract_call(ctx, step).await?,
        }
    }
    Ok(())
}

async fn run_forge_script(
    ctx: &ChainCtx,
    step: &FixtureStep,
    contracts_dir: &Path,
) -> anyhow::Result<()> {
    let cfg: ForgeScriptCfg = step
        .config
        .clone()
        .try_into()
        .map_err(|e| anyhow::anyhow!("forge-script config: {e}"))?;
    let wallet_name = cfg
        .sender_wallet
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("forge-script fixture missing sender_wallet"))?;
    let wallet = ctx
        .wallet_key(wallet_name)
        .ok_or_else(|| anyhow::anyhow!("unknown wallet: {wallet_name}"))?;
    let mut cmd = tokio::process::Command::new("forge");
    cmd.arg("script")
        .arg(&cfg.forge_script)
        .arg("--rpc-url")
        .arg(&ctx.rpc_url)
        .arg("--private-key")
        .arg(&wallet)
        .arg("--broadcast")
        .arg("--slow")
        .current_dir(contracts_dir);
    for (k, v) in &cfg.env {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .await
        .map_err(|e| anyhow::anyhow!("spawn forge script: {e}"))?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "forge script {} failed: {status}",
            cfg.forge_script
        ));
    }
    Ok(())
}

async fn run_jsonrpc(ctx: &ChainCtx, step: &FixtureStep) -> anyhow::Result<()> {
    let cfg: JsonrpcCfg = step
        .config
        .clone()
        .try_into()
        .map_err(|e| anyhow::anyhow!("jsonrpc config: {e}"))?;
    let iterations = cfg.iterations.unwrap_or(1);
    let client = reqwest::Client::new();
    for i in 0..iterations {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": cfg.method,
            "params": cfg.params,
            "id": i + 1,
        });
        let resp = client
            .post(&ctx.rpc_url)
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("jsonrpc post: {e}"))?
            .error_for_status()
            .map_err(|e| anyhow::anyhow!("jsonrpc status: {e}"))?;
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("jsonrpc parse: {e}"))?;
        if let Some(err) = body.get("error") {
            return Err(anyhow::anyhow!("jsonrpc {}: {err}", cfg.method));
        }
    }
    Ok(())
}

/// ABI-encoded contract call fixture.
#[derive(Clone, Debug, Deserialize)]
struct ContractCallCfg {
    /// Deployed contract name (must be in deployments.json).
    contract: String,
    /// Solidity function signature, e.g. "setAuthorized(address,bool)".
    method: String,
    /// Args in manifest form (typed or literal).
    #[serde(default)]
    args: Vec<toml::Value>,
    /// Wallet name that signs the call.
    from: String,
}

async fn run_contract_call(ctx: &ChainCtx, step: &FixtureStep) -> anyhow::Result<()> {
    let cfg: ContractCallCfg = step
        .config
        .clone()
        .try_into()
        .map_err(|e| anyhow::anyhow!("contract-call config: {e}"))?;
    let contract_addr = ctx.address_of(&cfg.contract)?;
    let provider = ctx.wallet_provider(&cfg.from)?;
    let from = ctx.wallet_address(&cfg.from)?;
    let calldata = encode_call(&cfg.method, &cfg.args, ctx)?;

    let tx = TransactionRequest::default()
        .with_from(from)
        .with_to(contract_addr)
        .with_input(calldata)
        .with_chain_id(ctx.chain_id);
    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|e| anyhow::anyhow!("submit {}.{}: {e}", cfg.contract, cfg.method))?;
    let receipt = pending
        .get_receipt()
        .await
        .map_err(|e| anyhow::anyhow!("receipt {}.{}: {e}", cfg.contract, cfg.method))?;
    if !receipt.status() {
        return Err(anyhow::anyhow!(
            "{}.{} reverted (tx={})",
            cfg.contract,
            cfg.method,
            receipt.transaction_hash
        ));
    }
    tracing::info!(
        contract = %cfg.contract,
        method = %cfg.method,
        "contract-call ok"
    );
    Ok(())
}

/// Parse a solidity function signature into (name, arg_types).
fn parse_signature(sig: &str) -> anyhow::Result<(&str, Vec<&str>)> {
    let open = sig
        .find('(')
        .ok_or_else(|| anyhow::anyhow!("bad signature: {sig}"))?;
    let close = sig
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("bad signature: {sig}"))?;
    let name = &sig[..open];
    let args = &sig[open + 1..close];
    let types = if args.is_empty() {
        Vec::new()
    } else {
        args.split(',').map(|s| s.trim()).collect()
    };
    Ok((name, types))
}

fn encode_call(
    method_sig: &str,
    raw_args: &[toml::Value],
    ctx: &ChainCtx,
) -> anyhow::Result<Bytes> {
    let (_name, types) = parse_signature(method_sig)?;
    if types.len() != raw_args.len() {
        return Err(anyhow::anyhow!(
            "{method_sig}: expected {} args, got {}",
            types.len(),
            raw_args.len()
        ));
    }
    let selector = keccak256(method_sig.as_bytes());
    let mut out = Vec::with_capacity(4 + 32 * types.len());
    out.extend_from_slice(&selector[..4]);
    let mut values = Vec::with_capacity(types.len());
    for (ty, raw) in types.iter().zip(raw_args.iter()) {
        values.push(crate::deploy::coerce_toml_public(ty, raw, &ctx.addresses)?);
    }
    let tuple = DynSolValue::Tuple(values);
    out.extend_from_slice(&tuple.abi_encode_params());
    Ok(Bytes::from(out))
}

async fn run_rust(
    ctx: &ChainCtx,
    registry: &FixtureRegistry,
    step: &FixtureStep,
) -> anyhow::Result<()> {
    let cfg: RustCfg = step
        .config
        .clone()
        .try_into()
        .map_err(|e| anyhow::anyhow!("rust fixture config: {e}"))?;
    let handler = registry
        .get(&cfg.handler)
        .ok_or_else(|| anyhow::anyhow!("no registered rust fixture: {}", cfg.handler))?;
    let args: toml::Value = toml::Value::Table(cfg.rest.into_iter().collect());
    handler.apply(ctx, args).await
}
