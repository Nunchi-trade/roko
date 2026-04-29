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
    /// Mirage JSON-RPC URL for knowledge graph calls (chain.post_insight, etc.).
    /// Defaults to `http://127.0.0.1:8545` if not set.
    pub rpc_url: Option<String>,
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
            "chain.approve" => self.handle_approve(args).await,
            "chain.swap" => self.handle_swap(args).await,
            "chain.add_liquidity" => self.handle_add_liquidity(args).await,
            "chain.remove_liquidity" => self.handle_remove_liquidity(args).await,
            "chain.get_pool_info" => self.handle_get_pool_info(args).await,
            "chain.get_position" => self.handle_get_position(args).await,
            "chain.wallet_create" => self.handle_wallet_create(args).await,
            "chain.wallet_export_address" => self.handle_wallet_export_address(args).await,
            "chain.post_insight" => self.handle_post_insight(args).await,
            "chain.search_insights" => self.handle_search_insights(args).await,
            "chain.confirm_insight" => self.handle_confirm_insight(args).await,
            other => ToolResult::err(ToolError::Other(format!("unknown chain tool: {other}"))),
        }
    }
}

/// ERC-20 function selectors (first 4 bytes of keccak256 of the signature).
///
/// These are standard Solidity ABI selectors computed from the canonical
/// function signatures. Using raw selectors avoids pulling in a full ABI
/// encoder dependency.
mod abi {
    /// `approve(address,uint256)` selector: `0x095ea7b3`
    pub const APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

