# Section B: Demo Features

Source: `tmp/demo/tasks/` (T1.1-T3.8 + T2.5)
Target crate: `crates/roko-demo/`
Target events: A16Z Demo (Apr 25), Consensus Miami (May 7)

**CRITICAL**: All tasks must apply corrections from `tmp/demo/tasks/ERRATA.md` (contract function signatures, ID types, token amounts, dependency versions).

---

## B.01 — InsightBoard getInsight Binding (T2.5)

**Status**: NOT DONE
**Priority**: P0 (blocks B.08, B.10)
**Estimated LOC**: ~15
**Dependencies**: None (first in build order)

### Files to modify

- `crates/roko-demo/src/bindings.rs` — Add `getInsight` to `InsightBoard` sol! block

### Context

Solidity `InsightBoard.sol` already has `getInsight(uint256 id)`. Only the Rust binding is missing. Needed by knowledge loop (B.08) and InsightBoard enhancements (B.10).

### Implementation details

1. Add to the `InsightBoard` sol! block in `bindings.rs`:
   ```rust
   function getInsight(uint256 id) external view returns (
       address poster,
       bytes32 contentHash,
       string memory uri,
       uint256 pheromone,
       uint256 confirmations,
       uint256 timestamp
   );
   ```
2. Option A: struct return with `Insight` struct in the macro
3. Option B (fallback): named tuple return as shown above
4. All IDs are `U256`, NOT `u64` (per ERRATA)

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
```

---

## B.02 — FeeDistributor Solidity Contract (T1.3)

**Status**: NOT DONE
**Priority**: P0 (blocks B.09)
**Estimated LOC**: ~120
**Dependencies**: None

### Files to modify

- `contracts/src/FeeDistributor.sol` — **NEW FILE**
- `contracts/test/FeeDistributor.t.sol` — **NEW FILE**

### Context

No fee distribution mechanism exists. BountyMarket pays 100% to worker. Need 40/30/20/10 split (validators/data providers/agent/treasury).

### Implementation details

1. Create `FeeDistributor.sol`:
   - `distribute(uint256 jobId, uint256 amount, address winner, address[] validators, address[] dataProviders)` — external
   - Basis points: 4000 validators / 3000 data providers / 2000 agent / 1000 treasury
   - `cumulativeEarnings` mapping: `address => uint256`
   - Events: `FeesDistributed(jobId, amount, winner, validatorShare, dataShare, agentShare, treasuryShare)`, `EarningsCredited(address, uint256)`
   - Treasury address set in constructor
   - Requires ERC-20 token approval before calling
2. Create `FeeDistributor.t.sol` with 7 tests:
   - Basic split, empty validators, empty data providers, single validator, cumulative earnings, events emitted, rounding (no tokens lost)

### Verify command

```bash
cd contracts && forge test --match-contract FeeDistributor -v 2>&1 | tail -10
```

---

## B.03 — Real LLM Providers (T1.1)

**Status**: NOT DONE
**Priority**: P0 (blocks B.07)
**Estimated LOC**: ~200
**Dependencies**: None

### Files to modify

- `crates/roko-demo/src/scenarios/llm.rs` — **NEW FILE** — Provider implementations
- `crates/roko-demo/src/main.rs` — Add `--llm-backend` CLI flag

### Context

All demo scenarios use `StubLlm`. Need real AI reasoning for demo. `StubLlm` must remain unchanged for testing.

### Implementation details

1. Create `scenarios/llm.rs` with `LlmProvider` trait:
   ```rust
   #[async_trait]
   pub trait LlmProvider: Send + Sync {
       async fn generate(&self, prompt: &str, slots: &[&str]) -> anyhow::Result<HashMap<String, String>>;
       fn label(&self) -> &str;
   }
   ```
2. Implement `ClaudeApiProvider`:
   - Anthropic Messages API, env `ANTHROPIC_API_KEY`
   - Default model: `claude-sonnet-4-20250514`
   - Parse slot values from response
3. Implement `OllamaProvider`:
   - Env `OLLAMA_URL` (default `http://localhost:11434`), `OLLAMA_MODEL` (default `gemma3:7b`)
