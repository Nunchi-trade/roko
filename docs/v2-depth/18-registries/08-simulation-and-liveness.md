# Simulation and Liveness

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How pre-flight EVM simulation emerges as a Connect Cell feeding the Verify protocol, and how on-chain heartbeat emerges as a Hot Flow publishing liveness Signals that anchor agent vitality to economic stake.

---

## 1. The Design Error This Corrects

The source architecture treated mirage-rs as a standalone application (`apps/mirage-rs/`) with its own RPC server, its own state management, and its own testing infrastructure. The chain agent heartbeat was described as a "9-step cognitive mapping" -- a domain-specific loop with its own terminology (OBSERVE, RETRIEVE, ANALYZE, GATE, SIMULATE, VALIDATE, EXECUTE, VERIFY, REFLECT) that duplicated the universal cognitive loop rather than specializing it.

The result: mirage-rs could not participate in the standard Verify pipeline. A `TxSimGate` was invented as a special gate type rather than using the standard Verify protocol with a simulation-backed evidence source. The heartbeat was a separate scheduling concern rather than an instance of the Hot Flow pattern that already governs agent execution.

This depth doc redesigns simulation as a **Connect Cell** (external system I/O with lifecycle management) that provides evidence to standard Verify Cells, and heartbeat as a **Hot Flow** (tick-driven Graph that stays resident) publishing liveness Signals to both the local Bus and the on-chain Store.

---

## 2. mirage-rs as Connect Cell

mirage-rs is an in-process EVM fork. In unified terms, it is a Connector specialization: a Cell implementing the Connect protocol that provides simulated chain execution as a service to other Cells.

```rust
/// SimulationConnector: a Connect Cell wrapping mirage-rs.
///
/// Provides pre-flight transaction simulation to any Cell that needs
/// to verify a chain operation before committing real capital.
///
/// Unlike ChainConnector (which talks to a live chain), SimulationConnector
/// talks to a local, ephemeral, forkable EVM instance. The key property:
/// simulations are free, instant, and reversible.
pub struct SimulationConnector {
    /// Cell identity.
    id: CellId,

    /// The mirage-rs instance (revm-based EVM simulator).
    mirage: Arc<MirageInstance>,

    /// Fork state: which block this simulation is forked from.
    /// Refreshed periodically to keep simulations accurate.
    fork_block: AtomicU64,

    /// Snapshot stack: enables checkpoint/restore for nested simulations.
    snapshots: Vec<SnapshotId>,

    /// Configuration: chain ID, block time, extensions enabled.
    config: SimulationConfig,
}

#[async_trait]
impl Connect for SimulationConnector {
    async fn connect(&mut self, config: &ConnectConfig) -> Result<()> {
        // Fork from the current head of the target chain.
        // After this, the simulation has a local copy of all chain state
        // and can execute transactions without network calls.
        self.mirage.fork(config.rpc_url(), config.fork_block()).await?;
        self.fork_block.store(config.fork_block(), Ordering::SeqCst);
        Ok(())
    }

    async fn query(&self, request: &QueryRequest) -> Result<QueryResponse> {
        // Simulate a transaction without modifying state.
        // Returns: success/failure, gas used, state diffs, events emitted.
        let result = self.mirage.simulate(&request.tx).await?;
        Ok(QueryResponse::Simulation(SimulationResult {
            success: result.success,
            gas_used: result.gas_used,
            state_diffs: result.state_diffs,
            events: result.events,
            return_data: result.output,
        }))
    }

    async fn execute(&self, request: &ExecuteRequest) -> Result<ExecuteResponse> {
        // Execute a transaction WITH state modification (in the local fork).
        // Used for multi-step simulation scenarios.
        let receipt = self.mirage.execute(&request.tx).await?;
        Ok(ExecuteResponse::Receipt(receipt))
    }

    async fn health(&self) -> HealthStatus {
        // Simulation health = fork freshness.
        // If the fork is more than N blocks stale, simulation confidence degrades.
        let staleness = current_block() - self.fork_block.load(Ordering::SeqCst);
        if staleness < 5 { HealthStatus::Healthy }
        else if staleness < 50 { HealthStatus::Degraded }
        else { HealthStatus::Unhealthy }
    }

    async fn disconnect(&mut self) -> Result<()> {
        // Drop all snapshots and release the fork state.
        self.snapshots.clear();
        self.mirage.reset().await?;
        Ok(())
    }
}
```

