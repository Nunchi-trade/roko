# Section A: Dashboard Backend (sdb-spec)

Source: `tmp/sdb-spec/` (items 01-10)
Target crates: `apps/mirage-rs/`, `crates/roko-serve/`, `crates/roko-cli/`
Total estimated LOC: ~1,110

---

## A.01 — Agent Owner Field

**Status**: NOT DONE
**Priority**: P0
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `apps/mirage-rs/src/chain/agent.rs` — Add `owner` field to `AgentEntry`
- `apps/mirage-rs/src/http_api/agent.rs` — Add `owner` to `RegisterAgentRequest`, add query param

### Context

Dashboard Ask panel has a hardcoded `MY_AGENTS` array. No way to query "which agents belong to this user." `POST /api/agents` has no `owner` field. Sam needs `fetchAgents({ owner: wallet.address })`.

### Implementation details

1. Add `pub owner: String` to `AgentEntry` struct in `chain/agent.rs`
2. Update `AgentRegistry::register()` to accept `owner: String` parameter
3. Add `list_agents_by_owner(&self, owner: &str) -> Vec<&AgentEntry>` method to `AgentRegistry`
4. Add `#[serde(default)] pub owner: String` to `RegisterAgentRequest` in `http_api/agent.rs`
5. Create `AgentListQuery` struct with `owner: Option<String>` — use as `axum::extract::Query` param on `list_agents()`
6. Filter by owner when query param present, return all when absent
7. Include `owner` in all JSON responses

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
cargo test -p mirage-rs --lib -- agent 2>&1 | tail -10
# Then: curl http://localhost:8545/api/agents?owner=0xABC | jq '.[] | .owner'
```

---

## A.02 — Agent Skills Endpoints

**Status**: NOT DONE
**Priority**: P0
**Estimated LOC**: ~150
**Dependencies**: None

### Files to modify

- `apps/mirage-rs/src/chain/agent.rs` — Add `SkillConfig` struct, `skills` field to `AgentEntry`
- `apps/mirage-rs/src/http_api/skills.rs` — **NEW FILE** — 3 endpoint handlers
- `apps/mirage-rs/src/http_api/mod.rs` — Wire skill routes

### Context

Dashboard Strategy tab has per-agent skill toggles (8 skills: ISFR Observer, DeFi Router, Risk Sentinel, Knowledge Curator, Prediction Agent, Market Maker, Hedge Agent, Self-Tuner) with config sliders, all in local React state. Needs backend persistence. `PUT /api/config` on roko-serve is global — per-agent config must live on mirage-rs.

### Implementation details

1. Define `SkillConfig` struct:
   ```rust
   pub struct SkillConfig {
       pub enabled: bool,
       pub gamma_interval_s: u64,    // >= 1
       pub confidence_threshold: u8, // 0-100
       pub parameters: serde_json::Value,
   }
   ```
2. Add `skills: HashMap<String, SkillConfig>` field to `AgentEntry` (with `#[serde(default)]`)
3. Add `get_skills()`, `set_skills()`, `set_skill()` methods to `AgentRegistry`
4. Create `http_api/skills.rs` with 3 handlers:
   - `GET /api/agents/{id}/skills` — return all skills for agent
   - `PUT /api/agents/{id}/skills` — replace all skills
   - `PUT /api/agents/{id}/skills/{skill}` — update single skill
