# Section C: Architecture Migration (UX)

Source: `tmp/ux/` (files 00-06)
Goal: Extract application state from mirage-rs into per-agent servers + aggregator service

Three phases:
- **Phase 1** (now to demo): sdb-spec backend items (Section A) — additive, nothing breaks
- **Phase 2** (post-demo +4 weeks): Agent server crate + aggregator — Sam changes one env var
- **Phase 3** (+4-8 weeks): mirage-rs cleanup — delete extracted code, net -4,500 LOC

---

## C.01 — roko-agent-server Crate (Builder + Core)

**Status**: NOT DONE
**Priority**: P0 (Phase 2)
**Estimated LOC**: ~480
**Dependencies**: None (can start in parallel with Section A)

### Files to modify

- `crates/roko-agent-server/` — **NEW CRATE**
  - `Cargo.toml`
  - `src/lib.rs` — `AgentServer`, `AgentServerBuilder`
  - `src/state.rs` — `AgentState` shared state
  - `src/features/health.rs` — `/health`, `/capabilities` (always on)
  - `src/features/messaging.rs` — `/message`, `/stream`
  - `src/features/predictions.rs` — `/predictions/*`
  - `src/features/research.rs` — `/research`
  - `src/features/tasks.rs` — `/tasks/*`
  - `src/auth/bearer.rs` — `BearerAuth` middleware
  - `src/registration.rs` — On-chain registration logic
- `Cargo.toml` (workspace) — Add member

### Context

Each agent runs its own HTTP server. Builder API enables feature-by-feature opt-in. This replaces the monolithic mirage-rs ChainContext for agent-specific state.

Source design: `tmp/ux/01-agent-server-design.md`

### Implementation details

1. Create crate with deps: `roko-core`, `roko-agent`, `roko-learn`, `roko-neuro`, `roko-chain`, `axum`, `tokio`, `tower`
2. `AgentServerBuilder` with fluent API:
   ```rust
   let server = AgentServer::builder()
       .agent_id("agent-001")
       .bind("0.0.0.0:0")           // OS-assigned port
       .health()                     // Always on: /health, /capabilities
       .messaging()                  // POST /message, WS /stream
       .predictions()                // GET/POST /predictions/*
       .research()                   // POST /research
       .tasks()                      // GET/POST /tasks/*
       .auth(BearerAuth::new(secret))
       .on_start(|addr| { /* Update Agent Card */ })
       .build()
       .serve()
       .await;
   ```
3. `AgentState` struct (shared via `Arc`):
   - `agent_id: String`
   - `capabilities: Vec<String>`
   - `chain_client: Option<ChainClient>`
   - `llm_backend: Box<dyn LlmBackend>`
   - `knowledge_store: Option<KnowledgeStore>`
   - `prediction_store: Option<PredictionStore>`
   - `task_queue: VecDeque<TaskEntry>`
   - `metrics: AgentMetrics`
4. Feature modules (~100 LOC each):
   - `health.rs`: `GET /health` (200 OK + uptime), `GET /capabilities` (agent card JSON)
   - `messaging.rs`: `POST /message` (enqueue + process), `WS /stream` (real-time output)
   - `predictions.rs`: `GET/POST /predictions`, `GET /predictions/{id}`, `GET /predictions/residuals`
   - `research.rs`: `POST /research` (delegate to roko-agent research handler)
   - `tasks.rs`: `GET /tasks`, `POST /tasks/{id}/accept`, `POST /tasks/{id}/complete`
5. `BearerAuth` middleware: extract `Authorization: Bearer <token>` header, validate against stored secret
6. `/health` and `/capabilities` are public (no auth required)

### Verify command

```bash
cargo build -p roko-agent-server 2>&1 | tail -5
cargo test -p roko-agent-server 2>&1 | tail -10
```

---

## C.02 — mirage-rs Feature-Gated Extraction

**Status**: NOT DONE
**Priority**: P1 (Phase 2)
**Estimated LOC**: ~100 (feature gates, no deletion yet)
**Dependencies**: C.01

