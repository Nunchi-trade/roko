//! `tempo_tail` — Workstream A demo binary.
//!
//! Connects to the Tempo "Moderato" public testnet (or any RPC passed via
//! `--rpc`), subscribes to the verified-header stream, reads a single
//! account at each new height with an EIP-1186 state proof, verifies the
//! proof locally, and renders a one-line summary.
//!
//! The default address is `block.miner` of the latest block at startup —
//! a guaranteed-real, queryable account on the configured network. We do
//! not ship a fabricated demo address (zero-fabrication policy). Pass
//! `--address 0x...` to track a specific account.
//!
//! Run:
//!   `cargo run -p roko-tempo --example tempo_tail`
//!   `cargo run -p roko-tempo --example tempo_tail -- --address 0xabc...`
//!   `cargo run -p roko-tempo --example tempo_tail -- --rpc <url> --chain-id <id>`

use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use roko_tempo::{LightClient, MODERATO_CHAIN_ID, MODERATO_RPC_URL, TempoLightClient};

#[derive(Parser, Debug)]
#[command(about = "Tail Tempo testnet headers + verify account state at each block")]
struct Args {
    /// Tempo-compatible JSON-RPC HTTP endpoint.
    #[arg(long, default_value = MODERATO_RPC_URL)]
    rpc: String,
    /// Expected chain id at the endpoint (Moderato testnet = 42431).
    #[arg(long, default_value_t = MODERATO_CHAIN_ID)]
    chain_id: u64,
    /// Account to track. If omitted, uses the latest block's `miner`.
    #[arg(long)]
    address: Option<String>,
    /// How many headers to follow before exiting.
    #[arg(long, default_value_t = 5)]
    headers: u32,
    /// Per-iteration timeout (s) for waiting on the next header.
    #[arg(long, default_value_t = 30)]
    header_timeout_s: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!(
        "[tempo-tail] connecting rpc={} chain_id={}",
        args.rpc, args.chain_id
    );
    let lc = TempoLightClient::with_rpc(&args.rpc, args.chain_id)
        .await
        .with_context(|| format!("connecting to {} ({})", args.rpc, args.chain_id))?;

    let anchor = lc
        .latest_verified()
        .context("backend should hold a TOFU anchor after connect")?;
    println!(
        "[tempo-tail] tofu anchor height={} state_root={} parent={}",
        anchor.height, anchor.state_root, anchor.parent_hash
    );

    let address = match args.address {
        Some(a) => a,
        None => derive_address_from_anchor(&args.rpc, args.chain_id, anchor.height)
            .await
            .context("deriving demo address from latest block.miner")?,
    };
    println!("[tempo-tail] tracking address={address}");

    for _ in 0..args.headers {
        let header = match tokio::time::timeout(
            Duration::from_secs(args.header_timeout_s),
            lc.await_next_header(),
        )
        .await
        {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => {
                eprintln!("[tempo-tail] header stream errored: {e}");
                break;
            }
            Err(_) => {
                eprintln!(
                    "[tempo-tail] no new header within {}s — exiting",
                    args.header_timeout_s
                );
                break;
            }
        };

        let proof = match lc.read_account_at(&address, header.height).await {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[tempo-tail] read_account_at failed at {}: {e}", header.height);
                continue;
            }
        };
        match lc.verify_account(&proof) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("[tempo-tail] proof did NOT verify at {}: {e}", header.height);
                continue;
            }
        }

        println!(
            "[tempo-tail] block={:>7} state_root={}…{} nonce={:>4} code_hash={}…{} storage_hash={}…{}  (verified, quorum={})",
            header.height,
            &header.state_root[..10],
            &header.state_root[header.state_root.len() - 4..],
            proof.nonce,
            &proof.code_hash[..10],
            &proof.code_hash[proof.code_hash.len() - 4..],
            &proof.storage_hash[..10],
            &proof.storage_hash[proof.storage_hash.len() - 4..],
            header.attestation.quorum_id,
        );
    }

    println!("[tempo-tail] done.");
    Ok(())
}

/// Pull `block.miner` of the configured anchor height as a guaranteed-real
/// demo address. We open a separate, untyped alloy provider for this lookup
/// so we don't push a "give me the raw block" method onto the [`LightClient`]
/// trait surface.
async fn derive_address_from_anchor(rpc_url: &str, _chain_id: u64, height: u64) -> Result<String> {
    use alloy::providers::{Provider, ProviderBuilder};
    use alloy::rpc::types::eth::{BlockNumberOrTag, BlockTransactionsKind};

    let url = reqwest::Url::parse(rpc_url)?;
    let provider = ProviderBuilder::new().connect_http(url);
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(height))
        .kind(BlockTransactionsKind::Hashes)
        .await
        .context("eth_getBlockByNumber")?
        .context("anchor block missing")?;
    Ok(format!("{:#x}", block.header.beneficiary))
}
