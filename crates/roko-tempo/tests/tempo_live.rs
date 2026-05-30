//! Integration tests against a live Tempo-compatible RPC endpoint.
//!
//! Default endpoint: `https://rpc.moderato.tempo.xyz` (Tempo testnet
//! "Moderato"). Override with `ROKO_TEST_RPC_URL` and
//! `ROKO_TEST_CHAIN_ID` for devnets / alternate providers. The suite
//! silently no-ops if the endpoint is unreachable so CI without network
//! egress (or with the testnet down) stays green.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use roko_tempo::{LcError, LightClient, MODERATO_CHAIN_ID, MODERATO_RPC_URL, TempoLightClient};

fn rpc_url() -> String {
    std::env::var("ROKO_TEST_RPC_URL").unwrap_or_else(|_| MODERATO_RPC_URL.into())
}

fn chain_id() -> u64 {
    std::env::var("ROKO_TEST_CHAIN_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(MODERATO_CHAIN_ID)
}

async fn live_or_skip() -> Option<TempoLightClient> {
    let url = rpc_url();
    let cid = chain_id();
    match tokio::time::timeout(
        Duration::from_secs(10),
        TempoLightClient::with_rpc(&url, cid),
    )
    .await
    {
        Ok(Ok(lc)) => Some(lc),
        Ok(Err(e)) => {
            eprintln!("skip tempo_live ({url}, chain_id={cid}): {e}");
            None
        }
        Err(_) => {
            eprintln!("skip tempo_live ({url}): connect timeout");
            None
        }
    }
}

#[tokio::test]
async fn live_header_chain_advances() {
    let Some(lc) = live_or_skip().await else {
        return;
    };
    let mut prev = lc.latest_verified().expect("tofu anchor");
    for _ in 0..3 {
        let next = match tokio::time::timeout(Duration::from_secs(20), lc.await_next_header()).await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => panic!("await_next_header failed: {e}"),
            Err(_) => {
                eprintln!("skip: no new header in 20s — testnet may be idle");
                return;
            }
        };
        assert_eq!(next.parent_hash, prev.block_hash, "parent hash chains");
        assert!(next.height > prev.height, "height monotone");
        prev = next;
    }
}

#[tokio::test]
async fn live_account_proof_verifies_for_block_miner() {
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy::rpc::types::eth::{BlockNumberOrTag, BlockTransactionsKind};

    let Some(lc) = live_or_skip().await else {
        return;
    };
    let anchor = lc.latest_verified().unwrap();

    // Pull miner via a side-channel provider (the LightClient trait doesn't
    // expose raw block info; we keep that out of the trait surface).
    let url = reqwest::Url::parse(&rpc_url()).unwrap();
    let provider = ProviderBuilder::new().connect_http(url);
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(anchor.height))
        .kind(BlockTransactionsKind::Hashes)
        .await
        .unwrap()
        .unwrap();
    let miner = format!("{:#x}", block.header.beneficiary);

    let proof = lc.read_account_at(&miner, anchor.height).await.unwrap();
    assert_eq!(proof.against_state_root, anchor.state_root);
    lc.verify_account(&proof).expect("real proof verifies");
}

#[tokio::test]
async fn live_proof_against_wrong_state_root_fails() {
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy::rpc::types::eth::{BlockNumberOrTag, BlockTransactionsKind};

    let Some(lc) = live_or_skip().await else {
        return;
    };
    let anchor = lc.latest_verified().unwrap();

    let url = reqwest::Url::parse(&rpc_url()).unwrap();
    let provider = ProviderBuilder::new().connect_http(url);
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(anchor.height))
        .kind(BlockTransactionsKind::Hashes)
        .await
        .unwrap()
        .unwrap();
    let miner = format!("{:#x}", block.header.beneficiary);

    let mut proof = lc.read_account_at(&miner, anchor.height).await.unwrap();
    // Substitute a state root of the right shape that does NOT match.
    proof.against_state_root =
        "0x0000000000000000000000000000000000000000000000000000000000000001".into();
    let err = lc
        .verify_account(&proof)
        .expect_err("tampered state root must fail verification");
    assert!(matches!(err, LcError::InvalidProof { .. }), "got {err:?}");
}
