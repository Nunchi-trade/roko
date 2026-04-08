//! §37.19 — Seed a running `mirage-rs` with demo insights + pheromones.
//!
//! Populates a running mirage-rs instance with ~50 `InsightEntry`s and 20
//! pheromones via JSON-RPC, so the browser demo (§37.18) and the chain-watcher
//! example have realistic `DeFi` content to slice against.
//!
//! # Prerequisites
//!
//! The target `mirage-rs` MUST be started with the chain subsystems enabled:
//!
//! ```bash
//! cargo run -p mirage-rs --features chain --bin mirage-rs -- \
//!     --enable-hdc --enable-knowledge --enable-stigmergy
//! ```
//!
//! # Usage
//!
//! ```bash
//! cargo run -p mirage-rs --features chain --example seed_chain_fixtures -- \
//!     --rpc-url http://127.0.0.1:8545
//! ```
//!
//! The default `--rpc-url` is `http://127.0.0.1:8545`. If no mirage is
//! listening, the seeder exits cleanly with a friendly error.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};

/// DeFi-flavoured insights spanning AMMs, lending, liquidations, MEV, oracles,
/// bridges, restaking, and governance. 50 entries as per §37.19.
const DEMO_INSIGHTS: &[(&str, &str, &str)] = &[
    // (author, kind, content)
    (
        "alice",
        "insight",
        "uniswap v3 STF revert means insufficient allowance on input token",
    ),
    (
        "alice",
        "heuristic",
        "if gas price > 200 gwei, prefer batching over individual swaps",
    ),
    (
        "alice",
        "heuristic",
        "USDC balance reads are cheap to cache for 12 seconds",
    ),
    (
        "alice",
        "insight",
        "Uniswap v4 singleton hook calls add 20k gas when permissionless",
    ),
    (
        "alice",
        "strategy_fragment",
        "route stable→stable via Curve 3pool; volatile→volatile via UniV3",
    ),
    (
        "bob",
        "warning",
        "sandwich attackers target WETH/USDC 0.05% on high-slippage trades",
    ),
    (
        "bob",
        "warning",
        "LUSD peg drift observed; vault liquidations likely within 3 blocks",
    ),
    (
        "bob",
        "causal_link",
        "ETH price -> Aave DAI borrowing demand (r=0.68, 5-block lag)",
    ),
    (
        "bob",
        "warning",
        "Frax sFRAX redemption rate drops when AMO parks more into Convex",
    ),
    (
        "bob",
        "heuristic",
        "Balancer boosted pools: pre-warm the linear wrappers before swap",
    ),
    (
        "carol",
        "strategy_fragment",
        "deposit LP -> stake in gauge -> claim CRV weekly",
    ),
    (
        "carol",
        "strategy_fragment",
        "detect arb: fetch 5 dex quotes simultaneously, pick top-2",
    ),
    (
        "carol",
        "insight",
        "pool reserves drift 0.2% between blocks during high volume",
    ),
    (
        "carol",
        "heuristic",
        "Paraswap + 1inch price divergence > 0.3% => arb window open",
    ),
    (
        "carol",
        "insight",
        "Kyber Elastic concentrated ticks cluster between 0.997 and 1.003 on stables",
    ),
    (
        "dave",
        "heuristic",
        "Curve 3pool: TWAP over 15 blocks is stable for oracle use",
    ),
    (
        "dave",
        "warning",
        "Compound cTokens revert on transferFrom when paused",
    ),
    (
        "dave",
        "insight",
        "MEV boost: builder A preferred for bundles with > 3 simulations",
    ),
    (
        "dave",
        "causal_link",
        "stETH/ETH ratio < 0.995 -> Lido validator exits accelerate",
    ),
    (
        "dave",
        "heuristic",
        "call eth_estimateGas with 1.2x buffer on Arbitrum Nitro",
    ),
    (
        "eve",
        "causal_link",
        "stETH depeg -> Lido withdraw queue latency (r=0.74, 2-day lag)",
    ),
    (
        "eve",
        "heuristic",
        "Arbitrum gas = mainnet/40; re-estimate every 10 min",
    ),
    (
        "eve",
        "warning",
        "Euler finance: oracle manipulation possible with 100k TWAP depth",
    ),
    (
        "eve",
        "insight",
        "GMX v2 funding fees flip sign during rapid BTC moves",
    ),
    (
        "eve",
        "strategy_fragment",
        "hedge perp long on GMX with short on Hyperliquid to capture funding",
    ),
    (
        "frank",
        "warning",
        "Cross-chain bridge delays spike 4x during Ethereum congestion",
    ),
    (
        "frank",
        "insight",
        "Optimism sequencer posts L2 calldata every ~60s average",
    ),
    (
        "frank",
        "heuristic",
        "Base gas: estimate with 1.5x multiplier when opBNB congested",
    ),
    (
        "frank",
        "causal_link",
        "zkSync Era block gas limit -> LayerZero message relay time",
    ),
    (
        "frank",
        "strategy_fragment",
        "use Across bridge for ETH L1->L2; Socket for USDC L2->L2",
    ),
    (
        "grace",
        "insight",
        "EigenLayer restaked validator count doubles every ~40 days",
    ),
    (
        "grace",
        "warning",
        "LRT depeg risk: Renzo, KelpDAO, EtherFi all share slashing surface",
    ),
    (
        "grace",
        "causal_link",
        "AVS slashing event -> LRT TVL outflow within 24 hours",
    ),
    (
        "grace",
        "heuristic",
        "price ezETH/ETH, rsETH/ETH quoted on Uniswap v3 0.05%; Curve lags",
    ),
    (
        "grace",
        "strategy_fragment",
        "restake stETH -> mint LRT -> loop via Pendle PT/YT split",
    ),
    (
        "henry",
        "insight",
        "governance vote quorum dips 30% during US holidays",
    ),
    (
        "henry",
        "warning",
        "Compound proposal 199 flash-loan attack vector unfixed on forks",
    ),
    (
        "henry",
        "causal_link",
        "governance token airdrop announcement -> TVL inflow (4h lag)",
    ),
    (
        "henry",
        "heuristic",
        "monitor Tally + Snapshot feed; Snapshot shows soft-consensus earlier",
    ),
    (
        "henry",
        "strategy_fragment",
        "veTokenomics: bribe via Warden/Hidden-Hand on Convex, Aura",
    ),
    (
        "iris",
        "insight",
        "Chainlink OCR2 median update interval averages 25s on ETH mainnet",
    ),
    (
        "iris",
        "warning",
        "Pyth pull-oracle staleness can exceed 60s during L2 outages",
    ),
    (
        "iris",
        "causal_link",
        "Chainlink feed deviation 0.5% -> Aave liquidation ramp begins",
    ),
    (
        "iris",
        "heuristic",
        "prefer Chainlink Data Streams for perp mark prices; lower latency",
    ),
    (
        "iris",
        "strategy_fragment",
        "aggregate Chainlink + Pyth + UniV3 TWAP; median of 3",
    ),
    (
        "jules",
        "insight",
        "Pendle PT yields track underlying + implied yield spread",
    ),
    (
        "jules",
        "warning",
        "Pendle YT approaches zero near expiry; unwinding late bleeds capital",
    ),
    (
        "jules",
        "causal_link",
        "Aave USDC borrow rate -> Pendle aUSDC PT discount widens",
    ),
    (
        "jules",
        "heuristic",
        "roll Pendle positions 7-14 days before expiry to avoid gamma decay",
    ),
    (
        "jules",
        "strategy_fragment",
        "buy PT at > 10% APY, hedge with perp short on underlying",
    ),
];