### Snapshot/Restore as Nested Verify

The snapshot mechanism enables a critical pattern: **simulate, verify, then decide whether to commit**. This is the Verify protocol with a simulation-backed evidence source:

```rust
/// Simulate a transaction, verify the outcome, revert the simulation state.
/// The real chain is never touched. Only if verification passes does the
/// agent proceed to actual execution.
async fn simulate_and_verify(
    sim: &SimulationConnector,
    verify: &dyn Verify,
    tx: &TransactionRequest,
) -> Result<SimulationVerdict> {
    // 1. Take a snapshot (checkpoint the current simulation state)
    let snapshot = sim.mirage.snapshot().await?;

    // 2. Execute the transaction in the simulation
    let result = sim.mirage.execute(tx).await?;

    // 3. Build a Signal from the simulation result (for Verify to evaluate)
    let evidence_signal = Signal::new(Kind::SimulationResult)
        .with_payload(result.clone())
        .with_provenance(Provenance::Simulated { fork_block: sim.fork_block() });

    // 4. Run the standard Verify protocol against the simulated result
    let verdict = verify.verify_pre(&evidence_signal).await;

    // 5. Revert: undo the simulation state change
    // The real chain was never modified.
    sim.mirage.revert(snapshot).await?;

    Ok(SimulationVerdict { result, verdict })
}
```

---

## 3. The Simulation Pipeline

Pre-flight simulation for chain operations is a **Pipeline Graph** -- a linear chain of Cells where each can reject, transform, or pass through:

```toml
# Pipeline: Simulation -> Verification -> Execution
# Expressed as a Graph definition in TOML.

[graph]
name = "chain.preflight"
kind = "pipeline"

[[cells]]
name = "simulate"
type = "SimulationConnector"
protocol = "connect"
config = { chain_id = 31337, fork_mode = "latest" }

[[cells]]
name = "confidence"
type = "ConfidenceScorer"
protocol = "score"
config = { min_confidence = 0.5 }

[[cells]]
name = "policy_check"
type = "PolicyVerify"
protocol = "verify"
config = { max_gas = 500000, max_slippage = 0.005, approved_assets = ["ETH", "USDC", "KORAI"] }

[[cells]]
name = "execute"
type = "ChainConnector"
protocol = "connect"
config = { rpc_url = "wss://korai-rpc.example.com" }

# Pipeline edges: simulate -> score confidence -> verify policy -> execute
[[edges]]
from = "simulate"
to = "confidence"

[[edges]]
from = "confidence"
to = "policy_check"
# If confidence < 0.5, pipeline short-circuits here (re-simulate with fresher fork)

[[edges]]
from = "policy_check"
to = "execute"
# If policy check fails, pipeline short-circuits here (do not execute)
```

### Simulation Confidence as Score

The source architecture's `SimulationConfidence` struct maps directly to the Score protocol. A Score Cell evaluates how likely the simulation matches reality:

```rust
/// ConfidenceScorer: a Score Cell that rates simulation fidelity.
///
/// Factors: state freshness, oracle independence, ordering independence,
/// contract verification status, gas estimation confidence.
pub struct ConfidenceScorer {
    id: CellId,
    min_confidence: f64,
}

#[async_trait]
impl Score for ConfidenceScorer {
    async fn score(&self, signal: &Signal) -> ScoreResult {
        let sim_result = signal.payload::<SimulationResult>()?;

        let freshness = compute_freshness(sim_result.fork_block);
        let oracle_indep = compute_oracle_independence(&sim_result.state_diffs);
        let ordering_indep = compute_ordering_independence(&sim_result.tx);
        let verification = compute_contract_verification(&sim_result.contracts_called);
        let gas_conf = compute_gas_confidence(sim_result.gas_used, &sim_result.tx);

        let confidence = freshness * 0.25
            + oracle_indep * 0.25
            + ordering_indep * 0.25
            + verification * 0.15
            + gas_conf * 0.10;

        if confidence < self.min_confidence {
            ScoreResult::BelowThreshold {
                score: confidence,
                recommendation: "Re-simulate with fresher fork state",
            }
        } else {
            ScoreResult::Pass { score: confidence }
        }
    }
}
```

### What mirage-rs Cannot Simulate

The honest boundaries of simulation fidelity are themselves expressible as Score factors:

| Factor | Impact on Confidence | Mitigation |
|---|---|---|
| MEV / tx ordering | `ordering_independence` drops to 0.3 for AMM swaps | Use private mempool for production |
| Cross-block state changes | `freshness` degrades with fork staleness | Re-fork immediately before execution |
| Private mempool (40-60% of block space) | Cannot be scored (invisible) | Accept as irreducible uncertainty |
| EVM implementation divergences | ~7.21% of contracts affected (OpDiffer 2025) | Differential testing against go-ethereum |
| Korai HDC precompile (Stylus) | Gas cost may differ from local emulation | Calibrate against testnet benchmarks |

---

## 4. Chain Heartbeat as Hot Flow

The chain agent heartbeat is a **Hot Flow**: a Graph that stays resident between firings, re-executing on each tick. In the unified model, there is no separate "heartbeat scheduler" -- the heartbeat is an instance of the same Hot Flow pattern used by all tick-driven agent behavior.

```rust
/// ChainHeartbeatFlow: a Hot Flow that publishes liveness Signals.
///
/// On each tick:
/// 1. Publish a heartbeat Pulse on the federated Bus (gossip network)
/// 2. Periodically (every N ticks): submit heartbeat to on-chain contract
/// 3. On-chain heartbeat maintains stake eligibility
///
/// If the heartbeat lapses (agent goes offline), on-chain stake becomes
/// slashable after a grace period.
pub struct ChainHeartbeatFlow {
    /// Flow identity (includes run_id for resume capability).
    id: FlowId,

    /// The agent's passport (carries stake info, tier, capabilities).
    passport: PassportSignal,

    /// Bus handle for publishing heartbeat Pulses.
    bus: BusHandle,

    /// ChainConnector for submitting on-chain heartbeat transactions.
    chain: Arc<ChainConnector>,

    /// Tick configuration.
    config: HeartbeatConfig,

    /// Liveness state: tracks missed ticks for alerting.
    state: HeartbeatState,
}

/// Heartbeat configuration.
pub struct HeartbeatConfig {
    /// How often to publish a gossip heartbeat Pulse.
    /// Default: every 700ms (one per GossipSub heartbeat interval).
    pub gossip_interval: Duration,

    /// How often to submit an on-chain heartbeat transaction.
    /// Default: every 100 blocks (~40 seconds on Korai at 400ms block time).
    pub chain_interval_blocks: u64,

    /// Grace period: how many missed on-chain heartbeats before slash eligibility.
    /// Default: 10 (= ~400 seconds = ~6.7 minutes of downtime).
    pub grace_period_heartbeats: u64,

    /// Stake at risk: percentage of domain stake slashable per missed period.
    /// Default: 1% per missed period beyond grace.
    pub slash_rate_per_period: f64,
}
```

### The Heartbeat Pulse

Each gossip heartbeat is a Pulse on `heartbeat.*` containing the agent's current state summary:

```rust
/// HeartbeatPulse: published on the federated Bus every tick.
/// Other agents use this for liveness detection and load balancing.
pub struct HeartbeatPulse {
    /// Which agent is alive.
    passport_id: u256,

    /// Current tier (Protocol/Sovereign/Worker/Edge).
    tier: PassportTier,

    /// Capability bitmask (what this agent can do).
    capabilities: u64,

    /// Current load factor: 0.0 = idle, 1.0 = fully loaded.
    /// Used by the job marketplace for load-aware assignment.
    load_factor: f64,

    /// Currently active domains (what the agent is working on).
    active_domains: Vec<String>,

    /// Active job count.
    active_jobs: u32,

    /// Software version (for compatibility checking).
    version: String,
}
```

### On-Chain Heartbeat as Vitality Anchor

The on-chain heartbeat ties the agent's network liveness to its economic stake. This is the bridge between the abstract vitality model (see unified spec 05-AGENT) and concrete economic incentives:

```
On-chain vitality = heartbeat_regularity x stake x reputation

Where:
  heartbeat_regularity = (heartbeats_sent / heartbeats_expected) over trailing window
  stake = sum of all domain_stakes in passport
  reputation = average reputation across staked domains

If vitality drops below threshold:
  - Agent enters "Conservation" behavioral phase (reduced autonomy)
  - Job marketplace reduces assignment priority
  - After grace period: stake becomes slashable
```

This creates a **feedback loop**: an agent that goes offline loses vitality, which reduces job flow, which reduces revenue, which reduces the ability to maintain stake, which further reduces vitality. The economic pressure ensures honest liveness reporting.

---

## 5. The Chain Cognitive Loop (9 Steps as Cell Composition)

The source architecture's 9-step chain heartbeat (OBSERVE, RETRIEVE, ANALYZE, GATE, SIMULATE, VALIDATE, EXECUTE, VERIFY, REFLECT) is not a custom loop. It is the standard cognitive loop with domain-specific Cells:

```toml
# The chain agent's cognitive loop as a Graph.
# This is an instance of the universal Loop pattern with chain-specific Cells.

[graph]
name = "chain.cognitive"
kind = "loop"
tick = "heartbeat.theta"  # Fires on theta tick (deliberative speed)

# Step 1-2: SENSE (OBSERVE + RETRIEVE)
[[cells]]
name = "sense"
type = "ChainWitnessFeed"
protocol = "connect"
config = { chains = ["ethereum", "korai"], filter = "binary_fuse" }

# Step 3: ASSESS (ANALYZE)
[[cells]]
name = "assess"
type = "Triage"
protocol = "score"
config = { curiosity_threshold = 0.3 }

# Step 4: ROUTE (GATE)
[[cells]]
name = "route"
type = "EfeRouter"
protocol = "route"
config = { tiers = ["ignore", "monitor", "act"] }

# Step 5-6: VERIFY PRE (SIMULATE + VALIDATE)
[[cells]]
name = "simulate"
type = "SimulationConnector"
protocol = "connect"

[[cells]]
name = "validate"
type = "PolicyVerify"
protocol = "verify"
config = { position_limit_usd = 10000, max_gas = 500000, slippage = 0.005 }

# Step 7: ACT (EXECUTE)
[[cells]]
name = "execute"
type = "ChainConnector"
protocol = "connect"
config = { custody_mode = "local_key" }

# Step 8: VERIFY POST
[[cells]]
name = "verify_post"
type = "DiffVerify"
protocol = "verify"
config = { compare = "simulation_vs_actual" }

# Step 9: REACT (REFLECT)
[[cells]]
name = "reflect"
type = "EpisodeReact"
protocol = "react"
config = { persist = true, learn = true, publish_knowledge = true }

# Edges (with conditional routing)
[[edges]]
from = "sense"
to = "assess"

[[edges]]
from = "assess"
to = "route"

[[edges]]
from = "route"
to = "simulate"
condition = "tier == act"  # Only simulate if routing decision is "act"

[[edges]]
from = "simulate"
to = "validate"

[[edges]]
from = "validate"
to = "execute"
condition = "verdict == pass"  # Only execute if validation passes

[[edges]]
from = "execute"
to = "verify_post"

[[edges]]
from = "verify_post"
to = "reflect"

# Feedback edge (Loop pattern): reflect -> sense
[[edges]]
from = "reflect"
to = "sense"
kind = "feedback"
```