4. Implement `MultiProvider`:
   - Round-robin across providers with per-provider labels
5. Factory function `create_provider(backend: &str) -> Box<dyn LlmProvider>`:
   - `"stub"` → `StubLlm`, `"claude"` → `ClaudeApiProvider`, `"ollama"` → `OllamaProvider`, `"multi"` → `MultiProvider`
6. Add `--llm-backend` CLI flag to `main.rs` (default `"stub"`)

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --llm-backend stub yield-routing 2>&1 | tail -10
```

---

## B.04 — Event Stream Infrastructure (T1.4)

**Status**: NOT DONE
**Priority**: P0 (blocks B.07)
**Estimated LOC**: ~180
**Dependencies**: None

### Files to modify

- `crates/roko-demo/src/events.rs` — **NEW FILE** — `DemoEvent` enum + `EventEmitter` trait
- `crates/roko-demo/src/ws_server.rs` — **NEW FILE** — WebSocket broadcast server
- `crates/roko-demo/src/lib.rs` — Add modules
- `crates/roko-demo/src/main.rs` — Add `--events` and `--ws-port` flags
- `crates/roko-demo/Cargo.toml` — Add `tokio-tungstenite` dep

### Context

Dashboard needs real-time events from demo scenarios. Currently no event infrastructure exists.

### Implementation details

1. Define `DemoEvent` enum with 20 variants, tagged `#[serde(tag = "type", rename_all = "snake_case")]`:
   - `ScenarioStarted`, `RoundStarted`, `RoundCompleted`, `ScenarioCompleted`
   - `JobPosted`, `JobAssigned`, `AgentBid`, `ExecutionStarted`, `ExecutionCompleted`
   - `ValidationVote`, `ValidationComplete`, `FeesDistributed`
   - `InsightPosted`, `InsightConfirmed`, `KnowledgeQueried`
   - `CFactorMeasured`, `ReputationUpdated`, `AgentSlashed`
   - `KnowledgeGraphUpdate`, `Error`
2. Define `EventEmitter` trait: `async fn emit(&self, event: DemoEvent)`
3. Implement 3 backends:
   - `NullEmitter` — no-op
   - `NdjsonEmitter` — println per event (newline-delimited JSON)
   - `WsEmitter` — tokio-tungstenite broadcast to all connected clients
4. `CompositeEmitter` — fans out to multiple backends
5. `create_emitter(mode: &str, ws_port: u16) -> Box<dyn EventEmitter>` factory
6. CLI flags: `--events none|ndjson|ws|both` and `--ws-port 9090`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson yield-routing 2>&1 | head -20
```

---

## B.05 — Yield Routing Scenario Skeleton (T1.2)

**Status**: NOT DONE
**Priority**: P0 (blocks B.07)
**Estimated LOC**: ~250
**Dependencies**: None (but benefits from B.01, B.02)

### Files to modify

- `demo/scenarios/yield-routing.toml` — **NEW FILE** — Fixture config
- `crates/roko-demo/src/scenarios/yield_routing.rs` — **NEW FILE** — Scenario implementation
- `demo/prompts/yield-router.md` — **NEW FILE** — Prompt template
- `crates/roko-demo/src/scenarios/mod.rs` — Register scenario
- `demo/manifest.toml` — Add scenario entry

### Context

Core demo scenario: "Route 100K USDC → ETH, maximize output" with 5 agents bidding, EVM execution, consortium validation, knowledge posting, and C-Factor measurement across 2 rounds.

### Implementation details

1. Create fixture TOML deploying all 6 contracts (format: `[[fixtures]]` flat array per ERRATA)
2. Register 5 workers (`worker0`-`worker4`) with model labels `["claude-sonnet-4", "gemma-27b", "gemma-7b", "claude-haiku", "llama-3.2"]`
3. Seed 3 baseline insights on InsightBoard
4. Two-round spine:
   - Post job on BountyMarket: `postJob(specHash, bounty, deadline, minTier)` (4 args per ERRATA)
   - Collect bids from 5 agents (LLM `route_proposal` slot)
   - Select winner by max `expected_output`
   - Assign winner, submit result
   - Assemble 3-validator committee, vote, resolve: `resolve(id, accepted)` (2 args per ERRATA)
   - Winner posts insight to InsightBoard: `post(contentHash, uri)` (2 args per ERRATA)
   - Round 2 repeats with insights from round 1
5. Uses `StubLlm` throughout (real LLM wired in B.07)
6. Token amounts: use `10u128.pow(18)` NOT `1e18 as u128` (per ERRATA)

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- yield-routing 2>&1 | tail -20
```

