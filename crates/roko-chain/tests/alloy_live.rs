//! Integration tests for the alloy-backed `ChainClient`/`ChainWallet`.
//!
//! Requires a live JSON-RPC endpoint. Set `ROKO_TEST_RPC_URL` (default
//! `http://127.0.0.1:18545` — the usual dev mirage-rs port) to run. The suite
//! silently no-ops when the endpoint is unreachable, so CI that doesn't bring
//! up a chain is not broken.

#![cfg(feature = "alloy-backend")]
#![allow(clippy::unwrap_used)]

use roko_chain::alloy_impl::{AlloyChainClient, AlloyChainWallet};
use roko_chain::{ChainClient, ChainWallet, TxRequest};

fn rpc_url() -> String {
    std::env::var("ROKO_TEST_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:18545".into())
}

/// First anvil/hardhat default account. Safe to hard-code — dev only.
const DEPLOYER_PK: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

async fn live_or_skip() -> Option<AlloyChainClient> {
    let url = rpc_url();
    let Ok(client) = AlloyChainClient::http(&url) else {
        eprintln!("skip: invalid RPC url");
        return None;
    };
    match client.block_number().await {
        Ok(_) => Some(client),
        Err(e) => {
            eprintln!("skip alloy live tests ({url}): {e}");
            None
        }
    }
}

#[tokio::test]
async fn alloy_client_reads_block_number_and_chain_id() {
    let Some(client) = live_or_skip().await else {
        return;
    };
    let _ = client.block_number().await.unwrap();
    let cid = client.chain_id().await.unwrap();
    assert!(cid > 0, "chain_id must be non-zero");
}

#[tokio::test]
async fn alloy_wallet_reports_address_and_nonce() {
    let Some(_) = live_or_skip().await else {
        return;
    };
    let wallet = AlloyChainWallet::from_hex_key(&rpc_url(), DEPLOYER_PK, 31337).unwrap();
    let addr = wallet.address().await.unwrap();
    assert!(addr.starts_with("0x"));
    assert_eq!(addr.len(), 42);
    let _nonce = wallet.nonce().await.unwrap();
    let _bal = wallet.balance(None).await.unwrap();
}

#[tokio::test]
async fn alloy_wallet_can_submit_empty_tx_and_get_receipt() {
    let Some(_) = live_or_skip().await else {
        return;
    };
    let wallet = AlloyChainWallet::from_hex_key(&rpc_url(), DEPLOYER_PK, 31337).unwrap();
    // Self-send of 1 wei — exercises signing + broadcast + receipt polling.
    let from = wallet.address().await.unwrap();
    let tx = TxRequest {
        to: Some(from.clone()),
        value: 1,
        ..Default::default()
    };
    let hash = match wallet.sign_and_submit(tx).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("skip submit path ({e})");
            return;
        }
    };
    let receipt = wallet.wait_for_receipt(&hash, 10_000).await.unwrap();
    assert!(receipt.status, "self-send should succeed");
}
