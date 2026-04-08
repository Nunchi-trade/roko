//! §37.16 — in-process chain watcher demo.
//!
//! Spins up an in-process `PheromoneBus` + `InsightBus`, attaches `VecSink`
//! subscribers, and simulates a short mirage run: seed ~12 pheromones + 8
//! insights, then loop 5 times decaying pheromones and emitting synthetic new
//! activity. Each tick drains the subscriber sinks and prints new events.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p mirage-rs --features roko --example roko_chain_watcher
//! ```
//!
//! Expected runtime: well under 10 seconds. No network, no HTTP, no LLM.
//!
//! This example demonstrates the push-based subscription surface built in
//! §37.11–§37.14 without requiring a running mirage RPC server.

use std::sync::Arc;

use mirage_rs::chain::projection::project_tokens;
use mirage_rs::chain::{InsightId, KnowledgeKind, PheromoneField, PheromoneKind};
use mirage_rs::roko_bridge::{
    BackpressurePolicy, InsightBus, InsightEvent, PheromoneBus, PheromoneEvent, SubscriptionStats,
    VecSink,
};

/// A seed tuple for pheromones: (kind, label to project, intensity).
const PHEROMONE_SEEDS: &[(PheromoneKind, &str, f32)] = &[
    (
        PheromoneKind::Threat,
        "MEV sandwich on WETH/USDC 0.05%",
        0.92,
    ),
    (
        PheromoneKind::Threat,
        "Aave v3 liquidation cascade warming up",
        0.88,
    ),
    (
        PheromoneKind::Threat,
        "stETH/ETH curve pool depeg risk",
        0.77,
    ),
    (
        PheromoneKind::Threat,
        "Euler oracle manipulation on low-TVL pool",
        0.79,
    ),
    (
        PheromoneKind::Opportunity,
        "arbitrage UniV3 vs Sushi 0.4%",
        0.65,
    ),
    (
        PheromoneKind::Opportunity,
        "Curve 3pool imbalance 2.1%",
        0.71,
    ),
    (
        PheromoneKind::Opportunity,
        "just-in-time LP on whale swap",
        0.80,
    ),
    (
        PheromoneKind::Opportunity,
        "Pendle PT 12% APY, 30d to expiry",
        0.69,
    ),
    (
        PheromoneKind::Wisdom,
        "Permit2 saves 1 tx on first allowance",
        0.62,
    ),
    (
        PheromoneKind::Wisdom,
        "Arbitrum gas buffer 3x for L1 calldata",
        0.58,
    ),
    (
        PheromoneKind::Wisdom,
        "median 3 oracles beats any single feed",
        0.60,
    ),
    (
        PheromoneKind::Wisdom,
        "LRT positions: diversify across 3 protocols",
        0.57,
    ),
];

/// Seed tuple for insight events: (kind, author, content).
const INSIGHT_SEEDS: &[(KnowledgeKind, &str, &str)] = &[
    (
        KnowledgeKind::Insight,
        "alice",
        "Uniswap v3 STF revert = insufficient allowance",
    ),
    (
        KnowledgeKind::Heuristic,
        "alice",
        "USDC balance reads cacheable for 12 seconds",
    ),
    (KnowledgeKind::Warning, "bob", "LUSD peg drift observed"),
    (
        KnowledgeKind::CausalLink,
        "bob",
        "ETH price -> Aave DAI borrow demand (r=0.68)",
    ),
    (
        KnowledgeKind::StrategyFragment,
        "carol",
        "detect arb: 5 quotes, pick top-2",
    ),
    (
        KnowledgeKind::Heuristic,
        "dave",
        "Curve 3pool TWAP stable over 15 blocks",
    ),
    (
        KnowledgeKind::Warning,
        "eve",
        "Euler manipulation possible at 100k depth",
    ),
    (
        KnowledgeKind::Insight,
        "grace",
        "EigenLayer validator count doubles / 40d",
    ),
];

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    rt.block_on(async_main())
}