---

## B.06 — Multi-Model Labeling (T3.2)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~30
**Dependencies**: B.03

### Files to modify

- `crates/roko-demo/src/scenarios/llm.rs` — Add `fn label(&self) -> &str` default method
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Per-agent model labels

### Context

Each of the 5 demo agents should display a distinct model label regardless of actual backend.

### Implementation details

1. Add `fn label(&self) -> &str` default method to `LlmProvider` trait (default: `"unknown"`)
2. Override in each concrete type: `StubLlm` → `"stub"`, `ClaudeApiProvider` → `&self.model`, `OllamaProvider` → `&self.model`, `MultiProvider` → `"multi"`
3. In `yield_routing.rs`, add `fn agent_model(index: usize) -> &'static str` returning from `AGENT_MODELS = ["claude-sonnet-4", "gemma-27b", "gemma-7b", "claude-haiku", "llama-3.2"]`
4. Use `agent_model(i)` instead of global `model_label` in bid events and logs

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
```

---

## B.07 — Full Yield Routing with LLM + Events (T2.1)

**Status**: NOT DONE
**Priority**: P0 (main integration task)
**Estimated LOC**: ~200
**Dependencies**: B.03, B.04, B.05

### Files to modify

- `crates/roko-demo/src/scenarios/mod.rs` — Add `events: Arc<dyn EventEmitter>` to `Scenario::spine`
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Emit events at every step
- `crates/roko-demo/src/scenarios/job_board.rs` — Accept `_events` param
- `crates/roko-demo/src/scenarios/consortium.rs` — Accept `_events` param
- `crates/roko-demo/src/scenarios/defi_routing.rs` — Accept `_events` param
- `crates/roko-demo/src/scenarios/flywheel.rs` — Accept `_events` param
- `crates/roko-demo/src/main.rs` — Wire emitter into scenario runner

### Context

This is the main integration task combining LLM providers (B.03), event stream (B.04), and yield routing skeleton (B.05) into a fully working demo scenario.

### Implementation details

1. Add `events: Arc<dyn EventEmitter>` parameter to `Scenario::spine()` trait method
2. Update existing scenarios to accept `_events` parameter (no-op)
3. In `yield_routing.rs`, emit events at every step. Full event order:
   ```
   ScenarioStarted → RoundStarted → JobPosted → KnowledgeQueried×5 →
   AgentBid×5 → JobAssigned → ExecutionStarted → ExecutionCompleted →
   ValidationVote×3 → ValidationComplete → InsightPosted×5 →
   RoundCompleted → [repeat round 2] → CFactorMeasured → ScenarioCompleted
   ```
4. Replace `StubLlm` calls with `provider.generate()` calls using `create_provider(backend)`
5. Wire emitter creation in `main.rs` and pass to scenario runner

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson --llm-backend stub yield-routing 2>&1 | grep '"type"' | head -20
```

---

## B.08 — Knowledge Loop Integration (T2.2)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~80
**Dependencies**: B.01, B.07

### Files to modify

- `crates/roko-demo/src/scenarios/yield_routing.rs` — Add knowledge query/post logic

### Context

Agents should query InsightBoard before bidding and post new insights after winning. Currently no knowledge loop — agents bid blindly.

### Implementation details

