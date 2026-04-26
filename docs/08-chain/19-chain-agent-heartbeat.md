# Chain Agent Heartbeat: 9-Step Cognitive Mapping

> The chain agent's heartbeat is the chain-domain specialization of the canonical seven-step loop, driven by `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses. The historical 9-step chain wording remains useful as a fine-grained decomposition inside that tick-driven loop, especially for SIMULATE and VALIDATE before capital-at-risk actions.

> See also `tmp/refinements/09-phase-2-implications.md` and [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).


> **Implementation**: Specified

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md), [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md), [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md)
**Key sources**: `refactoring-prd/05-agent-types.md` §3, `bardo-backup/tmp/agent-chain/01-overview.md`, `roko/tmp/implementation-plans/12b-chain-layer.md` §H

---

## Abstract

Every Roko agent runs the canonical seven-step cognitive loop defined by the Synapse architecture (see topic [01-synapse](../00-architecture/INDEX.md)): SENSE → ASSESS → COMPOSE → ACT → VERIFY → PERSIST + BROADCAST → REACT. The chain agent's heartbeat is a domain-specific parameterization of this loop, with the historical 9-step chain flow used as a finer-grained breakdown inside the tick-driven runtime.

The critical additions are SIMULATE (step 5) and VALIDATE (step 6) — pre-flight checks that do not exist in the coding agent's baseline. Before committing capital, the chain agent simulates the proposed action in mirage-rs and validates it against safety policies. These checks live inside the universal `VERIFY` step rather than creating a separate kernel loop.

REF09 makes the runtime framing explicit: `HeartbeatPolicy` publishes `heartbeat.*` Pulses on the Bus; ChainWitness turns relevant chain activity into `chain.*` Pulses on `ChainBus`; the chain agent consumes those topics and queries `ChainSubstrate` for durable on-chain state. The result is ordinary Bus/Substrate composition rather than a bespoke chain scheduler.

---

## The 9-Step Mapping

```
Canonical Loop      Chain Heartbeat Detail                             Primary Fabric / Operator
──────────────      ──────────────────────                             ─────────────────────────
SENSE               1. OBSERVE (consume `chain.*` Pulses)             ChainBus.subscribe()
                    2. RETRIEVE (query durable chain state)           ChainSubstrate.query()
ASSESS              3. ANALYZE (curiosity, regime shifts)             Scorer + [Daimon cross-cut]
                    4. GATE (T0/T1/T2 routing)                        Router.select()
COMPOSE             assemble tx context, custody, and policy bundle   Composer.compose()
ACT                 7. EXECUTE (submit tx, invoke tools)              Agent.execute()
VERIFY              5. SIMULATE + 6. VALIDATE + 8. VERIFY             Gate.verify()
PERSIST             write episode, receipts, and durable outcomes     Substrate.put()
BROADCAST           publish `chain.*`, `gate.*`, `heartbeat.*`        Bus.publish() / ChainBus.publish()
REACT               9. REFLECT (episode, predictions, policy followup) Policy.decide()
```

### Step 1: OBSERVE

The agent perceives on-chain state through the ChainWitness pipeline and `ChainBus` topics:

- **Input**: Block headers, transaction logs, price feeds via WebSocket subscription
- **Processing**: Binary Fuse filter pre-screening (see [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md))
- **Output**: Filtered, normalized `chain.*` Pulses published on `ChainBus` and durable state queried from `ChainSubstrate`
- **Synapse mapping**: `ChainBus.subscribe()` + `ChainSubstrate.query()` — live chain transport plus durable reads

This step runs continuously in its own Tokio task, outside the heartbeat clock. The heartbeat gates the agent's *cognitive response* to what it sees, not the seeing itself. Block ingestion at 12-second intervals (Ethereum) or 50ms intervals (Nunchi) is too fast for deliberative processing at every block, so the gamma consumer drains buffered `chain.*` Pulses when `heartbeat.gamma.tick` fires.

### Step 2: RETRIEVE

The agent retrieves relevant knowledge to contextualize what it observed:

- **Input**: Events from the triage pipeline with curiosity scores
- **Processing**: 4-factor knowledge retrieval from NeuroStore:
  1. **Semantic similarity**: HDC vector comparison between the event and stored knowledge
  2. **Recency**: More recent entries score higher
  3. **Confidence**: Entries with more confirmations score higher
  4. **Relevance**: Domain-specific relevance to the current event type
