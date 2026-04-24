//! End-to-end integration tests for the chain tool domain.
//!
//! Every test in this file talks to a **live mirage-rs endpoint** — no mocks.
//! The endpoint defaults to `https://mirage-devnet.up.railway.app` (Anvil
//! account #0, Chain ID 1) and can be overridden via environment variables.
//!
//! If the endpoint is unreachable the tests will fail, not silently skip.

use std::sync::Arc;

use serde_json::json;

use roko_chain::alloy_impl::{AlloyChainClient, AlloyChainWallet};
use roko_chain::tools::CHAIN_TOOL_NAMES;
use roko_chain::{ChainClient, ChainWallet};
use roko_cli::chain_handler::ChainToolHandler;
use roko_cli::chain_registry::{chain_aware_resolver, chain_handler_map};
use roko_core::config::schema::{ChainConfig, RokoConfig};
use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolHandler, ToolResult};

// ── Constants ────────────────────────────────────────────────────────────────

const MIRAGE_RPC: &str = "https://mirage-devnet.up.railway.app";
const CHAIN_ID: u64 = 1;
/// Anvil account #0 private key — safe for devnet only.
const DEPLOYER_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
/// Anvil account #0 address.
const DEPLOYER: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
/// Anvil account #1 address (recipient for transfers).
const RECIPIENT: &str = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";

// ── Helpers ──────────────────────────────────────────────────────────────────

fn rpc_url() -> String {
    std::env::var("MIRAGE_RPC_URL").unwrap_or_else(|_| MIRAGE_RPC.to_string())
}

fn make_client() -> Arc<AlloyChainClient> {
    Arc::new(AlloyChainClient::http(&rpc_url()).expect("AlloyChainClient::http must succeed"))
}

fn make_wallet() -> Arc<AlloyChainWallet> {
    Arc::new(
        AlloyChainWallet::from_hex_key(&rpc_url(), DEPLOYER_KEY, CHAIN_ID)
            .expect("AlloyChainWallet::from_hex_key must succeed"),
    )
}

fn make_call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall::new("test-call-1", name, arguments)
}

fn test_ctx() -> (tempfile::TempDir, ToolContext) {
    let tmp = tempfile::tempdir().expect("create tempdir");
    let ctx = ToolContext::testing(tmp.path());
    (tmp, ctx)
}

fn parse_ok_json(result: &ToolResult) -> serde_json::Value {
    match result {
        ToolResult::Ok { content, .. } => {
            serde_json::from_str(content).expect("ToolResult content should be valid JSON")
        }
        ToolResult::Err(e) => panic!("expected ToolResult::Ok but got Err: {e}"),
    }
}

// ── Test 1: chain.balance reads deployer ETH balance from mirage ─────────

#[tokio::test]
async fn chain_balance_reads_deployer_balance() {
    let client = make_client();
    let handler = ChainToolHandler {
        client: client as Arc<dyn ChainClient>,
        wallet: None,
        tool_name: "chain.balance".to_string(),
    };

    let call = make_call("chain.balance", json!({ "address": DEPLOYER }));
    let (_tmp, ctx) = test_ctx();
    let result = handler.execute(call, &ctx).await;

    let body = parse_ok_json(&result);
    let balance: u128 = body["balance_wei"]
        .as_str()
        .expect("balance_wei field")
        .parse()
        .expect("parseable u128");
    assert!(
        balance > 100_000_000_000_000_000,
        "deployer should have >0.1 ETH on mirage, got {balance} wei"
    );
}

// ── Test 2: chain.transfer sends 1 wei on mirage ────────────────────────