1. Add `query_insights()` function: reads all board insights via `getInsight()`, sorts by pheromone (highest first)
2. Before each agent bids, query InsightBoard and inject results as `prior_insights` in LLM context
3. After each round, winner LLM-generates insight text (`insight_content` slot)
4. Winner posts insight: `InsightBoard.post(contentHash, uri)` with URI `demo://yield-routing:{content}`
5. Two other workers confirm the insight (call `confirm()` on InsightBoard)
6. Seed 3 initial insights before round 1 so agents have prior knowledge from start
7. Emit `KnowledgeQueried` event per agent, `InsightPosted` after posting, `InsightConfirmed` per confirmation

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson yield-routing 2>&1 | grep 'knowledge_queried\|insight_posted' | wc -l
# Should see 5 knowledge_queried per round + insight posts
```

---

## B.09 — Fee Distribution Wiring (T2.3)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~60
**Dependencies**: B.02, B.07

### Files to modify

- `crates/roko-demo/src/bindings.rs` — Add `FeeDistributor` sol! block
- `demo/scenarios/yield-routing.toml` — Add FeeDistributor to deploy list
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Wire fee distribution after resolution

### Context

BountyMarket pays 100% to worker. Need to wire the FeeDistributor contract (B.02) into the yield routing scenario for 40/30/20/10 split.

### Implementation details

1. Add `FeeDistributor` sol! block to `bindings.rs` with `distribute()` function signature
2. Add FeeDistributor to `yield-routing.toml` deploy list
3. After `BountyMarket::resolve()` succeeds:
   - Winner approves FeeDistributor for bounty amount
   - Call `FeeDistributor::distribute(jobId, bountyAmount, winnerAddr, validatorAddrs, dataProviderAddrs)`
4. Emit `DemoEvent::FeesDistributed` with amounts formatted as "N DAEJI"
5. All token amounts use `10u128.pow(18)` (per ERRATA)

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson yield-routing 2>&1 | grep 'fees_distributed'
```

---

## B.10 — C-Factor Benchmark (T2.4)

**Status**: NOT DONE
**Priority**: P0 (deadline: Apr 15)
**Estimated LOC**: ~100
**Dependencies**: B.07, B.08

### Files to modify

- `crates/roko-demo/src/benchmark.rs` — **NEW FILE** — Benchmark runner
- `crates/roko-demo/src/lib.rs` — Add module
- `crates/roko-demo/src/main.rs` — Add `benchmark c-factor` subcommand

### Context

Need to prove "the loop made itself smarter" — measure output improvement between cold (no insights) and warm (with round-1 insights) runs.

### Implementation details

1. Create `benchmark.rs` with `CFactorReport` type:
   ```rust
   pub struct CFactorReport {
       pub run_a: RoundOutcome, // cold run (empty InsightBoard)
       pub run_b: RoundOutcome, // warm run (with round-1 insights)
       pub c_factor: CFactorMetrics,
   }
   pub struct CFactorMetrics {
       pub output_improvement_pct: f64, // target: >= 15%
   }
   ```
2. Factor out `run_cold_round()` and `run_warm_round()` from `yield_routing.rs`
3. `run_benchmark()`: run cold round → seed insights → run warm round → compute improvement
4. JSON to stdout, human summary to stderr
5. `--output file.json` writes to file
6. Add `benchmark c-factor` subcommand to `main.rs`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- benchmark c-factor 2>&1 | tail -5
```

---

## B.11 — TUI Demo Mode (T3.1)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~200
**Dependencies**: B.04, B.07

### Files to modify

- `crates/roko-demo/src/tui.rs` — **NEW FILE** — ratatui terminal UI
- `crates/roko-demo/src/main.rs` — Add `tui` subcommand
- `crates/roko-demo/Cargo.toml` — Add `ratatui = "0.29"`, `crossterm = "0.28"` (direct deps, NOT workspace per ERRATA)

### Context

4-panel ratatui layout for live demo visualization. Receives events via channel from scenario runner.

### Implementation details

1. Define `TuiState` with:
   - `agents: Vec<AgentState>` — id, model, reputation, earned, status (Idle/Querying/Bidding/Executing/Winner)
   - `log: Vec<LogEntry>` — timestamp, message, color
   - `knowledge: KnowledgeState` — insight count, latest insights
   - `economics: EconomicsState` — total distributed, per-agent earnings, treasury
2. 4-panel layout:
   - Top bar: title + current round
   - Middle row: agents panel (20%) + activity log (80%)
   - Bottom row: knowledge panel (20%) + economics panel (80%)
3. `ChannelEmitter` wraps `mpsc::Sender<DemoEvent>` to implement `EventEmitter`
4. Render loop: drain channel → update state → draw → poll keyboard (j/k scroll, q/Esc quit) → 50ms tick
5. Always restore terminal on exit (raw mode cleanup in Drop or panic hook)
6. CLI: `tui [--scenario yield-routing]`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- tui --help 2>&1 | head -5
```