/// 20 pheromones covering threat/opportunity/wisdom across `DeFi` situations.
const DEMO_PHEROMONES: &[(&str, &str, f32)] = &[
    // (kind, content, intensity)
    ("threat", "MEV bot sandwich on WETH/USDC 0.05%", 0.92),
    ("threat", "liquidation cascade imminent on Aave v3", 0.88),
    ("threat", "stETH/ETH curve pool depeg risk", 0.77),
    ("threat", "Renzo ezETH oracle deviation > 0.8%", 0.84),
    (
        "threat",
        "Euler oracle manipulation detected on low-TVL pool",
        0.79,
    ),
    (
        "threat",
        "bridge relay delay spiking beyond 30 min median",
        0.66,
    ),
    (
        "threat",
        "governance proposal 42 contains proxy upgrade hook",
        0.73,
    ),
    (
        "opportunity",
        "arbitrage 0.4% between Uniswap v3 and Sushiswap",
        0.65,
    ),
    ("opportunity", "Curve pool imbalance 2.1% on 3pool", 0.71),
    (
        "opportunity",
        "just-in-time LP during known whale swap",
        0.80,
    ),
    (
        "opportunity",
        "Pendle PT trading at 12% APY, 30 days to expiry",
        0.69,
    ),
    (
        "opportunity",
        "GMX BTC perp funding flipped positive; go short carry",
        0.58,
    ),
    (
        "opportunity",
        "Aave USDC supply APY up 4% after CompoundV3 cap hit",
        0.62,
    ),
    (
        "wisdom",
        "set arbitrum gas buffer to 3x for L1-posting calldata",
        0.58,
    ),
    (
        "wisdom",
        "use eth_call with pending tag for reliable pool state",
        0.55,
    ),
    (
        "wisdom",
        "Permit2 saves 1 tx when user sets allowance once",
        0.62,
    ),
    (
        "wisdom",
        "median 3 oracles beats trusting any single feed",
        0.60,
    ),
    (
        "wisdom",
        "LRT positions: diversify across at least 3 protocols",
        0.57,
    ),
    (
        "wisdom",
        "monitor Tally for governance votes 48h before execution",
        0.53,
    ),
    (
        "wisdom",
        "check bridge canonical vs non-canonical asset wrappers",
        0.51,
    ),
];

