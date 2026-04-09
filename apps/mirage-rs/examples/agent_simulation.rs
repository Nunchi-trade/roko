//! 20-agent continuous simulation for the mirage-rs dashboard.
//!
//! Spawns 20 tokio tasks, each acting as a distinct agent with unique personality,
//! DeFi focus area, and behavior pattern. Agents continuously post insights, deposit
//! pheromones, confirm/challenge entries, create tasks, and send heartbeats.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p mirage-rs --features chain,roko --example agent_simulation -- \
//!     --rpc-url http://127.0.0.1:8545
//! ```

use rand::{Rng, SeedableRng, rngs::StdRng};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Agent definitions
// ---------------------------------------------------------------------------

struct AgentDef {
    id: &'static str,
    role: &'static str,
    kind: AgentKind,
    pace_ms: u64,
    insights: &'static [(&'static str, &'static str)], // (kind, content)
    pheromones: &'static [(&'static str, &'static str)], // (kind, content)
    queries: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum AgentKind {
    Watcher,
    Security,
    Strategy,
    Validator,
    Synthesizer,
    Infra,
}

const AGENTS: &[AgentDef] = &[
    // ── DeFi Watchers (6) ──
    AgentDef {
        id: "roko-alpha-amm",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 18000,
        insights: &[
            (
                "insight",
                "uniswap v3 WETH/USDC 0.05% pool: concentrated liquidity below $2400 strike suggests bearish positioning",
            ),
            (
                "insight",
                "curve 3pool imbalanced: USDT at 38% share, possible depeg pressure building",
            ),
            (
                "insight",
                "balancer weighted pool rebalance: 60/40 ETH/DAI shifting to 70/30 after large deposit",
            ),
            (
                "heuristic",
                "uniswap v3 tick spacing compression indicates volatility expectation rising",
            ),
            (
                "insight",
                "sushiswap migration volume: $12M moved from v1 to v2 routes in last hour",
            ),
            (
                "warning",
                "pancakeswap BSC pool TVL drop 15% in 3h — possible liquidity flight",
            ),
            (
                "insight",
                "uniswap v4 hook deployment: new dynamic fee hook on ETH/USDC pair",
            ),
            (
                "pattern",
                "AMM arbitrage cycle: ETH price deviation >0.3% triggers cross-DEX rebalance within 2 blocks",
            ),
            (
                "insight",
                "curve stETH/ETH pool: slight depeg 0.9987 — likely validator withdrawal queue",
            ),
            (
                "heuristic",
                "concentrated liquidity providers tend to withdraw when 24h vol drops below 50% of 7d average",
            ),
        ],
        pheromones: &[
            (
                "opportunity",
                "uniswap v3 WETH/USDC: LP fee APR spike to 42% — concentrated range 2380-2420",
            ),
            (
                "threat",
                "curve 3pool severe imbalance: USDT weight >40% — possible bank run",
            ),
            (
                "opportunity",
                "balancer flash loan arb: ETH/DAI price gap 0.4% between pools",
            ),
            (
                "wisdom",
                "AMM pool depth inversely correlated with gas price during congestion",
            ),
        ],
        queries: &["uniswap pool revert", "AMM liquidity", "DEX volume"],
    },
    AgentDef {
        id: "roko-beta-lending",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 20000,
        insights: &[
            (
                "insight",
                "aave v3 WETH utilization at 89%: borrow rate spike from 3.2% to 7.8% in last hour",
            ),
            (
                "warning",
                "compound cETH liquidation wave: $4.2M liquidated in 15 blocks",
            ),
            (
                "insight",
                "morpho blue ETH market: 92% utilization, rate curve knee at 85%",
            ),
            (
                "insight",
                "spark protocol DAI borrow rate stable at 5.5% despite ETH volatility",
            ),
            (
                "heuristic",
                "lending liquidation cascade probability rises exponentially above 85% utilization",
            ),
            (
                "warning",
                "aave v3 health factor distribution: 23% of positions below 1.15 — cascade risk",
            ),
            (
                "insight",
                "compound III USDC supply rate 4.1% — highest in 3 months",
            ),
            (
                "pattern",
                "lending protocol utilization follows 8h cycle aligned with Asia/Europe/US trading hours",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "aave v3 liquidation cascade imminent: $18M positions with HF < 1.05",
            ),
            (
                "opportunity",
                "morpho blue: supply rate 6.2% with low utilization — yield opportunity",
            ),
            (
                "wisdom",
                "lending rates mean-revert within 48h after utilization spikes above 90%",
            ),
        ],
        queries: &["lending liquidation", "borrow rate", "utilization"],
    },
    AgentDef {
        id: "roko-gamma-mev",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 12000,
        insights: &[
            (
                "insight",
                "sandwich attack on uniswap v2: victim lost 2.3% on 50 ETH swap, searcher profit ~$340",
            ),
            (
                "insight",
                "flashbots block 19234567: 3 private txs, total MEV extracted $12,400",
            ),
            (
                "warning",
                "backrun detected: large DEX trade followed by immediate arb, $890 extracted",
            ),
            (
                "insight",
                "MEV-boost relay distribution: 67% Flashbots, 18% bloXroute, 15% other",
            ),
            (
                "heuristic",
                "sandwich probability >60% for uniswap v2 swaps exceeding 10 ETH without slippage protection",
            ),
            (
                "insight",
                "JIT liquidity provision detected: $2.1M concentrated position added and removed in same block",
            ),
            (
                "pattern",
                "MEV extraction per block averages $2,800 during high-vol periods vs $450 during quiet",
            ),
            (
                "anti_knowledge",
                "MEV is decreasing myth: per-block MEV up 34% YoY even with PBS",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "sandwich attack surge: 12 attacks in last 50 blocks on uniswap v2",
            ),
            (
                "threat",
                "frontrunning spike: 8 pending mempool transactions being targeted",
            ),
            (
                "wisdom",
                "MEV-aware routing reduces sandwich losses by ~85% on average",
            ),
        ],
        queries: &["MEV sandwich", "frontrunning", "flashbots"],
    },
    AgentDef {
        id: "roko-delta-bridge",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 25000,
        insights: &[
            (
                "insight",
                "wormhole bridge: $45M ETH transferred L1→Arbitrum in last 6h",
            ),
            (
                "warning",
                "across bridge delay: median fill time increased from 2min to 18min",
            ),
            (
                "insight",
                "stargate TVL up 8% this week — USDC flows dominant",
            ),
            (
                "insight",
                "LayerZero message volume: 340K messages/day, 40% increase from last week",
            ),
            (
                "warning",
                "synapse bridge: anomalous withdrawal pattern — $8M single tx from new address",
            ),
            (
                "insight",
                "optimism bridge: 7d finality queue at 1,200 pending withdrawals",
            ),
            (
                "heuristic",
                "bridge exploit likelihood increases when TVL exceeds 10x historical average",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "bridge anomaly: unusual $8M withdrawal from synapse — investigate",
            ),
            (
                "opportunity",
                "across bridge: fast fill discount 0.15% — savings on large transfers",
            ),
            (
                "wisdom",
                "cross-chain transfers safest during low-congestion windows (UTC 06:00-10:00)",
            ),
        ],
        queries: &["bridge exploit", "cross-chain", "LayerZero"],
    },
    AgentDef {
        id: "roko-epsilon-oracle",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 15000,
        insights: &[
            (
                "insight",
                "chainlink ETH/USD feed: heartbeat 3600s, deviation threshold 0.5%, last update 420s ago",
            ),
            (
                "warning",
                "chainlink LINK/ETH feed stale: last update 4200s ago, exceeds heartbeat",
            ),
            (
                "insight",
                "pyth network: ETH price $2,387.45, confidence interval ±$1.20",
            ),
            (
                "insight",
                "redstone oracle: gas-optimized pull model, 230K gas saved per update vs push",
            ),
            (
                "heuristic",
                "oracle front-running risk highest in 200ms window after off-chain price moves >1%",
            ),
            (
                "warning",
                "UMA optimistic oracle: disputed price for stETH/ETH at 0.998, 2h challenge period",
            ),
            (
                "pattern",
                "chainlink deviation triggers cluster around CPI/FOMC announcement times ±5 min",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "oracle stale: LINK/ETH chainlink feed 70 min without update",
            ),
            (
                "threat",
                "price deviation: pyth vs chainlink ETH price diverged 0.8% — possible manipulation",
            ),
            (
                "wisdom",
                "multi-oracle comparison reduces price manipulation risk by 94%",
            ),
        ],
        queries: &["oracle price", "chainlink", "price feed"],
    },
    AgentDef {
        id: "roko-zeta-governance",
        role: "watcher",
        kind: AgentKind::Watcher,
        pace_ms: 30000,
        insights: &[
            (
                "insight",
                "aave governance: proposal AIP-312 to increase WETH supply cap passed with 89% approval",
            ),
            (
                "insight",
                "uniswap governance: fee switch vote scheduled for next week — 180M UNI quorum needed",
            ),
            (
                "insight",
                "compound governance: proposal 232 executed — new COMP distribution rate",
            ),
            (
                "insight",
                "MakerDAO: DSR increased to 8% — significant DAI demand shift expected",
            ),
            (
                "warning",
                "governance attack attempt: flash-borrowed 5M tokens used to vote on malicious proposal",
            ),
            (
                "strategy",
                "governance arbitrage: buy proposal token before favorable vote, sell after execution",
            ),
        ],
        pheromones: &[
            (
                "opportunity",
                "MakerDAO DSR at 8% — significant yield opportunity for stablecoin holders",
            ),
            (
                "threat",
                "governance attack: flash-loan vote manipulation attempt detected",
            ),
            (
                "wisdom",
                "governance proposals with >80% early support have 97% passage rate historically",
            ),
        ],
        queries: &["governance vote", "proposal", "DAO"],
    },
    // ── Security Analysts (4) ──
    AgentDef {
        id: "roko-eta-audit",
        role: "security",
        kind: AgentKind::Security,
        pace_ms: 35000,
        insights: &[
            (
                "warning",
                "reentrancy pattern detected in new DeFi contract 0x1a2b...3c4d — uses .call instead of .transfer",
            ),
            (
                "warning",
                "unchecked return value: ERC20 transfer in yield aggregator may silently fail",
            ),
            (
                "anti_knowledge",
                "WRONG: selfdestruct is deprecated but still dangerous in delegatecall context",
            ),
            (
                "insight",
                "audit findings: 3 critical, 7 high, 12 medium in new DEX fork — deployment risky",
            ),
            (
                "warning",
                "integer overflow risk: Solidity 0.7 contract without SafeMath on reward calculation",
            ),
            (
                "heuristic",
                "contracts deployed without verification within 24h have 8x higher exploit probability",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "critical vulnerability: reentrancy in new DeFi contract — do not interact",
            ),
            (
                "threat",
                "unverified contract deployed with $2M TVL — high risk",
            ),
        ],
        queries: &["reentrancy", "vulnerability", "audit"],
    },
    AgentDef {
        id: "roko-theta-exploit",
        role: "security",
        kind: AgentKind::Security,
        pace_ms: 22000,
        insights: &[
            (
                "warning",
                "price oracle manipulation detected: flash loan + large swap to skew TWAP",
            ),
            (
                "anti_knowledge",
                "EXPLOIT: infinite mint vulnerability in reward token — emergency shutdown needed",
            ),
            (
                "insight",
                "exploit postmortem: $3.2M drained via read-only reentrancy on balancer vault",
            ),
            (
                "warning",
                "suspicious contract: self-destructing proxy upgrading to unverified implementation",
            ),
            (
                "insight",
                "honeypot token detected: buy tax 0%, sell tax 100% — scam contract",
            ),
            (
                "heuristic",
                "80% of DeFi exploits in 2024 involve oracle manipulation or access control flaws",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "active exploit: flash loan attack draining lending pool — avoid deposits",
            ),
            ("threat", "honeypot detected: new token with 100% sell tax"),
            (
                "wisdom",
                "time-weighted average price (TWAP) oracles with >30 min window resist 99% of manipulation",
            ),
        ],
        queries: &["exploit", "flash loan attack", "drain"],
    },
    AgentDef {
        id: "roko-iota-rugcheck",
        role: "security",
        kind: AgentKind::Security,
        pace_ms: 28000,
        insights: &[
            (
                "warning",
                "rug pull indicators: owner can mint unlimited tokens, no timelock, liquidity not locked",
            ),
            (
                "insight",
                "token contract analysis: 94% of tokens launched today have owner mint capabilities",
            ),
            (
                "warning",
                "liquidity removal detected: $500K pulled from new memecoin pool within 4h of launch",
            ),
            (
                "heuristic",
                "tokens without liquidity lock for >6 months have 23x higher rug pull probability",
            ),
            (
                "insight",
                "safe token checklist passed: renounced ownership, locked liquidity 12mo, no hidden fees",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "rug pull in progress: liquidity being removed from SCAM/WETH pair",
            ),
            (
                "wisdom",
                "always verify: locked liquidity, renounced ownership, audit report before aping",
            ),
        ],
        queries: &["rug pull", "token safety", "liquidity lock"],
    },
    AgentDef {
        id: "roko-kappa-phishing",
        role: "security",
        kind: AgentKind::Security,
        pace_ms: 32000,
        insights: &[
            (
                "warning",
                "phishing contract mimics uniswap router — approve() drains all tokens",
            ),
            (
                "warning",
                "malicious airdrop: claiming triggers unlimited approval to attacker address",
            ),
            (
                "insight",
                "address poisoning attack: zero-value transfers from similar-looking addresses",
            ),
            (
                "anti_knowledge",
                "SCAM: fake 'revoke approval' site actually grants new approvals",
            ),
            (
                "heuristic",
                "contracts requesting unlimited approvals from new addresses are 90% likely malicious",
            ),
        ],
        pheromones: &[
            (
                "threat",
                "active phishing: fake uniswap router contract draining approvals",
            ),
            (
                "threat",
                "address poisoning campaign targeting top 1000 ETH wallets",
            ),
        ],
        queries: &["phishing", "malicious contract", "approval drain"],
    },
    // ── Strategy/Pattern Agents (4) ──
    AgentDef {
        id: "roko-lambda-yield",
        role: "strategist",
        kind: AgentKind::Strategy,
        pace_ms: 40000,
        insights: &[
            (
                "strategy",
                "optimal yield: stake ETH → mint stETH → deposit Aave → borrow USDC → farm Curve = 12.4% APR",
            ),
            (
                "strategy",
                "EigenLayer restaking: 3.2% native + 4.1% AVS rewards = 7.3% total with slashing risk <2%",
            ),
            (
                "pattern",
                "yield farming rotation: move to highest APR every 7d nets 2.3% more than static",
            ),
            (
                "strategy",
                "delta-neutral yield: long spot ETH + short perp = funding rate capture 8-15% APR",
            ),
            (
                "insight",
                "Pendle YT pricing implies market expects ETH staking yield to drop to 2.8% by Dec",
            ),
        ],
        pheromones: &[
            (
                "opportunity",
                "yield: Aave stETH loop yielding 12.4% APR — check health factor limits",
            ),
            (
                "opportunity",
                "EigenLayer restaking: new AVS launched with 8% bonus rewards for first week",
            ),
            (
                "wisdom",
                "sustainable yield sources: staking, lending, LP fees. Everything else is token emissions.",
            ),
        ],
        queries: &["yield farming", "staking rewards", "APR"],
    },
    AgentDef {
        id: "roko-mu-momentum",
        role: "analyst",
        kind: AgentKind::Strategy,
        pace_ms: 20000,
        insights: &[
            (
                "pattern",
                "ETH 24h volume up 45% — breakout signal when vol increase sustains >3h",
            ),
            (
                "insight",
                "DeFi TVL momentum: $48.2B → $52.1B in 7d, led by liquid staking protocols",
            ),
            (
                "pattern",
                "gas price 3-day EMA crossing above 20-day EMA — historically precedes 15% ETH move",
            ),
            (
                "insight",
                "on-chain metrics: active addresses up 12%, new addresses up 8% — accumulation phase",
            ),
            (
                "heuristic",
                "gas price spikes >100 gwei lasting >30 min correlate with 73% chance of >5% daily move",
            ),
        ],
        pheromones: &[
            (
                "opportunity",
                "momentum signal: volume breakout on ETH pairs — trend continuation likely",
            ),
            (
                "wisdom",
                "volume precedes price: sustained volume increase for 6h+ confirms trend 80% of time",
            ),
        ],
        queries: &["volume momentum", "TVL trend", "gas price"],
    },
    AgentDef {
        id: "roko-nu-correlation",
        role: "analyst",
        kind: AgentKind::Strategy,
        pace_ms: 45000,
        insights: &[
            (
                "pattern",
                "ETH/BTC 30d correlation dropped to 0.72 from 0.91 — DeFi narrative divergence",
            ),
            (
                "insight",
                "stablecoin supply correlation with DeFi TVL: r=0.94, 2-week lag",
            ),
            (
                "pattern",
                "L2 gas usage inversely correlated with L1 congestion: r=-0.67",
            ),
            (
                "insight",
                "cross-protocol: Aave utilization rate leads Compound by 4h on average",
            ),
        ],
        pheromones: &[(
            "wisdom",
            "ETH/BTC decorrelation historically precedes major DeFi narrative shifts within 2 weeks",
        )],
        queries: &["correlation", "ETH BTC", "cross-protocol"],
    },
    AgentDef {
        id: "roko-xi-seasonal",
        role: "analyst",
        kind: AgentKind::Strategy,
        pace_ms: 50000,
        insights: &[
            (
                "pattern",
                "Tuesday-Thursday: highest DEX volume. Weekend: lowest. Deviation signals event.",
            ),
            (
                "pattern",
                "month-end: lending utilization spikes 15-20% as institutions settle positions",
            ),
            (
                "insight",
                "seasonal: Q1 historically strongest for DeFi TVL growth, Q3 weakest",
            ),
            (
                "heuristic",
                "options expiry Fridays (monthly): expect 3x normal volatility in 2h window around 08:00 UTC",
            ),
        ],
        pheromones: &[(
            "wisdom",
            "seasonal pattern: avoid large LP positions through monthly options expiry windows",
        )],
        queries: &["seasonal pattern", "time-based", "weekly cycle"],
    },
    // ── Meta/Validation Agents (4) ──
    AgentDef {
        id: "roko-omicron-validator",
        role: "validator",
        kind: AgentKind::Validator,
        pace_ms: 10000,
        insights: &[],
        pheromones: &[],
        queries: &["confirmed insight", "validated"],
    },
    AgentDef {
        id: "roko-pi-synthesizer",
        role: "synthesizer",
        kind: AgentKind::Synthesizer,
        pace_ms: 30000,
        insights: &[
            (
                "meta",
                "synthesis: 3 agents agree — lending utilization approaching cascade threshold",
            ),
            (
                "meta",
                "cross-signal: AMM imbalance + oracle delay + high gas = elevated exploit risk",
            ),
            (
                "meta",
                "consensus: 5/6 DeFi watchers reporting increased MEV activity this hour",
            ),
            (
                "meta",
                "pattern convergence: volume momentum + seasonal low + governance catalyst = high conviction",
            ),
        ],
        pheromones: &[],
        queries: &["lending cascade", "MEV activity", "consensus"],
    },
    AgentDef {
        id: "roko-rho-contrarian",
        role: "validator",
        kind: AgentKind::Validator,
        pace_ms: 15000,
        insights: &[
            (
                "anti_knowledge",
                "contrarian view: yield farming APRs are unsustainable — emissions will end",
            ),
            (
                "anti_knowledge",
                "challenge: 'safe token' label misleading — ownership renounced but proxy upgradeable",
            ),
        ],
        pheromones: &[],
        queries: &["overrated", "misleading", "unsustainable"],
    },
    AgentDef {
        id: "roko-sigma-archiver",
        role: "synthesizer",
        kind: AgentKind::Synthesizer,
        pace_ms: 45000,
        insights: &[
            (
                "meta",
                "hourly summary: 23 insights posted, 8 confirmed, 2 challenged. Top topic: lending.",
            ),
            (
                "meta",
                "agent status: 20/20 agents active, avg 3.2 insights/min, 1.1 pheromones/min",
            ),
            (
                "meta",
                "knowledge growth: 15 new insights this hour, 3 duplicates pruned, net +12",
            ),
        ],
        pheromones: &[],
        queries: &["summary", "hourly", "activity"],
    },
    // ── Infrastructure (2) ──
    AgentDef {
        id: "roko-tau-health",
        role: "monitor",
        kind: AgentKind::Infra,
        pace_ms: 20000,
        insights: &[
            (
                "insight",
                "system health: API latency p50=8ms p99=45ms, 0 errors in last 5min",
            ),
            (
                "insight",
                "knowledge store: 127 entries, 42 confirmed, 3 challenged, 0 pruned",
            ),
            (
                "insight",
                "pheromone field: 34 active, total intensity 18.7, threat:12 opp:14 wisdom:8",
            ),
        ],
        pheromones: &[(
            "wisdom",
            "system healthy: all 20 agents responsive, API latency nominal",
        )],
        queries: &["system health", "latency", "uptime"],
    },
    AgentDef {
        id: "roko-upsilon-pruner",
        role: "monitor",
        kind: AgentKind::Infra,
        pace_ms: 60000,
        insights: &[
            (
                "insight",
                "decay sweep: 0 entries pruned, 3 moved to decaying state",
            ),
            (
                "meta",
                "knowledge lifecycle: avg entry lifespan 45min before decay, confirmed entries live 4x longer",
            ),
        ],
        pheromones: &[],
        queries: &[],
    },
];

// ---------------------------------------------------------------------------
// Task templates for diverse task creation
// ---------------------------------------------------------------------------

const TASK_TEMPLATES: &[(&str, &str, &str, &str)] = &[
    // (title, description, kind, priority)
    (
        "Analyze Uniswap V3 pool depth",
        "Investigate concentrated liquidity positions and identify thin areas susceptible to large swaps",
        "analyze",
        "high",
    ),
    (
        "Monitor Aave liquidation queue",
        "Track positions approaching liquidation threshold and estimate cascade risk",
        "monitor",
        "critical",
    ),
    (
        "Research EigenLayer AVS security",
        "Evaluate slashing conditions and economic security of new AVS deployments",
        "research",
        "medium",
    ),
    (
        "Validate bridge transfer anomaly",
        "Cross-reference Wormhole transfer pattern against known exploit signatures",
        "validate",
        "high",
    ),
    (
        "Report weekly DeFi TVL summary",
        "Compile TVL changes across top 20 protocols with trend analysis",
        "report",
        "low",
    ),
    (
        "Investigate oracle price deviation",
        "Compare Chainlink, Pyth, and Redstone feeds for ETH/USD discrepancy",
        "analyze",
        "high",
    ),
    (
        "Track governance proposal impact",
        "Model the on-chain effects of Aave AIP-312 supply cap increase",
        "research",
        "medium",
    ),
    (
        "Monitor MEV relay distribution",
        "Track block builder market share changes and censorship patterns",
        "monitor",
        "medium",
    ),
    (
        "Validate yield strategy safety",
        "Audit the stETH leverage loop for liquidation risk under 20% drawdown",
        "validate",
        "high",
    ),
    (
        "Analyze cross-chain flow patterns",
        "Map USDC flow between L1 and L2s to identify liquidity migration trends",
        "analyze",
        "medium",
    ),
    (
        "Research new lending protocol",
        "Evaluate Morpho Blue market parameters and risk model",
        "research",
        "low",
    ),
    (
        "Monitor gas price anomalies",
        "Detect unusual gas price patterns that may indicate MEV bot activity",
        "monitor",
        "medium",
    ),
    (
        "Investigate token launch pattern",
        "Analyze recent memecoin launches for rug pull indicators",
        "validate",
        "high",
    ),
    (
        "Track staking yield trends",
        "Monitor ETH staking APR across Lido, Rocket Pool, Coinbase for divergence",
        "monitor",
        "low",
    ),
    (
        "Analyze DEX aggregator routing",
        "Compare 1inch, Paraswap, CowSwap routing efficiency on large trades",
        "analyze",
        "medium",
    ),
];

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

async fn api_post(client: &Client, base: &str, path: &str, body: Value) -> Option<Value> {
    let url = format!("{}/api{}", base, path);
    match client.post(&url).json(&body).send().await {
        Ok(resp) => resp.json().await.ok(),
        Err(_) => None,
    }
}

async fn api_get(client: &Client, base: &str, path: &str) -> Option<Value> {
    let url = format!("{}/api{}", base, path);
    match client.get(&url).send().await {
        Ok(resp) => resp.json().await.ok(),
        Err(_) => None,
    }
}

async fn register_agent(client: &Client, base: &str, id: &str, role: &str) {
    api_post(
        client,
        base,
        "/agents",
        json!({"id": id, "pubkey": "", "role": role}),
    )
    .await;
}

async fn heartbeat(client: &Client, base: &str, id: &str, tokens: u64, cost: f64) {
    let path = format!("/agents/{}/heartbeat", id);
    api_post(
        client,
        base,
        &path,
        json!({"tokens_used": tokens, "cost_usd": cost}),
    )
    .await;
}

async fn post_insight(
    client: &Client,
    base: &str,
    kind: &str,
    content: &str,
    author: &str,
    deps: &[String],
    stake: u64,
) {
    api_post(
        client,
        base,
        "/knowledge/entries",
        json!({
            "kind": kind, "content": content, "author": author,
            "enabled_by": deps, "stake_wei": stake
        }),
    )
    .await;
}

async fn deposit_pheromone(client: &Client, base: &str, kind: &str, content: &str, intensity: f32) {
    api_post(
        client,
        base,
        "/pheromones",
        json!({
            "kind": kind, "content": content, "intensity": intensity
        }),
    )
    .await;
}

async fn get_recent_entries(client: &Client, base: &str, limit: usize) -> Vec<Value> {
    let path = format!(
        "/knowledge/entries?limit={}&sort=created_at&order=desc",
        limit
    );
    api_get(client, base, &path)
        .await
        .and_then(|v| v.get("items").cloned().or(Some(v)))
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

async fn confirm_entry(client: &Client, base: &str, id: &str, confirmer: &str) {
    let path = format!("/knowledge/entries/{}/confirm", id);
    api_post(client, base, &path, json!({"confirmer": confirmer})).await;
}

async fn challenge_entry(client: &Client, base: &str, id: &str, challenger: &str) {
    let path = format!("/knowledge/entries/{}/challenge", id);
    api_post(client, base, &path, json!({"challenger": challenger})).await;
}

async fn create_task(
    client: &Client,
    base: &str,
    title: &str,
    desc: &str,
    kind: &str,
    priority: &str,
    creator: &str,
    tags: &[&str],
) {
    api_post(
        client,
        base,
        "/tasks",
        json!({
            "title": title, "description": desc, "kind": kind,
            "priority": priority, "creator": creator,
            "tags": tags, "stake_wei": StdRng::from_entropy().gen_range(1000u64..100000)
        }),
    )
    .await;
}

async fn claim_and_complete_task(client: &Client, base: &str, agent_id: &str) {
    // Find open tasks
    if let Some(data) = api_get(client, base, "/tasks?state=open&limit=5").await {
        let tasks = data
            .get("items")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if let Some(task) = tasks.first() {
            if let Some(tid) = task.get("id").and_then(|v| v.as_u64()) {
                // Assign
                api_post(
                    client,
                    base,
                    &format!("/tasks/{}/assign", tid),
                    json!({"assignee": agent_id}),
                )
                .await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                // Start
                api_post(client, base, &format!("/tasks/{}/start", tid), json!({})).await;
                tokio::time::sleep(Duration::from_secs(
                    2 + StdRng::from_entropy().gen_range(0u64..5),
                ))
                .await;
                // Complete (80%) or fail (20%)
                if StdRng::from_entropy().gen_range(0.0f32..1.0) < 0.8 {
                    api_post(
                        client,
                        base,
                        &format!("/tasks/{}/complete", tid),
                        json!({"result_insight_id": null}),
                    )
                    .await;
                } else {
                    api_post(
                        client,
                        base,
                        &format!("/tasks/{}/fail", tid),
                        json!({"reason": "timeout or insufficient data"}),
                    )
                    .await;
                }
            }
        }
    }
}

async fn trigger_decay(client: &Client, base: &str) {
    api_post(client, base, "/knowledge/decay", json!({})).await;
}

async fn post_trace(
    client: &Client,
    base: &str,
    agent_id: &str,
    cycle: u64,
    phase: &str,
    reads: &[&str],
    reasoning: &str,
    action: &str,
    action_id: &str,
) {
    let path = format!("/agents/{}/trace", agent_id);
    api_post(
        client,
        base,
        &path,
        json!({
            "cycle": cycle,
            "phase": phase,
            "reads": reads,
            "reasoning": reasoning,
            "action": action,
            "action_id": action_id,
            "timestamp": 0
        }),
    )
    .await;
}

// ---------------------------------------------------------------------------
// Agent loop
// ---------------------------------------------------------------------------

async fn agent_loop(client: Client, base: String, def: &'static AgentDef) {
    let mut rng = StdRng::from_entropy(); // Send-safe RNG for async tasks
    let mut cycle = 0u64;

    loop {
        cycle += 1;

        // Heartbeat every cycle
        let tokens: u64 = rng.gen_range(100..2000);
        let cost: f64 = tokens as f64 * 0.00003; // ~$0.03 per 1K tokens
        heartbeat(&client, &base, def.id, tokens, cost).await;

        // Post cognitive traces (Retrieve → Reason → Act → Verify)
        let trace_reads: Vec<&str> = if !def.queries.is_empty() {
            vec![def.queries[rng.gen_range(0..def.queries.len())]]
        } else {
            vec!["chain_state"]
        };
        let reasoning = format!("cycle {} analysis of {} domain signals", cycle, def.role);
        post_trace(
            &client,
            &base,
            def.id,
            cycle,
            "retrieve",
            &trace_reads,
            &format!("scanning {} data sources", trace_reads.len()),
            "scan",
            &format!("scan:{}", cycle),
        )
        .await;
        post_trace(
            &client,
            &base,
            def.id,
            cycle,
            "reason",
            &trace_reads,
            &reasoning,
            "evaluate",
            &format!("eval:{}", cycle),
        )
        .await;

        match def.kind {
            AgentKind::Watcher => {
                // Post an insight
                if !def.insights.is_empty() {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    let stake: u64 = rng.gen_range(1000..50000);
                    post_insight(&client, &base, kind, content, def.id, &[], stake).await;
                    eprintln!("[{}] {} posted {} insight", chrono_now(), def.id, kind);
                }
                // Maybe deposit pheromone (40% chance)
                if !def.pheromones.is_empty() && rng.gen_range(0.0f32..1.0) < 0.4 {
                    let (kind, content) = def.pheromones[rng.gen_range(0..def.pheromones.len())];
                    let intensity: f32 = rng.gen_range(0.3..1.0);
                    deposit_pheromone(&client, &base, kind, content, intensity).await;
                    eprintln!("[{}] {} deposited {} pheromone", chrono_now(), def.id, kind);
                }
                // Create a task (50% chance)
                if rng.gen_range(0.0f32..1.0) < 0.5 {
                    let tmpl = TASK_TEMPLATES[rng.gen_range(0..TASK_TEMPLATES.len())];
                    create_task(
                        &client,
                        &base,
                        tmpl.0,
                        tmpl.1,
                        tmpl.2,
                        tmpl.3,
                        def.id,
                        &["defi"],
                    )
                    .await;
                    eprintln!("[{}] {} created task: {}", chrono_now(), def.id, tmpl.0);
                }
            }
            AgentKind::Security => {
                // Occasionally claim and complete a task
                if rng.gen_range(0.0f32..1.0) < 0.3 {
                    claim_and_complete_task(&client, &base, def.id).await;
                }
                if !def.insights.is_empty() {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    let stake: u64 = rng.gen_range(5000..100000);
                    post_insight(&client, &base, kind, content, def.id, &[], stake).await;
                    eprintln!("[{}] {} posted {} alert", chrono_now(), def.id, kind);
                }
                if !def.pheromones.is_empty() && rng.gen_range(0.0f32..1.0) < 0.5 {
                    let (kind, content) = def.pheromones[rng.gen_range(0..def.pheromones.len())];
                    deposit_pheromone(&client, &base, kind, content, rng.gen_range(0.5..1.0)).await;
                }
            }
            AgentKind::Strategy => {
                if !def.insights.is_empty() {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    post_insight(
                        &client,
                        &base,
                        kind,
                        content,
                        def.id,
                        &[],
                        rng.gen_range(2000..30000),
                    )
                    .await;
                    eprintln!("[{}] {} posted {} analysis", chrono_now(), def.id, kind);
                }
                if !def.pheromones.is_empty() && rng.gen_range(0.0f32..1.0) < 0.3 {
                    let (kind, content) = def.pheromones[rng.gen_range(0..def.pheromones.len())];
                    deposit_pheromone(&client, &base, kind, content, rng.gen_range(0.4..0.9)).await;
                }
            }
            AgentKind::Validator => {
                // Get recent entries and confirm/challenge
                let entries = get_recent_entries(&client, &base, 15).await;
                let mut confirmed = 0;
                let mut challenged = 0;
                for entry in &entries {
                    let id = entry.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        continue;
                    }
                    let state_str = entry.get("state").and_then(|v| v.as_str()).unwrap_or("");
                    if state_str == "pruned" || state_str == "stale" {
                        continue;
                    }

                    if def.id == "roko-rho-contrarian" {
                        // Contrarian: challenge more often
                        if rng.gen_range(0.0f32..1.0) < 0.3 {
                            challenge_entry(&client, &base, id, def.id).await;
                            challenged += 1;
                        } else if rng.gen_range(0.0f32..1.0) < 0.4 {
                            confirm_entry(&client, &base, id, def.id).await;
                            confirmed += 1;
                        }
                    } else {
                        // Regular validator: confirm most, challenge few
                        if rng.gen_range(0.0f32..1.0) < 0.6 {
                            confirm_entry(&client, &base, id, def.id).await;
                            confirmed += 1;
                        } else if rng.gen_range(0.0f32..1.0) < 0.1 {
                            challenge_entry(&client, &base, id, def.id).await;
                            challenged += 1;
                        }
                    }
                    if confirmed + challenged >= 5 {
                        break;
                    }
                }
                if !def.insights.is_empty() && rng.gen_range(0.0f32..1.0) < 0.3 {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    post_insight(&client, &base, kind, content, def.id, &[], 0).await;
                }
                eprintln!(
                    "[{}] {} validated: {} confirmed, {} challenged",
                    chrono_now(),
                    def.id,
                    confirmed,
                    challenged
                );
                // Claim and complete a task (70% chance)
                if rng.gen_range(0.0f32..1.0) < 0.7 {
                    claim_and_complete_task(&client, &base, def.id).await;
                }
            }
            AgentKind::Synthesizer => {
                if !def.insights.is_empty() {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    // Try to find related entries for enabled_by
                    let query = if !def.queries.is_empty() {
                        def.queries[rng.gen_range(0..def.queries.len())]
                    } else {
                        ""
                    };
                    let deps: Vec<String> = if !query.is_empty() {
                        let hits = api_get(
                            &client,
                            &base,
                            &format!("/knowledge/search?q={}&k=3", query),
                        )
                        .await
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default();
                        hits.iter()
                            .filter_map(|h| {
                                h.get("id").and_then(|v| v.as_str()).map(|s| s.to_string())
                            })
                            .collect()
                    } else {
                        vec![]
                    };
                    post_insight(
                        &client,
                        &base,
                        kind,
                        content,
                        def.id,
                        &deps,
                        rng.gen_range(0..10000),
                    )
                    .await;
                    eprintln!(
                        "[{}] {} synthesized with {} deps",
                        chrono_now(),
                        def.id,
                        deps.len()
                    );
                }
            }
            AgentKind::Infra => {
                if !def.insights.is_empty() {
                    let (kind, content) = def.insights[rng.gen_range(0..def.insights.len())];
                    post_insight(&client, &base, kind, content, def.id, &[], 0).await;
                }
                if !def.pheromones.is_empty() && rng.gen_range(0.0f32..1.0) < 0.3 {
                    let (kind, content) = def.pheromones[rng.gen_range(0..def.pheromones.len())];
                    deposit_pheromone(&client, &base, kind, content, 0.5).await;
                }
                // Pruner triggers decay
                if def.id == "roko-upsilon-pruner" {
                    trigger_decay(&client, &base).await;
                    eprintln!("[{}] {} triggered decay", chrono_now(), def.id);
                }
                // Task creation for infra
                if cycle % 3 == 0 {
                    let tmpl = TASK_TEMPLATES[rng.gen_range(0..TASK_TEMPLATES.len())];
                    create_task(
                        &client,
                        &base,
                        tmpl.0,
                        tmpl.1,
                        tmpl.2,
                        tmpl.3,
                        def.id,
                        &["infra"],
                    )
                    .await;
                }
            }
        }

        // Post act + verify traces
        let action_name = match def.kind {
            AgentKind::Watcher => "post_insight",
            AgentKind::Security => "post_alert",
            AgentKind::Strategy => "post_analysis",
            AgentKind::Validator => "validate_entries",
            AgentKind::Synthesizer => "synthesize",
            AgentKind::Infra => "monitor_health",
        };
        post_trace(
            &client,
            &base,
            def.id,
            cycle,
            "act",
            &[],
            &format!("executing {}", action_name),
            action_name,
            &format!("{}:{}", action_name, cycle),
        )
        .await;
        post_trace(
            &client,
            &base,
            def.id,
            cycle,
            "verify",
            &[],
            &format!("verified {} outcome for cycle {}", action_name, cycle),
            "check_result",
            &format!("verify:{}", cycle),
        )
        .await;

        // Sleep with jitter
        let jitter: u64 = rng.gen_range(0..def.pace_ms / 3);
        tokio::time::sleep(Duration::from_millis(def.pace_ms + jitter)).await;
    }
}

fn chrono_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let base = std::env::args()
        .position(|a| a == "--rpc-url")
        .and_then(|i| std::env::args().nth(i + 1))
        .unwrap_or_else(|| "http://127.0.0.1:8545".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client");

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║     ROKO AGENT SIMULATION · 20 AGENTS               ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Target: {}", base);
    println!("╠══════════════════════════════════════════════════════╣");

    // Register all agents
    for def in AGENTS {
        register_agent(&client, &base, def.id, def.role).await;
        println!(
            "║  ● {:<28} role={:<12} pace={}s",
            def.id,
            def.role,
            def.pace_ms / 1000
        );
    }
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ All 20 agents registered. Starting continuous loop. ║");
    println!("║ Press Ctrl+C to stop.                               ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();

    // Spawn all agent tasks
    let mut handles = Vec::new();
    for def in AGENTS {
        let client = client.clone();
        let base = base.clone();
        handles.push(tokio::spawn(async move {
            // Stagger startup so agents don't all fire at once
            let delay = StdRng::from_entropy().gen_range(0u64..5000);
            tokio::time::sleep(Duration::from_millis(delay)).await;
            agent_loop(client, base, def).await;
        }));
    }

    // Wait for all (runs forever until Ctrl+C)
    for h in handles {
        let _ = h.await;
    }
}