### Files to modify

- `apps/mirage-rs/Cargo.toml` — Add `legacy-api` feature flag
- `apps/mirage-rs/src/http_api/*.rs` — Gate all route modules behind `#[cfg(feature = "legacy-api")]`
- `apps/mirage-rs/src/chain/*.rs` — Gate behind `#[cfg(feature = "chain")]`

### Context

mirage-rs accumulated ~30 REST endpoints and application state that don't belong in an EVM simulator. Phase 2 gates all extracted code behind feature flags so it can be removed in Phase 3.

Source design: `tmp/ux/02-mirage-extraction.md`

**What stays in mirage-rs permanently**: `fork.rs`, `rpc.rs`, `replay.rs`, `scenario.rs`, `/health`, `/stats` (EVM-only metrics), JSON-RPC endpoint.

**What gets gated (eventually removed)**:
- `chain/` module (HDC index, InsightEntry, pheromone field, agent topology, ChainContext)
- `http_api/` module (9 files, ~30 REST endpoints across 6 concerns)

### Implementation details

1. Add feature flags to `Cargo.toml`:
   ```toml
   [features]
   default = ["binary", "chain", "legacy-api"]
   binary = []
   library = []
   sim-gas = []
   chain = []
   legacy-api = ["chain"]
   ```
2. Gate all `http_api/` routes behind `#[cfg(feature = "legacy-api")]`
3. Gate all `chain/` types behind `#[cfg(feature = "chain")]`
4. Transform `ApiState`:
   ```rust
   pub struct ApiState {
       #[cfg(feature = "chain")]
       pub chain: Arc<RwLock<ChainContext>>,
       pub current_block: BlockNumberFn,
       #[cfg(feature = "legacy-api")]
       pub projection_cache: ProjectionCache,
       pub started_at: Instant,
       #[cfg(feature = "legacy-api")]
       pub subs: Option<SubscriptionManager>,
   }
   ```
5. Ensure `cargo build -p mirage-rs --no-default-features --features binary` compiles (pure EVM mode)

### Verify command

```bash
# Full build (backward compatible)
cargo build -p mirage-rs 2>&1 | tail -5
# Minimal build (pure EVM)
cargo build -p mirage-rs --no-default-features --features binary 2>&1 | tail -5
```

---

## C.03 — Aggregator Service on roko-serve

**Status**: NOT DONE
**Priority**: P0 (Phase 2)
**Estimated LOC**: ~400
**Dependencies**: C.01

### Files to modify

- `crates/roko-serve/src/routes/aggregator.rs` — **NEW FILE** — Aggregator handlers
- `crates/roko-serve/src/routes/mod.rs` — Wire aggregator routes
- `crates/roko-serve/src/state.rs` — Add agent discovery state

### Context

Thin backend on roko-serve that presents the **identical `/api/*` shape** as mirage-rs but sources data from agent servers + chain. Sam changes one env var (`NEXT_PUBLIC_API_URL`) and nothing else changes.

Source design: `tmp/ux/04-dashboard-migration.md`

### Implementation details

1. Agent discovery (2 sources):
   - `ProcessSupervisor` for locally-managed agents (get agent URLs from process registry)
   - ERC-8004 Identity Registry for chain-discovered agents (fetch Agent Card URIs, parse `endpoints.rest`)
2. Create `aggregator.rs` with fan-out handlers:
   - `GET /api/agents` — query all known agent servers' `/health` + `/capabilities`, merge into list
   - `GET /api/agents/{id}/stats` — proxy to specific agent server
   - `GET /api/agents/{id}/message` — proxy to agent's `/message` endpoint
   - `GET /api/predictions/*` — fan-out to all agents' `/predictions`, merge + paginate
   - `GET /api/knowledge/*` — read from InsightBoard contract (or chain indexer)
   - `GET /api/tasks/*` — merge BountyMarket contract data + roko-serve task state
3. Caching layer with per-route TTLs:
   - Agent list: 30s
   - Agent health: 5s
   - Capabilities: 60s
   - Predictions: 10s
   - Knowledge board: 30s