5. Validation: `gamma_interval_s >= 1`, `confidence_threshold` in 0..=100, no duplicate skill names
6. Broadcast via `agent_bus` on successful update
7. Wire routes in `http_api/mod.rs`

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
cargo test -p mirage-rs --lib -- skill 2>&1 | tail -10
```

---

## A.03 — C-Factor + Frequency + Cost Tier Endpoints

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~50 remaining
**Dependencies**: None

### Files to modify

- `crates/roko-serve/src/routes/status.rs` — C-Factor endpoint already exists at `/api/metrics/c_factor` (line 32, handler at line 114)
- `crates/roko-serve/src/routes/learning.rs` — Add `GET /api/learning/cost-tiers`
- `apps/mirage-rs/src/http_api/agent.rs` — Extend `GET /api/agents/{id}/stats` with `operating_frequency`

### Context

C-Factor HTTP endpoint is DONE at `/api/metrics/c_factor` in `roko-serve/src/routes/status.rs`. It reads from `composite_path` + `compute_fleet_cfactor()`. What remains: cost-tier distribution endpoint and operating_frequency extension to mirage-rs agent stats.

`CFactor` struct with all 10 sub-metrics exists in `crates/roko-learn/src/cfactor.rs`.
`CascadeRouter` in `crates/roko-learn/src/cascade_router.rs` tracks model tier usage.

### Implementation details

1. Add `GET /api/learning/cost-tiers` in `routes/learning.rs`:
   - Read `CascadeRouter` state from disk (`.roko/learn/cascade-router.json`)
   - Return T0/T1/T2 distribution as `{ "T0": count, "T1": count, "T2": count, "total": count }`
2. Extend agent stats response in mirage-rs `http_api/agent.rs`:
   - Add `operating_frequency: String` field (one of: `"idle"`, `"reactive"`, `"active"`, `"intensive"`)
   - Derive from recent task count in the agent's history

### Verify command

```bash
cargo build -p roko-serve -p mirage-rs 2>&1 | tail -5
# Then: curl http://localhost:6677/api/learning/cost-tiers | jq
# Then: curl http://localhost:8545/api/agents/agent-001/stats | jq '.operating_frequency'
```

---

## A.04 — Task Artifacts

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~100
**Dependencies**: None

### Files to modify

- `apps/mirage-rs/src/chain/task.rs` — Add `TaskArtifact`, `CompletionMetadata` structs, extend `TaskEntry`
- `apps/mirage-rs/src/http_api/task.rs` — Extend `CompleteTaskRequest`, add `get_task_artifacts` handler
- `apps/mirage-rs/src/http_api/mod.rs` — Wire new route

### Context

`POST /api/tasks/{id}/complete` currently accepts only `{ result_insight_id }`. Dashboard My Jobs panel had rich mock deliverables that were stripped when going live. Needs `artifacts`, `summary`, `completion_metadata`.

### Implementation details

1. Define structs in `chain/task.rs`:
   ```rust
   pub struct TaskArtifact {
       pub kind: String,        // "code", "report", "data", "config"
       pub label: String,
       pub content_hash: String,
       pub uri: Option<String>,
       pub size_bytes: Option<u64>,
   }

   pub struct CompletionMetadata {
       pub duration_ms: u64,
       pub model_used: Option<String>,
       pub tokens_in: Option<u64>,
       pub tokens_out: Option<u64>,
       pub cost_usd: Option<f64>,
   }
   ```
2. Add to `TaskEntry` (all `#[serde(default)]`):
   - `artifacts: Vec<TaskArtifact>`
   - `summary: Option<String>`
   - `completion_metadata: Option<CompletionMetadata>`