The three cognitive speeds map to tick frequencies:

| Speed | Tick Topic | Steps Driven | Example |
|---|---|---|---|
| Gamma (fast) | `heartbeat.gamma.tick` | SENSE, ROUTE | Consume queued chain Pulses, quick T0/T1/T2 routing |
| Theta (medium) | `heartbeat.theta.tick` | Full loop when route=act | Simulation, validation, execution |
| Delta (slow) | `heartbeat.delta.tick` | REFLECT (deep) | Episode consolidation, knowledge publication, prediction calibration |

---

## 6. Current Implementation Status

The source document catalogs 6 planned Solidity contracts and the implementation state of the chain layer. Expressed as a Cell readiness matrix:

| Cell | Crate | Rust Code | Contract | Status |
|---|---|---|---|---|
| SimulationConnector (mirage-rs) | `apps/mirage-rs/` | Built (141 tests) | N/A (local) | **Shipping** |
| ChainConnector | `crates/roko-chain/` | Trait defined, mock impl | N/A | **Built (mock)** |
| ChainHeartbeatFlow | `crates/roko-chain/` | Not yet | Agent Registry (`0xA100`) | **Tier 6** |
| PolicyVerify (chain) | `crates/roko-chain/` | Stub | N/A | **Stub** |
| ConfidenceScorer | Not yet | Not yet | N/A | **Tier 6** |
| DiffVerify (sim vs actual) | Not yet | Not yet | N/A | **Tier 6** |

### The Six Contracts as Store Cells

The 6 planned Solidity contracts are Store Cells with on-chain backends:

| Contract | Address | Unified Expression | Build Order |
|---|---|---|---|
| KORAI Token | Genesis | Store Cell (ERC-20 + demurrage + ERC-3009) | 1st |
| Agent Registry | `0xA100` | Store Cell (ERC-721 soulbound passports) | 2nd |
| Reputation Registry | `0xA200` | Store Cell (EMA scores + feedback auth) | 3rd |
| Validation Registry | `0xA300` | Store Cell (work proofs + gate results) | 4th |
| Escrow | Governance | Store Cell (job budget custody) | 5th |
| Marketplace (Spore) | Governance | Graph Cell (job lifecycle orchestration) | 6th |

The dependency chain: KORAI Token -> Agent Registry -> Reputation Registry -> Validation Registry -> Escrow -> Marketplace.

---

## 7. The Simulation-to-Execution Pipeline

The full lifecycle of a chain operation, from idea to on-chain commitment:

```
1. PROPOSE: Agent decides to execute a chain operation.
   - Signal: Kind::Proposal { tx: TransactionRequest }

2. SIMULATE: SimulationConnector executes the tx in the local fork.
   - Takes snapshot, executes, captures result, reverts.
   - Signal: Kind::SimulationResult { success, gas, diffs, confidence }

3. SCORE: ConfidenceScorer rates simulation fidelity.
   - If score < 0.5: re-fork and re-simulate (Pipeline short-circuit).
   - Signal: Kind::Score { confidence: 0.87 }

4. VERIFY PRE: PolicyVerify checks safety constraints.
   - Position limits, approved assets, gas budget, slippage tolerance.
   - If Verdict::Reject: abort (Pipeline short-circuit).
   - Signal: Kind::Verdict { pass: true, evidence: PolicyEvidence }

5. EXECUTE: ChainConnector signs and submits the real transaction.
   - Wallet signs, RPC submits, waits for receipt.
   - Signal: Kind::Receipt { tx_hash, block, gas_used, status }

6. VERIFY POST: DiffVerify compares simulation prediction vs. actual.
   - Gas difference? State diff mismatch? Unexpected events?
   - Feeds prediction error into calibration loop.
   - Signal: Kind::Verdict { pass: true, prediction_error: 0.03 }

7. PERSIST: Store Cell records the episode.
   - Episode Signal in `.roko/episodes.jsonl`.
   - On-chain: Validation Registry records work proof.

8. REACT: EpisodeReact updates agent models.
   - Calibrate gas estimates.
   - Update Daimon somatic markers (positive outcome -> positive affect for similar patterns).
   - Publish knowledge if episode revealed useful insight.
```

