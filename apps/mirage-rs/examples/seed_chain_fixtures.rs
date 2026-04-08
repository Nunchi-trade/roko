//! §37.19 — Seed a running `mirage-rs` with demo insights + pheromones.
//!
//! Populates a running mirage-rs instance with ~50 `InsightEntry`s and 20
//! pheromones via JSON-RPC, so the browser demo (§37.18) and the chain-watcher
//! example have realistic `DeFi` content to slice against. Then exercises every
//! HTTP REST API endpoint as a verification pass.
//!
//! # Phases
//!
//! 1. **JSON-RPC seeding** — posts insights + deposits pheromones via JSON-RPC.
//! 2. **REST API verification** — exercises every REST endpoint and prints a
//!    verification report showing which endpoints responded successfully.
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

    println!("seed_chain_fixtures: phase 1 (JSON-RPC seeding) done.");
    println!(
        "  insights:   {insights_ok} accepted / {insights_err} failed  (of {})",
        DEMO_INSIGHTS.len()
    );
    println!(
        "  pheromones: {pheromones_ok} deposited / {pheromones_err} failed (of {})",
        DEMO_PHEROMONES.len()
    );
    println!();

    // -----------------------------------------------------------------------
    // Phase 2: Exercise every HTTP REST API endpoint as a verification pass.
    // -----------------------------------------------------------------------
    println!("seed_chain_fixtures: phase 2 — REST API verification");
    println!("  base url: {rpc_url}/api");
    println!();

    let base = format!("{rpc_url}/api");
    let rest_errors = exercise_rest_api(&client, &base).await;

    println!();
    if insights_err > 0 || pheromones_err > 0 || rest_errors > 0 {
        eprintln!(
            "seed_chain_fixtures: completed with errors (rpc={}, rest={})",
            insights_err + pheromones_err,
            rest_errors
        );
        ExitCode::from(1)
    } else {
        println!("seed_chain_fixtures: all phases completed successfully.");
        ExitCode::SUCCESS
    }
}