3. Extend `TaskStore::complete()` to accept and store these fields
4. Extend `CompleteTaskRequest` in `http_api/task.rs` with matching optional fields
5. Add `get_task_artifacts` handler: `GET /api/tasks/{id}/artifacts` — returns `Vec<TaskArtifact>`
6. Wire route in `mod.rs`

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
cargo test -p mirage-rs --lib -- task 2>&1 | tail -10
```

---

## A.05 — Agent Messaging

**Status**: NOT DONE
**Priority**: P0
**Estimated LOC**: ~150
**Dependencies**: None

### Files to modify

- `crates/roko-serve/src/routes/agents.rs` — Add `POST /api/agents/{id}/message` handler
- `crates/roko-serve/src/events.rs` — Add `AgentOutput` variant to `ServerEvent`
- `apps/mirage-rs/src/http_api/agent.rs` — Extend `get_agent_heartbeat` with `busy` field

### Context

Dashboard Ask panel currently calls OpenRouter directly from the browser. This is architecturally wrong — the LLM call should happen inside the agent via roko-serve.

What already exists:
- `crates/roko-serve/src/routes/run.rs`: `POST /api/run` + `GET /api/run/{id}/status`
- `crates/roko-serve/src/routes/ws.rs`: WebSocket at `/ws` streaming `ServerEvent`
- `crates/roko-serve/src/routes/sse.rs`: SSE endpoint
- `crates/roko-serve/src/events.rs`: `ServerEvent` enum

### Implementation details

1. Add `POST /api/agents/{id}/message` route in `routes/agents.rs`:
   - Accept `SendMessageRequest { message: String, context: Option<Value> }`
   - Create a targeted run: prefix prompt with `[agent:{id}]`
   - Call existing run infrastructure from `routes/run.rs`
   - Return `{ run_id: String }` immediately (async execution)
2. Add `AgentOutput` variant to `ServerEvent` in `events.rs`:
   ```rust
   AgentOutput {
       agent_id: String,
       run_id: String,
       content: String,
       done: bool,
   }
   ```
3. Emit `AgentOutput` events through existing WS/SSE infrastructure as agent produces output
4. Extend mirage-rs `get_agent_heartbeat` response with `busy: bool` field (true when agent has active run)
5. Wire route in `routes/mod.rs`

### Verify command

```bash
cargo build -p roko-serve -p mirage-rs 2>&1 | tail -5
# Then: curl -X POST http://localhost:6677/api/agents/agent-001/message \
#   -H 'Content-Type: application/json' -d '{"message":"hello"}' | jq '.run_id'
```

---

## A.06 — ISFR Proxy

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `apps/mirage-rs/src/http_api/isfr.rs` — **NEW FILE** — 2 proxy handlers
- `apps/mirage-rs/src/http_api/mod.rs` — Wire routes

### Context

Dashboard ISFR card, sidebar sparkline, and network stats show hardcoded "6.40%". The `isfr-service` provides real data but the dashboard talks only to mirage-rs. Need proxy endpoints.

### Implementation details

1. Create `http_api/isfr.rs` with 2 proxy handlers:
   - `GET /api/isfr/current` — proxies to `isfr-service` at `GET /v1/isfr/current`
   - `GET /api/isfr/history` — proxies to `isfr-service` at `GET /v1/isfr/history`
2. `ISFR_SERVICE_URL` defaults to `http://localhost:8546`, configurable via env var `ISFR_SERVICE_URL`
3. Use `reqwest::Client` (or whatever HTTP client mirage-rs already uses) for proxying
4. Pass through query params (e.g., `?period=24h`)
5. Return 502 with message if isfr-service is unreachable
6. Wire routes in `mod.rs`

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
# Then: curl http://localhost:8545/api/isfr/current | jq
```

---

## A.07 — Prediction Endpoints (Mirofish)

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~400
**Dependencies**: None

### Files to modify

- `apps/mirage-rs/src/chain/prediction.rs` — **NEW FILE** — `PredictionSession`, `PredictionClaim`, `PredictionStore`
- `apps/mirage-rs/src/chain/mod.rs` — Add `prediction_store` and `prediction_bus` to `ChainContext`
- `apps/mirage-rs/src/http_api/prediction.rs` — **NEW FILE** — 7 endpoint handlers
- `apps/mirage-rs/src/http_api/mod.rs` — Wire routes

### Context

Mirofish (prediction engine) has a full dashboard UI with simulated lifecycle but no backend. This is the first deflationary mechanism in the points economy.

Existing code that helps:
- `crates/roko-learn/src/prediction.rs`: `PredictionRecord` + `CalibrationTracker`
- `crates/roko-neuro/src/knowledge_store.rs`: HDC search, temporal decay
- `apps/mirage-rs/src/chain/task.rs`: `TaskStore` with `stake_wei`

### Implementation details

1. Create `chain/prediction.rs` with:
   - `SessionState` enum: `Open`, `Active`, `Resolving`, `Resolved`, `Expired`
   - `ClaimState` enum: `Pending`, `Accepted`, `Rejected`, `Expired`
   - `PredictionSession` struct: `id`, `creator`, `topic`, `deadline`, `resolution_criteria`, `state`, `claims`, `created_at`
   - `PredictionClaim` struct: `id`, `session_id`, `agent_id`, `prediction`, `confidence`, `interval_width`, `state`, `score`, `created_at`
   - `PredictionStore` struct with in-memory storage (`Vec<PredictionSession>`)
   - Methods: `create_session()`, `submit_claim()`, `resolve_session()`, `get_calibration(agent_id)`
   - Difficulty weight formula: `weight = novelty * (1/interval_width) * tightness`
   - `PredictionEvent` enum for WS broadcast
2. Add `prediction_store: Arc<RwLock<PredictionStore>>` and `prediction_bus: broadcast::Sender<PredictionEvent>` to `ChainContext` in `chain/mod.rs`
3. Create `http_api/prediction.rs` with 7 handlers:
   - `POST /api/predictions/sessions` — create session
   - `GET /api/predictions/sessions` — list sessions (with `?state=` filter)
   - `GET /api/predictions/sessions/{id}` — get session with claims
   - `POST /api/predictions/sessions/{id}/resolve` — resolve with outcome
   - `POST /api/predictions/claims` — submit claim to session
   - `GET /api/predictions/claims` — list claims (with `?agent_id=` filter)
   - `GET /api/predictions/calibration/{agent_id}` — calibration stats
4. Wire `prediction_bus` into WS handler (follow `pheromone_bus` pattern in existing code)
5. Wire routes in `mod.rs`

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
cargo test -p mirage-rs --lib -- prediction 2>&1 | tail -10
```

---

## A.08 — roko chat CLI

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~80
**Dependencies**: A.05 (agent messaging must exist first)

### Files to modify

- `crates/roko-cli/src/main.rs` — Add `Chat` variant to CLI enum
- `crates/roko-cli/src/chat.rs` — **NEW FILE** — `run_chat_repl()` function

