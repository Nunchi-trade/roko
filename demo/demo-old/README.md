# Roko Demo Environment

Manifest-driven deploy + fixture + multi-agent orchestrator for the roko stack.
Runs **scripted scenarios** against `mirage-rs` (roko's in-process EVM), showing
ERC-8004 identity, ERC-8183 escrow, 2-of-3 consortium validation, and knowledge
flywheel pheromone curation.

---

## Table of contents

1. [State: what exists](#state-what-exists)
2. [Quick start](#quick-start)
3. [Running scenarios](#running-scenarios)
4. [Example outputs](#example-outputs)
5. [Test suites](#test-suites)
6. [What gets run under the hood](#what-gets-run-under-the-hood)
7. [Extending the demo](#extending-the-demo)
8. [Building a UI](#building-a-ui)
9. [Docker](#docker)
10. [Known limitations](#known-limitations)

---

## State: what exists

| Layer | Location | Status |
|---|---|---|
| **Solidity contracts** | `roko/contracts/src/*.sol` | 6 contracts, 36 forge tests green |
| **Deploy + fixture orchestrator** | `crates/roko-demo/` | 4 unit tests, 4 scenarios run E2E |
| **Alloy chain backend** | `crates/roko-chain/src/alloy_impl.rs` | behind `alloy-backend` feature; 3 live integration tests |
| **Typed contract bindings** | `crates/roko-demo/src/bindings.rs` | alloy `sol!` for all 6 contracts |
| **Scenario scripted spines** | `crates/roko-demo/src/scenarios/*.rs` | job-board, consortium, defi-routing, flywheel |
| **Declarative config** | `roko/demo/*.toml`, `roko/demo/prompts/*.md` | manifest + 4 scenario TOMLs + wallets + prompt templates |
| **Docker profile** | `docker/demo.Dockerfile` + `docker/docker-compose.yml` | `--profile demo` service |
| **LLM integration** | `crates/roko-demo/src/scenarios/llm.rs` | `StubLlm` deterministic stub; pluggable via `LlmProvider` trait |
| **UI** | — | **not built**; see §[Building a UI](#building-a-ui) |

### Contracts (`roko/contracts/src/`)

| Contract | Purpose | EIP |
|---|---|---|
| `MockERC20.sol` | "DAEJI" test token with open `mint()` faucet | ERC-20 |
| `AgentRegistry.sol` | Agent identity + capabilities + heartbeat liveness | ERC-8004 compat |
| `WorkerRegistry.sol` | Stake bonds, EMA reputation (α=0.2), tier thresholds, 30-day halving decay | — (Korai spec) |
| `BountyMarket.sol` | 4-state escrow (Open→Funded→Assigned→Submitted→Terminal), slash on reject | ERC-8183 style |
| `ConsortiumValidator.sol` | 3-agent blockhash-seeded committees, 2-of-3 voting, calls `market.resolve` | — |
| `InsightBoard.sol` | Post/confirm insights, pheromone weights, claimable earnings per confirmation | — |

### Scenarios (`roko/demo/scenarios/`)

| Scenario | Agents | Flow |
|---|---|---|
| `job-board` | 1 poster + 5 workers | 3 full job lifecycles: post → assign → submit → resolver accepts → pay |
| `consortium` | 1 poster + 1 worker + 3 validators | Post → submit → committee of 3 → each votes → 2-of-3 triggers resolve |
| `defi-routing` | 1 poster + 5 workers | Single job raced, first worker wins payout |
| `flywheel` | 3 posters + 3 confirmers | 3 rounds × 3 insights posted × 3 confirmers → 18 confirmations |

---

## Quick start

```bash
cd /Users/will/dev/uniswap/bardo/roko

# 1. Build mirage-rs once
cargo build -p mirage-rs --bin mirage-rs --release

# 2. Start mirage in the background
/Users/will/dev/uniswap/bardo/.mori/cache/cargo-target/release/mirage-rs \
  --host 127.0.0.1 --port 18545 --chain-id 31337 &

# 3. Run a scenario end-to-end
export ROKO_MIRAGE_URL=http://127.0.0.1:18545
cargo run -p roko-demo -- --demo-dir demo --runtime-dir demo/.runtime up job-board

# 4. Verify on-chain invariants
cargo run -p roko-demo -- --demo-dir demo --runtime-dir demo/.runtime verify job-board
```

Between scenarios: kill + restart mirage (`pkill -9 -f 'mirage-rs --host'`) and
`rm -rf demo/.runtime` to reset.

> **Note**: the cargo target dir is `$BARDO/.mori/cache/cargo-target/` because
> `bardo/.cargo/config.toml` pins it there. Substitute your own path if running
> elsewhere.

---

## Running scenarios

The orchestrator CLI lives at `crates/roko-demo/src/main.rs` and exposes:

```
roko-demo --demo-dir <DIR> --runtime-dir <DIR> [--rpc-url URL] <CMD>

CMD:
  list                 # print all registered scenarios from manifest
  deploy <scenario>    # deploy contracts only (writes deployments.json)
  seed <scenario>      # run fixtures only (requires prior deploy)
  up <scenario>        # deploy + seed + run scripted spine
  up <scenario> --no-agents
                       # deploy + seed, skip the spine
  verify <scenario>    # assert bytecode + expected events
  benchmark c-factor [yield-routing] [--output file.json]
                       # cold vs warm benchmark on the yield-routing spine
  tournament [yield-routing] [--rounds N]
                       # multi-round learning curve on one deployment
  autonomous [yield-routing] [--agents N] [--jobs N]
                       # concurrent poster/agent loop over the same spine
  tui [--scenario yield-routing]
                       # ratatui live demo view driven by emitted events
  register-agent --wallet worker0 --name alpha --model gemma-7b
                       # mint, stake, worker-register, and agent-register in one command
```

Environment overrides (in precedence order):
- `--rpc-url` flag
- `ROKO_MIRAGE_URL` env var
- `[defaults].rpc_url` in `demo/manifest.toml`
- fallback: `http://127.0.0.1:8545`

Additional runtime flags:
- `--llm-backend <stub|claude|ollama|multi>` selects the scenario provider
- `--events <none|ndjson|ws|both>` chooses event sinks for dashboards/streaming
- `--persist-reputation` restores and saves `demo/.runtime/reputation.json`

All 4 scenarios run in **~240-700ms** each against mirage.

---

## Example outputs

### `roko-demo list`

```text
job-board             Job-board clade: N bidders race on a posted bounty (ERC-8183 escrow)
consortium            3-agent 2-of-3 validation committee
defi-routing          Multi-pool routing benchmark with knowledge-base insights
flywheel              Knowledge flywheel: posters earn as confirmers accumulate pheromone weight
```

### `roko-demo up job-board`

```text
INFO roko_demo: deploying 4 contracts rpc=http://127.0.0.1:18545 chain_id=31337
INFO roko_demo::deploy: deployed contract=MockERC20     address=0x5fbdb2315678afecb367f032d93f642f64180aa3 block=2
INFO roko_demo::deploy: deployed contract=AgentRegistry address=0xe7f1725e7734ce288f8367e1bb143e90bb3f0512 block=3
INFO roko_demo::deploy: deployed contract=WorkerRegistry address=0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0 block=4
INFO roko_demo::deploy: deployed contract=BountyMarket  address=0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9 block=5
INFO roko_demo: deploy complete deployments=demo/.runtime/job-board/deployments.json
INFO roko_demo::fixtures: running fixture fixture=authorize-market-to-update-reputation kind=ContractCall
INFO roko_demo::fixtures: contract-call ok contract=WorkerRegistry method=setAuthorized(address,bool)
INFO roko_demo::fixtures: fixtures complete: 1 step(s)
INFO roko_demo: running scripted spine scenario="job-board" timeout_s=300
INFO roko_demo::scenarios::job_board: job-board: preparing wallets + registrations
INFO roko_demo::scenarios::job_board: job-board: spine running, target=3 jobs
INFO roko_demo::scenarios::job_board: posted round=0 job_id=0 bounty_wei=10000000000000000000
INFO roko_demo::scenarios::job_board: assigned round=0 worker=worker0
INFO roko_demo::scenarios::job_board: resolved round=0 job_id=0
INFO roko_demo::scenarios::job_board: posted round=1 job_id=1 bounty_wei=40000000000000000000
INFO roko_demo::scenarios::job_board: assigned round=1 worker=worker1
INFO roko_demo::scenarios::job_board: resolved round=1 job_id=1
INFO roko_demo::scenarios::job_board: posted round=2 job_id=2 bounty_wei=70000000000000000000
INFO roko_demo::scenarios::job_board: assigned round=2 worker=worker2
INFO roko_demo::scenarios::job_board: resolved round=2 job_id=2
INFO roko_demo::scenarios::job_board: job-board: all 3 rounds complete
INFO roko_demo: spine complete
```

### `demo/.runtime/job-board/deployments.json`

```json
{
  "chain_id": 31337,
  "contracts": {
    "AgentRegistry":  "0xe7f1725e7734ce288f8367e1bb143e90bb3f0512",
    "BountyMarket":   "0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9",
    "MockERC20":      "0x5fbdb2315678afecb367f032d93f642f64180aa3",
    "WorkerRegistry": "0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0"
  },
  "deployed_at_block": 5
}
```

### `roko-demo verify job-board`

```text
ok:   WorkerRegistry at 0x9fe46736679d2d9a65f0992f2272de9f3c7fa6e0 (4902 bytes)
ok:   BountyMarket   at 0xcf7ed3acca5a467e9e704c703e8d87f634fb0fc9 (4095 bytes)
ok:   AgentRegistry  at 0xe7f1725e7734ce288f8367e1bb143e90bb3f0512 (2634 bytes)
ok:   MockERC20      at 0x5fbdb2315678afecb367f032d93f642f64180aa3 (1864 bytes)
ok:   BountyMarket.JobResolved fired 3 times (>= 1)

verify: OK
```

### `roko-demo up consortium`

```text
INFO roko_demo: deploying 4 contracts
INFO roko_demo::deploy: deployed contract=MockERC20           address=0x5fbd… block=2
INFO roko_demo::deploy: deployed contract=WorkerRegistry      address=0xe7f1… block=3
INFO roko_demo::deploy: deployed contract=BountyMarket        address=0x9fe4… block=4
INFO roko_demo::deploy: deployed contract=ConsortiumValidator address=0xcf7e… block=5
INFO roko_demo::fixtures: contract-call ok contract=WorkerRegistry method=setAuthorized(address,bool)  # market
INFO roko_demo::fixtures: contract-call ok contract=WorkerRegistry method=setAuthorized(address,bool)  # consortium
INFO roko_demo::fixtures: contract-call ok contract=BountyMarket   method=setResolver(address)
INFO roko_demo::scenarios::consortium: committee: 0xa0Ee… 0x2361… 0x14dC…
INFO roko_demo::scenarios::consortium: vote validator=validator2 approve=true
INFO roko_demo::scenarios::consortium: vote validator=validator1 approve=true
INFO roko_demo::scenarios::consortium: vote validator=validator0 approve=true
INFO roko_demo: spine complete
```

### `roko-demo up flywheel`

Emits 9 `InsightPosted` + 18 `InsightConfirmed` events:

```text
INFO roko_demo::deploy: deployed contract=MockERC20    address=0x5fbd…
INFO roko_demo::deploy: deployed contract=InsightBoard address=0xe7f1…
INFO roko_demo: running scripted spine scenario="flywheel"
INFO roko_demo: spine complete
ok:   InsightBoard at 0xe7f1725e7734ce288f8367e1bb143e90bb3f0512 (2749 bytes)
ok:   MockERC20    at 0x5fbdb2315678afecb367f032d93f642f64180aa3 (1864 bytes)
ok:   InsightBoard.InsightConfirmed fired 18 times (>= 1)
verify: OK
```

---

## Test suites

```bash
cd /Users/will/dev/uniswap/bardo/roko

# Solidity tests — 36 tests
(cd contracts && forge test)

# Rust unit tests — no network, no chain needed
cargo test -p roko-chain --lib                      # 52 tests
cargo test -p roko-demo --lib                       # 4 tests
cargo test -p mirage-rs --lib                       # 141 tests (regression)

# Live integration — needs mirage running on $ROKO_TEST_RPC_URL
ROKO_TEST_RPC_URL=http://127.0.0.1:18545 \
  cargo test -p roko-chain --features alloy-backend --test alloy_live   # 3 tests
```

Current green totals:

| Suite | Count |
|---|---|
| forge (contracts) | **36** |
| roko-chain unit | **52** |
| roko-chain alloy live | **3** |
| roko-demo unit | **4** |
| mirage-rs regression | **141** |
| **Total** | **236** |

---

## What gets run under the hood

A scenario's `up` command executes **4 phases** in order.

### Phase 1 — Chain warmup
`roko-demo` pokes `eth_blockNumber`, and if the tip is 0, calls `evm_mine` once
to populate mirage's `blocks_by_number` map so alloy's `eth_getBlockByNumber`
lookups succeed. (`crates/roko-demo/src/deploy.rs::warmup_chain`)

### Phase 2 — Deploy contracts
- Loads forge artifacts from `contracts/out/<Name>.sol/<Name>.json` (runs
  `forge build` if `out/` is missing).
- ABI-encodes constructor args from the scenario TOML.
- Submits each deploy tx via `alloy::providers::ProviderBuilder::new().wallet(..).connect_http(url)`.
- Waits for receipts; extracts `contractAddress` from each.
- Writes `demo/.runtime/<scenario>/deployments.json`.

Addresses are deterministic: same deployer EOA (from `wallets.toml`) + fresh
mirage genesis → same deploy order → same nonce-derived addresses.

### Phase 3 — Run fixtures
Each `[[fixtures]]` entry dispatches on `kind`:

| Kind | Backend |
|---|---|
| `contract-call` | Parse solidity signature `method(args)`, ABI-encode args (resolving `$contract(Name)` refs), submit via alloy + wallet from `from =` |
| `jsonrpc` | POST a `{method, params}` directly to the chain |
| `forge-script` | Shell out `forge script <path> --broadcast --slow --private-key <resolved>` in `contracts/` dir |
| `rust` | Look up handler by name in `FixtureRegistry`, invoke with TOML args |

### Phase 4 — Scripted spine
Each scenario's `Scenario::spine(ctx, manifest, llm)` runs inline in the
`roko-demo` process. No subprocess fan-out, no separate containers — just
sequential alloy calls.

LLM "leaves" (bounded-random stubs today) produce structured JSON for params
like `bounty_amount`, `submission_content`, `approve`. The spine validates and
signs the resulting tx itself — **the LLM never sees a private key**.

### Phase 5 — Verify (optional separate command)
Reads `deployments.json`, asserts:
1. Every listed contract has bytecode at its address.
2. Every `[success.expected_events]` fired ≥ `min_count` times (via `eth_getLogs`
   filtered by contract address + topic0).

---

## Extending the demo

The **config is the API**. Adding new things rarely requires touching glue
code.

### Adding a new contract

1. Write `roko/contracts/src/Foo.sol`.
2. Add a test at `roko/contracts/test/Foo.t.sol`, run `forge test`.
3. Reference it in the scenario's `[[deploy.contracts]]`:
   ```toml
   [[deploy.contracts]]
   name = "Foo"
   args = [
     { type = "address", value = "$contract(MockERC20)" },
     { type = "uint256", value = 42 },
   ]
   ```
4. Add a `sol!` block in `crates/roko-demo/src/bindings.rs` if scenarios need
   typed calls:
   ```rust
   #[sol(rpc)]
   contract Foo {
       function doThing(uint256 amount) external;
       event ThingDone(address indexed who, uint256 amount);
   }
   ```

Supported constructor arg types: `uint8..uint256`, `int8..int256`, `address`,
`string`, `bool`, `bytes`, `bytesN`.

Special refs inside `value`:
- `"$deployer"` → the deploying EOA address
- `"$contract(Name)"` → address of a previously-deployed contract in this run

### Adding a new fixture

Drop another `[[fixtures]]` block into the scenario TOML. For example, to mint
tokens to an agent:

```toml
[[fixtures]]
name = "fund-alice"
kind = "contract-call"
contract = "MockERC20"
method = "mint(address,uint256)"
from = "deployer"
args = ["0xa11ce000000000000000000000000000000000a1", 10000000000000000000000]
```

Or call a custom mirage precompile:

```toml
[[fixtures]]
name = "seed-insights"
kind = "jsonrpc"
method = "chain_postInsight"
params = [{ author = "alice", kind = "insight", content = "test" }]
iterations = 10
```

### Adding a new scenario

1. Create `roko/demo/scenarios/my-scenario.toml` with `[deploy]`, `[[fixtures]]`,
   `[[agents]]`, `[success]` sections.
2. Add an entry to `roko/demo/manifest.toml`:
   ```toml
   [[scenarios]]
   name = "my-scenario"
   path = "scenarios/my-scenario.toml"
   description = "what it does"
   ```
3. Implement the scripted spine at
   `crates/roko-demo/src/scenarios/my_scenario.rs`:
   ```rust
   use async_trait::async_trait;
   use std::sync::Arc;
   use crate::chain_ctx::ChainCtx;
   use crate::manifest::Scenario as ScenarioManifest;
   use crate::scenarios::{LlmProvider, Scenario};

   pub struct MyScenario;

   #[async_trait]
   impl Scenario for MyScenario {
       fn name(&self) -> &'static str { "my-scenario" }
       async fn spine(
           &self,
           ctx: Arc<ChainCtx>,
           _manifest: &ScenarioManifest,
           _llm: Arc<dyn LlmProvider>,
       ) -> anyhow::Result<()> {
           // ... your script ...
           Ok(())
       }
   }
   ```
4. Register in `crates/roko-demo/src/scenarios/mod.rs`:
   ```rust
   pub mod my_scenario;
   // …
   pub fn all() -> Vec<Box<dyn Scenario>> {
       vec![
           Box::new(job_board::JobBoard),
           Box::new(my_scenario::MyScenario),  // ← add here
           // ...
       ]
   }
   ```
5. `cargo run -p roko-demo -- up my-scenario` — done.

### Adding a new agent role (prompt-level)

1. Write a prompt template at `roko/demo/prompts/<role>.md` using `{{placeholder}}`
   variables that your spine will substitute.
2. Add a wallet entry to `roko/demo/wallets.toml`.
3. Declare the agent in the scenario TOML:
   ```toml
   [[agents]]
   role = "new-role"
   wallet = "new{i}"
   prompt_template = "prompts/new-role.md"
   count = 3
   scripted_actions = ["my_action"]
   llm_slots = ["decision", "rationale"]
   ```
4. The scenario's `spine()` drives behaviour — use
   `llm.fill(LlmRequest { slot: "decision", .. }).await?` inside it.

### Swapping the LLM stub for a real model

`StubLlm` in `crates/roko-demo/src/scenarios/llm.rs` is pluggable. Implement
`LlmProvider` against your preferred backend:

```rust
pub struct ClaudeCliProvider { /* ... */ }

#[async_trait]
impl LlmProvider for ClaudeCliProvider {
    async fn fill(&self, req: LlmRequest) -> anyhow::Result<serde_json::Value> {
        // 1. render prompt template (demo/prompts/<role>.md) with `req.context`
        // 2. spawn `claude -p "<rendered prompt>"` subprocess
        // 3. parse stdout as JSON
        // 4. validate against the requested slot schema
    }
}
```

Then pass it to `main.rs` where `StubLlm::new()` is instantiated today.

---

## Building a UI

**There is no UI today**. All observation is via logs + `verify`. Here are
three paths, ordered by scope.

### Option 1: ratatui TUI (~500 LOC)

New `roko-demo watch <scenario>` subcommand that polls alloy every N ms and
renders a TUI with:

```
┌─ roko-demo / job-board ────────────────────────────────────────────┐
│ block: 18           chain_id: 31337        rpc: mirage:8545        │
├─ contracts ────────────────────────────────────────────────────────┤
│   MockERC20      0x5fbd…80aa3     AgentRegistry   0xe7f1…0512      │
│   WorkerRegistry 0x9fe4…a6e0      BountyMarket    0xcf7e…0fc9      │
├─ wallets ──────────────────────────────────────────────────────────┤
│   poster0   0x7099…79C8    985.0 DAEJI   nonce 4                   │
│   worker0   0x3C44…93BC     10.01 DAEJI  nonce 3  (Standard, 1.0K) │
│   worker1   0x90F7…3b906    10.01 DAEJI  nonce 3  (Standard, 1.0K) │
├─ jobs ─────────────────────────────────────────────────────────────┤
│   #0  Funded→Terminal  bounty 10 DAEJI  worker0  ACCEPTED          │
│   #1  Funded→Terminal  bounty 40 DAEJI  worker1  ACCEPTED          │
│   #2  Funded→Terminal  bounty 70 DAEJI  worker2  ACCEPTED          │
└────────────────────────────────────────────────────────────────────┘
```

Reuse: `ChainCtx`, `bindings::*`, `verify::Deployments`, alloy event filters.
New: a `crates/roko-demo/src/ui.rs` module, ratatui + crossterm deps.

**Why it's cheap**: you already have typed bindings + `get_logs` working; the
TUI is just `tokio::select!` over a poll timer + event filter.

### Option 2: Axum + HTMX dashboard (~1k LOC)

`roko-demo serve --port 8080` launches an HTTP server that renders the same
data as HTML + HTMX-swaps panels every 2s. Add:
- `axum`, `tower-http/fs`, `askama` or hand-rolled templates
- `GET /` — dashboard page
- `GET /fragments/jobs` — HTMX partial, swapped into `#jobs-panel`
- `GET /fragments/events?since=<block>` — streaming events

Richer than the TUI, browser-friendly, and a stepping stone to (3).

### Option 3: Timeline + agent graph (2-3k LOC)

Full web app rendering:
- **Timeline view**: stacked events by scenario round showing lifecycle
  transitions as swimlanes.
- **Graph view**: D3 / dagre SVG — nodes are agents + jobs, edges are tx
  causality. Animates as new events land.

Best for demoing the "agent clade" narrative. Needs frontend build tooling
(vite/esbuild) and a small JSON API.

### What would NOT work as a UI

Do not try to stream `roko-demo`'s tracing logs into the UI — they contain
debug noise. Always use **chain state + events** as the source of truth,
queried via alloy. That keeps the UI independent of tracing config and lets
you rewind across reruns.

---

## Docker

A `demo` compose profile bundles mirage + the orchestrator:

```bash
cd /Users/will/dev/uniswap/bardo/roko/docker
SCENARIO=job-board docker compose --profile demo up --build --exit-code-from roko-demo
```

Services invoked:
- **mirage** (always) — EVM fork simulator on :8545, healthcheck on `eth_blockNumber`
- **roko-demo** (profile=demo only) — depends on mirage healthy, runs `up $SCENARIO` then exits
- **prometheus** + **grafana** (always) — scrape mirage metrics; no demo-specific dashboards yet

Exit code 0 means the scenario completed + all success criteria met.

```bash
# cycle through all 4
for s in job-board consortium defi-routing flywheel; do
  docker compose down -v
  SCENARIO=$s docker compose --profile demo up --build --exit-code-from roko-demo
done
```

---

## Known limitations

**Mirage-rs is not a full anvil replacement.** It's a fork simulator; the
following have been added/patched to make scenarios work but still have rough
edges:

- `eth_getLogs` scans stored receipts only — supports `address` + `topics[0]`
  filters; does not support topic slot matching (topics[1..3]) or `blockHash`
  filter yet.
- `evm_mine` must be called once (or `warmup_chain` in roko-demo does it) to
  materialise a local block 0 — mirage lazy-inits its `blocks_by_number` map.
- Subscription RPCs (`eth_subscribe`) return "internal error" — fine for polling
  clients (alloy/viem default to polling) but may break WebSocket-first tools.
- Forge's broadcast-and-watch loop **does not work** reliably against mirage
  (forge thinks txs are dropped after mining). `roko-demo` uses alloy directly
  to sidestep this; `forge script` is still usable for local pure-EVM deploys
  but not against mirage from the orchestrator.

**Scripted spine is not truly autonomous.** Agents today execute a fixed
sequence with stub-LLM fills. Swap `StubLlm` for `ClaudeCliProvider` (or an
OpenAI/Ollama impl) to get real autonomy. Prompt templates in
`demo/prompts/*.md` already reference `{{wallet_address}}`, `{{job_id}}`,
`{{recent_jobs}}` etc. — your LlmProvider impl should render those substitutions.

**No reputation accrual across reruns.** Each scenario spins up fresh workers
with default reputation (0.5). If you want cumulative behaviour, either:
- persist mirage state between runs, or
- add a `[[fixtures]]` step that bumps reputation via `updateReputation` calls
  before the spine.

---

## File map

```
roko/
├── contracts/               # Foundry project
│   ├── foundry.toml
│   ├── remappings.txt
│   ├── lib/                 # forge-std, OpenZeppelin v5 (git clones)
│   ├── src/                 # 6 .sol contracts
│   ├── test/                # 6 .t.sol forge tests (36 tests)
│   └── script/Deploy.s.sol  # reference forge deploy (not used by roko-demo)
│
├── demo/                    # declarative config
│   ├── manifest.toml        # scenario registry
│   ├── wallets.toml         # 10 dev wallets (anvil defaults)
│   ├── scenarios/           # 4 scenario TOMLs
│   ├── prompts/             # 4 prompt templates for LLM roles
│   └── .runtime/            # written by roko-demo (deployments.json per scenario)
│
├── crates/
│   ├── roko-demo/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs          # CLI entry (list/deploy/seed/up/verify)
│   │       ├── lib.rs
│   │       ├── manifest.rs      # TOML schema + loaders
│   │       ├── deploy.rs        # alloy-based deployer + ABI encoder
│   │       ├── fixtures.rs      # forge-script / jsonrpc / rust / contract-call dispatch
│   │       ├── chain_ctx.rs     # provider + wallet resolver
│   │       ├── bindings.rs      # alloy sol! for all 6 contracts
│   │       ├── verify.rs        # post-run invariant checks
│   │       └── scenarios/
│   │           ├── mod.rs       # Scenario trait + registry
│   │           ├── llm.rs       # LlmProvider + StubLlm
│   │           ├── job_board.rs
│   │           ├── consortium.rs
│   │           ├── defi_routing.rs
│   │           └── flywheel.rs
│   │
│   └── roko-chain/
│       ├── src/alloy_impl.rs    # real AlloyChainClient + AlloyChainWallet
│       └── tests/alloy_live.rs  # 3 live integration tests
│
└── docker/
    ├── docker-compose.yml   # extended with `demo` profile
    └── demo.Dockerfile      # multi-stage: forge → cargo → runtime
```