#[allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::suboptimal_flops,
    clippy::unused_async
)]
async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    println!("== roko_chain_watcher ==");
    println!("in-process pheromone + insight streaming demo\n");

    // Buses (shared between the "mirage" write path and the subscribers).
    let pheromone_bus = Arc::new(PheromoneBus::new());
    let insight_bus = Arc::new(InsightBus::new());

    // One VecSink subscriber per bus. In real use these would be
    // BroadcastSink / MpscSink tied to a long-lived async task.
    let pher_sink: Arc<VecSink<PheromoneEvent>> = Arc::new(VecSink::new());
    let ins_sink: Arc<VecSink<InsightEvent>> = Arc::new(VecSink::new());

    let pher_sub_id = pheromone_bus.register(pher_sink.clone(), BackpressurePolicy::DropOldest);
    let ins_sub_id = insight_bus.register(ins_sink.clone(), BackpressurePolicy::DropOldest);
    println!("registered subscribers: pheromone={pher_sub_id}, insight={ins_sub_id}");

    // Actual field that backs the pheromones — mirrors what chain_rpc holds.
    let mut field = PheromoneField::new(0.01);
    let mut now_secs: u64 = 1_700_000_000;

    // Seed pheromones: deposit into the field AND broadcast on the bus.
    for (kind, label, intensity) in PHEROMONE_SEEDS {
        let vector = project_tokens(label);
        let _id = field.deposit(*kind, vector, *intensity, now_secs);
        pheromone_bus.broadcast(*kind, vector, *intensity, now_secs);
    }
    println!(
        "seeded {} pheromones (field size = {})",
        PHEROMONE_SEEDS.len(),
        field.len()
    );

    // Seed insights: synthesise InsightEvent::Posted events on the insight bus.
    for (idx, (kind, author, content)) in INSIGHT_SEEDS.iter().enumerate() {
        let mut id_bytes = [0u8; 16];
        id_bytes[0] = idx as u8 + 1;
        insight_bus.broadcast(InsightEvent::Posted {
            id: InsightId(id_bytes),
            kind: *kind,
            content: (*content).to_string(),
            author: author.as_bytes().to_vec(),
            created_at: now_secs,
        });
    }
    println!("seeded {} insight events", INSIGHT_SEEDS.len());

    // Snapshot the seed counts so each tick prints only what arrived during
    // the tick.
    let seed_pher = pher_sink.events().len();
    let seed_ins = ins_sink.events().len();
    println!("initial drain: {seed_pher} pheromone events, {seed_ins} insight events\n");
    let mut last_pher_len = seed_pher;
    let mut last_ins_len = seed_ins;

    // Synthetic new pheromones pushed during each tick.
    let tick_pheromones: [(PheromoneKind, &str, f32); 5] = [
        (PheromoneKind::Threat, "new rug on FRAX/DAI pool", 0.73),
        (
            PheromoneKind::Opportunity,
            "fresh just-in-time LP window",
            0.81,
        ),
        (
            PheromoneKind::Wisdom,
            "set gas premium on basefee spike",
            0.55,
        ),
        (PheromoneKind::Threat, "Pyth oracle deviation > 1.5%", 0.88),
        (
            PheromoneKind::Opportunity,
            "cross-chain rate spread 0.8%",
            0.66,
        ),
    ];

    for (tick, (kind, label, intensity)) in tick_pheromones.iter().enumerate() {
        now_secs += 45 * 60; // advance 45 minutes per tick
        let evaporated = field.evaporate(now_secs);

        // Emit one synthetic new pheromone (both field + bus).
        let vector = project_tokens(label);
        let _id = field.deposit(*kind, vector, *intensity, now_secs);
        pheromone_bus.broadcast(*kind, vector, *intensity, now_secs);

        // Also emit a Decayed insight event for one of the seeded entries so
        // the insight sink shows activity across ticks.
        let mut id_bytes = [0u8; 16];
        id_bytes[0] = (tick % INSIGHT_SEEDS.len()) as u8 + 1;
        insight_bus.broadcast(InsightEvent::Decayed {
            id: InsightId(id_bytes),
            new_weight: 1.0 - 0.1 * (tick + 1) as f32,
            at: now_secs,
        });

        let new_pher: Vec<PheromoneEvent> =
            pher_sink.events().into_iter().skip(last_pher_len).collect();
        let new_ins: Vec<InsightEvent> = ins_sink.events().into_iter().skip(last_ins_len).collect();
        last_pher_len += new_pher.len();
        last_ins_len += new_ins.len();

        println!(
            "tick {tick} (t+{}m): field={} evap={} | new events: {} pheromones, {} insights",
            (tick + 1) * 45,
            field.len(),
            evaporated,
            new_pher.len(),
            new_ins.len()
        );
        for ev in &new_pher {
            println!(
                "    pher#{} kind={:?} intensity={:.2}",
                ev.id, ev.kind, ev.intensity
            );
        }
        for ev in &new_ins {
            println!("    insight: {}", describe_insight(ev));
        }
    }

    // Summary.
    let pher_stats = pheromone_bus
        .stats(pher_sub_id)
        .unwrap_or(SubscriptionStats::zero());
    let ins_stats = insight_bus
        .stats(ins_sub_id)
        .unwrap_or(SubscriptionStats::zero());
    let (cnt_threat, cnt_opp, cnt_wisdom) = count_by_kind(&pher_sink.events());
    println!("\n== summary ==");
    println!("  pheromone field: {} live entries", field.len());
    println!(
        "  pheromone sub #{pher_sub_id}: delivered={} dropped_oldest={} dropped_newest={} closed={}",
        pher_stats.delivered,
        pher_stats.dropped_oldest,
        pher_stats.dropped_newest,
        pher_stats.closed
    );
    println!(
        "  insight   sub #{ins_sub_id}: delivered={} dropped_oldest={} dropped_newest={} closed={}",
        ins_stats.delivered, ins_stats.dropped_oldest, ins_stats.dropped_newest, ins_stats.closed
    );
    println!(
        "  observed pheromone mix: threat={cnt_threat} opportunity={cnt_opp} wisdom={cnt_wisdom}"
    );

    Ok(())
}

fn count_by_kind(events: &[PheromoneEvent]) -> (usize, usize, usize) {
    let mut t = 0;
    let mut o = 0;
    let mut w = 0;
    for ev in events {
        match ev.kind {
            PheromoneKind::Threat => t += 1,
            PheromoneKind::Opportunity => o += 1,
            PheromoneKind::Wisdom => w += 1,
        }
    }
    (t, o, w)
}

fn describe_insight(ev: &InsightEvent) -> String {
    match ev {
        InsightEvent::Posted {
            id, kind, content, ..
        } => format!(
            "posted #{:02x}{:02x} ({:?}): {}",
            id.0[0], id.0[1], kind, content
        ),
        InsightEvent::StateTransition { id, from, to, .. } => format!(
            "transition #{:02x}{:02x}: {:?} -> {:?}",
            id.0[0], id.0[1], from, to
        ),
        InsightEvent::Confirmed { id, .. } => {
            format!("confirmed #{:02x}{:02x}", id.0[0], id.0[1])
        }
        InsightEvent::Challenged { id, .. } => {
            format!("challenged #{:02x}{:02x}", id.0[0], id.0[1])
        }
        InsightEvent::Decayed { id, new_weight, .. } => format!(
            "decayed #{:02x}{:02x} -> weight {:.3}",
            id.0[0], id.0[1], new_weight
        ),
    }
}
