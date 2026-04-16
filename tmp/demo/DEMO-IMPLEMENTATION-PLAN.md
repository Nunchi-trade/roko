# Roko Demo: Implementation Plan

> **Source**: Derived from `/Users/will/dev/nunchi/collaboration/tmp/roko-demo-gaps.md` + collaboration repo call notes, sprint plans, and dashboard specs.
>
> **Target events**: A16Z Demo (Apr 25) · Consensus Miami (May 7)
> **Demo format**: 5 minutes total. 60s problem → 120s live loop → 60s flywheel/economics → 60s ask.
> **One claim to prove live**: "Every participant earned value, and the loop made itself smarter."
> **Scope**: Roko-side implementation (roko-demo crate, contracts, mirage-rs). Dashboard integration is Sam's concern — we emit events/data, he consumes.

---

## Table of Contents

1. [Current State Summary](#1-current-state-summary)
2. [The Demo Scenario: What Must Happen On Screen](#2-the-demo-scenario)
3. [Critical Gaps (Must-Have)](#3-critical-gaps)
4. [Stretch Goals](#4-stretch-goals)
5. [Implementation Tasks — Ordered by Dependency](#5-implementation-tasks)
6. [File-Level Change Map](#6-file-level-change-map)
7. [Architecture Notes](#7-architecture-notes)
8. [Contracts & Chain State](#8-contracts--chain-state)
9. [TUI Demo Mode Design](#9-tui-demo-mode-design)
10. [Testing & Verification](#10-testing--verification)
11. [Pre-Demo Checklist](#11-pre-demo-checklist)
12. [Timeline & Dependencies](#12-timeline--dependencies)
13. [Additional Demo Workflows](#13-additional-demo-workflows)

---

## 1. Current State Summary

### What roko-demo Has Today

**Binary**: `roko-demo` (`crates/roko-demo/`)

**Working infrastructure:**
- Contract deployment pipeline (`deploy.rs`) — deploys 6 Solidity contracts to mirage-rs
- TOML-driven manifest system (`manifest.rs`) — scenario configs in `demo/scenarios/`
- 10 Anvil dev wallets (`demo/wallets.toml`)
- Invariant verification (`verify.rs`) — bytecode checks + event count assertions
- `ChainCtx` for RPC + wallet management

**4 existing scenarios** (all use `StubLlm`):

| Scenario | What it does | Demo relevance |
|----------|-------------|----------------|
| `job-board` | 3 rounds: post→assign→submit→accept. Round-robin workers. | **Close** to demo needs — needs real LLM, real routing |
| `consortium` | 1 job with 3-validator committee voting (2-of-3) | **Useful** — consortium verification is part of demo flow |
| `defi-routing` | Single job, worker0 always wins, route_proposal is hashed not executed | **Far** — needs complete rewrite for real DeFi routing |
| `flywheel` | 3 rounds × 3 posters × 3 confirmers on InsightBoard | **Close** — knowledge accumulation loop works |

**6 Solidity contracts** (all tested with forge):

| Contract | Address | Demo Role |
|----------|---------|-----------|
| `MockERC20.sol` | Deterministic | DAEJI token for payments |
| `AgentRegistry.sol` | Deterministic | Agent identity (ERC-8004) |
| `WorkerRegistry.sol` | Deterministic | Stake bonds, EMA reputation, 30-day decay |
| `BountyMarket.sol` | Deterministic | 4-state escrow (Open→Funded→Assigned→Submitted→Terminal) |
| `ConsortiumValidator.sol` | Deterministic | 3-agent committee, 2-of-3 voting |
| `InsightBoard.sol` | Deterministic | Post/confirm insights, pheromone weights, claimable earnings |

**mirage-rs** (`apps/mirage-rs/`):
- In-process EVM via `revm`, JSON-RPC on :8545
- Supports mainnet fork with lazy upstream reads
- Copy-on-write scenario branching
- `evm_mine`, `evm_snapshot`, `evm_revert` methods
- `chain` feature adds knowledge store, pheromone field, agent registry (in-process)
- `roko` feature adds `SimulationGate`, `HdcSubstrate`, `ChainSubstrate`

### What's Stubbed / Missing

1. **`StubLlm` everywhere** — no real AI reasoning. Returns bounded-random structured output.
2. **`defi-routing` is fake** — no real pool data, no routing decisions, worker0 always wins.
3. **No knowledge query→post loop** — InsightBoard exists but agents don't query it before executing.
4. **No fee distribution** — BountyMarket pays 100% to worker. No 40/30/20/10 split.
5. **No C-Factor measurement** — no benchmark infrastructure at all.
6. **No event stream** — scenarios run and verify invariants but emit nothing for dashboard.
7. **`AgentRegistry` never called** — deployed but no spine uses it.
8. **`InsightBoard` never funded** — `claim()` would revert (no DAEJI to distribute).
9. **`eth_getLogs` stubbed in mirage-rs** — event verification is unreliable.
10. **Prompt templates unused** — `demo/prompts/` files exist but no code reads them.

---

## 2. The Demo Scenario

### `yield-routing-demo` — The Full Live Loop

This is the single scenario that must work for the A16Z demo. Everything else is bonus.

```
Phase 1: Setup (pre-demo, not shown)
  - mirage-rs boots with mainnet fork at block N
  - 6 contracts deployed deterministically
  - 5 agents registered in AgentRegistry with different LLM backends
  - InsightBoard seeded with 3 prior insights (from previous "runs")

Phase 2: Job Posted (shown — 15s)
  - Poster creates job: "Route 100K USDC → ETH, maximize output"
  - BountyMarket.postJob(description, bountyAmount=1000 DAEJI)
  - Event: JobPosted

Phase 3: Agent Bidding (shown — 20s)
  - 5 agents read job from BountyMarket
  - Each agent queries InsightBoard for relevant insights
  - Each agent calls its LLM with: job + pool data + insights
  - Each agent submits bid: { route, expected_output, confidence }
  - BountyMarket.assign(bestBid) based on reputation + bid quality
  - Events: AgentBid × 5, JobAssigned

Phase 4: Execution (shown — 30s)
  - Winning agent executes route on mirage-rs
  - Actual EVM transactions: approve → swap on Aave/Compound/Morpho
  - Output measured: actual ETH received
  - Gate: compare actual vs expected (within tolerance)
  - Events: ExecutionStarted, ExecutionCompleted

Phase 5: Verification & Payment (shown — 20s)
  - ConsortiumValidator: 3 validators verify execution
  - 2-of-3 vote approves
  - FeeDistributor splits payment: 40/30/20/10
  - WorkerRegistry updates reputation (EMA)
  - Events: ValidationVote × 3, FeesDistributed, ReputationUpdated

Phase 6: Knowledge Posting (shown — 15s)
  - Winning agent posts new insight to InsightBoard
  - Other agents confirm/deny
  - Pheromone weight updated
  - Events: InsightPosted, InsightConfirmed × N

Phase 7: Round 2 — C-Factor (shown — 20s)
  - Same job posted again
  - Agents now have Round 1 insights available
  - Run same flow
  - Compare: Round 2 output > Round 1 output by ≥15%
  - Events: CFactorMeasured { improvement: "+18.3%" }
```

### Prompt Template for Yield Router Agent

```markdown
# Role
You are a DeFi routing agent competing for jobs on the Nunchi network.

# Job
{{job_description}}

# Available Pools (from on-chain state)
{{pool_data}}
<!-- Example:
| Pool | Protocol | TVL | Utilization | Supply Rate | Borrow Rate |
|------|----------|-----|-------------|-------------|-------------|
| USDC/ETH | Aave V3 | $450M | 78% | 3.2% | 5.1% |
| USDC/ETH | Compound V3 | $180M | 65% | 2.8% | 4.7% |
| USDC/ETH | Morpho | $95M | 42% | 4.1% | 6.2% |
-->

# Prior Knowledge (from InsightBoard)
{{prior_insights}}
<!-- Example:
- Insight #12 (weight: 0.87): "Morpho pools under 50% utilization consistently
  offer 15-20% better rates during low-gas windows (base fee < 15 gwei)"
- Insight #8 (weight: 0.72): "Splitting across 2 pools reduces slippage by ~8%
  on orders > 50K USDC"
- Insight #15 (weight: 0.65): "Aave V3 flash loan + swap is 3% cheaper than
  direct swap for amounts > 80K"
-->

# Instructions
Analyze the pools and prior knowledge. Propose a routing strategy.

# Output (JSON)
{
  "route": [
    { "pool": "morpho-usdc-eth", "amount_usdc": 60000, "reason": "..." },
    { "pool": "aave-v3-usdc-eth", "amount_usdc": 40000, "reason": "..." }
  ],
  "expected_output_eth": 52.34,
  "confidence": 0.85,
  "reasoning": "Split 60/40 based on Insight #12 (Morpho under-utilization advantage)
    and #8 (split reduces slippage)..."
}
```

### Key Demo Lines

- "Fork the code, you can't fork the knowledge" (Wikipedia analogy)
- "The trading IS the knowledge production"
- "Context beats compute — a $5/mo Gemma on our network beats a $200/mo frontier model running solo"

### What Must NOT Be in the Demo

- No architecture diagrams, no terminal output, no code on screen
- No jargon: say "agent" not "Golem", "shared knowledge" not "InsightStore"
- No TBD placeholders or unverified numbers
- No subjective scoring — everything EVM-deterministic

---

## 3. Critical Gaps

### Gap 1: Real LLM Integration (replaces StubLlm)

**Current state**: `StubLlm` returns bounded-random structured output via an `AtomicU64` counter. No real AI reasoning.

**Needed**: Full agentic agents making real LLM calls for routing decisions.

**What to build**:
- Implement `LlmProvider` trait for Claude API (via `reqwest` + Anthropic Messages API)
- Implement `LlmProvider` for Ollama (local Gemma/Llama, HTTP API on default port)
- Each agent receives: job description + pool data + InsightBoard insights → produces structured JSON routing strategy
- Agent diversity: at least 2 different model backends (Claude + Ollama) visible in events
- Temperature=0, seed if available for demo reliability
- Structured JSON output mode for reliable parsing

**`LlmProvider` trait** (existing):
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value>;
}
```

**New implementations**:
```rust
struct ClaudeApiProvider { client: reqwest::Client, api_key: String, model: String }
struct OllamaProvider { client: reqwest::Client, model: String, base_url: String }
struct MultiProvider { providers: Vec<(String, Arc<dyn LlmProvider>)> }  // label + provider
```

**Key constraint**: The EVM execution is deterministic; the LLM routing decision introduces variance. This is fine — it makes the demo feel real. But temperature=0 keeps it reproducible.

**Env vars**: `ANTHROPIC_API_KEY` for Claude, Ollama on `http://localhost:11434`.

**Files to modify/create**:
- `crates/roko-demo/src/scenarios/llm.rs` — add `ClaudeApiProvider`, `OllamaProvider`, `MultiProvider`
- `crates/roko-demo/src/main.rs` — CLI flag `--llm-backend <stub|claude|ollama|multi>` (default: stub)

### Gap 2: DeFi Routing Scenario with Real Pool Data

**Current state**: `defi-routing` scenario is fake — worker0 always wins, route_proposal is hashed but never validated or executed.

**Needed**: Agents route trades across real pool state, execute on mirage-rs fork, compare output amounts.

**What to build**:
- New scenario: `yield-routing` (don't modify `defi-routing`, keep it as a simpler test)
- mirage-rs forks Ethereum mainnet at a specific block (deterministic snapshot)
- Scenario reads real pool contracts (Aave V3, Compound V3, Morpho) from the fork via alloy calls
- Job: "Route 100K USDC → ETH, maximize output"
- Agents read on-chain pool state (TVL, utilization, rates)
- Agents propose routing strategies (single pool, split across pools, consider gas)
- Execution: simulate the swap on mirage-rs fork, measure output
- Oracle: compare output amounts. Winner = highest ETH received. Deterministic.

**Key pool addresses (Ethereum mainnet)**:
- Aave V3 Pool: `0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2`
- Compound V3 (USDC): `0xc3d688B66703497DAA19211EEdff47f25384cdc3`
- USDC: `0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48`
- WETH: `0xC02aaA39b223FE8D0A0e5c4F27eAD9083C756Cc2`
- Uniswap V3 Router: `0xE592427A0AEce92De3Edee1F18E0157C05861564`

**Files to create/modify**:
- `demo/scenarios/yield-routing.toml` — new scenario config
- `crates/roko-demo/src/scenarios/yield_routing.rs` — scenario spine
- `crates/roko-demo/src/scenarios/pool_reader.rs` — alloy calls to read pool state
- `demo/prompts/yield-router.md` — prompt template for routing agents

### Gap 3: Knowledge Query → Knowledge Post Loop

**Current state**: `InsightBoard.sol` supports post/confirm. `flywheel` scenario posts insights. But there's no integration between job execution and knowledge.

**Needed**: Agents query InsightBoard BEFORE executing, and post new learnings AFTER.

**What to build**:
- Pre-execution knowledge query: agent reads InsightBoard for insights (scan events or add topic-based view function)
- Knowledge injection into prompt: retrieved insights become part of `{{prior_insights}}` in agent's context
- Post-execution knowledge write: winning agent posts what it learned as new insight
- Confirmation loop: other agents confirm/deny the insight (pheromone weighting)
- **C-Factor measurement**: Run same job twice. First run = no prior knowledge. Second run = with knowledge from first run. Measure delta. Target ≥15%.

**InsightBoard interaction pattern**:
```
Before execution:
  agent → InsightBoard.nextInsightId() → iterate 0..N → read each insight
  (or: scan InsightPosted events client-side, filter by topic)

After execution:
  winner → InsightBoard.post(contentHash) → insightId
  other agents → InsightBoard.confirm(insightId) × N
```

**Files to modify**:
- `crates/roko-demo/src/scenarios/yield_routing.rs` — add knowledge query/post steps
- `contracts/src/InsightBoard.sol` — may need `getInsight(id)` view function or topic field
- `demo/prompts/yield-router.md` — add `{{prior_insights}}` placeholder

### Gap 4: Fee Distribution Visibility

**Current state**: `BountyMarket.sol` pays full amount to winning worker on `resolveAccepted`. No fee split.

**Needed**: Every job completion shows a visible fee breakdown.

**What to build**:
- New `FeeDistributor.sol` contract (or modify BountyMarket):
  - 40% → validator pool (ConsortiumValidator participants)
  - 30% → data providers (InsightBoard authors whose insights were used)
  - 20% → winning agent
  - 10% → treasury address
- Emit events with each split amount for dashboard/TUI consumption
- Track cumulative earnings per participant (queryable)

**Contract interface**:
```solidity
contract FeeDistributor {
    event FeesDistributed(
        uint256 indexed jobId,
        uint256 validatorShare,
        uint256 dataProviderShare,
        uint256 agentShare,
        uint256 treasuryShare
    );

    function distribute(
        uint256 jobId,
        uint256 totalAmount,
        address agent,
        address[] calldata validators,
        address[] calldata dataProviders
    ) external;

    function cumulativeEarnings(address) external view returns (uint256);
}
```

**Files to create/modify**:
- `contracts/src/FeeDistributor.sol` — new contract
- `contracts/test/FeeDistributor.t.sol` — forge tests
- `crates/roko-demo/src/bindings.rs` — add FeeDistributor ABI
- `crates/roko-demo/src/deploy.rs` — deploy FeeDistributor
- `demo/scenarios/yield-routing.toml` — include FeeDistributor in deploy

### Gap 5: C-Factor Benchmark

**Current state**: No benchmark infrastructure. Demo uses placeholder values (40% speed, 27pt accuracy).

**Needed**: Deterministic, reproducible measurement of knowledge-assisted vs knowledge-less performance.

**What to build**:
- Benchmark harness: runs yield-routing scenario twice on same mirage-rs fork
  - Run A: agents have empty InsightBoard (cold start)
  - Run B: agents have InsightBoard populated from Run A
  - Measure: output ETH amount, gas used, execution time for both runs
  - Report: % improvement, per-agent breakdown
- Standalone command: `roko-demo benchmark c-factor`
- Results deterministic given same fork block + same LLM seed
- Output: structured JSON report + human-readable summary

**Report format**:
```json
{
  "fork_block": 19500000,
  "run_a": {
    "winner": "agent-1",
    "model": "claude-sonnet-4",
    "output_eth": 52.2,
    "gas_used": 185000,
    "insights_available": 0,
    "duration_ms": 12500
  },
  "run_b": {
    "winner": "agent-1",
    "model": "gemma-7b",
    "output_eth": 61.8,
    "gas_used": 142000,
    "insights_available": 4,
    "duration_ms": 8200
  },
  "c_factor": {
    "output_improvement_pct": 18.4,
    "gas_improvement_pct": 23.2,
    "speed_improvement_pct": 34.4
  }
}
```

**Due date**: Apr 15 — blocks Sam's comparison panel and benchmark chart.

**Files to create**:
- `crates/roko-demo/src/benchmark.rs` — benchmark harness
- `crates/roko-demo/src/main.rs` — add `benchmark` subcommand

### Gap 6: Event Stream for Dashboard

**Current state**: Scenarios run and verify invariants but emit nothing consumable.

**Needed**: Dashboard (Sam's AI Studio) needs live events from roko.

**What to build**:
- Event types emitted during scenario execution:
  ```rust
  enum DemoEvent {
      JobPosted { id: u64, description: String, bounty_amount: u64 },
      AgentBid { agent_id: String, model: String, strategy_summary: String, expected_output: f64 },
      KnowledgeQueried { agent_id: String, insights_found: usize },
      ExecutionStarted { agent_id: String, route: serde_json::Value },
      ExecutionCompleted { agent_id: String, output_amount: f64, gas_used: u64 },
      ValidationVote { validator: String, approved: bool },
      FeesDistributed { validator_share: u64, data_provider_share: u64, agent_share: u64, treasury_share: u64 },
      InsightPosted { agent_id: String, topic: String, content: String },
      InsightConfirmed { agent_id: String, insight_id: u64 },
      ReputationUpdated { agent_id: String, old_rep: f64, new_rep: f64 },
      CFactorMeasured { run_a_output: f64, run_b_output: f64, improvement_pct: f64 },
      RoundStarted { round: u32 },
      RoundCompleted { round: u32 },
  }
  ```
- Transport options (implement both, CLI flag selects):
  - **NDJSON stdout** (default) — pipe to dashboard or log
  - **WebSocket server** on configurable port — real-time for dashboard

**Files to create**:
- `crates/roko-demo/src/events.rs` — event types + `EventEmitter` trait
- `crates/roko-demo/src/ws_server.rs` — WebSocket server (tokio-tungstenite)
- Integrate emitter into all scenario spines

---

## 4. Stretch Goals

Ordered by demo impact. Each is independent of the others.

### Stretch 1: TUI Demo Mode (ratatui)

Standalone terminal visualization. See [Section 9](#9-tui-demo-mode-design) for full design.

- Consumes same event stream as dashboard (Gap 6)
- 4-panel layout: Agents, Activity Log, Knowledge, Economics
- Color coding: green=success, yellow=in-progress, red=failures
- Command: `roko-demo tui`
- Estimated: ~500-800 LOC
- **Depends on**: Gap 6 (event stream)

### Stretch 2: Multi-Model Visible in Demo

Each agent's LLM backend labeled in events and TUI:
- Agent A: Claude Sonnet 4 (cloud)
- Agent B: Gemma 27B (local via Ollama)
- Agent C: Gemma 7B (local, small — the "7B + knowledge beats 70B" test)
- Agent D: Claude Haiku (cheap, fast)
- Agent E: Llama 3.2 (local)

If the small model wins because of knowledge, that's the strongest demo line.
- **Depends on**: Gap 1 (real LLM providers)

### Stretch 3: Multi-Round Tournament

Run 5-10 rounds of yield-routing. Show:
- Learning curve: performance improves each round
- Reputation dynamics: best agents get higher reputation
- Knowledge growth: InsightBoard fills with increasingly valuable insights
- Fee accumulation: agents' earnings compound
- Command: `roko-demo tournament --rounds 10`
- **Depends on**: Gaps 1-6

### Stretch 4: Knowledge Graph Visualization

Real-time graph showing:
- Insight nodes appearing on InsightBoard
- Connections between related insights
- Agent queries drawing edges from agent → insight
- Pheromone weights as node size/color
- Output: JSON for Sam's dashboard to render (or ratatui canvas for TUI)
- **Depends on**: Gap 3 (knowledge loop)

### Stretch 5: Reputation Persistence Across Runs

Currently reputation resets each scenario run. Persist WorkerRegistry reputation across runs:
- Serialize on-chain reputation state to JSON after each run
- Reload on next run (either re-deploy with initial values or use mirage-rs snapshot)
- Agents build track records across demo sessions
- **Depends on**: Gap 2 (yield routing)

### Stretch 6: One-Click Agent Deploy

Show in TUI: "Register new agent → stake bond → ready to bid" in 60 seconds.
- Uses AgentRegistry + WorkerRegistry contracts
- Demonstrates low barrier to entry
- Command: `roko-demo register-agent --model gemma-7b --stake 1000`
- **Depends on**: Gap 1

### Stretch 7: Autonomous Agent Loop

Graduate from scripted spines to true autonomy:
- Agent monitors BountyMarket for new jobs (event subscription or polling)
- Agent self-selects jobs matching its capabilities
- Agent queries knowledge, executes, posts results
- No orchestrator — agents run independently
- Command: `roko-demo autonomous --agents 5 --jobs 3`
- **Depends on**: Gaps 1-3

### Stretch 8: Adversarial Agent / Slashing

One agent posts bad knowledge (intentionally wrong routing advice):
- Consortium validators catch it
- Agent gets slashed (WorkerRegistry bond reduced)
- Demonstrates trust/verification layer
- Requires: `WorkerRegistry.sol` slashing for quality + new scenario variant
- **Depends on**: Gaps 1-4

---

## 5. Implementation Tasks — Ordered by Dependency

### Tier 1: Foundation (do first — no cross-dependencies)

#### T1.1 — Real LLM Provider Trait Implementation
- **What**: Implement `LlmProvider` for Claude API and Ollama
- **Where**: `crates/roko-demo/src/scenarios/llm.rs` + potentially new `llm_providers.rs`
- **Acceptance**:
  - `cargo test -p roko-demo` passes
  - Can call Claude API with structured JSON prompt and get valid JSON response
  - Can call Ollama and get structured JSON response
  - CLI flag `--llm-backend <stub|claude|ollama|multi>` works
  - `MultiProvider` round-robins across backends, labeling each agent
- **Context**: Existing `StubLlm` implements `LlmProvider::fill(req)`. New providers implement same trait. `LlmRequest` has `slot: String` and `context: Value`. For yield routing, the key slot is `route_proposal` with context containing pool data + prior insights.
- **Env vars**: `ANTHROPIC_API_KEY`, `OLLAMA_URL` (default `http://localhost:11434`)
- **Estimate**: Medium

#### T1.2 — Yield Routing Scenario Skeleton
- **What**: New scenario `yield-routing` that reads pool data from mirage-rs mainnet fork
- **Where**: `demo/scenarios/yield-routing.toml` + `crates/roko-demo/src/scenarios/yield_routing.rs` + `pool_reader.rs`
- **Acceptance**:
  - Scenario boots mirage-rs with mainnet fork
  - Reads Aave V3/Compound V3 pool state via alloy calls
  - Prints pool data table (TVL, utilization, rates)
  - No LLM calls yet — just the data pipeline + contract deployment
- **Context**: mirage-rs supports lazy mainnet reads. Need pool ABIs: Aave V3 Pool `getReserveData`, Compound V3 `getSupplyRate`. Hardcode mainnet addresses.
- **Key addresses** (Ethereum mainnet):
  - Aave V3 Pool: `0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2`
  - Compound V3 (USDC): `0xc3d688B66703497DAA19211EEdff47f25384cdc3`
  - USDC: `0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48`
  - WETH: `0xC02aaA39b223FE8D0A0e5c4F27eAD9083C756Cc2`
- **Estimate**: Large (alloy ABI calls, pool state parsing, mirage-rs fork config)

#### T1.3 — FeeDistributor Contract
- **What**: New Solidity contract for 40/30/20/10 fee split
- **Where**: `contracts/src/FeeDistributor.sol`, `contracts/test/FeeDistributor.t.sol`
- **Acceptance**:
  - `forge test` passes
  - Job resolution splits payment correctly (40% validators, 30% data, 20% agent, 10% treasury)
  - Events emitted per split with amounts
  - Cumulative earnings queryable per address
  - Integrates with MockERC20 for DAEJI transfers
- **Context**: BountyMarket currently pays 100% to worker in `resolveAccepted`. FeeDistributor sits between BountyMarket and recipients. BountyMarket resolves → transfers to FeeDistributor → FeeDistributor splits.
- **Estimate**: Small-Medium

#### T1.4 — Event Types + Emitter Infrastructure
- **What**: Define `DemoEvent` enum and `EventEmitter` trait with NDJSON + WebSocket backends
- **Where**: `crates/roko-demo/src/events.rs`, `crates/roko-demo/src/ws_server.rs`
- **Acceptance**:
  - `DemoEvent` covers all event types listed in Gap 6
  - `NdjsonEmitter` writes to stdout
  - `WsEmitter` serves WebSocket on configurable port
  - CLI flags: `--events ndjson|ws|both|none`, `--ws-port <N>`
  - Events are serde-serializable JSON
- **Context**: Existing scenarios have no event hooks. The emitter will be passed into scenario spines as `Arc<dyn EventEmitter>`.
- **Estimate**: Medium

### Tier 2: Integration (after Tier 1)

#### T2.1 — Full Yield Routing with LLM Agents
- **What**: Wire real LLM providers into yield-routing scenario. Agents reason about pool data and produce routing strategies.
- **Where**: `crates/roko-demo/src/scenarios/yield_routing.rs`
- **Acceptance**:
  - 5 agents with different LLM backends each produce a routing strategy JSON
  - Strategies vary (agents make different decisions based on their models)
  - Best strategy selected by on-chain oracle (highest expected output)
  - Winner assigned via BountyMarket
  - Execution on mirage-rs fork (actual swap transactions)
  - All steps emit DemoEvents
- **Depends on**: T1.1, T1.2, T1.4
- **Estimate**: Large

#### T2.2 — Knowledge Query/Post Integration
- **What**: Agents query InsightBoard before executing, post new insights after
- **Where**: `crates/roko-demo/src/scenarios/yield_routing.rs`, `demo/prompts/yield-router.md`
- **Acceptance**:
  - Agent's prompt includes `{{prior_insights}}` populated from InsightBoard on-chain reads
  - After execution, winning agent posts new insight via `InsightBoard.post()`
  - Other agents confirm via `InsightBoard.confirm()`
  - Second run shows agents using first run's insights
  - `KnowledgeQueried` and `InsightPosted` events emitted
- **Depends on**: T2.1
- **Estimate**: Medium

#### T2.3 — Fee Distribution Wiring
- **What**: Wire FeeDistributor into yield-routing scenario flow
- **Where**: `crates/roko-demo/src/scenarios/yield_routing.rs`, bindings, deploy
- **Acceptance**:
  - After ConsortiumValidator approval, FeeDistributor splits payment
  - `FeesDistributed` event emitted with per-recipient amounts
  - Cumulative earnings visible via contract reads
- **Depends on**: T1.3, T2.1
- **Estimate**: Small

#### T2.4 — C-Factor Benchmark Command
- **What**: `roko-demo benchmark c-factor` runs yield-routing twice, measures improvement
- **Where**: `crates/roko-demo/src/benchmark.rs`, `main.rs`
- **Acceptance**:
  - Produces JSON report with Run A (cold) and Run B (warm) metrics
  - Improvement is measurable (target ≥15% — adjust knowledge seeds if needed)
  - Deterministic given same fork block + same LLM seed
  - Human-readable summary printed to stderr
  - JSON report written to file or stdout
- **Depends on**: T2.2
- **Estimate**: Medium
- **CRITICAL DUE DATE**: Apr 15 — blocks Sam's comparison panel and benchmark chart

#### T2.5 — InsightBoard Enhancements
- **What**: Add topic-based filtering and `getInsight(id)` view function to InsightBoard.sol
- **Where**: `contracts/src/InsightBoard.sol`, `contracts/test/InsightBoard.t.sol`
- **Acceptance**:
  - `getInsight(id)` returns (poster, contentHash, pheromoneWeight, confirmCount)
  - Optional: topic field on insights for client-side filtering
  - `forge test` passes
- **Depends on**: Nothing (can be done in parallel with T1)
- **Estimate**: Small

### Tier 3: Polish & Stretch

#### T3.1 — TUI Demo Mode
- **What**: ratatui-based terminal UI showing live demo
- **Where**: `crates/roko-demo/src/tui.rs` (or `tui/` module)
- **Acceptance**: See [Section 9](#9-tui-demo-mode-design)
- **Depends on**: T1.4 (event stream)
- **Estimate**: Medium-Large (500-800 LOC)

#### T3.2 — Multi-Model Labeling
- **What**: Each agent's LLM backend visible in events and TUI
- **Where**: Events + TUI rendering
- **Acceptance**: Events include `model: "claude-sonnet-4"` or `model: "gemma-7b"`. TUI shows model name per agent.
- **Depends on**: T1.1, T1.4
- **Estimate**: Small

#### T3.3 — Multi-Round Tournament
- **What**: Run 5-10 rounds, show learning curve
- **Where**: `crates/roko-demo/src/scenarios/yield_routing.rs` + new `tournament.rs`
- **Acceptance**:
  - `roko-demo tournament --rounds 10` runs N rounds of yield-routing
  - Performance data: round → output amount (clear upward trend)
  - Reputation dynamics visible (best agents climb)
  - Knowledge growth (InsightBoard fills each round)
  - Fee accumulation per agent
  - Events emitted per round
- **Depends on**: T2.1, T2.2, T2.3
- **Estimate**: Medium

#### T3.4 — Knowledge Graph JSON Output
- **What**: Emit knowledge graph state as JSON for dashboard visualization
- **Where**: `crates/roko-demo/src/events.rs` or new `knowledge_graph.rs`
- **Acceptance**:
  - After each round, emit graph state: nodes (insights), edges (confirmations, agent queries)
  - Node attributes: id, content, pheromone_weight, confirmations
  - Edge attributes: agent_id, query_time, confirmation_time
  - JSON output consumable by Sam's dashboard
- **Depends on**: T2.2
- **Estimate**: Small

#### T3.5 — Reputation Persistence
- **What**: Save/restore WorkerRegistry reputation across scenario runs
- **Where**: New persistence module in roko-demo
- **Acceptance**:
  - After scenario run, serialize reputation state to `demo/.runtime/reputation.json`
  - On next run, if file exists, bootstrap reputation (either via initial contract state or mirage-rs snapshot)
  - Second run starts with first run's reputation scores
- **Depends on**: T2.1
- **Estimate**: Small

#### T3.6 — One-Click Agent Registration
- **What**: `roko-demo register-agent` command for quick agent onboarding
- **Where**: New command in `main.rs`
- **Acceptance**:
  - `roko-demo register-agent --name "my-agent" --model gemma-7b --stake 1000`
  - Registers in AgentRegistry + WorkerRegistry
  - Mints DAEJI, stakes bond, sets tier
  - Agent is ready to bid on next job
  - Takes <60 seconds
- **Depends on**: T1.2 (needs contracts deployed)
- **Estimate**: Small

#### T3.7 — Autonomous Agent Loop
- **What**: Agents poll BountyMarket, self-select jobs, execute independently
- **Where**: New `autonomous.rs` module
- **Acceptance**:
  - `roko-demo autonomous --agents 5 --jobs 3`
  - Spawns 5 agent tasks that poll for new jobs
  - Posts 3 jobs over time
  - Agents discover, claim, execute, and report without orchestration
  - Knowledge accumulates naturally
- **Depends on**: T2.1, T2.2
- **Estimate**: Large

#### T3.8 — Adversarial Agent / Slashing
- **What**: Bad-knowledge agent + slashing demo
- **Where**: Scenario variant + WorkerRegistry changes
- **Acceptance**:
  - One agent posts intentionally bad insight
  - Validators reject via ConsortiumValidator
  - WorkerRegistry slashes agent's stake
  - Events show the slashing flow
  - Agent reputation drops visibly
- **Depends on**: T2.1, T2.2, T2.3
- **Estimate**: Medium

---

## 6. File-Level Change Map

### New Files

| File | Task | Description |
|------|------|-------------|
| `crates/roko-demo/src/scenarios/yield_routing.rs` | T1.2 | Yield routing scenario spine |
| `crates/roko-demo/src/scenarios/pool_reader.rs` | T1.2 | Alloy calls to read pool state |
| `crates/roko-demo/src/events.rs` | T1.4 | DemoEvent types + EventEmitter trait |
| `crates/roko-demo/src/ws_server.rs` | T1.4 | WebSocket event server |
| `crates/roko-demo/src/benchmark.rs` | T2.4 | C-Factor benchmark harness |
| `crates/roko-demo/src/tournament.rs` | T3.3 | Multi-round tournament |
| `crates/roko-demo/src/autonomous.rs` | T3.7 | Autonomous agent loop |
| `crates/roko-demo/src/tui.rs` | T3.1 | ratatui TUI (or `tui/` module) |
| `contracts/src/FeeDistributor.sol` | T1.3 | Fee split contract |
| `contracts/test/FeeDistributor.t.sol` | T1.3 | Fee split tests |
| `demo/scenarios/yield-routing.toml` | T1.2 | Scenario config |
| `demo/prompts/yield-router.md` | T1.2 | Prompt template |

### Modified Files

| File | Task | Changes |
|------|------|---------|
| `crates/roko-demo/src/main.rs` | T1.1, T1.4, T2.4, T3.3 | Add CLI flags + subcommands |
| `crates/roko-demo/src/scenarios/llm.rs` | T1.1 | Add ClaudeApiProvider, OllamaProvider, MultiProvider |
| `crates/roko-demo/src/scenarios/mod.rs` | T1.2 | Register yield_routing scenario |
| `crates/roko-demo/src/bindings.rs` | T1.3, T1.2 | Add FeeDistributor ABI + pool ABIs |
| `crates/roko-demo/src/deploy.rs` | T1.3 | Deploy FeeDistributor |
| `crates/roko-demo/src/verify.rs` | T2.1 | Add yield-routing verification invariants |
| `crates/roko-demo/Cargo.toml` | T1.1, T1.4, T3.1 | Add reqwest, tokio-tungstenite, ratatui deps |
| `contracts/src/InsightBoard.sol` | T2.5 | Add getInsight view, maybe topic field |
| `contracts/src/BountyMarket.sol` | T2.3 | Route payment through FeeDistributor |
| `demo/manifest.toml` | T1.2 | Register yield-routing scenario |

### Untouched (stable)

| File | Why |
|------|-----|
| `crates/roko-core/` | Stable kernel — traits don't change |
| `contracts/src/MockERC20.sol` | Works as-is |
| `contracts/src/AgentRegistry.sol` | Works as-is |
| `contracts/src/WorkerRegistry.sol` | Maybe minor change for slashing (T3.8) |
| `contracts/src/ConsortiumValidator.sol` | Works as-is |
| `crates/roko-demo/src/scenarios/job_board.rs` | Keep as simpler test |
| `crates/roko-demo/src/scenarios/consortium.rs` | Keep as-is |
| `crates/roko-demo/src/scenarios/flywheel.rs` | Keep as-is |

---

## 7. Architecture Notes

### Agent ↔ Chain Interaction Pattern

```
Agent (LLM)
  ├── reads: BountyMarket.getOpenJobs()
  ├── reads: InsightBoard.getInsights(topic)
  ├── reads: Pool contracts via mirage-rs (getReserveData, etc.)
  ├── reasons: LLM call with job + pools + insights → routing strategy
  ├── writes: BountyMarket.submitWork(jobId, resultHash)
  └── writes: InsightBoard.postInsight(topic, content)

Orchestrator (yield-routing spine)
  ├── deploys contracts to mirage-rs
  ├── spawns agents (each with own wallet + LLM backend)
  ├── posts jobs to BountyMarket
  ├── triggers ConsortiumValidator after submission
  ├── triggers FeeDistributor after validation
  └── emits DemoEvents for TUI/dashboard

mirage-rs
  ├── EVM execution (forked mainnet state)
  ├── JSON-RPC on :8545
  ├── Deterministic (same block → same results)
  └── Fast (~4000 blocks/sec)
```

### LLM Provider Architecture

```rust
// Existing trait
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value>;
}

// New: Claude API
struct ClaudeApiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,       // "claude-sonnet-4-20250514"
    temperature: f32,    // 0.0 for demo
}

// New: Ollama (local)
struct OllamaProvider {
    client: reqwest::Client,
    model: String,       // "gemma3:7b", "gemma3:27b", "llama3.2"
    base_url: String,    // "http://localhost:11434"
}

// New: Multi-backend (one provider per agent)
struct MultiProvider {
    providers: Vec<(String, Arc<dyn LlmProvider>)>,  // (label, provider)
    counter: AtomicUsize,                              // round-robin
}
```

### Event Flow

```
Scenario Spine
    │
    ├── emit(JobPosted { ... })
    ├── for each agent:
    │     ├── emit(KnowledgeQueried { ... })
    │     ├── llm.fill(route_proposal)
    │     └── emit(AgentBid { ... })
    ├── select winner
    ├── emit(ExecutionStarted { ... })
    ├── execute on mirage-rs
    ├── emit(ExecutionCompleted { ... })
    ├── consortium vote
    ├── emit(ValidationVote { ... }) × 3
    ├── fee distribution
    ├── emit(FeesDistributed { ... })
    ├── post insight
    ├── emit(InsightPosted { ... })
    └── if round 2:
          emit(CFactorMeasured { ... })
          │
          ▼
    EventEmitter (Arc<dyn EventEmitter>)
    ├── NdjsonEmitter → stdout
    └── WsEmitter → WebSocket clients
```

### Signal Flow

```
Job Signal ──→ Agent reads ──→ Knowledge Query Signal ──→ LLM Prompt Signal
     │                              │                          │
     │                              ▼                          ▼
     │                        InsightBoard              LLM Response Signal
     │                        (on-chain read)                  │
     │                                                         ▼
     │                                                   Routing Strategy Signal
     │                                                         │
     ▼                                                         ▼
BountyMarket ◄──────────────────────────── Execution Signal (EVM txns)
     │                                                         │
     ▼                                                         ▼
FeeDistribution Signal                                   Gate Verdict Signal
     │                                                         │
     ▼                                                         ▼
Earnings per participant                         New Insight Signal → InsightBoard
```

---

## 8. Contracts & Chain State

### Existing Contracts

| Contract | Needs Changes? | What |
|----------|---------------|------|
| `MockERC20.sol` | No | DAEJI token, open mint |
| `AgentRegistry.sol` | No | Agent identity, ERC-8004 |
| `WorkerRegistry.sol` | Maybe (T3.8) | Slashing for adversarial demo |
| `BountyMarket.sol` | Yes (T2.3) | Route payment through FeeDistributor |
| `ConsortiumValidator.sol` | No | 2-of-3 voting works |
| `InsightBoard.sol` | Yes (T2.5) | Add getInsight view, maybe topic field |

### New Contract

| Contract | Purpose |
|----------|---------|
| `FeeDistributor.sol` | Split payments 40/30/20/10. Emit events. Track cumulative. |

### mirage-rs Fork Configuration

```toml
# For yield-routing scenario
[fork]
rpc_url = "https://eth-mainnet.g.alchemy.com/v2/{KEY}"  # or ROKO_RPC_URL env
block_number = 19500000  # Pick block with interesting rate differentials
chain_id = 1

# Our contracts deploy on top of the fork
# Pool contracts (Aave, Compound, Morpho) are read from forked state
# DAEJI, BountyMarket, etc. are freshly deployed
```

### InsightBoard Data Model (for knowledge loop)

Current InsightBoard stores:
- `contentHash` (bytes32) — keccak of content
- `poster` (address)
- `pheromoneWeight` (uint256) — incremented on confirm
- `earnings` (mapping address → uint256)

For the demo, actual insight content should be stored off-chain (or as a simple string in the contract — gas isn't a concern on mirage-rs). The spine will:
1. Post insight content as keccak hash on-chain
2. Store full text in a local map keyed by insightId
3. Retrieve by reading the local map + on-chain pheromone weights

---

## 9. TUI Demo Mode Design

### Layout

```
┌─────────────────────────────────────────────────────────────────┐
│ NUNCHI DEMO — Yield Routing                          Round 1/2  │
├─────────────┬───────────────────────────────────────────────────┤
│ AGENTS      │ ACTIVITY LOG                                     │
│             │                                                   │
│ ● Agent-1   │ 12:01:03 Job posted: "Route 100K USDC → ETH"    │
│   claude-s4 │ 12:01:04 Agent-1 querying InsightBoard...        │
│   rep: 0.87 │ 12:01:04 Agent-1 found 3 prior insights          │
│   earned: 0 │ 12:01:05 Agent-2 querying InsightBoard...        │
│             │ 12:01:05 Agent-3 found 0 insights (cold start)   │
│ ● Agent-2   │ 12:01:06 All agents reasoning...                  │
│   gemma-27b │ 12:01:08 Agent-1 bid: 52.34 ETH (split Morpho+  │
│   rep: 0.72 │ 12:01:08 Agent-2 bid: 51.89 ETH (Aave single)  │
│   earned: 0 │ 12:01:09 Agent-3 bid: 50.12 ETH (Compound)     │
│             │ 12:01:09 Winner: Agent-1 (best output)           │
│ ○ Agent-3   │ 12:01:10 Executing route on EVM fork...          │
│   gemma-7b  │ 12:01:10 ✓ Swap 60K via Morpho: 31.4 ETH       │
│   rep: 0.45 │ 12:01:10 ✓ Swap 40K via Aave: 20.8 ETH         │
│   earned: 0 │ 12:01:11 Total output: 52.2 ETH                  │
│             │ 12:01:11 Validating (2/3 consortium)...           │
│ ○ Agent-4   │ 12:01:12 ✓ Validated. Fees distributed:          │
│   haiku     │           Validators: 400 DAEJI                   │
│   rep: 0.60 │           Data providers: 300 DAEJI               │
│             │           Agent-1: 200 DAEJI                      │
│ ○ Agent-5   │           Treasury: 100 DAEJI                     │
│   llama-3.2 │ 12:01:13 Agent-1 posted insight: "Morpho pool   │
│   rep: 0.55 │   under 50% util → 15% better rate confirmed"   │
├─────────────┼───────────────────────────────────────────────────┤
│ KNOWLEDGE   │ ECONOMICS                                         │
│             │                                                   │
│ 3 insights  │ Round 1: 52.2 ETH output (no prior knowledge)   │
│ +1 new      │ Round 2: 61.8 ETH output (with knowledge)       │
│ 18 confirms │ C-Factor: +18.4% improvement                    │
│ top: 0.87w  │                                                   │
│             │ Total fees: 2000 DAEJI across 2 rounds           │
│             │ Top earner: Agent-1 (580 DAEJI)                  │
└─────────────┴───────────────────────────────────────────────────┘
```

### Implementation Notes

- Use `ratatui` 0.29 (already in workspace — roko-cli TUI uses it)
- Consume events from `EventEmitter` via an `mpsc` channel
- `roko-demo tui` runs scenario + renders TUI simultaneously (tokio::select)
- Panels: Agents (left), Activity Log (right-top), Knowledge (bottom-left), Economics (bottom-right)
- Color coding: green=success, yellow=in-progress, red=failures/slashing
- Keyboard: `q` quit, `r` re-run, `1`/`2` switch rounds, space to pause/resume
- Estimated: ~500-800 LOC

---

## 10. Testing & Verification

### For Each Task

1. **Unit tests**: Test new code in isolation (LLM providers with mock HTTP, pool reader with mock RPC)
2. **Integration test**: Run full scenario end-to-end on mirage-rs
3. **Verification invariants**: Extend `verify.rs` with yield-routing checks

### Yield-Routing Verification

```rust
// After yield-routing scenario completes:
assert!(job_resolved_events >= 2, "Both rounds must complete");
assert!(insight_posted_events >= 1, "At least one insight posted");
assert!(fee_distributed_events >= 2, "Fees distributed each round");
assert!(round_2_output > round_1_output, "Knowledge must improve output");
assert!(
    (round_2_output - round_1_output) / round_1_output >= 0.15,
    "C-Factor must be >= 15%"
);
```

### Contract Tests

```bash
cd contracts && forge test
# Must pass: FeeDistributor splits correctly
# Must pass: InsightBoard getInsight view returns data
# Must pass: BountyMarket → FeeDistributor integration
```

### LLM Provider Tests

```rust
#[tokio::test]
async fn test_claude_provider_structured_output() {
    // Requires ANTHROPIC_API_KEY
    let provider = ClaudeApiProvider::new(/* ... */);
    let req = LlmRequest { slot: "route_proposal".into(), context: pool_data() };
    let result = provider.fill(req).await.unwrap();
    assert!(result["route"].is_array());
    assert!(result["expected_output_eth"].is_number());
}

#[tokio::test]
async fn test_ollama_provider_structured_output() {
    // Requires Ollama running locally
    let provider = OllamaProvider::new("gemma3:7b", None);
    let req = LlmRequest { slot: "route_proposal".into(), context: pool_data() };
    let result = provider.fill(req).await.unwrap();
    assert!(result["route"].is_array());
}
```

---

## 11. Pre-Demo Checklist

- [ ] `cargo build -p roko-demo` — compiles clean
- [ ] `cargo test -p roko-demo` — all tests pass
- [ ] `cargo clippy -p roko-demo -- -D warnings` — no warnings
- [ ] `forge test` — all contract tests pass (including FeeDistributor)
- [ ] `roko-demo up yield-routing` — completes without errors (StubLlm)
- [ ] `roko-demo up yield-routing --llm-backend claude` — completes with real LLM
- [ ] `roko-demo up yield-routing --llm-backend multi` — multi-model demo works
- [ ] `roko-demo benchmark c-factor` — produces ≥15% improvement report
- [ ] `roko-demo tui` — renders correctly, no panics
- [ ] `roko-demo tournament --rounds 5` — completes, shows learning curve
- [ ] Event stream parseable by dashboard (`--events ndjson` output is valid JSON)
- [ ] WebSocket server works (`--events ws --ws-port 9090`)
- [ ] Works with `ANTHROPIC_API_KEY` set
- [ ] Works with Ollama running locally (`ollama serve`)
- [ ] mirage-rs mainnet fork boots in <5s
- [ ] Full scenario completes in <30s (including LLM calls)
- [ ] C-Factor numbers are real (not placeholders)

---

## 12. Timeline & Dependencies

### Dependency Graph

```
T1.1 (LLM Providers) ──┐
T1.2 (Yield Skeleton) ──┼──→ T2.1 (Full Yield + LLM) ──→ T2.2 (Knowledge Loop)
T1.4 (Event Stream) ────┘         │                              │
                                   │                              ├──→ T2.4 (C-Factor) ★ Apr 15
T1.3 (FeeDistributor) ────────────┼──→ T2.3 (Fee Wiring)       │
                                   │                              ├──→ T3.3 (Tournament)
T2.5 (InsightBoard) ──────────────┘                              ├──→ T3.4 (Knowledge Graph)
                                                                  └──→ T3.7 (Autonomous)

T1.4 (Event Stream) ──→ T3.1 (TUI)
T1.1 (LLM Providers) ──→ T3.2 (Multi-Model Labels)
T2.1 ──→ T3.5 (Reputation Persistence)
T1.2 ──→ T3.6 (One-Click Register)
T2.1 + T2.2 + T2.3 ──→ T3.8 (Adversarial)
```

### Suggested Sprint Schedule

**Week 1 (Apr 10-11 remaining)**:
- T1.1 Real LLM Providers
- T1.3 FeeDistributor Contract
- T1.4 Event Types + Emitter
- T2.5 InsightBoard Enhancements

**Week 2 (Apr 12-15)**: ★ C-Factor deadline
- T1.2 Yield Routing Skeleton
- T2.1 Full Yield Routing with LLM
- T2.2 Knowledge Loop
- T2.4 C-Factor Benchmark → **deliver numbers to Sam by Apr 15**

**Week 3 (Apr 16-18)**:
- T2.3 Fee Distribution Wiring
- T3.1 TUI Demo Mode
- T3.2 Multi-Model Labeling
- T3.3 Multi-Round Tournament

**Week 4 (Apr 19-23)**: Polish + rehearsal
- T3.4 Knowledge Graph JSON
- T3.5 Reputation Persistence
- T3.6 One-Click Registration
- T3.7 Autonomous Loop
- T3.8 Adversarial Agent
- Demo rehearsals
- Backup video recording (Apr 21-23)

**Apr 25**: A16Z Demo

---

## 13. Additional Demo Workflows

Beyond the primary yield-routing demo, these additional workflows strengthen the narrative.

### Workflow A: "Small Model Beats Big Model" (C-Factor Showcase)

**Setup**: 2 agents. Agent A = Claude Opus (expensive, no knowledge). Agent B = Gemma 7B (cheap, with InsightBoard knowledge from prior runs).

**Flow**:
1. Post same yield-routing job
2. Agent A reasons from scratch — produces good but not optimal route
3. Agent B queries InsightBoard, finds 5 prior insights, produces optimal route
4. Agent B wins despite being 10x cheaper
5. Display: "$5/mo Gemma + Nunchi knowledge > $200/mo frontier model solo"

**What it proves**: The network's knowledge is the moat, not the model.

### Workflow B: "New Agent Onboarding" (60-second registration)

**Setup**: Live terminal showing agent registration.

**Flow**:
1. `roko-demo register-agent --name "new-trader" --model gemma-7b --stake 1000`
2. Agent registered in AgentRegistry (SoulBound NFT minted)
3. Stake bonded in WorkerRegistry
4. Agent immediately eligible for jobs
5. Show agent bidding on next job within 30 seconds of registration

**What it proves**: Low barrier to entry. Any model, any developer.

### Workflow C: "Knowledge Flywheel" (5-round tournament)

**Setup**: 5 agents, 5 rounds of yield-routing.

**Flow**:
1. Round 1: All agents cold-start. Mediocre performance.
2. Round 2-3: Knowledge accumulates. Performance improves.
3. Round 4-5: Best agents dominate. Small models with good knowledge outperform.
4. Show: learning curve chart, reputation leaderboard, fee accumulation.

**What it proves**: The network gets smarter over time. Compounding knowledge effect.

### Workflow D: "Trust & Verification" (adversarial scenario)

**Setup**: 6 agents, one is adversarial.

**Flow**:
1. Normal job cycle
2. Adversarial agent posts false insight: "Compound V3 has 0.1% utilization" (lie)
3. ConsortiumValidator committee reviews
4. 2 of 3 validators reject based on on-chain evidence
5. Adversarial agent slashed (stake reduced)
6. Bad insight's pheromone weight drops to 0
7. Show: self-correcting network, trust through economics

**What it proves**: Bad data is economically punished. Network self-curates.

### Workflow E: "Autonomous Agents" (no orchestrator)

**Setup**: 5 independent agent processes, no central coordinator.

**Flow**:
1. Start 5 agents: `roko-demo autonomous --agents 5`
2. Post 3 jobs over 2 minutes
3. Agents discover jobs by polling BountyMarket
4. Agents self-assign based on capability + reputation
5. Agents execute, verify, post knowledge — all independently
6. Show: decentralized coordination emerging from incentives

**What it proves**: This is a protocol, not a platform. No central point of failure.

---

## Appendix A: Environment & Dependencies

- **Rust**: stable 1.91+ (check `rust-toolchain.toml`)
- **Foundry**: `forge build`, `forge test` (contract compilation)
- **Ollama**: `brew install ollama`, `ollama serve`, `ollama pull gemma3:7b`, `ollama pull gemma3:27b`
- **Claude API**: `ANTHROPIC_API_KEY` env var
- **Ethereum RPC** (for mainnet fork): `ROKO_RPC_URL` or Alchemy/Infura key
- **mirage-rs**: `cargo run -p mirage-rs -- --fork-url $ROKO_RPC_URL --fork-block 19500000`

## Appendix B: Key Repo Paths

| Path | What | When to Touch |
|------|------|---------------|
| `crates/roko-demo/src/main.rs` | Demo CLI entry | Add subcommands |
| `crates/roko-demo/src/scenarios/` | Scenario implementations | Add yield-routing |
| `crates/roko-demo/src/scenarios/llm.rs` | LLM provider trait + stub | Add real providers |
| `crates/roko-demo/src/deploy.rs` | Contract deployer | Add FeeDistributor |
| `crates/roko-demo/src/bindings.rs` | alloy sol! bindings | Add FeeDistributor + pool ABIs |
| `crates/roko-demo/src/verify.rs` | Post-run invariant checks | Add yield-routing checks |
| `contracts/src/` | Solidity contracts | Add FeeDistributor |
| `demo/scenarios/` | Scenario TOML configs | Add yield-routing.toml |
| `demo/prompts/` | Prompt templates | Add yield-router.md |
| `apps/mirage-rs/` | EVM fork simulator | Ensure mainnet fork config |
| `Cargo.toml` | Workspace members | If adding new crates |

## Appendix C: Dashboard Integration Points (for Sam)

Roko emits, Sam consumes. No API server in roko — just structured event output.

| What Roko Emits | Format | Sam Consumes As |
|-----------------|--------|-----------------|
| DemoEvent stream | NDJSON stdout or WebSocket | Live activity feed, swarm view |
| C-Factor report | JSON file | Comparison panel, benchmark chart |
| Knowledge graph state | JSON in event stream | Knowledge graph visualization |
| Per-agent metrics | In DemoEvent fields | Agent cards, leaderboard |
| Fee breakdown | FeesDistributed events | Fee distribution panel |
| Multi-round data | Per-round events | Learning curve chart |
