# `mirage-rs` examples

Runnable demos of the chain extensions, roko-bridge surface, and HTTP REST API.
Each example is gated behind the feature it exercises.

| Example | Feature | Description |
|---|---|---|
| `seed_chain_fixtures` | `chain` | Seed 50 insights + 20 pheromones via JSON-RPC, then verify all REST endpoints |
| `roko_chain_watcher` | `roko` | In-process push subscription demo (no network) |
| `persona_chain_native` | `roko` | HDC semantic search demo (no network) |

## Prerequisites

Start mirage-rs with all chain subsystems enabled:

```bash
cargo run -p mirage-rs --features chain --bin mirage-rs -- \
    --enable-hdc --enable-knowledge --enable-stigmergy
```

This starts the JSON-RPC server on `http://127.0.0.1:8545` with the REST API
available at `http://127.0.0.1:8545/api/*`.

State is persisted to `.roko/state/` by default, so seeded data survives restarts.
Add `--no-persist` if you want a clean slate every run, or `--state-dir /tmp/mirage`
to use a custom location.

---

## `seed_chain_fixtures`

Seeds a running mirage-rs with 50 `InsightEntry`s and 20 pheromones via JSON-RPC,
then exercises every HTTP REST API endpoint as a verification pass. Covers AMMs,
lending, liquidations, MEV, oracles, bridges, restaking, and governance.

**Run**:

```bash
cargo run -p mirage-rs --features chain --example seed_chain_fixtures -- \
    --rpc-url http://127.0.0.1:8545
```

The `--rpc-url` flag defaults to `http://127.0.0.1:8545`. If no mirage is
listening the seeder prints a friendly error and exits with code 1.

**Expected output** (trimmed):

```
seed_chain_fixtures: target = http://127.0.0.1:8545
seed_chain_fixtures: connected. stats={"insights":0,"pheromones":0,"toggles":{...}}
seed_chain_fixtures: phase 1 (JSON-RPC seeding) done.
  insights:   50 accepted / 0 failed  (of 50)
  pheromones: 20 deposited / 0 failed (of 20)

seed_chain_fixtures: phase 2 — REST API verification
  base url: http://127.0.0.1:8545/api

  [PASS] GET /api/health
  [PASS] GET /api/stats
  [PASS] POST /api/pheromones (deposit)
  [PASS] GET /api/pheromones (list)
  [PASS] GET /api/pheromones/summary
  [PASS] POST /api/pheromones/query (semantic search)
  [PASS] GET /api/pheromones/heatmap
  [PASS] GET /api/pheromones/7/projection
  [PASS] POST /api/knowledge/entries (post insight)
  [PASS] GET /api/knowledge/entries (list)
  [PASS] POST /api/knowledge/entries/.../confirm
  [PASS] POST /api/knowledge/entries/.../challenge
  [PASS] POST /api/knowledge/decay
  [PASS] GET /api/knowledge/edges
  [PASS] GET /api/knowledge/search
  [PASS] GET /api/knowledge/kinds
  [PASS] POST /api/agents (register rest-agent-alpha)
  [PASS] POST /api/agents (register rest-agent-beta)
  [PASS] POST /api/agents (register rest-agent-gamma)
  [PASS] GET /api/agents (list)
  [PASS] POST /api/agents/rest-agent-alpha/heartbeat
  [PASS] POST /api/agents/rest-agent-beta/heartbeat
  [PASS] POST /api/agents/rest-agent-gamma/heartbeat
  [PASS] GET /api/agents/rest-agent-alpha/heartbeat
  [PASS] GET /api/agents/rest-agent-alpha/stats
  [PASS] GET /api/agents/rest-agent-alpha/trace
  [PASS] GET /api/agents/topology

  REST API verification: 27 passed, 0 failed (of 27)

seed_chain_fixtures: all phases completed successfully.
```

---

## `roko_chain_watcher`