4. Parallel requests via `tokio::join!` or `futures::join_all` for fan-out
5. WS multiplexer: `GET /api/ws` — subscribe to N agent streams, forward all events through one connection
6. Response shapes must match existing mirage-rs responses exactly (same JSON field names)

### Verify command

```bash
cargo build -p roko-serve 2>&1 | tail -5
cargo test -p roko-serve --lib -- aggregator 2>&1 | tail -10
```

---

## C.04 — Auth Model (Bearer Tokens + Discovery)

**Status**: NOT DONE
**Priority**: P1 (Phase 2)
**Estimated LOC**: ~120
**Dependencies**: C.01, C.03

### Files to modify

- `crates/roko-serve/src/routes/agents.rs` — Token generation + distribution
- `crates/roko-agent-server/src/auth/bearer.rs` — Token validation
- `crates/roko-agent-server/src/registration.rs` — Agent Card updates

### Context

Two separate auth systems: HTTP auth (bearer tokens for API access) and chain auth (wallet keypair for contract writes). These are independent. Not every agent has a wallet.

Source design: `tmp/ux/03-auth-and-discovery.md`

### Implementation details

1. Token flow:
   - Agent registers with roko-serve → roko-serve generates bearer token (random 256-bit)
   - roko-serve stores token → agent_id mapping
   - roko-serve optionally updates Agent Card on chain with endpoint URLs
   - Dashboard queries roko-serve for agent list + tokens
   - Dashboard uses tokens to query per-agent servers directly
2. Token endpoints on roko-serve:
   - `POST /api/agents/{id}/token` — generate new token (returns token, never stored in plaintext — hash only)
   - `GET /api/agents/{id}/token` — check if token exists (returns boolean, not the token)
3. Token validation on agent-server:
   - `BearerAuth` middleware extracts `Authorization: Bearer <token>` header
   - Compares against stored secret (set at agent startup)
   - `/health` and `/capabilities` bypass auth (public)
4. Token rotation: on agent restart + periodic (24h)
5. Rate limiting: fixed per-token limits (configurable via `AgentServerBuilder`)

### Verify command

```bash
cargo build -p roko-serve -p roko-agent-server 2>&1 | tail -5
cargo test -p roko-agent-server --lib -- auth 2>&1 | tail -10
```

---

## C.05 — ERC-8004 Agent Card Integration

**Status**: NOT DONE
**Priority**: P1 (Phase 2)
**Estimated LOC**: ~100
**Dependencies**: C.01, C.04

### Files to modify

- `crates/roko-agent-server/src/registration.rs` — Agent Card JSON construction + on-chain update
- `crates/roko-chain/src/` — Add Identity Registry bindings if missing

### Context

Agent Card JSON has `endpoints` field with `mcp`, `a2a`, `websocket`, `rest` URLs. The `roko-agent-server` adds a `rest` endpoint on startup. Discovery = read Identity Registry → fetch Agent Card URI → parse endpoints → query agent directly.

### Implementation details

1. On agent server startup (`on_start` callback):
   - Construct Agent Card JSON with `endpoints.rest` = actual bound address
   - If wallet available: call `updateAgentCardUri(passportId, cardUri)` on Identity Registry
   - If no wallet: register card URL with roko-serve for proxy discovery
2. Agent Card JSON structure:
   ```json
   {
     "name": "agent-001",
     "capabilities": ["research", "trading"],
     "endpoints": {
       "rest": "http://host:port",
       "websocket": "ws://host:port/stream",
       "a2a": null,
       "mcp": null
     },
     "domain_tags": ["roko"],
     "version": "0.1.0"
   }
   ```
3. Filtering Roko agents from global registry: capability bitmask (bit 15) for on-chain fast-filtering + `"roko"` domain tag in Agent Card for off-chain confirmation
4. Add Identity Registry contract bindings to `roko-chain` if not already present

### Verify command