#[tokio::test]
async fn chain_transfer_sends_wei_on_mirage() {
    let client = make_client();
    let wallet = make_wallet();

    let handler = ChainToolHandler {
        client: client as Arc<dyn ChainClient>,
        wallet: Some(wallet.clone() as Arc<dyn ChainWallet>),
        tool_name: "chain.transfer".to_string(),
    };

    let call = make_call("chain.transfer", json!({ "to": RECIPIENT, "amount": "1" }));
    let (_tmp, ctx) = test_ctx();
    let result = handler.execute(call, &ctx).await;

    let body = parse_ok_json(&result);
    let tx_hash = body["tx_hash"].as_str().expect("tx_hash field");
    assert!(tx_hash.starts_with("0x"), "tx_hash must be 0x-prefixed");
    assert_eq!(tx_hash.len(), 66, "tx_hash must be 32 bytes hex");

    // Verify the receipt landed on-chain.
    let receipt = wallet
        .wait_for_receipt(&roko_chain::TxHash::new(tx_hash), 30_000)
        .await
        .expect("receipt within 30s");
    assert!(receipt.status, "transfer must succeed");
    assert!(receipt.block_number > 0, "tx must be mined");
}

// ── Test 3: chain.transfer without wallet returns clear error ───────────

#[tokio::test]
async fn chain_transfer_without_wallet_returns_error() {
    let client = make_client();

    let handler = ChainToolHandler {
        client: client as Arc<dyn ChainClient>,
        wallet: None,
        tool_name: "chain.transfer".to_string(),
    };

    let call = make_call("chain.transfer", json!({ "to": RECIPIENT, "amount": "1" }));
    let (_tmp, ctx) = test_ctx();
    let result = handler.execute(call, &ctx).await;

    match &result {
        ToolResult::Err(ToolError::Other(msg)) => {
            assert!(
                msg.contains("no wallet configured"),
                "error should mention missing wallet, got: {msg}"
            );
        }
        other => panic!("expected ToolError::Other, got: {other:?}"),
    }
}

// ── Test 4: chain.wallet_info returns real address/balance/nonce ─────────

#[tokio::test]
async fn chain_wallet_info_returns_live_details() {
    let client = make_client();
    let wallet = make_wallet();

    let handler = ChainToolHandler {
        client: client as Arc<dyn ChainClient>,
        wallet: Some(wallet as Arc<dyn ChainWallet>),
        tool_name: "chain.wallet_info".to_string(),
    };

    let call = make_call("chain.wallet_info", json!({}));
    let (_tmp, ctx) = test_ctx();
    let result = handler.execute(call, &ctx).await;

    let body = parse_ok_json(&result);
    let address = body["address"].as_str().expect("address field");
    assert!(
        address.starts_with("0x"),
        "address should be 0x-prefixed, got: {address}"
    );

    let balance: u128 = body["balance_wei"]
        .as_str()
        .expect("balance_wei field")
        .parse()
        .expect("parseable u128");
    assert!(balance > 0, "deployer wallet should have funds");

    assert!(body["nonce"].is_u64(), "nonce should be present as u64");
}

// ── Test 5: chain_handler_map creates all 14 handlers (live client) ─────

#[tokio::test]
async fn chain_handler_map_creates_all_14_tools() {
    let client = make_client();
    let map = chain_handler_map(client as Arc<dyn ChainClient>, None);

    assert_eq!(map.len(), 14, "should create exactly 14 handlers");

    for name in CHAIN_TOOL_NAMES {
        let handler = map
            .get(name)
            .unwrap_or_else(|| panic!("missing tool: {name}"));
        assert_eq!(handler.name(), name, "handler name mismatch");
    }
}

// ── Test 6: chain_aware_resolver resolves chain + std tools ─────────────

#[tokio::test]
async fn chain_aware_resolver_resolves_both_domains() {
    let client = make_client();
    let map = chain_handler_map(client as Arc<dyn ChainClient>, None);
    let resolver = chain_aware_resolver(map);

    // Chain tool resolves.
    assert!(
        resolver("chain.balance").is_some(),
        "chain.balance should resolve"
    );
    assert!(
        resolver("chain.transfer").is_some(),
        "chain.transfer should resolve"
    );

    // Std builtin falls through.
    assert!(
        resolver("read_file").is_some(),
        "read_file should resolve via std"
    );
    assert!(resolver("bash").is_some(), "bash should resolve via std");

    // Unknown returns None.
    assert!(
        resolver("nonexistent_tool").is_none(),
        "unknown tool should be None"
    );
}