#[tokio::main]
async fn main() -> ExitCode {
    let rpc_url = parse_rpc_url();
    eprintln!("seed_chain_fixtures: target = {rpc_url}");

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("seed_chain_fixtures: failed to build HTTP client: {e}");
            return ExitCode::from(2);
        }
    };

    // Quick smoke-check: call chain_stats. If mirage is down, exit cleanly.
    match call_rpc(&client, &rpc_url, "chain_stats", json!({})).await {
        Ok(v) => {
            eprintln!("seed_chain_fixtures: connected. stats={v}");
        }
        Err(e) => {
            eprintln!("seed_chain_fixtures: cannot reach mirage-rs at {rpc_url}: {e}");
            eprintln!(
                "seed_chain_fixtures: start mirage with `--enable-hdc --enable-knowledge \
                 --enable-stigmergy` and try again."
            );
            return ExitCode::from(1);
        }
    }

    let mut insights_ok = 0usize;
    let mut insights_err = 0usize;
    for (author, kind, content) in DEMO_INSIGHTS {
        let params = json!({
            "author": author,
            "kind": kind,
            "content": content,
        });
        match call_rpc(&client, &rpc_url, "chain_postInsight", params).await {
            Ok(_) => insights_ok += 1,
            Err(e) => {
                insights_err += 1;
                eprintln!("  insight post failed ({kind}/{author}): {e}");
            }
        }
    }

    let mut pheromones_ok = 0usize;
    let mut pheromones_err = 0usize;
    for (kind, content, intensity) in DEMO_PHEROMONES {
        let params = json!({
            "kind": kind,
            "content": content,
            "intensity": intensity,
        });
        match call_rpc(&client, &rpc_url, "chain_depositPheromone", params).await {
            Ok(_) => pheromones_ok += 1,
            Err(e) => {
                pheromones_err += 1;
                eprintln!("  pheromone deposit failed ({kind}): {e}");
            }
        }
    }

    println!("seed_chain_fixtures: done.");
    println!(
        "  insights:   {insights_ok} accepted / {insights_err} failed  (of {})",
        DEMO_INSIGHTS.len()
    );
    println!(
        "  pheromones: {pheromones_ok} deposited / {pheromones_err} failed (of {})",
        DEMO_PHEROMONES.len()
    );

    if insights_err > 0 || pheromones_err > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn parse_rpc_url() -> String {
    let mut args = env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--rpc-url" {
            if let Some(v) = args.next() {
                return v;
            }
        } else if let Some(v) = flag.strip_prefix("--rpc-url=") {
            return v.to_string();
        }
    }
    "http://127.0.0.1:8545".to_string()
}

async fn call_rpc(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = client
        .post(url)
        .json(&req)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("decode error: {e}"))?;
    if let Some(err) = body.get("error") {
        return Err(format!("rpc error: {err}"));
    }
    Ok(body.get("result").cloned().unwrap_or(Value::Null))
}