---

## 8. Differential Testing as Verify Calibration

mirage-rs includes differential testing: replay a real mainnet transaction through simulation and compare results. In unified terms, this is the **predict-publish-correct** Loop applied to simulation fidelity:

```rust
/// DifferentialCalibration: a Loop that improves simulation accuracy.
///
/// 1. PREDICT: Simulate a transaction before execution.
/// 2. PUBLISH: Record the prediction as a Pulse.
/// 3. EXECUTE: Submit the real transaction.
/// 4. OBSERVE: Read the actual result from chain.
/// 5. CORRECT: Compare prediction vs. actual, update calibration factors.
///
/// Over time, confidence scoring improves because the Score Cell
/// learns the systematic biases of its simulation backend.
pub struct DifferentialCalibration {
    /// Running statistics: gas prediction error by tx type.
    gas_error_by_type: BTreeMap<TxType, RunningStats>,

    /// Running statistics: state diff accuracy.
    state_accuracy: RunningStats,

    /// Calibration adjustments applied to future simulations.
    adjustments: CalibrationAdjustments,
}

pub struct CalibrationAdjustments {
    /// Multiply gas estimates by this factor (learned from historical error).
    /// Starts at 1.0; if simulations consistently underestimate gas by 15%,
    /// converges to 1.15.
    pub gas_multiplier: f64,

    /// Reduce confidence by this amount for oracle-dependent transactions
    /// (learned from historical oracle divergence).
    pub oracle_confidence_penalty: f64,

    /// Reduce confidence by this amount for MEV-exposed transactions.
    pub mev_confidence_penalty: f64,
}
```

---

## What This Enables

1. **Simulation as standard infrastructure**: Any Cell in any Graph can request a simulation via the SimulationConnector. Code intelligence Cells can simulate contract deployments. DeFi Cells can simulate trade sequences. Research Cells can simulate governance proposals. Simulation is not chain-specific -- it is a general-purpose "what-if" service.

2. **Economic liveness guarantees**: On-chain heartbeat ties agent availability to economic stake. An agent that claims to be available but is not loses stake. This creates honest signaling for the job marketplace -- agents with regular heartbeats and high stake are genuinely available.

3. **Progressive trust escalation**: The Pipeline pattern (simulate -> score -> verify -> execute -> verify_post) ensures that capital-at-risk operations pass multiple checkpoints. Each checkpoint can short-circuit the pipeline, preventing losses. Trust is earned through each stage, not assumed.

4. **Calibration from experience**: The predict-publish-correct Loop applied to simulation means that simulation accuracy improves over time. An agent that has executed 1000 transactions has calibrated gas estimates, learned MEV patterns, and identified oracle sensitivity -- all automatically through the standard learning machinery.

5. **Domain-agnostic cognitive loop**: The chain agent's 9-step heartbeat is just the universal cognitive loop with chain-specific Cells. Adding a new domain (e.g., "social media agent") requires only new Cells for SENSE and ACT -- the ASSESS, ROUTE, VERIFY, PERSIST, REACT steps are reusable.

---

## Feedback Loops

1. **Simulation accuracy -> confidence -> execution gate**: As simulation accuracy improves (through differential calibration), confidence scores increase, which means fewer operations are blocked by the "re-simulate" short-circuit. Better simulation -> more efficient execution.

2. **Heartbeat regularity -> vitality -> job flow -> revenue -> stake maintenance**: Regular heartbeats maintain high vitality, which maintains job priority, which generates revenue, which maintains stake, which enables continued operation. Irregular heartbeats trigger the opposite cascade.