```bash
cargo build -p roko-agent-server -p roko-chain 2>&1 | tail -5
```

---

## C.06 — mirage-rs REST Deletion (Phase 3)

**Status**: NOT DONE
**Priority**: P2 (Phase 3, +4-8 weeks post-demo)
**Estimated LOC**: -4,500 (deletion)
**Dependencies**: C.02, C.03

### Files to modify

- `apps/mirage-rs/src/chain/` — Delete entire module
- `apps/mirage-rs/src/http_api/` — Delete entire module (except health/stats)
- `apps/mirage-rs/Cargo.toml` — Remove `chain`, `legacy-api`, `roko` features

### Context

After aggregator is stable and dashboard is switched, remove all extracted code from mirage-rs. Net result: mirage-rs drops from ~30 REST routes to 3 (health, EVM stats, JSON-RPC).

### Implementation details

1. Remove `chain` feature and all `#[cfg(feature = "chain")]` code
2. Remove `legacy-api` feature and all `#[cfg(feature = "legacy-api")]` code
3. Remove `roko` feature
4. Delete `chain/` module entirely (ChainContext, agent topology, HDC index, InsightEntry, pheromone field)
5. Delete `http_api/` module except health and stats endpoints
6. Simplify `ApiState` to only: `current_block`, `started_at`
7. Remove unused dependencies from `Cargo.toml`

### Verify command

```bash
cargo build -p mirage-rs --no-default-features --features binary 2>&1 | tail -5
cargo test -p mirage-rs 2>&1 | tail -10
# Verify no /api/agents, /api/tasks, /api/predictions routes respond
```

---

## C.07 — Dashboard URL Migration

**Status**: NOT DONE
**Priority**: P1 (Phase 2)
**Estimated LOC**: ~10 (config change)
**Dependencies**: C.03

### Files to modify

- Dashboard (Kauri/Next.js) — Change `NEXT_PUBLIC_API_URL` from mirage-rs to roko-serve aggregator

### Context

Sam changes one env var. All API shapes are identical. No code changes needed in dashboard — the aggregator returns the same JSON shapes.

### Implementation details

1. Change `NEXT_PUBLIC_API_URL` from `http://mirage-rs:8545/api` to `http://roko-serve:6677/api`
2. Verify all dashboard pages load correctly
3. Verify WebSocket connection works through aggregator multiplexer
4. Keep mirage-rs URL as fallback env var (`NEXT_PUBLIC_MIRAGE_URL`) during transition

### Verify command

```bash
# Dashboard smoke test
curl http://roko-serve:6677/api/agents | jq length
curl http://roko-serve:6677/api/predictions/sessions | jq length
# Compare with mirage-rs output shape
diff <(curl -s http://mirage-rs:8545/api/agents | jq 'keys') \
     <(curl -s http://roko-serve:6677/api/agents | jq 'keys')
```

---

## C.08 — WS Multiplexer

**Status**: NOT DONE
**Priority**: P2 (Phase 2)
**Estimated LOC**: ~100
**Dependencies**: C.01, C.03

### Files to modify

- `crates/roko-serve/src/routes/aggregator.rs` — Add WS multiplexer handler

### Context

Dashboard currently gets one WS connection from mirage-rs. With N agent servers, the aggregator must multiplex N agent WS streams + M roko-serve events into one connection. This prevents N*M connections from dashboard.

### Implementation details

1. `GET /api/ws` handler on aggregator:
   - On connection: discover all agent servers
   - Connect to each agent's `/stream` WebSocket
   - Also subscribe to roko-serve internal event bus
   - Forward all events through single client connection, tagged with source
2. Event wrapping:
   ```json
   {
     "source": "agent-001",
     "event": { /* original event */ }
   }
   ```
3. Handle agent disconnection gracefully (log warning, continue forwarding from remaining agents)
4. Reconnect to agents on discovery refresh (new agents appear, old ones removed)

### Verify command

```bash
cargo build -p roko-serve 2>&1 | tail -5
# Then: websocat ws://localhost:6677/api/ws | head -5
```