// ── Test 7: ChainConfig round-trips through TOML ────────────────────────

#[test]
fn chain_config_round_trips_through_toml() {
    let mut config = RokoConfig::default();
    config.chain = ChainConfig {
        rpc_url: Some(MIRAGE_RPC.to_string()),
        chain_id: Some(CHAIN_ID),
        wallet_key: Some(DEPLOYER_KEY.to_string()),
        identity_registry: Some("0x84eA74d481Ee0A5332c457a4d796187F6Ba67fEB".to_string()),
        reputation_registry: Some("0x9E545E3C0baAB3E08CdfD552C960A1050f373042".to_string()),
        validation_registry: Some("0xa82fF9aFd8f496c3d6ac40E2a0F282E47488CFc9".to_string()),
        deployer: Some(DEPLOYER.to_string()),
        agent_registry: None,
        bounty_market: None,
    };

    let toml_str = toml::to_string_pretty(&config).expect("serialize to TOML");
    assert!(
        toml_str.contains("[chain]"),
        "TOML should contain [chain] section"
    );
    assert!(
        toml_str.contains("mirage-devnet"),
        "TOML should contain RPC URL"
    );

    let decoded: RokoConfig = toml::from_str(&toml_str).expect("deserialize from TOML");
    assert_eq!(decoded.chain.rpc_url, config.chain.rpc_url);
    assert_eq!(decoded.chain.chain_id, config.chain.chain_id);
    assert_eq!(decoded.chain.wallet_key, config.chain.wallet_key);
    assert_eq!(
        decoded.chain.identity_registry,
        config.chain.identity_registry
    );
    assert_eq!(
        decoded.chain.reputation_registry,
        config.chain.reputation_registry
    );
    assert_eq!(
        decoded.chain.validation_registry,
        config.chain.validation_registry
    );
    assert_eq!(decoded.chain.deployer, config.chain.deployer);
}

// ── Test 8: Full round-trip — balance before, transfer, balance after ───

#[tokio::test]
async fn full_round_trip_balance_transfer_balance() {
    let client = make_client();
    let wallet = make_wallet();
    let (_tmp, ctx) = test_ctx();

    // Step 1: Read recipient balance before.
    let bal_handler = ChainToolHandler {
        client: client.clone() as Arc<dyn ChainClient>,
        wallet: None,
        tool_name: "chain.balance".to_string(),
    };
    let result = bal_handler
        .execute(
            make_call("chain.balance", json!({ "address": RECIPIENT })),
            &ctx,
        )
        .await;
    let before: u128 = parse_ok_json(&result)["balance_wei"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Step 2: Transfer 1000 wei from deployer to recipient.
    let tx_handler = ChainToolHandler {
        client: client.clone() as Arc<dyn ChainClient>,
        wallet: Some(wallet as Arc<dyn ChainWallet>),
        tool_name: "chain.transfer".to_string(),
    };
    let result = tx_handler
        .execute(
            make_call(
                "chain.transfer",
                json!({ "to": RECIPIENT, "amount": "1000" }),
            ),
            &ctx,
        )
        .await;
    let tx_hash = parse_ok_json(&result)["tx_hash"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(tx_hash.starts_with("0x"));

    // Step 3: Read recipient balance after.
    let result = bal_handler
        .execute(
            make_call("chain.balance", json!({ "address": RECIPIENT })),
            &ctx,
        )
        .await;
    let after: u128 = parse_ok_json(&result)["balance_wei"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    // Use >= because other tests in this suite may also send to RECIPIENT.
    assert!(
        after >= before + 1000,
        "recipient balance should increase by at least 1000 wei (before={before}, after={after})"
    );
}