3. **Post-execution verification -> prediction calibration -> pre-execution simulation**: Every actual execution provides ground truth for calibrating future simulations. The DiffVerify Cell's output feeds back into the ConfidenceScorer's calibration, making future simulations more accurate.

4. **Slash history -> behavioral caution -> smaller positions -> lower risk -> better reputation**: Agents with slash history (recorded in passport) adopt more conservative PolicyVerify thresholds, which reduces position sizes, which reduces risk of future slashing, which allows reputation recovery.

---

## Open Questions

1. **Simulation freshness vs. cost**: Forking the chain is expensive (requires RPC calls to fetch remote state). How often should the SimulationConnector re-fork? Every block (400ms on Korai) is too expensive. Every 100 blocks (~40s) may be too stale for time-sensitive operations. Should freshness be demand-driven (re-fork only when a simulation is requested)?

2. **Heartbeat gas cost**: Submitting an on-chain heartbeat transaction costs gas. If KORAI gas is expensive, heartbeat costs could eat into agent revenue. Should heartbeats be batched (one transaction covers N heartbeat periods)? Should there be a heartbeat subsidy for low-tier agents?

3. **Simulation of simulations**: If agent A simulates a transaction that would trigger agent B's simulation (e.g., A's swap changes the price that B is monitoring), can mirage-rs model multi-agent interactions? This is the "agents modeling agents" problem -- computationally expensive but necessary for accurate MEV prediction.

4. **Heartbeat manipulation**: An agent could run a minimal heartbeat process that publishes liveness Signals while the actual agent logic is offline (zombie heartbeat). How to detect this? One approach: heartbeat Pulses must include a proof-of-work (hash of recent Bus activity) that requires actually processing gossip.

5. **Formal verification scope**: mirage-rs supports Certora, Halmos, and Kontrol for formal verification of Korai contracts. Which properties should be formally verified vs. fuzz-tested vs. unit-tested? What is the cost/benefit of formal verification for each of the 6 contracts?

---

## Implementation Tasks

1. **Wrap mirage-rs as `SimulationConnector` Cell** implementing Connect protocol in `crates/roko-chain/src/simulation.rs`. The existing `apps/mirage-rs/` code becomes the backend; the Cell provides the protocol-compliant interface.

2. **Implement `ConfidenceScorer` Cell** implementing Score protocol, computing simulation fidelity from freshness, oracle independence, ordering independence, contract verification, and gas confidence factors.

3. **Implement `PolicyVerify` Cell** implementing Verify protocol for chain-specific safety constraints: position limits, approved assets, gas budget, slippage tolerance, exposure limits.

4. **Define `ChainHeartbeatFlow`** as a Hot Flow in `crates/roko-chain/src/heartbeat.rs` that publishes HeartbeatPulse on the federated Bus and periodically submits on-chain heartbeat transactions.

5. **Implement `DiffVerify` Cell** implementing Verify protocol that compares simulation predictions against actual execution results, computing prediction error metrics.

6. **Implement `DifferentialCalibration` Loop** that feeds post-execution ground truth back into simulation confidence scoring, maintaining running statistics per transaction type.

7. **Define the pre-flight Pipeline Graph** (`chain.preflight`) in TOML: SimulationConnector -> ConfidenceScorer -> PolicyVerify -> ChainConnector, with short-circuit conditions at each stage.

8. **Define the chain cognitive Loop Graph** (`chain.cognitive`) in TOML: the 9-step chain heartbeat expressed as standard Cells with conditional edges and three-speed tick routing.

9. **Implement heartbeat slashing logic** in the Agent Registry contract: if `blocks_since_last_heartbeat > grace_period`, slash `slash_rate_per_period` of domain stakes.

10. **Connect mirage-rs chain extensions** to `roko-primitives` for HDC precompile emulation and to passport/reputation structs for registry emulation, enabling full Korai chain simulation locally.