Pure in-process demo of the push-based subscription surface.
Spins up a `PheromoneBus` + `InsightBus`, registers a `VecSink` subscriber on
each, seeds 12 pheromones and 8 insights, then loops 5 times emitting a new
pheromone + decay event per tick. Prints a summary table with subscription
stats and the observed pheromone mix.

No network, no HTTP server, no LLM. Runs in under a second.

**Run**:

```bash
cargo run -p mirage-rs --features roko --example roko_chain_watcher
```

**Expected output** (trimmed):

```
== roko_chain_watcher ==
registered subscribers: pheromone=sub#1, insight=sub#1
seeded 12 pheromones (field size = 12)
seeded 8 insight events
initial drain: 12 pheromone events, 8 insight events

tick 0 (t+45m): field=13 evap=0 | new events: 1 pheromones, 1 insights
    pher#13 kind=Threat intensity=0.73
    insight: decayed #0100 -> weight 0.900
...
== summary ==
  pheromone field: 17 live entries
  pheromone sub #sub#1: delivered=17 dropped_oldest=0 dropped_newest=0 closed=false
  insight   sub #sub#1: delivered=13 dropped_oldest=0 dropped_newest=0 closed=false
  observed pheromone mix: threat=6 opportunity=6 wisdom=5
```

## `persona_chain_native`

Builds an `HdcSubstrate` for a chain-native Uniswap-analyst persona, puts 3
insights about Uniswap behaviour, then runs a semantic `Substrate::query` and
prints the top-3 hits ranked by HDC similarity x effective score.

Deterministic: no LLM, no RNG, no network. The text projection is a stable
hash of input tokens.

**Run**:

```bash
cargo run -p mirage-rs --features roko --example persona_chain_native
```

**Expected output**:

```
== persona_chain_native ==
persona = chain-native/uniswap-analyst

writing 3 insights into HdcSubstrate
  put [uniV3-stf-revert] -> hash <hex>
  put [uniV3-twap-depth] -> hash <hex>
  put [uniV4-hook-gas] -> hash <hex>

query (text_query) = "uniswap v3 STF reverts on low allowance"
top-3 hits:
  #1: effective_score=2.720  body="uniswap v3 STF revert typically means insufficient allowance on the input token"
  #2: effective_score=2.720  body="uniswap v3 TWAP oracle accuracy depends on pool liquidity depth; thin pools are manipulable"
  #3: effective_score=2.720  body="uniswap v4 hook invocations add ~20k gas when hooks are permissionless and untrusted"

ok: top hit matches the expected STF-revert insight.
persona_chain_native: done (3 entries in substrate).
```

---

## HTTP REST API curl examples

All endpoints live under `/api` on the same port as the JSON-RPC server (default 8545).
Requires chain extensions to be enabled.

### Health and stats

```bash
# Health check — subsystem status and counts
curl http://127.0.0.1:8545/api/health | jq

# Aggregated dashboard stats
curl http://127.0.0.1:8545/api/stats | jq
```

### Pheromone field

```bash
# List pheromones sorted by intensity (top 10)
curl 'http://127.0.0.1:8545/api/pheromones?sort=intensity&order=desc&limit=10' | jq

# Filter by kind
curl 'http://127.0.0.1:8545/api/pheromones?kind=threat&min_intensity=0.5' | jq

# Deposit a pheromone
curl -X POST http://127.0.0.1:8545/api/pheromones \
  -H 'Content-Type: application/json' \
  -d '{
    "kind": "threat",
    "content": "flash loan attack draining AAVE v3 WETH pool",
    "intensity": 1.0,
    "half_life_secs": 7200
  }' | jq

# Per-kind summary statistics
curl http://127.0.0.1:8545/api/pheromones/summary | jq

# Semantic search (HDC similarity)
curl -X POST http://127.0.0.1:8545/api/pheromones/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "flash loan attack on lending protocol", "k": 10}' | jq

# Time-bucketed heatmap (1-hour buckets)
curl 'http://127.0.0.1:8545/api/pheromones/heatmap?bucket_seconds=3600' | jq

# Decay projection for pheromone ID 7
curl 'http://127.0.0.1:8545/api/pheromones/7/projection?duration_secs=3600&points=60' | jq
```

