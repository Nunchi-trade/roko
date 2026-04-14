//! §37.17 — chain-native persona demo.
//!
//! Builds an `HdcSubstrate` for a synthetic "chain-native" agent persona, puts
//! three insights about Uniswap behaviour into it via `Substrate::put`, then
//! runs a semantic `Substrate::query` and prints the top-3 hits by HDC
//! similarity.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p mirage-rs --features roko --example persona_chain_native
//! ```
//!
//! Deterministic: no LLM, no RNG, no network. The projection is a stable hash
//! of the input tokens.

use mirage_rs::roko_bridge::HdcSubstrate;
use roko_core::{Body, Context, Engram, Kind, Provenance, Query, Score, traits::Substrate};

const PERSONA: &str = "chain-native/uniswap-analyst";

const INSIGHTS: &[(&str, &str)] = &[
    // (short label, content)
    (
        "uniV3-stf-revert",
        "uniswap v3 STF revert typically means insufficient allowance on the input token",
    ),
    (
        "uniV3-twap-depth",
        "uniswap v3 TWAP oracle accuracy depends on pool liquidity depth; thin pools are manipulable",
    ),
    (
        "uniV4-hook-gas",
        "uniswap v4 hook invocations add ~20k gas when hooks are permissionless and untrusted",
    ),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    rt.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    println!("== persona_chain_native ==");
    println!("persona = {PERSONA}\n");

    let substrate = HdcSubstrate::new(PERSONA);

    // 1) Build signals for each insight and put them into the substrate.
    println!("writing {} insights into HdcSubstrate", INSIGHTS.len());
    for (label, content) in INSIGHTS {
        let sig = Engram::builder(Kind::Insight)
            .body(Body::text(*content))
            .provenance(Provenance::agent(PERSONA).with_session("demo"))
            .score(Score::new(0.85, 0.6, 1.0, 1.0))
            .build();
        let hash = substrate.put(sig).await?;
        println!("  put [{label}] -> hash {}", format_hash(&hash.0));
    }

    // 2) Run a semantic query via text_query tag.
    let query_text = "uniswap v3 STF reverts on low allowance";
    println!("\nquery (text_query) = {query_text:?}");
    let q = Query::all().with_tag("text_query", query_text).limit(3);
    let ctx = Context::now();
    let results = substrate.query(&q, &ctx).await?;

    println!("top-{} hits:", results.len());
    for (i, sig) in results.iter().enumerate() {
        let body = sig.body.as_text().unwrap_or("<non-text>");
        // `Engram::tag` / score / provenance are part of roko-core Engram.
        let effective = sig.score.effective();
        println!(
            "  #{}: effective_score={:.3}  body={:?}",
            i + 1,
            effective,
            body
        );
    }

    // Assertion-style sanity: top hit should be the STF-revert insight.
    if let Some(first) = results.first() {
        if first.body.as_text().unwrap_or("").contains("STF revert") {
            println!("\nok: top hit matches the expected STF-revert insight.");
        } else {
            println!(
                "\nwarn: top hit did not surface the STF-revert insight as expected; \
                 projection may have degenerate overlap on this corpus."
            );
        }
    }

    println!(
        "\npersona_chain_native: done ({} entries in substrate).",
        substrate.len().await?
    );
    Ok(())
}

fn format_hash(bytes: &[u8; 32]) -> String {
    // Short prefix of the content hash for readability.
    use std::fmt::Write as _;
    bytes[..6]
        .iter()
        .fold(String::with_capacity(12), |mut acc, b| {
            let _ = write!(&mut acc, "{b:02x}");
            acc
        })
}