- **Output**: Ranked context pack of relevant knowledge entries
- **Synapse mapping**: `Scorer.score()` — ranking knowledge by relevance

### Step 3: ANALYZE

The agent analyzes the combined observation + context:

- **Input**: Triage-scored events + retrieved knowledge context
- **Processing**:
  - Curiosity scoring (see [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md))
  - Regime shift detection: has the market environment changed? (volatility spike, liquidity drain, correlation break)
  - Daimon affect assessment: how does the agent "feel" about this situation? (somatic markers from past similar situations)
- **Output**: Analysis summary with affect valence and recommended action class
- **Synapse mapping**: Daimon cross-cut — the agent's affect system modulates analysis

The Daimon (see topic [07-daimon](../09-daimon/INDEX.md)) provides somatic markers — fast heuristic signals based on accumulated experience. If the agent has been burned by similar patterns before, the somatic marker is negative, triggering more cautious downstream behavior (smaller position sizes, stronger simulation requirements).

### Step 4: GATE (Routing)

The agent decides how to respond to the analysis:

- **Input**: Analysis summary with affect valence
- **Processing**: Three-tier routing decision:
  - **T0 (ignore)**: Low curiosity, negative or neutral affect → log and continue
  - **T1 (monitor)**: Moderate curiosity → add to watchlist, update statistical models
  - **T2 (act)**: High curiosity or positive affect → proceed to simulation
- **Output**: Routing decision (ignore / monitor / act)
- **Synapse mapping**: `Router.select()` — choosing the response tier

### Step 5: SIMULATE (Chain-Specific)

If the routing decision is T2 (act), the agent simulates the proposed action:

- **Input**: Proposed transaction(s) from the action generator
- **Processing**: mirage-rs pre-flight simulation (see [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md)):
  - Fork current chain state
  - Execute proposed transactions
  - Analyze state diffs, gas usage, token flows
  - Compare against expected outcomes
- **Output**: Simulation results with predicted state changes, gas costs, and risk assessment
- **Synapse mapping**: Domain-specific extension — no direct analog in the universal loop

This step is what distinguishes chain agents from coding agents. A coding agent's proposed action (writing a file) is reversible — you can always revert a commit. A chain agent's proposed action (submitting a transaction) may be irreversible — gas is spent, swaps are executed, positions are opened. Simulation provides a safe preview before commitment.

### Step 6: VALIDATE (Chain-Specific)

The agent validates the simulation results against safety policies:

- **Input**: Simulation results from step 5
- **Processing**: PolicyCage validation:
  - **Position limits**: Does the proposed action exceed maximum position sizes?
  - **Approved assets**: Are all tokens in the transaction on the agent's approved list?
  - **Gas budget**: Is the gas cost within the agent's budget for this type of operation?
  - **Slippage tolerance**: Is the expected price impact within acceptable bounds?
  - **Exposure limits**: Does the post-transaction portfolio exceed concentration limits?
- **Output**: Validation pass/fail with reasons
- **Synapse mapping**: `Gate.verify()` — pre-execution validation

If validation fails, the heartbeat returns to step 4 and routes to T1 (monitor) instead of T2 (act). The agent watches the opportunity without committing capital.

### Step 7: EXECUTE

The agent executes the validated action:

- **Input**: Validated transaction(s) from step 6
- **Processing**: Sign and submit via `ChainWallet.sign_and_submit()` (see [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md))
- **Output**: Transaction hash(es)
- **Synapse mapping**: `Agent.execute()` — performing the action

### Step 8: VERIFY

The agent verifies that the execution matched expectations:

- **Input**: Transaction receipt(s) and post-execution state
- **Processing**: Compare predicted state (from simulation) against actual state:
  - Did the transaction succeed (receipt status = 1)?
  - Does the actual gas usage match the predicted gas usage (within tolerance)?
  - Do the actual state changes match the predicted state changes?
  - Were there any unexpected side effects (unexpected token transfers, unexpected events)?
- **Output**: Verification pass/fail with prediction error measurements
- **Synapse mapping**: `Gate.verify()` — post-execution verification

Prediction errors are recorded for the Oracle system (see topic [09-oracle](../20-technical-analysis/INDEX.md)). Over time, the agent calibrates its predictions: "My gas estimates are typically 15% too low for Uniswap V3 swaps" → next time, inflate the estimate by 15%.