/// Exercise every REST API endpoint and print a verification report.
/// Returns the number of endpoint failures.
async fn exercise_rest_api(client: &reqwest::Client, base: &str) -> usize {
    let mut pass = 0usize;
    let mut fail = 0usize;

    // Helper closure results are collected into these counters.
    macro_rules! check {
        ($label:expr, $result:expr) => {
            match $result {
                Ok(body) => {
                    println!("  [PASS] {}", $label);
                    let _ = body; // suppress unused warning
                    pass += 1;
                }
                Err(e) => {
                    eprintln!("  [FAIL] {}: {}", $label, e);
                    fail += 1;
                }
            }
        };
    }

    // --- Health & Stats ---------------------------------------------------

    check!(
        "GET /api/health",
        rest_get(client, &format!("{base}/health")).await
    );

    check!(
        "GET /api/stats",
        rest_get(client, &format!("{base}/stats")).await
    );

    // --- Pheromone Field --------------------------------------------------

    // Deposit a pheromone via REST (distinct from JSON-RPC seeding above).
    check!(
        "POST /api/pheromones (deposit)",
        rest_post(
            client,
            &format!("{base}/pheromones"),
            json!({
                "kind": "threat",
                "content": "REST API test: flash loan detected on compound fork",
                "intensity": 0.95,
                "half_life_secs": 7200
            }),
        )
        .await
    );

    // List pheromones.
    let pheromone_list = rest_get(
        client,
        &format!("{base}/pheromones?sort=intensity&order=desc&limit=5"),
    )
    .await;
    check!("GET /api/pheromones (list)", pheromone_list.as_ref().map(|v| v.clone()).map_err(|e| e.clone()));

    // Extract a pheromone ID for the projection endpoint.
    let pheromone_id: Option<u64> = pheromone_list
        .ok()
        .and_then(|v| v.get("pheromones")?.as_array()?.first()?.get("id")?.as_u64());

    check!(
        "GET /api/pheromones/summary",
        rest_get(client, &format!("{base}/pheromones/summary")).await
    );

    check!(
        "POST /api/pheromones/query (semantic search)",
        rest_post(
            client,
            &format!("{base}/pheromones/query"),
            json!({"query": "flash loan attack", "k": 5}),
        )
        .await
    );

    check!(
        "GET /api/pheromones/heatmap",
        rest_get(
            client,
            &format!("{base}/pheromones/heatmap?bucket_seconds=3600"),
        )
        .await
    );

    if let Some(pid) = pheromone_id {
        check!(
            &format!("GET /api/pheromones/{pid}/projection"),
            rest_get(
                client,
                &format!("{base}/pheromones/{pid}/projection?duration_secs=3600&points=10"),
            )
            .await
        );
    } else {
        eprintln!("  [SKIP] GET /api/pheromones/{{id}}/projection — no pheromone ID available");
    }

    // --- Knowledge Graph --------------------------------------------------

    // Post an insight via REST.
    let post_insight_resp = rest_post(
        client,
        &format!("{base}/knowledge/entries"),
        json!({
            "kind": "warning",
            "content": "REST API test: never use delegatecall to untrusted contracts",
            "author": "rest-verifier",
            "enabled_by": [],
            "stake_wei": 0
        }),
    )
    .await;
    check!("POST /api/knowledge/entries (post insight)", post_insight_resp.as_ref().map(|v| v.clone()).map_err(|e| e.clone()));

    // Extract the insight ID for confirm/challenge.
    let insight_id: Option<String> = post_insight_resp
        .ok()
        .and_then(|v| v.get("id")?.as_str().map(String::from));

    // List entries.
    check!(
        "GET /api/knowledge/entries (list)",
        rest_get(
            client,
            &format!("{base}/knowledge/entries?sort=weight&order=desc&limit=5"),
        )
        .await
    );

    // Confirm the insight we just posted.
    if let Some(ref iid) = insight_id {
        check!(
            &format!("POST /api/knowledge/entries/{iid}/confirm"),
            rest_post(
                client,
                &format!("{base}/knowledge/entries/{iid}/confirm"),
                json!({"confirmer": "rest-verifier-bob", "stake_wei": 0}),
            )
            .await
        );

        check!(
            &format!("POST /api/knowledge/entries/{iid}/challenge"),
            rest_post(
                client,
                &format!("{base}/knowledge/entries/{iid}/challenge"),
                json!({"challenger": "rest-verifier-carol", "stake_wei": 0}),
            )
            .await
        );
    } else {
        eprintln!("  [SKIP] POST /api/knowledge/entries/{{id}}/confirm — no insight ID available");
        eprintln!(
            "  [SKIP] POST /api/knowledge/entries/{{id}}/challenge — no insight ID available"
        );
    }

    // Trigger decay sweep.
    check!(
        "POST /api/knowledge/decay",
        rest_post(
            client,
            &format!("{base}/knowledge/decay"),
            json!({}),
        )
        .await
    );

    // Edges.
    check!(
        "GET /api/knowledge/edges",
        rest_get(
            client,
            &format!("{base}/knowledge/edges?similarity_threshold=0.3"),
        )
        .await
    );

    // Semantic search.
    check!(
        "GET /api/knowledge/search",
        rest_get(
            client,
            &format!("{base}/knowledge/search?q=delegatecall+proxy&k=5"),
        )
        .await
    );

    // Kinds.
    check!(
        "GET /api/knowledge/kinds",
        rest_get(client, &format!("{base}/knowledge/kinds")).await
    );

    // --- Agent Registry ---------------------------------------------------

    // Register agents via REST.
    for (agent_id, role) in &[
        ("rest-agent-alpha", "researcher"),
        ("rest-agent-beta", "validator"),
        ("rest-agent-gamma", "executor"),
    ] {
        check!(
            &format!("POST /api/agents (register {agent_id})"),
            rest_post(
                client,
                &format!("{base}/agents"),
                json!({
                    "id": agent_id,
                    "pubkey": format!("0x{agent_id}"),
                    "role": role,
                }),
            )
            .await
        );
    }

    // List agents.
    check!(
        "GET /api/agents (list)",
        rest_get(client, &format!("{base}/agents")).await
    );

    // Heartbeat for each registered agent.
    for (agent_id, tokens, cost, tasks) in &[
        ("rest-agent-alpha", 5000u64, 0.15f64, 1u64),
        ("rest-agent-beta", 3000, 0.09, 2),
        ("rest-agent-gamma", 8000, 0.24, 3),
    ] {
        check!(
            &format!("POST /api/agents/{agent_id}/heartbeat"),
            rest_post(
                client,
                &format!("{base}/agents/{agent_id}/heartbeat"),
                json!({
                    "tokens_used": tokens,
                    "cost_usd": cost,
                    "tasks_completed": tasks,
                }),
            )
            .await
        );
    }

    // GET heartbeat for one agent.
    check!(
        "GET /api/agents/rest-agent-alpha/heartbeat",
        rest_get(
            client,
            &format!("{base}/agents/rest-agent-alpha/heartbeat"),
        )
        .await
    );

    // GET stats for one agent.
    check!(
        "GET /api/agents/rest-agent-alpha/stats",
        rest_get(
            client,
            &format!("{base}/agents/rest-agent-alpha/stats"),
        )
        .await
    );

    // GET trace for one agent.
    check!(
        "GET /api/agents/rest-agent-alpha/trace",
        rest_get(
            client,
            &format!("{base}/agents/rest-agent-alpha/trace?limit=10"),
        )
        .await
    );

    // Topology.
    check!(
        "GET /api/agents/topology",
        rest_get(client, &format!("{base}/agents/topology")).await
    );

    // --- Summary ----------------------------------------------------------

    println!();
    println!("  REST API verification: {pass} passed, {fail} failed (of {})", pass + fail);

    fail
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

// ---------------------------------------------------------------------------
// REST API helpers
// ---------------------------------------------------------------------------

/// Perform a GET request to a REST endpoint, returning the parsed JSON body.
async fn rest_get(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("decode error: {e}"))?;
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("HTTP {}: {}", status.as_u16(), msg));
    }
    Ok(body)
}

/// Perform a POST request to a REST endpoint with a JSON body.
async fn rest_post(client: &reqwest::Client, url: &str, body: Value) -> Result<Value, String> {
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("http error: {e}"))?;
    let status = resp.status();
    let resp_body: Value = resp
        .json()
        .await
        .map_err(|e| format!("decode error: {e}"))?;
    if !status.is_success() {
        let msg = resp_body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("HTTP {}: {}", status.as_u16(), msg));
    }
    Ok(resp_body)
}