### Knowledge graph

```bash
# List entries sorted by weight
curl 'http://127.0.0.1:8545/api/knowledge/entries?sort=weight&order=desc&limit=10' | jq

# Filter by kind and state
curl 'http://127.0.0.1:8545/api/knowledge/entries?kind=warning&state=confirmed' | jq

# Post a new insight
curl -X POST http://127.0.0.1:8545/api/knowledge/entries \
  -H 'Content-Type: application/json' \
  -d '{
    "kind": "warning",
    "content": "never call selfdestruct in a proxy implementation contract",
    "author": "agent:alice",
    "enabled_by": [],
    "stake_wei": 2000000000000000
  }' | jq

# Confirm an insight (replace ID with actual hex ID)
curl -X POST http://127.0.0.1:8545/api/knowledge/entries/a1b2c3d4.../confirm \
  -H 'Content-Type: application/json' \
  -d '{"confirmer": "agent:bob", "stake_wei": 1000000000000000}' | jq

# Challenge an insight
curl -X POST http://127.0.0.1:8545/api/knowledge/entries/a1b2c3d4.../challenge \
  -H 'Content-Type: application/json' \
  -d '{"challenger": "agent:carol", "stake_wei": 1000000000000000}' | jq

# Trigger decay sweep
curl -X POST http://127.0.0.1:8545/api/knowledge/decay \
  -H 'Content-Type: application/json' \
  -d '{}' | jq

# HDC similarity and dependency edges (for graph visualization)
curl 'http://127.0.0.1:8545/api/knowledge/edges?similarity_threshold=0.5&max_hdc_edges_per_node=5' | jq

# Semantic search
curl 'http://127.0.0.1:8545/api/knowledge/search?q=proxy+destruction+safety&k=5' | jq

# Kind metadata and counts
curl http://127.0.0.1:8545/api/knowledge/kinds | jq
```

### Agent registry

```bash
# Register an agent
curl -X POST http://127.0.0.1:8545/api/agents \
  -H 'Content-Type: application/json' \
  -d '{"id": "agent:alice", "pubkey": "0xabc123", "role": "researcher"}' | jq

# List all agents
curl http://127.0.0.1:8545/api/agents | jq

# Send a heartbeat with usage stats
curl -X POST http://127.0.0.1:8545/api/agents/agent:alice/heartbeat \
  -H 'Content-Type: application/json' \
  -d '{"tokens_used": 5000, "cost_usd": 0.15, "tasks_completed": 1}' | jq

# Get latest heartbeat
curl http://127.0.0.1:8545/api/agents/agent:alice/heartbeat | jq

# Get aggregated stats
curl http://127.0.0.1:8545/api/agents/agent:alice/stats | jq

# Get activity trace
curl 'http://127.0.0.1:8545/api/agents/agent:alice/trace?limit=20' | jq

# Agent interaction topology (for d3.js force-directed graph)
curl http://127.0.0.1:8545/api/agents/topology | jq
```

### WebSocket streaming (requires `roko` feature)

```bash
# Subscribe to all event channels
websocat ws://127.0.0.1:8545/api/ws

# Pheromones only
websocat 'ws://127.0.0.1:8545/api/ws?insights=false'

# Insights only
websocat 'ws://127.0.0.1:8545/api/ws?pheromones=false'
```

Each frame is a JSON object:

```json
{"channel": "pheromone", "data": {"id": 7, "kind": "threat", "intensity": 1.0, "depositedAt": 1712400000}}
{"channel": "insight", "data": {"type": "posted", "id": "abc123", "kind": "warning", "author": "agent:alice"}}
{"type": "connected", "pheromones": true, "insights": true}
{"type": "lagged", "channel": "pheromone", "missed": 12}
```
