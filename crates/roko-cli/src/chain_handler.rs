//! [`ToolHandler`] implementation that dispatches `chain.*` tool calls to
//! a live [`ChainClient`] / [`ChainWallet`].
//!
//! Each `ChainToolHandler` instance handles exactly one tool name (e.g.
//! `chain.balance`). The [`chain_registry`](crate::chain_registry) module
//! constructs a full map of these handlers from the 14 canonical chain tools.

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use roko_chain::{ChainClient, ChainWallet, TxRequest};
use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolHandler, ToolResult};

/// A single chain tool backed by a live client and optional wallet.
///
/// Construct one per tool name via [`crate::chain_registry::chain_handler_map`].
pub struct ChainToolHandler {
    /// Read-only chain backend.
    pub client: Arc<dyn ChainClient>,
    /// Signing wallet (required for `chain.transfer` and friends).
    pub wallet: Option<Arc<dyn ChainWallet>>,
    /// Canonical tool name this handler serves (e.g. `"chain.balance"`).
    pub tool_name: String,
}

#[async_trait]
impl ToolHandler for ChainToolHandler {
    fn name(&self) -> &str {
        &self.tool_name
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let args = &call.arguments;
        match self.tool_name.as_str() {
            "chain.balance" => self.handle_balance(args).await,
            "chain.transfer" => self.handle_transfer(args).await,
            "chain.simulate_tx" => self.handle_simulate_tx(args).await,
            "chain.gas_estimate" => self.handle_gas_estimate(args).await,
            "chain.wallet_info" => self.handle_wallet_info(args).await,
            "chain.wallet_list" => self.handle_wallet_list(args).await,
            other => ToolResult::err(ToolError::Other(format!(
                "chain tool not yet implemented: {other}"
            ))),
        }
    }
}

impl ChainToolHandler {
    // ── chain.balance ────────────────────────────────────────────────────

    async fn handle_balance(&self, args: &serde_json::Value) -> ToolResult {
        let address = match args.get("address").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: address".into(),
                ));
            }
        };

        let block = args.get("block").and_then(|v| v.as_u64());

        match self.client.get_balance(address, block).await {
            Ok(balance) => {
                let body = json!({ "balance_wei": balance.to_string() });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
        }
    }

    // ── chain.transfer ──────────────────────────────────────────────────

    async fn handle_transfer(&self, args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.transfer".into(),
                ));
            }
        };

        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: to".into(),
                ));
            }
        };

        let amount_str = match args.get("amount").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: amount".into(),
                ));
            }
        };

        let value: u128 = match amount_str.parse() {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(ToolError::SchemaInvalid(format!("invalid amount: {e}")));
            }
        };

        let tx = TxRequest {
            to: Some(to),
            value,
            ..TxRequest::default()
        };

        match wallet.sign_and_submit(tx).await {
            Ok(tx_hash) => {
                let body = json!({ "tx_hash": tx_hash.as_str() });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
        }
    }

    // ── chain.simulate_tx ───────────────────────────────────────────────

    async fn handle_simulate_tx(&self, args: &serde_json::Value) -> ToolResult {
        let to = args.get("to").and_then(|v| v.as_str()).map(String::from);
        let from = args.get("from").and_then(|v| v.as_str()).map(String::from);
        let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("0x");
        let value_str = args.get("value").and_then(|v| v.as_str()).unwrap_or("0");
        let block = args.get("block").and_then(|v| v.as_u64());

        let value: u128 = value_str.parse().unwrap_or(0);
        let data_bytes = hex_decode(data);

        let tx = TxRequest {
            to,
            from,
            value,
            data: data_bytes,
            ..TxRequest::default()
        };

        match self.client.eth_call(&tx, block).await {
            Ok(result) => {
                let output_hex = bytes_to_hex(&result.output);
                let body = json!({
                    "output": output_hex,
                    "gas_used": result.gas_used,
                });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
        }
    }

    // ── chain.gas_estimate ──────────────────────────────────────────────

    async fn handle_gas_estimate(&self, args: &serde_json::Value) -> ToolResult {
        let to = args.get("to").and_then(|v| v.as_str()).map(String::from);
        let from = args.get("from").and_then(|v| v.as_str()).map(String::from);
        let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("0x");
        let value_str = args.get("value").and_then(|v| v.as_str()).unwrap_or("0");

        let value: u128 = value_str.parse().unwrap_or(0);
        let data_bytes = hex_decode(data);

        let tx = TxRequest {
            to,
            from,
            value,
            data: data_bytes,
            ..TxRequest::default()
        };

        match self.client.eth_call(&tx, None).await {
            Ok(result) => {
                // Apply 1.2x safety buffer to gas estimate.
                let buffered = (result.gas_used as f64 * 1.2) as u64;
                let body = json!({
                    "gas_estimate": buffered,
                    "gas_used_raw": result.gas_used,
                });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
        }
    }

    // ── chain.wallet_info ───────────────────────────────────────────────

    async fn handle_wallet_info(&self, _args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.wallet_info".into(),
                ));
            }
        };

        let address = match wallet.address().await {
            Ok(a) => a,
            Err(e) => return ToolResult::err(ToolError::Other(e.to_string())),
        };

        let balance = match wallet.balance(None).await {
            Ok(b) => b,
            Err(e) => return ToolResult::err(ToolError::Other(e.to_string())),
        };

        let nonce = match wallet.nonce().await {
            Ok(n) => n,
            Err(e) => return ToolResult::err(ToolError::Other(e.to_string())),
        };

        let body = json!({
            "address": address,
            "balance_wei": balance.to_string(),
            "nonce": nonce,
        });
        ToolResult::structured(body.to_string())
    }

    // ── chain.wallet_list ───────────────────────────────────────────────

    async fn handle_wallet_list(&self, _args: &serde_json::Value) -> ToolResult {
        match &self.wallet {
            Some(w) => match w.address().await {
                Ok(addr) => {
                    let body = json!({ "wallets": [addr] });
                    ToolResult::structured(body.to_string())
                }
                Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
            },
            None => {
                let empty: Vec<String> = Vec::new();
                let body = json!({ "wallets": empty });
                ToolResult::structured(body.to_string())
            }
        }
    }
}

// ── Hex helpers (no external `hex` crate needed) ─────────────────────────

/// Encode a byte slice as a `0x`-prefixed lowercase hex string.
fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decode a `0x`-prefixed hex string to bytes.
/// Returns an empty vec on invalid input (the chain backend will surface a
/// clearer error).
fn hex_decode(s: &str) -> Vec<u8> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.is_empty() {
        return Vec::new();
    }
    // Process two hex chars at a time.
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut chars = s.chars();
    while let Some(hi) = chars.next() {
        let lo = match chars.next() {
            Some(c) => c,
            None => break, // odd-length: discard trailing nibble
        };
        let byte = match (hi.to_digit(16), lo.to_digit(16)) {
            (Some(h), Some(l)) => (h as u8) << 4 | l as u8,
            _ => return Vec::new(), // invalid hex
        };
        out.push(byte);
    }
    out
}