### Context

`roko run` stores output in binary signals, not stdout. No interactive REPL for developers. Also serves as testing tool for the agent messaging pipeline (A.05).

### Implementation details

1. Add `Chat` variant to CLI enum in `main.rs`:
   ```rust
   Chat {
       #[clap(long, default_value = "default")]
       agent: String,
       #[clap(long, default_value = "http://localhost:6677")]
       serve_url: String,
   }
   ```
2. Create `chat.rs` with `pub async fn run_chat_repl(agent: &str, serve_url: &str) -> anyhow::Result<()>`:
   - Print banner: `"roko chat (agent: {agent}, server: {serve_url})"`
   - stdin read loop with `"you> "` prompt
   - `POST {serve_url}/api/agents/{agent}/message` with user input
   - Poll `GET {serve_url}/api/run/{run_id}/status` every 500ms until complete
   - Print agent response with `"agent> "` prefix
   - EOF (Ctrl-D) exits cleanly with `"\nbye."`
3. Add `mod chat;` to `main.rs`
4. Handle connection errors gracefully (print error, continue loop)

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo run -p roko-cli -- chat --help 2>&1 | head -5
```

---

## A.09 — Research Intent

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~20
**Dependencies**: None

### Files to modify

- `crates/roko-serve/src/routes/research.rs` — Extend `ResearchTopicRequest` with `intent` field

### Context

Research reports are structured like internal docs, not trader/risk-manager actionable outputs. Need intent selector with 5 output formats: `position`, `evaluate`, `monitor`, `explore`, `audit`.

Existing code: `POST /api/research/topic`, `GET /api/research`, `POST /api/research/analyze` in `crates/roko-serve/src/routes/research.rs`.

### Implementation details

1. Add `#[serde(default = "default_intent")] pub intent: String` to `ResearchTopicRequest`
2. Add `fn default_intent() -> String { "explore".to_string() }`
3. Validate intent is one of: `"position"`, `"evaluate"`, `"monitor"`, `"explore"`, `"audit"` — return 400 Bad Request if not
4. Pass `intent` through to the runtime's research handler so the synthesizer adjusts report structure:
   - `position` → ends with directional recommendation + confidence + key risk
   - `evaluate` → ends with risk scores + red flags + comparison to alternatives
   - `monitor` → ends with timeline of changes + impact assessment + alerts to set
   - `explore` → ends with landscape map + key players + knowledge gaps (default)
   - `audit` → ends with checklist of verified claims + unverified gaps + severity

### Verify command

```bash
cargo build -p roko-serve 2>&1 | tail -5
# Then: curl -X POST http://localhost:6677/api/research/topic \
#   -H 'Content-Type: application/json' \
#   -d '{"topic":"ETH staking yields","intent":"position"}' | jq '.intent'
# Bad intent should 400:
# curl -X POST http://localhost:6677/api/research/topic \
#   -d '{"topic":"test","intent":"invalid"}' -w '%{http_code}'
```

---

## A.10 — Task Improve / Feedback

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~50
**Dependencies**: A.04 (task artifacts must exist first)

### Files to modify

- `apps/mirage-rs/src/chain/task.rs` — Add `parent_task_id`, `create_improvement()` method
- `apps/mirage-rs/src/http_api/task.rs` — Add `ImproveTaskRequest`, `improve_task` handler
- `apps/mirage-rs/src/http_api/mod.rs` — Wire route

### Context

After a job is delivered, there's no way to say "improve this." No iteration loop. This creates the feedback mechanism for iterative task refinement.

### Implementation details

1. Add `parent_task_id: Option<u64>` to `TaskEntry` in `chain/task.rs` (with `#[serde(default)]`)
2. Add `TaskStore::create_improvement(parent_id: u64, feedback: &str, creator: &str, now: u64) -> Result<TaskEntry>`:
   - Verify parent task exists and is `Completed` — error if not
   - Create child task with:
     - `kind: "improvement"`
     - Same `assignee` and `tags` as parent
     - `parent_task_id: Some(parent_id)`
     - `spec: feedback` (the improvement instructions)
     - Auto-assigned to same agent
3. Add `ImproveTaskRequest { feedback: String }` struct in `http_api/task.rs`
4. Add `improve_task` handler:
   - Validate `feedback` is non-empty — 400 if empty
   - Call `create_improvement()`
   - Fire `TaskEvent::Created`
   - Return new task entry as JSON
5. Add `POST /api/tasks/{id}/improve` route in `mod.rs`

### Verify command

```bash
cargo build -p mirage-rs 2>&1 | tail -5
cargo test -p mirage-rs --lib -- improve 2>&1 | tail -10
```
