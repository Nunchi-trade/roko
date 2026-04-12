# Chain Heartbeat Variant

> Chain agents add SIMULATE (mirage-rs pre-flight) and VALIDATE (position limits) steps between ATTEND and ACT because chain actions are financially irreversible.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md), [01-universal-loop-mapping.md](./01-universal-loop-mapping.md)
**Key sources**: `refactoring-prd/05-agent-types.md` §3, legacy `bardo-backup/prd/01-golem/02-heartbeat.md`

---

## Abstract

The universal Synapse loop is domain-agnostic — it handles coding, research, operations, and any custom domain without modification. However, chain agents interact with blockchains where actions are financially irreversible. A submitted transaction cannot be undone. A failed swap burns gas. A liquidation event destroys capital.

To address this, chain agents extend the universal loop with two additional steps between ATTEND and ACT: **SIMULATE** (run the proposed transaction in a local EVM fork via mirage-rs) and **VALIDATE** (check against position limits, approved assets, and safety constraints). These steps do not modify the universal loop — they are domain-specific Gate implementations that inject into the existing pipeline.

This document specifies the full chain heartbeat mapping, explains why the two additional steps exist, describes the mirage-rs simulation capability, and provides the complete 11-step chain heartbeat table.

---

## Why Chain Actions Need Extra Steps

### Financial Irreversibility

In the coding domain, most actions are reversible. A bad commit can be reverted. A broken test can be fixed. The cost of a mistake is time, not money.

In the chain domain, actions are irreversible by design:

| Action | Reversibility | Cost of Mistake |
|---|---|---|
| Submit a swap transaction | Irreversible once mined | Gas burned + slippage loss + potential sandwich attack |
| Provide liquidity to a pool | Reversible (withdraw) but with IL | Impermanent loss, withdrawal fee, gas |
| Execute a trade | Irreversible | Full position loss in worst case |
| Interact with a malicious contract | Irreversible | Complete wallet drain possible |
| Deploy a contract | Irreversible (mutable proxy aside) | Gas + deployment cost + any funds sent |

The two additional steps — SIMULATE and VALIDATE — provide a safety net before committing capital. They are the DeFi equivalent of a dry-run compilation: verify the action will succeed before executing it for real.

### The Cost of Skipping Verification

Without SIMULATE: The agent submits a swap, the transaction reverts on-chain, gas is burned, the swap fails, and the agent has no explanation for why. In MEV-intensive environments, the transaction might succeed but at a worse price than expected due to frontrunning.

Without VALIDATE: The agent accumulates positions beyond risk limits, exceeds approved asset lists, or violates position sizing constraints. These are policy violations that could result in financial harm.

---

## Full Chain Heartbeat Mapping (11 Steps)

The chain heartbeat is the universal Synapse loop plus two domain-specific steps:

```
Universal Loop          Chain Heartbeat                    Synapse Trait
─────────────          ────────────────                   ────────────
PERCEIVE            1. OBSERVE (blocks, logs, prices)     Substrate.query()
EVALUATE            2. RETRIEVE (4-factor scoring)        Scorer.score()
                    3. ANALYZE (PAD, regime shifts)       [Daimon cross-cut]
ATTEND              4. GATE (T0/T1/T2 routing)           Router.select()
                    5. SIMULATE (mirage-rs pre-flight)    [domain-specific Gate]
                    6. VALIDATE (PolicyCage, limits)      Gate.verify()
INTEGRATE           7. COMPOSE (context assembly)         Composer.compose()
ACT                 8. EXECUTE (submit tx, invoke tools)  Agent.execute()
VERIFY              9. VERIFY (predicted vs. actual)      Gate.verify()
PERSIST            10. PERSIST (store with lineage)       Substrate.put()
ADAPT              11. REFLECT (episode, predictions)     Policy.decide()
+ META-COGNIZE         (Daimon self-assessment)           Daimon.assess()
```

### Step 5: SIMULATE (Chain-Specific)

Run the proposed transaction in a local EVM fork before broadcasting it to the network. This step uses mirage-rs, Roko's in-process EVM simulator:

**What mirage-rs checks:**
- **Revert detection**: Will the transaction revert? If so, why? (insufficient balance, slippage too low, contract paused, etc.)
- **Gas estimation**: What will the actual gas cost be? Is it within budget?
- **State change verification**: What state changes will the transaction produce? Are they expected?
- **Sandwich attack vulnerability**: Can a frontrunner extract value from this transaction?
- **Price impact calculation**: What is the actual price impact of this swap given current pool state?
- **Multi-step simulation**: For complex strategies (flash loan → swap → deposit → borrow), simulate the entire chain in sequence.

mirage-rs is an in-process EVM simulator built on revm (Rust EVM implementation). It runs locally — no network calls, no external dependencies, no gas costs. Simulation takes 1-50ms depending on transaction complexity.

```rust
// Simplified mirage-rs simulation step
pub async fn simulate_transaction(
    state: &AgentState,
    deliberation: &Deliberation,
) -> Result<SimulationResult> {
    let fork = mirage_rs::fork_latest(state.rpc_url()).await?;
    let sim = fork.simulate(deliberation.proposed_tx())?;

    SimulationResult {
        success: sim.success,
        gas_used: sim.gas_used,
        state_changes: sim.state_diff,
        revert_reason: sim.revert_reason,
        price_impact: compute_price_impact(&sim),
        sandwich_vulnerable: detect_sandwich_vulnerability(&sim, state),
    }
}
```

**When SIMULATE is skipped:**
- T0 ticks (no LLM call, no action proposed)
- T1 ticks where the LLM recommends no action
- Read-only operations (queries, balance checks)

### Step 6: VALIDATE (Chain-Specific)

Check the simulated transaction against safety constraints before execution:

**Safety checks:**
- **Position limits**: Maximum position size per asset, maximum total exposure, maximum number of concurrent positions.
- **Approved asset list**: Only interact with whitelisted tokens and protocols.
- **Slippage tolerance**: Maximum acceptable slippage per swap.
- **Gas budget**: Maximum gas willing to spend per transaction.
- **Rate limits**: Maximum transactions per time period.
- **Capability tokens**: Typed, unforgeable authorization tokens that the type system enforces at compile time. A `WriteTool` call requires a `Capability<WriteTool>` token — the type system prevents execution without a valid token, even if the LLM is compromised.
- **Risk engine**: Five-layer risk assessment (Kelly sizing, adaptive guardrails, portfolio-level constraints).

If validation fails, the transaction is rejected and the agent receives feedback about why. The failure feeds back into the Daimon (decreased Pleasure) and the CascadeRouter (this strategy path failed).

---

## Three Custody Modes

Chain agents operate in one of three custody modes that determine how transaction signing works:

| Mode | Key Storage | Authorization | Use Case |
|---|---|---|---|
| **Delegation** (enclave) | Secure hardware (HSM, TEE) | Time-delayed, multi-sig | Production deployment with real capital |
| **Embedded** (ERC-4337) | Account abstraction contract | Session keys, gas sponsorship | Production deployment with smart wallet |
| **Local key** (dev) | Local keystore file | Direct signing, no delay | Development and testing with mirage-rs |

The custody mode affects the EXECUTE step: delegation mode requires waiting for time-delay approval, embedded mode uses session key signing, and local mode signs immediately. The SIMULATE and VALIDATE steps are custody-mode-independent — they always run locally.

---

## Sleepwalker Variant (Observer Mode)

The Sleepwalker is a named capability configuration (not a separate agent type) where the agent observes without acting. Setting `phenotype = "sleepwalker"` in `roko.toml` activates a reduced 3-step heartbeat:

```
OBSERVE  → Deterministic probes (free, $0.00, ~80% of ticks suppress here)
REFLECT  → LLM analysis of anomalies / regime changes / hypothesis validation
PUBLISH  → Push confirmed insights to Agent Mesh with pricing metadata
```

The Sleepwalker variant:
- **Never reaches ACT**: No SIMULATE, no VALIDATE, no EXECUTE. The `observatory` tool profile loads only read-only tools — the type system prevents write-tool invocation at compile time.
- **Enhanced dreaming**: ~40% of runtime budget allocated to dreams (vs. 10-20% for execution agents). More synthesis, less execution replay.
- **Revenue model**: Earns by selling typed knowledge artifacts (Insights, Warnings, CausalLinks) through the Agent Mesh marketplace, not by trading.
- **Dream allocation shift**: REM (imagination + hypothesis) takes 45-55% of dream time (vs. 30-40% for standard agents). NREM (replay + consolidation) takes 35-40% (vs. 40-50%).