### Step 9: REFLECT

The agent reflects on the full cycle and updates its models:

- **Input**: Complete episode (observation → action → outcome)
- **Processing**:
  - Record episode to `.roko/episodes.jsonl`
  - Update Daimon affect markers (positive outcome → positive marker for this pattern)
  - Update NeuroStore knowledge (new insights from the episode)
  - Update Oracle predictions (calibrate estimates against actuals)
  - If the episode revealed useful knowledge, post to Nunchi chain
- **Output**: Updated agent state, potential knowledge entry for the chain
- **Synapse mapping**: `Policy.decide()` — adaptation from experience

---

## Three Cognitive Speeds

The 9 steps operate at different speeds:

| Speed | Steps | Trigger | Processing Type |
|---|---|---|---|
| **Gamma** (fast) | 1-OBSERVE, 4-GATE | `heartbeat.gamma.tick`; consume queued `chain.*` Pulses | Reactive: filter, route, quick decisions |
| **Theta** (medium) | 2-RETRIEVE, 3-ANALYZE, 5-SIMULATE, 6-VALIDATE, 7-EXECUTE, 8-VERIFY | `heartbeat.theta.tick` when chain work merits deliberation | Deliberative: analysis, simulation, execution |
| **Delta** (slow) | 9-REFLECT | `heartbeat.delta.tick` plus significant episode backlog | Consolidative: learning, knowledge update |

Gamma remains the fast subscriber for fresh chain activity. Theta handles deeper chain reasoning when the gamma route escalates. Delta consolidates outcomes, heuristics, and prediction calibration on the slower topic.

---

## Chain Agent Configuration

A chain agent is configured in `roko.toml`:

```toml
[agent]
domain = "chain"
temperament = "balanced"  # conservative for high-risk operations

[substrate]
type = "chain"
chains = ["ethereum", "nunchi"]

[gates]
pipeline = ["tx_sim", "wallet", "verify_chain"]

[chain]
custody_mode = "local_key"  # "delegation" | "embedded" | "local_key"
position_limit_usd = 10000
approved_assets = ["ETH", "USDC", "NUNCHI"]
max_gas_per_tx = 500000
slippage_tolerance = 0.005  # 0.5%
```

---

## Academic Foundations

- Damasio, A.R. (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. — Somatic marker hypothesis; the Daimon's affect system in step 3 implements computational somatic markers.
- Friston, K. (2010). "The Free-Energy Principle: A Unified Brain Theory?" *Nature Reviews Neuroscience*. — Active inference framework; the OBSERVE → ANALYZE → EXECUTE → VERIFY → REFLECT cycle minimizes prediction error (free energy).
- Sumers, T.R. et al. (2023). "Cognitive Architectures for Language Agents." *arXiv:2309.02427*. — CoALA framework: the chain heartbeat is a domain-specific instance of the CoALA cognitive loop.

---

## Current Status and Gaps

**Scaffold:**
- Universal Synapse loop wired in `crates/roko-cli/src/orchestrate.rs`
- `ChainClient` and `ChainWallet` traits defined in `roko-chain`
- mirage-rs provides simulation backend for step 5

**Not yet built (Tier 6):**
- Chain-specific heartbeat orchestrator (§H12)
- Daimon affect integration for chain operations (§H13)
- PolicyCage validation for chain actions (§H14)
- Oracle prediction calibration for chain metrics (§H15)
- Episode recording with chain-specific metadata (§H16)
- Three-speed integration (Gamma/Theta/Delta) for chain ticks (§H17)

---

## Cross-References

- See [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) for step 1 (OBSERVE)
- See [16-triage-curiosity-midas.md](./16-triage-curiosity-midas.md) for step 3 (ANALYZE)
- See [18-mirage-rs-evm-simulator.md](./18-mirage-rs-evm-simulator.md) for step 5 (SIMULATE)
- See [17-chain-client-wallet-traits.md](./17-chain-client-wallet-traits.md) for step 7 (EXECUTE)
- See topic [01-synapse](../00-architecture/INDEX.md) for the universal Synapse loop
- See topic [07-daimon](../09-daimon/INDEX.md) for the affect system in step 3
- See `tmp/refinements/09-phase-2-implications.md` for the Phase 2+ heartbeat and `ChainBus` implications
- See [01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for Bus, Pulse, Topic, and `ChainBus` vocabulary