---

## B.12 — Multi-Round Tournament (T3.3)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~120
**Dependencies**: B.07

### Files to modify

- `crates/roko-demo/src/tournament.rs` — **NEW FILE**
- `crates/roko-demo/src/lib.rs` — Add module
- `crates/roko-demo/src/main.rs` — Add `tournament` subcommand
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Refactor to expose `prepare()`, `run_round()`, `post_winner_insight()`

### Context

Run N rounds without redeploying contracts. Track learning curve and agent rankings.

### Implementation details

1. Refactor `yield_routing.rs` to expose:
   - `pub async fn prepare(...) -> PreparedScenario`
   - `pub async fn run_round(prepared: &PreparedScenario, round: usize) -> RoundOutcome`
   - `pub async fn post_winner_insight(prepared: &PreparedScenario, outcome: &RoundOutcome)`
2. Create `tournament.rs`:
   - `run_tournament(rounds: usize, ...)` calls `prepare` once, loops N rounds
   - `TournamentReport`: `rounds: Vec<RoundOutcome>`, `learning_curve: Vec<(usize, f64)>`, `final_rankings: Vec<AgentRanking>`
   - `AgentRanking`: agent_id, wins, total_output, avg_confidence
3. CLI: `tournament [--rounds 5] [yield-routing]`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- tournament --rounds 3 2>&1 | tail -10
```

---

## B.13 — Knowledge Graph JSON (T3.4)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: B.08

### Files to modify

- `crates/roko-demo/src/events.rs` — Add `KnowledgeGraphUpdate` variant
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Emit graph events

### Context

Dashboard needs a graph visualization of knowledge flow between agents and insights.

### Implementation details

1. Add to `DemoEvent`:
   ```rust
   KnowledgeGraphUpdate {
       round: usize,
       nodes: Vec<KnowledgeNode>,
       edges: Vec<KnowledgeEdge>,
   }
   ```
2. `KnowledgeNode`: `{ id, content, poster, pheromone_weight, confirmations }`
3. `KnowledgeEdge`: `{ from, to, kind }` where kind is `"posted"`, `"confirmed"`, or `"queried"`
4. `build_knowledge_nodes()` queries `InsightBoard` on-chain for current state
5. Emit after `InsightConfirmed` events and before `RoundCompleted` each round

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson yield-routing 2>&1 | grep 'knowledge_graph_update'
```

---

## B.14 — Reputation Persistence (T3.5)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~50
**Dependencies**: B.07

### Files to modify

- `crates/roko-demo/src/scenarios/yield_routing.rs` — Add persistence module
- `crates/roko-demo/src/main.rs` — Add `--persist-reputation` flag

### Context

Worker/validator reputations are lost between runs. Need persistence so multi-session demos show learning.

### Implementation details

1. Add private `persistence` module in `yield_routing.rs`:
   - `WorkerSnapshot { address, reputation, bond, tier, wins, losses }`
   - `ReputationFile { workers: Vec<WorkerSnapshot>, saved_at: u64 }`