    /// `balanceOf(address)` selector: `0x70a08231`
    #[allow(dead_code)]
    pub const BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];

    /// Uniswap V3 SwapRouter `exactInputSingle((address,address,uint24,address,uint256,uint256,uint256,uint160))`
    /// selector: `0x414bf389`
    pub const EXACT_INPUT_SINGLE: [u8; 4] = [0x41, 0x4b, 0xf3, 0x89];

    /// Uniswap V3 NonfungiblePositionManager `mint((address,address,uint24,int24,int24,uint256,uint256,uint256,uint256,address,uint256))`
    /// selector: `0x88316456`
    pub const MINT: [u8; 4] = [0x88, 0x31, 0x64, 0x56];

    /// Uniswap V3 NonfungiblePositionManager `decreaseLiquidity((uint256,uint128,uint256,uint256,uint256))`
    /// selector: `0x0c49ccbe`
    pub const DECREASE_LIQUIDITY: [u8; 4] = [0x0c, 0x49, 0xcc, 0xbe];

    /// Uniswap V3 NonfungiblePositionManager `positions(uint256)`
    /// selector: `0x99fbab88`
    #[allow(dead_code)]
    pub const POSITIONS: [u8; 4] = [0x99, 0xfb, 0xab, 0x88];

    /// Uniswap V3 Pool `slot0()`
    /// selector: `0x3850c7bd`
    pub const SLOT0: [u8; 4] = [0x38, 0x50, 0xc7, 0xbd];

    /// Uniswap V3 Pool `liquidity()`
    /// selector: `0x1a686502`
    pub const LIQUIDITY: [u8; 4] = [0x1a, 0x68, 0x65, 0x02];

    /// uint256 max (2^256 - 1) encoded as a 32-byte big-endian word.
    pub const U256_MAX: [u8; 32] = [0xff; 32];

    /// Encode an address (hex `0x`-prefixed) into a 32-byte ABI word.
    ///
    /// ABI addresses are left-padded to 32 bytes.
    pub fn encode_address(hex_addr: &str) -> [u8; 32] {
        let mut word = [0u8; 32];
        let bytes = super::hex_decode(hex_addr);
        if bytes.len() <= 20 {
            let start = 32 - bytes.len();
            word[start..32].copy_from_slice(&bytes);
        }
        word
    }

    /// Encode a u128 as a 32-byte big-endian ABI word.
    pub fn encode_u128(val: u128) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[16..32].copy_from_slice(&val.to_be_bytes());
        word
    }

    /// Encode a u64 as a 32-byte big-endian ABI word.
    pub fn encode_u64(val: u64) -> [u8; 32] {
        let mut word = [0u8; 32];
        word[24..32].copy_from_slice(&val.to_be_bytes());
        word
    }

    /// Encode an i32 (tick) as a 32-byte ABI int24 word (sign-extended).
    pub fn encode_i32(val: i32) -> [u8; 32] {
        let mut word = if val < 0 { [0xff; 32] } else { [0u8; 32] };
        let bytes = val.to_be_bytes();
        word[28..32].copy_from_slice(&bytes);
        word
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

    // ── chain.approve ───────────────────────���────────────────────────────

    async fn handle_approve(&self, args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.approve".into(),
                ));
            }
        };

        let token = match args.get("token").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token".into(),
                ));
            }
        };

        let spender = match args.get("spender").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: spender".into(),
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

        // Build ERC-20 approve(address,uint256) calldata.
        let mut data = Vec::with_capacity(4 + 64);
        data.extend_from_slice(&abi::APPROVE);
        data.extend_from_slice(&abi::encode_address(spender));
        if amount_str == "max" {
            data.extend_from_slice(&abi::U256_MAX);
        } else {
            let value: u128 = match amount_str.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid amount: {e}"
                    )));
                }
            };
            data.extend_from_slice(&abi::encode_u128(value));
        }

        let tx = TxRequest {
            to: Some(token),
            data,
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

    // ── chain.swap ──────────────��────────────────────────────────────────

    async fn handle_swap(&self, args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.swap".into(),
                ));
            }
        };

        let token_in = match args.get("token_in").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_in".into(),
                ));
            }
        };

        let token_out = match args.get("token_out").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_out".into(),
                ));
            }
        };

        let amount_in: u128 = match args.get("amount_in").and_then(|v| v.as_str()) {
            Some(a) => match a.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid amount_in: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: amount_in".into(),
                ));
            }
        };

        let amount_out_min: u128 = match args.get("amount_out_min").and_then(|v| v.as_str()) {
            Some(a) => match a.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid amount_out_min: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: amount_out_min".into(),
                ));
            }
        };

        let fee: u64 = args.get("fee").and_then(|v| v.as_u64()).unwrap_or(3000);

        let deadline: u64 = args
            .get("deadline")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() + 1200) // now + 20 minutes
                    .unwrap_or(u64::MAX)
            });

        let recipient = match args.get("recipient").and_then(|v| v.as_str()) {
            Some(r) => r.to_string(),
            None => match wallet.address().await {
                Ok(a) => a,
                Err(e) => return ToolResult::err(ToolError::Other(e.to_string())),
            },
        };

        // Build Uniswap V3 exactInputSingle calldata.
        let mut data = Vec::with_capacity(4 + 32 * 8);
        data.extend_from_slice(&abi::EXACT_INPUT_SINGLE);
        data.extend_from_slice(&abi::encode_address(token_in));
        data.extend_from_slice(&abi::encode_address(token_out));
        data.extend_from_slice(&abi::encode_u64(fee)); // fee tier
        data.extend_from_slice(&abi::encode_address(&recipient));
        data.extend_from_slice(&abi::encode_u64(deadline));
        data.extend_from_slice(&abi::encode_u128(amount_in));
        data.extend_from_slice(&abi::encode_u128(amount_out_min));
        data.extend_from_slice(&[0u8; 32]); // sqrtPriceLimitX96 = 0 (no limit)

        // The router contract address must be configured; use a well-known
        // Uniswap V3 SwapRouter address as the default.
        let router = args
            .get("router")
            .and_then(|v| v.as_str())
            .unwrap_or("0xE592427A0AEce92De3Edee1F18E0157C05861564")
            .to_string();

        let tx = TxRequest {
            to: Some(router),
            data,
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

    // ── chain.add_liquidity ─────────���────────────────────────────────────

    async fn handle_add_liquidity(&self, args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.add_liquidity".into(),
                ));
            }
        };

        let token_a = match args.get("token_a").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_a".into(),
                ));
            }
        };

        let token_b = match args.get("token_b").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_b".into(),
                ));
            }
        };

        let amount_a: u128 = match args.get("amount_a").and_then(|v| v.as_str()) {
            Some(a) => match a.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid amount_a: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: amount_a".into(),
                ));
            }
        };

        let amount_b: u128 = match args.get("amount_b").and_then(|v| v.as_str()) {
            Some(a) => match a.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid amount_b: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: amount_b".into(),
                ));
            }
        };

        let fee: u64 = args.get("fee").and_then(|v| v.as_u64()).unwrap_or(3000);
        let tick_lower: i32 = args
            .get("tick_lower")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or(-887220);
        let tick_upper: i32 = args
            .get("tick_upper")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .unwrap_or(887220);

        let deadline: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() + 1200)
            .unwrap_or(u64::MAX);

        let recipient = match wallet.address().await {
            Ok(a) => a,
            Err(e) => return ToolResult::err(ToolError::Other(e.to_string())),
        };

        // Build Uniswap V3 NonfungiblePositionManager mint() calldata.
        let mut data = Vec::with_capacity(4 + 32 * 11);
        data.extend_from_slice(&abi::MINT);
        data.extend_from_slice(&abi::encode_address(token_a));
        data.extend_from_slice(&abi::encode_address(token_b));
        data.extend_from_slice(&abi::encode_u64(fee));
        data.extend_from_slice(&abi::encode_i32(tick_lower));
        data.extend_from_slice(&abi::encode_i32(tick_upper));
        data.extend_from_slice(&abi::encode_u128(amount_a)); // amount0Desired
        data.extend_from_slice(&abi::encode_u128(amount_b)); // amount1Desired
        data.extend_from_slice(&[0u8; 32]); // amount0Min = 0
        data.extend_from_slice(&[0u8; 32]); // amount1Min = 0
        data.extend_from_slice(&abi::encode_address(&recipient));
        data.extend_from_slice(&abi::encode_u64(deadline));

        // Default NonfungiblePositionManager address (Uniswap V3).
        let position_manager = args
            .get("position_manager")
            .and_then(|v| v.as_str())
            .unwrap_or("0xC36442b4a4522E871399CD717aBDD847Ab11FE88")
            .to_string();

        let tx = TxRequest {
            to: Some(position_manager),
            data,
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

    // ── chain.remove_liquidity ─────────────────��─────────────────────────

    async fn handle_remove_liquidity(&self, args: &serde_json::Value) -> ToolResult {
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.remove_liquidity".into(),
                ));
            }
        };

        let liquidity: u128 = match args.get("liquidity").and_then(|v| v.as_str()) {
            Some(l) => match l.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid liquidity: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: liquidity".into(),
                ));
            }
        };

        let token_id: u128 = match args.get("token_id").and_then(|v| v.as_str()) {
            Some(t) => match t.parse() {
                Ok(v) => v,
                Err(e) => {
                    return ToolResult::err(ToolError::SchemaInvalid(format!(
                        "invalid token_id: {e}"
                    )));
                }
            },
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_id (V3 NFT position ID)".into(),
                ));
            }
        };

        let amount_a_min: u128 = args
            .get("amount_a_min")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let amount_b_min: u128 = args
            .get("amount_b_min")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let deadline: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() + 1200)
            .unwrap_or(u64::MAX);

        // Build Uniswap V3 decreaseLiquidity calldata.
        let mut data = Vec::with_capacity(4 + 32 * 5);
        data.extend_from_slice(&abi::DECREASE_LIQUIDITY);
        data.extend_from_slice(&abi::encode_u128(token_id));
        data.extend_from_slice(&abi::encode_u128(liquidity));
        data.extend_from_slice(&abi::encode_u128(amount_a_min));
        data.extend_from_slice(&abi::encode_u128(amount_b_min));
        data.extend_from_slice(&abi::encode_u64(deadline));

        let position_manager = args
            .get("position_manager")
            .and_then(|v| v.as_str())
            .unwrap_or("0xC36442b4a4522E871399CD717aBDD847Ab11FE88")
            .to_string();

        let tx = TxRequest {
            to: Some(position_manager),
            data,
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

    // ── chain.get_pool_info ────────��─────────────────────────────────────

    async fn handle_get_pool_info(&self, args: &serde_json::Value) -> ToolResult {
        let pool_address = match args.get("pool").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: pool (pool contract address)".into(),
                ));
            }
        };

        // Query slot0() for current tick and sqrtPriceX96.
        let slot0_data = abi::SLOT0.to_vec();
        let slot0_tx = TxRequest {
            to: Some(pool_address.clone()),
            data: slot0_data,
            ..TxRequest::default()
        };

        let slot0_result = match self.client.eth_call(&slot0_tx, None).await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(ToolError::Other(format!("slot0 query failed: {e}"))),
        };

        // Query liquidity().
        let liquidity_data = abi::LIQUIDITY.to_vec();
        let liquidity_tx = TxRequest {
            to: Some(pool_address.clone()),
            data: liquidity_data,
            ..TxRequest::default()
        };

        let liquidity_result = match self.client.eth_call(&liquidity_tx, None).await {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::err(ToolError::Other(format!("liquidity query failed: {e}")));
            }
        };

        let slot0_hex = bytes_to_hex(&slot0_result.output);
        let liquidity_hex = bytes_to_hex(&liquidity_result.output);

        let body = json!({
            "pool": pool_address,
            "slot0_raw": slot0_hex,
            "liquidity_raw": liquidity_hex,
            "note": "slot0 contains: sqrtPriceX96 (bytes 0-32), tick (bytes 32-64), observationIndex, observationCardinality, observationCardinalityNext, feeProtocol, unlocked"
        });
        ToolResult::structured(body.to_string())
    }

    // ── chain.get_position ───────────────────────────────────────��───────

    async fn handle_get_position(&self, args: &serde_json::Value) -> ToolResult {
        let token_id = match args.get("token_id").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: token_id".into(),
                ));
            }
        };

        let token_id_u128: u128 = match token_id.parse() {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::err(ToolError::SchemaInvalid(format!("invalid token_id: {e}")));
            }
        };

        let position_manager = args
            .get("position_manager")
            .and_then(|v| v.as_str())
            .unwrap_or("0xC36442b4a4522E871399CD717aBDD847Ab11FE88");

        // Build positions(uint256) calldata.
        let mut data = Vec::with_capacity(4 + 32);
        data.extend_from_slice(&[0x99, 0xfb, 0xab, 0x88]); // positions selector
        data.extend_from_slice(&abi::encode_u128(token_id_u128));

        let tx = TxRequest {
            to: Some(position_manager.to_string()),
            data,
            ..TxRequest::default()
        };

        match self.client.eth_call(&tx, None).await {
            Ok(result) => {
                let output_hex = bytes_to_hex(&result.output);
                let body = json!({
                    "token_id": token_id,
                    "position_manager": position_manager,
                    "position_raw": output_hex,
                    "note": "positions() returns: nonce, operator, token0, token1, fee, tickLower, tickUpper, liquidity, feeGrowthInside0LastX128, feeGrowthInside1LastX128, tokensOwed0, tokensOwed1"
                });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(format!("position query failed: {e}"))),
        }
    }

    // ── chain.wallet_create ─────────────��────────────────────────────────

    async fn handle_wallet_create(&self, _args: &serde_json::Value) -> ToolResult {
        // Wallet creation requires a key management system (KMS, local keystore,
        // or hardware wallet) that is not yet wired into the ChainWallet trait.
        // The trait provides signing for a single pre-configured key pair but has
        // no API for generating new key material at runtime.
        ToolResult::err(ToolError::Other(
            "chain.wallet_create requires a key management backend (KMS or local keystore) \
             that is not yet integrated into the ChainWallet trait. The current trait \
             supports signing with a pre-configured key pair but cannot generate new \
             wallets at runtime. Wire a KeyManager into ChainToolHandler to enable this."
                .into(),
        ))
    }

    // ── chain.wallet_export_address ───────────��──────────────────────────

    async fn handle_wallet_export_address(&self, _args: &serde_json::Value) -> ToolResult {
        // With a single-wallet configuration, export the configured wallet's address.
        let wallet = match &self.wallet {
            Some(w) => w,
            None => {
                return ToolResult::err(ToolError::Other(
                    "no wallet configured for chain.wallet_export_address".into(),
                ));
            }
        };

        match wallet.address().await {
            Ok(address) => {
                let body = json!({ "address": address });
                ToolResult::structured(body.to_string())
            }
            Err(e) => ToolResult::err(ToolError::Other(e.to_string())),
        }
    }

    // ── chain.post_insight ──────────────────────────────────────────────

    async fn handle_post_insight(&self, args: &serde_json::Value) -> ToolResult {
        let kind = match args.get("kind").and_then(|v| v.as_str()) {
            Some(k) => k,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: kind".into(),
                ));
            }
        };

        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: content".into(),
                ));
            }
        };

        let confidence = match args.get("confidence").and_then(|v| v.as_f64()) {
            Some(c) => c,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: confidence".into(),
                ));
            }
        };

        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Build the content string with tags embedded for HDC projection.
        let tagged_content = if tags.is_empty() {
            content.to_string()
        } else {
            format!("[{}] {}", tags.join(", "), content)
        };

        let rpc_params = json!({
            "author": "roko-agent",
            "kind": kind,
            "content": tagged_content,
            "stakeWei": format!("{}", (confidence * 1_000_000.0) as u64),
        });

        match self.rpc_call("chain_postInsight", rpc_params).await {
            Ok(result) => ToolResult::structured(result.to_string()),
            Err(e) => ToolResult::err(ToolError::Other(e)),
        }
    }

    // ── chain.search_insights ───────────────────────────────────────────

    async fn handle_search_insights(&self, args: &serde_json::Value) -> ToolResult {
        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(5);

        // Combine tags and query for HDC-based search.
        let search_query = if tags.is_empty() {
            query.to_string()
        } else if query.is_empty() {
            tags.join(" ")
        } else {
            format!("{} {}", tags.join(" "), query)
        };

        let rpc_params = json!({
            "query": search_query,
            "k": limit,
        });

        match self.rpc_call("chain_searchInsights", rpc_params).await {
            Ok(result) => ToolResult::structured(result.to_string()),
            Err(e) => ToolResult::err(ToolError::Other(e)),
        }
    }

    // ── chain.confirm_insight ───────────────────────────────────────────

    async fn handle_confirm_insight(&self, args: &serde_json::Value) -> ToolResult {
        let id = match args.get("id").and_then(|v| v.as_str()) {
            Some(i) => i,
            None => {
                return ToolResult::err(ToolError::SchemaInvalid(
                    "missing required field: id".into(),
                ));
            }
        };

        let rpc_params = json!({
            "id": id,
            "confirmer": "roko-agent",
        });

        match self.rpc_call("chain_confirmInsight", rpc_params).await {
            Ok(result) => ToolResult::structured(result.to_string()),
            Err(e) => ToolResult::err(ToolError::Other(e)),
        }
    }

    // ── JSON-RPC helper ─────────────────────────────────────────────────

    /// Send a JSON-RPC 2.0 call to the mirage endpoint.
    async fn rpc_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let url = self.rpc_url.as_deref().unwrap_or("http://127.0.0.1:8545");

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": [params],
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("RPC request failed: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("failed to read RPC response: {e}"))?;

        if !status.is_success() {
            return Err(format!("RPC returned {status}: {text}"));
        }

        let rpc_resp: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("invalid RPC JSON: {e}"))?;

        if let Some(error) = rpc_resp.get("error") {
            let msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown RPC error");
            return Err(format!("RPC error: {msg}"));
        }

        Ok(rpc_resp
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }
}

// ── Hex helpers (no external `hex` crate needed) ────────���────────────────

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
