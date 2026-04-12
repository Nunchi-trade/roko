# MEV Detection and Protection (Chain Domain)

> **Layer**: L3 Harness (pre-flight simulation), L5 (chain-specific threat detection)
>
> **Crate**: Target: `roko-chain` (MEV detection), chain-domain safety extensions
>
> **Synapse traits**: `Gate` (verify transactions against MEV), `Scorer` (rate MEV exposure)
>
> **Prerequisites**: [08-threat-model.md](08-threat-model.md), [09-adaptive-risk.md](09-adaptive-risk.md)
>
> **Domain**: This document applies to chain-domain agents using `roko-chain`. General-purpose agents (code, research) do not face MEV threats.

---

## Overview

Maximal Extractable Value (MEV) is profit that block proposers and searchers extract by reordering, inserting, or censoring transactions within a block. For an autonomous trading agent, MEV is both a threat (transactions get sandwiched, reducing returns) and a signal (MEV patterns reveal market microstructure that informs decisions).

Daian et al. ("Flash Boys 2.0," IEEE S&P 2020, arXiv:1904.05234) established the theoretical framework. Zust et al. (ETH Zurich, 2021) identified 525,004 sandwich attacks over 12 months extracting 57,493 ETH (~$189M). Qin et al. ("Quantifying Blockchain Extractable Value," IEEE S&P 2022) formalized MEV profit calculations.

An agent that ignores MEV is donating money to searchers.

---

## MEV Attack Taxonomy

### Sandwich Attacks

The most common MEV attack against DEX traders. The attacker observes a pending swap, places a buy order before it (front-run) and a sell order after it (back-run), profiting from the price impact.

**Structure:**
1. Victim submits: swap X tokens for Y on DEX
2. Attacker front-runs: buy Y tokens, pushing the price up
3. Victim's swap executes at a worse price (higher cost for Y)
4. Attacker back-runs: sell Y tokens at the inflated price

### Front-Running

Pure front-running without a corresponding back-run. The attacker copies a profitable transaction (e.g., a liquidation or arbitrage) and submits it with higher gas priority.

### Back-Running

The attacker places a transaction immediately after a large trade to capture the arbitrage opportunity it creates. Less adversarial than sandwiching — the victim's transaction executes unmodified.

### JIT (Just-In-Time) Liquidity

The attacker observes a pending large swap on a concentrated liquidity AMM, provides concentrated liquidity in the exact tick range the swap will traverse, earns fees from the swap, and removes liquidity in the same block. Existing liquidity providers earn less in fees.

### Cyclic Arbitrage

Multi-hop trades through 2+ pools that start and end with the same token, profiting from price discrepancies across venues.

---

## Detection Algorithms

The MEV detector operates on block-level transaction data — either from mempool monitoring or post-execution block analysis.

### Sandwich Detection

Pattern: three transactions on the same pool where:
1. `tx_a` is a swap by address X in direction D
2. `tx_b` is a swap by address Y (victim) in direction D
3. `tx_c` is a swap by address X in direction !D
4. `tx_a.index < tx_b.index < tx_c.index`

The attacker (X) buys before the victim and sells after.

```rust
pub struct MevDetector {
    min_profit_threshold: U256,
    known_bots: HashMap<Address, String>,
}

pub struct SandwichBundle {
    pub attacker: Address,
    pub frontrun_tx: TxHash,
    pub victim_tx: TxHash,
    pub backrun_tx: TxHash,
    pub pool: Address,
    pub estimated_profit: U256,
    pub victim_impact_bps: u32,
}
```

### JIT Liquidity Detection

Pattern: within the same block on the same pool:
1. Address X adds concentrated liquidity
2. A large swap executes through that tick range
3. Address X removes liquidity

Fees earned = removal amounts minus addition amounts.

### Back-Run Detection

A swap by a known bot or high-gas-priority sender in the opposite direction, within 2 transaction indices of a large target swap on the same pool.

---

## Protection Strategies

### Pre-Flight Simulation

Before submitting any transaction, simulate it against a local fork of the chain state. The simulation detects:
- Expected price impact (compared to oracle prices)
- Gas cost relative to expected return
- Whether the transaction creates a sandwich opportunity (large price impact on low-liquidity pools)

If simulation shows the agent's transaction would be unprofitable after MEV extraction, the transaction is modified (smaller size, different timing) or cancelled.

### Private Mempool Routing

Transactions are submitted through private mempool services (Flashbots Protect on Ethereum/Base) that do not broadcast to the public mempool. This prevents searchers from observing the transaction before it is included in a block.

### Slippage Bounds

On-chain slippage bounds set the maximum acceptable price deviation:
- For swaps: `amountOutMinimum` parameter ensures the swap reverts if the price moves too far
- For LP operations: tick range limits concentration

