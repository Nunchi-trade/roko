//! `tempo_tail` — Workstream A demo binary.
//!
//! Spawns a Roko-style "Tempo light-client agent": subscribe to verified
//! headers, read a single account at each new height, verify the proof,
//! and render a one-line update — the same shape the Nunchi UI / agent
//! command center will render in production once the real commonware
//! backend lands (Phase 1).
//!
//! Run:
//!   cargo run -p roko-tempo --example tempo_tail
//!
//! Phase-0 mock chain ships 5 headers, then exits cleanly.

use anyhow::Result;
use roko_tempo::{LightClient, TempoLightClient};

const DEMO_ADDR: &str = "0x000000000000000000000000000000000000A55E";

#[tokio::main]
async fn main() -> Result<()> {
    let lc = TempoLightClient::demo();
    println!(
        "[tempo-tail] backend={} starting; tracking {DEMO_ADDR}",
        lc.name()
    );

    loop {
        let header = match lc.await_next_header().await {
            Ok(h) => h,
            Err(e) => {
                println!("[tempo-tail] header stream ended: {e}");
                break;
            }
        };
        let proof = lc.read_account_at(DEMO_ADDR, header.height).await?;
        lc.verify_account(&proof)?;
        let balance_eth = proof.balance_wei as f64 / 1e18;
        println!(
            "[tempo-tail] block={:>3} state_root={}…{} balance={:>5.2} ETH-equiv  (verified, quorum={})",
            header.height,
            &header.state_root[..10],
            &header.state_root[header.state_root.len() - 4..],
            balance_eth,
            header.attestation.quorum_id,
        );
    }

    println!("[tempo-tail] done.");
    Ok(())
}