2. `save_reputation()` — serialize to `demo/.runtime/reputation.json`
3. `restore_reputation()` — read on next run, re-apply via `updateReputation` loops
4. Add `runtime_dir: PathBuf` and `persist_reputation: bool` to `YieldRouting` struct
5. CLI: `--persist-reputation` flag

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --persist-reputation yield-routing 2>&1 | tail -5
ls demo/.runtime/reputation.json
```

---

## B.15 — One-Click Agent Registration (T3.6)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~60
**Dependencies**: B.05

### Files to modify

- `crates/roko-demo/src/main.rs` — Add `register-agent` subcommand

### Context

Streamline agent registration for demo setup. Currently requires manual EVM calls.

### Implementation details

1. Add `RegisterAgent { name, model, wallet, stake, scenario }` subcommand
2. Handler:
   - Read `deployments.json` for contract addresses
   - Mint DAEJI to worker address
   - Approve WorkerRegistry for stake amount
   - Call `WorkerRegistry.register(amount)` (ONE arg, caller is `msg.sender` per ERRATA)
   - Call `AgentRegistry.register(capabilities, passportHash)` (TWO args per ERRATA)
   - Read back `reputationOf` and `tier`
3. Print formatted confirmation:
   ```
   Agent registered!
   Name: {name}
   Model: {model}
   Wallet: {wallet}
   Stake: {stake} DAEJI
   Tier: Standard
   Time: {elapsed}s
   Ready to bid on jobs.
   ```

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- register-agent --help 2>&1 | head -5
```

---

## B.16 — Autonomous Agent Loop (T3.7)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~150
**Dependencies**: B.07, B.08

### Files to modify

- `crates/roko-demo/src/autonomous.rs` — **NEW FILE**
- `crates/roko-demo/src/lib.rs` — Add module
- `crates/roko-demo/src/main.rs` — Add `autonomous` subcommand

### Context

Move from scripted scenario to agents autonomously polling for jobs, bidding, and learning.

### Implementation details

1. Two concurrent task types:
   - `agent_loop` (5 concurrent): each polls for open jobs via `Mutex<Option<broadcast::Sender<AgentBid>>>`, queries InsightBoard, calls LLM, submits bid, waits for assignment, submits result, posts insight
   - `poster_loop`: posts N jobs with configurable interval, collects bids via broadcast, selects winner by max `expected_output`, calls assign + ConsortiumValidator + resolve
2. Use `tokio::select!` for timeouts and cancellation
3. CLI: `autonomous [--agents 5] [--jobs 3] [--interval 10] [--timeout 300]`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- autonomous --help 2>&1 | head -5
```

---

## B.17 — Adversarial Agent + Slashing (T3.8)

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~80
**Dependencies**: B.07

### Files to modify

- `crates/roko-demo/src/events.rs` — Add `AgentSlashed` variant
- `crates/roko-demo/src/scenarios/yield_routing.rs` — Add Phase 8

### Context

Demonstrate slashing mechanism for bad actors. Worker4 posts fabricated insight, gets caught by validators, loses bond.

### Implementation details

1. Add `AgentSlashed { agent_id, reason, slash_bps, new_bond, new_reputation }` to `DemoEvent`
2. Phase 8 (after both normal rounds):
   - 8a: `worker4` posts fabricated insight ("Compound V3 USDC/ETH pool has 99.9% utilization...")
   - 8b: Verification job posted with spec hash encoding bad insight ID; `worker4` assigned, submits `keccak256(b"bad-route")`
   - 8c: Committee assembled, all 3 validators vote `false`; job resolved rejected
   - 8d: Deployer calls `WorkerRegistry.slash(worker4_address, 2, 500)` — 5% bond reduction
   - Read new bond + reputation
3. Emit `AgentSlashed` event
4. Event order: `InsightPosted → JobPosted → JobAssigned → ValidationVote×3 → ValidationComplete → AgentSlashed → CFactorMeasured → ScenarioCompleted`

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
cargo run -p roko-demo -- --events ndjson yield-routing 2>&1 | grep 'agent_slashed'
```

---

## B.18 — InsightBoard Enhancements (T2.5 extended)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: B.01

### Files to modify

- `crates/roko-demo/src/bindings.rs` — Ensure full InsightBoard binding coverage

### Context

Several InsightBoard functions may need Rust bindings beyond `getInsight`. Confirm all needed functions have bindings: `post`, `confirm`, `getInsight`, `insightCount`, `pheromoneOf`.

### Implementation details

1. Audit `InsightBoard.sol` for all public functions
2. Ensure each has a corresponding entry in the `sol!` block in `bindings.rs`
3. Add any missing bindings
4. Confirm all ID types are `U256` (per ERRATA)

### Verify command

```bash
cargo build -p roko-demo 2>&1 | tail -5
```