The PolicyCage enforces these bounds in the smart contract — even if the agent's runtime is compromised, the on-chain bounds hold.

### Timing Strategies

- **Batch transactions**: Group multiple small transactions into a single larger one to reduce per-transaction MEV surface
- **Random delay**: Add small random delays between related transactions to break detectable patterns
- **Off-peak submission**: Submit non-time-sensitive transactions during low-MEV periods (weekends, low-volume hours)

---

## Integration with Gate Pipeline

The MEV detector integrates with Roko's Gate pipeline as a pre-execution Gate. Before any on-chain transaction is submitted, the MEV Gate runs:

```rust
pub struct MevGate {
    detector: MevDetector,
    max_acceptable_impact_bps: u32,
}

#[async_trait]
impl Gate for MevGate {
    async fn verify(&self, engram: &Signal) -> Result<Verdict> {
        // Extract transaction parameters from the Engram body
        let tx_params = parse_transaction_params(&engram.body)?;

        // Simulate against local fork
        let simulation = self.detector.simulate_transaction(&tx_params).await?;

        // Check if expected impact exceeds threshold
        if simulation.estimated_impact_bps > self.max_acceptable_impact_bps {
            return Ok(Verdict::Fail {
                reason: format!(
                    "MEV impact {}bps exceeds threshold {}bps",
                    simulation.estimated_impact_bps,
                    self.max_acceptable_impact_bps
                ),
                confidence: simulation.confidence,
            });
        }

        Ok(Verdict::Pass {
            confidence: simulation.confidence,
        })
    }
}
```

The MevGate sits in the chain-domain gate pipeline alongside other chain-specific gates (gas estimation, position limit, slippage bound). Gate verdicts are persisted as Engrams with full provenance, enabling forensic replay (see [15-forensic-ai.md](15-forensic-ai.md)) of why a transaction was submitted or rejected.

### Engram Flow for MEV Detection

When the MevDetector identifies a pattern, it produces Engrams that flow through the Synapse Loop:

1. **Observation Engram**: Raw block/mempool data containing the detected pattern
2. **Scorer**: Rates the pattern by estimated profit, confidence, and relevance
3. **Router**: Decides whether to store, alert, or act on the detection
4. **Policy**: If the pattern affects the agent's own transactions, emit a protective response
5. **Neuro**: Pattern stored as a `Kind::MarketSignal` for long-term learning

---

## MEV as Intelligence Signal

MEV patterns reveal market microstructure:

- **Sandwich frequency on a pool**: High sandwich rate indicates the pool is heavily monitored by searchers — the agent should route through alternative venues or use smaller order sizes
- **Arbitrage frequency between pools**: Frequent arbitrage between pool A and pool B indicates they are closely linked — the agent can use this for cross-venue price prediction
- **JIT liquidity presence**: Active JIT providers reduce the effective fee income for passive LPs — the agent should factor this into LP sizing decisions (see [09-adaptive-risk.md](09-adaptive-risk.md) §Layer 2)
- **Gas price spikes**: Sudden gas price increases often correlate with profitable MEV opportunities that searchers are competing for — this is a market microstructure signal

The `MevDetector` feeds detection results into the Neuro knowledge store as Engrams with `Kind::MarketSignal`, enabling the agent to learn MEV patterns over time and adapt its transaction strategy.

---

## Implementation Status

| Component | Status | Location |
|---|---|---|
| mirage-rs (in-process EVM simulator) | Built (141 tests) | `mirage-rs/` |
| MevDetector data structures | Design only | Target: `roko-chain` Tier 3 |
| Sandwich detection algorithm | Design only | Target: `roko-chain` Tier 3 |
| JIT liquidity detection | Design only | Target: `roko-chain` Tier 3 |
| Back-run detection | Design only | Target: `roko-chain` Tier 3 |
| MevGate integration | Design only | Target: `roko-gate` Tier 3 |
| Private mempool routing (Flashbots) | Design only | Target: `roko-chain` Tier 4 |

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Daian et al. (IEEE S&P 2020, arXiv:1904.05234) | Flash Boys 2.0 — foundational MEV framework |
| Qin et al. (IEEE S&P 2022) | Quantifying Blockchain Extractable Value |
| Zust et al. (ETH Zurich, 2021) | 525,004 sandwich attacks, 57,493 ETH extracted |
| Milionis et al. (2022) | LVR — Loss-Versus-Rebalancing for LP risk |
| Flashbots (2021) | MEV-Protect — private transaction submission |
| Kulkarni et al. (2023) | Towards a Theory of MEV — formal foundations |

---

## Related Topics

- [08-threat-model.md](08-threat-model.md) — MEV in the attack taxonomy
- [09-adaptive-risk.md](09-adaptive-risk.md) — Layer 5 domain threat detection
- [11-temporal-logic.md](11-temporal-logic.md) — Temporal verification of transaction sequences