| Dimension | Standard Agent | Sleepwalker |
|---|---|---|
| Tool profile | Per strategy | `observatory` (read-only) |
| Heartbeat path | Full 11-step chain heartbeat | OBSERVE → REFLECT → PUBLISH |
| Signing mode | Full trading key | Receive-only address |
| Dream REM ratio | 30-40% | 45-55% |
| Primary revenue | Vault fees + trading | Artifact sales via Mesh marketplace |

The name fits: a Sleepwalker moves through the environment in a continuous state of half-dream, perceiving without acting, synthesizing without executing.

### Sleepwalker-Specific Dream Triggers

Three triggers unique to the Sleepwalker phenotype (in addition to the standard SleepPressure-based schedule):

1. **Hypothesis density**: Staging buffer holds >15 unvalidated hypotheses.
2. **Regime shift**: Domain state classification changes (e.g., `calm` → `volatile`).
3. **Anomaly cluster**: >3 anomalies of the same type fire within one observation window.

---

## Comparison: Universal vs. Chain vs. Sleepwalker

| Step | Universal Loop | Chain (Full) | Sleepwalker |
|---|---|---|---|
| 1. PERCEIVE/OBSERVE | Substrate.query() | Blocks, logs, prices, probes | Probes only |
| 2. EVALUATE/RETRIEVE | Scorer.score() | 4-factor Neuro scoring | 4-factor Neuro scoring |
| 3. ANALYZE | Daimon cross-cut | PAD + regime detection | PAD + regime detection |
| 4. ATTEND/GATE | Router.select() | T0/T1/T2 gating | T0/T1 gating (no T2 actions) |
| 5. SIMULATE | _(skip)_ | mirage-rs EVM fork | _(skip)_ |
| 6. VALIDATE | _(skip)_ | PolicyCage, limits | _(skip)_ |
| 7. INTEGRATE/COMPOSE | Composer.compose() | AttentionAuction (VCG) | PromptComposer |
| 8. ACT/EXECUTE | Agent.execute() | Submit tx, invoke tools | _(skip — never acts)_ |
| 9. VERIFY | Gate.verify() | Blockchain receipt | _(skip)_ |
| 10. PERSIST | Substrate.put() | On-chain + off-chain | Off-chain only |
| 11. ADAPT/REFLECT | Policy.decide() | Episode + prediction + C-Factor | Episode + publish |
| META-COGNIZE | Daimon.assess() | "Should I reduce exposure?" | "Am I producing useful insights?" |

---

## Academic Foundations

- **Sumers et al. 2023** — CoALA framework (arXiv:2309.02427). The theoretical basis for the decision cycle.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176). Cascade cost optimization validating the T0/T1/T2 approach.
- **Kahneman 2011** — "Thinking, Fast and Slow". Dual-process theory underlying the tier gating system.

---

## Current Status and Gaps

**What exists:**
- `mirage-rs` (141 tests) implements the in-process EVM simulator for chain transaction simulation.
- `bardo-primitives/src/tier.rs` defines `InferenceTier` (T0/T1/T2) and `TierRouter`.
- The gate pipeline in `roko-gate` supports chained gate execution.

**What is missing:**
- The formal SIMULATE injection point in the orchestration loop for chain agents.
- The PolicyCage VALIDATE step is not yet wired into the Roko orchestration pipeline (it exists in legacy code).
- Sleepwalker phenotype configuration is designed but not implemented in `roko-cli`.

---

## Cross-References

- See [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md) for the CoALA theoretical foundation
- See [01-universal-loop-mapping.md](./01-universal-loop-mapping.md) for the universal Synapse loop
- See topic [08-chain](../08-chain/INDEX.md) for the full chain domain specification
- See topic [04-verification](../04-verification/INDEX.md) for the gate pipeline
- See topic [11-safety](../11-safety/INDEX.md) for capability tokens and safety constraints
