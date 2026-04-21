# 13-coordination -- Gap Checklist

Spec: `docs/13-coordination/` (13 files). Code: `crates/roko-orchestrator/src/coordination.rs`, `crates/roko-learn/src/cfactor.rs`.

Overall: ~15-20% implemented. Pheromone types, decay math, C-Factor measurement, and promotion cascade work. Agent Mesh transport, morphogenetic dynamics, and pheromone enrichment are the major gaps.

## Compliant (no action needed)
- Pheromone struct with kind, intensity, half_life, scope, confirmations (doc 03)
- PheromoneKind enum -- all 8 variants with correct default half-lives (doc 04)
- PheromoneScope enum -- Local, Subnet, Mesh, Global with rank ordering (doc 05)
- Pheromone decay formula -- exponential with confirmation extension (doc 03)
- Promotion cascade -- Pattern->Wisdom->Consensus with threshold checks (doc 04)
- Response thresholds -- Hill function with reinforce/habituate (doc 04)
- C-Factor measurement -- 11 components, leave-one-out, pathology detection (doc 11)
- SubnetId struct with validation (doc 08)
- CohortMetrics + WisdomGate (doc 11)

## Checklist

### COORD-01: Agent Mesh transport [CRITICAL BLOCKER]
- [x] Implement WebSocket relay for Mesh-scope pheromone sync

**Spec** (doc 06 `06-agent-mesh-sync.md`): Dual-transport architecture: (1) WebSocket relay for local/LAN mesh (priority implementation), (2) Iroh P2P for decentralized mesh (Phase 2+). The MeshBus carries ephemeral pheromone publications; the MeshSubstrate provides durable replication for persistent pheromone state. Key requirements: version-vector deduplication (each pheromone carries `agent_id + sequence_number` to prevent duplicate delivery), store-and-forward queue (offline agents receive missed pheromones on reconnect), partition tolerance (AP design per Brewer's CAP theorem — partition-aware morphogenetics with post-partition reconciliation). The doc also specifies a connection registry tracking mesh peer state, failure modes (network partitions, Byzantine agents), and security model.

**Current code**: No mesh transport at all. `PheromoneScope::Mesh(CollectiveId)` variant exists at `crates/roko-orchestrator/src/coordination.rs:262` with `rank()` returning `2` and `is_broader_than()` logic. `Pheromone` struct at line 286 has `scope: PheromoneScope` field. No runtime code publishes, relays, or subscribes to Mesh-scope pheromones. `roko-serve` has WebSocket infra at `crates/roko-serve/src/routes/ws.rs:21` with `handle_ws()` at line 38 and `EventBus` at `crates/roko-serve/src/event_bus.rs:1` with `replay_from(seq)` — this could serve as the relay backbone. `PheromoneAlert` event variant at `crates/roko-runtime/src/heartbeat.rs:572` exists but is not emitted.

**What to change**:
- Add `crates/roko-orchestrator/src/mesh_relay.rs` (or a module in `roko-serve`) with `MeshRelay` struct:
  ```
  pub struct MeshRelay {
      peers: HashMap<AgentId, PeerState>,
      version_vectors: HashMap<AgentId, u64>,
      store_forward_queue: HashMap<AgentId, Vec<Pheromone>>,
  }
  ```
- Implement `pub async fn publish(&self, pheromone: Pheromone)` — checks version vector, fans out to connected peers via WebSocket
- Implement `pub async fn subscribe(&self, agent_id: AgentId, kinds: Vec<PheromoneKind>) -> mpsc::Receiver<Pheromone>`
- Wire into `roko-serve` WebSocket handler at `crates/roko-serve/src/routes/ws.rs` — add `"pheromone_publish"` and `"pheromone_subscribe"` message types
- Add store-and-forward: when peer reconnects, replay queued pheromones from `store_forward_queue`

**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:256-284` — `PheromoneScope` enum with `Mesh(CollectiveId)`, `rank()`, `is_broader_than()`
- `crates/roko-orchestrator/src/coordination.rs:286-351` — `Pheromone` struct with `scope`, `kind`, `intensity`, `half_life`, `confirmations`
- `crates/roko-serve/src/routes/ws.rs:21-139` — existing WebSocket handler (`ClientMsg`:32, `handle_ws()`:38)
- `crates/roko-serve/src/event_bus.rs:1-50` — `EventBus` with `replay_from(seq)` (cursor-based replay infrastructure)
- `crates/roko-runtime/src/heartbeat.rs:572` — `PheromoneAlert` event variant (unused, ready to emit)
- `docs/13-coordination/06-agent-mesh-sync.md` — full mesh spec with partition tolerance, Byzantine detection
**Accept when**:
- [x] WebSocket relay server accepts pheromone publications -- MeshRelay::publish() at crates/roko-orchestrator/src/mesh_relay.rs:87
- [x] Agents can subscribe to Mesh-scope pheromone topics -- MeshRelay::subscribe() at mesh_relay.rs:138
- [x] Pheromones propagate to all mesh members -- fan-out to connected peers at mesh_relay.rs:104-130
- [x] Version-vector deduplication prevents duplicates -- version_vectors check at mesh_relay.rs:92-95
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'PheromoneScope::Mesh' crates/ --include='*.rs' | grep -v target/
grep -rn 'MeshRelay\|mesh_relay' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```

**Priority**: P1 (blocks all multi-agent coordination)

### COORD-02: Morphogenetic update function [CRITICAL]
- [x] Implement Gierer-Meinhardt reaction-diffusion dynamics

**Spec** (doc 07 `07-morphogenetic-specialization.md`): Gierer-Meinhardt reaction-diffusion dynamics for emergent role differentiation. The update rule applies per dimension `i` of the 8-dimensional strategy vector: `ds_i = alpha * returns_i * resource_pressure_scalar - beta * collective_i / collective_size - mu * (s_i - baseline) + N(0, sigma_noise)`. After computing deltas for all 8 dimensions, the strategy vector is re-normalized to sum to 1.0. The 8 strategy dimensions are: depth, breadth, execution, verification, time_horizon, exploration, exploitation, coordination. Turing's instability condition is satisfied because activation (returns from individual experience) is slow/local while inhibition (collective pheromone signals) is fast/global. The doc also specifies Turing pattern stability analysis (linear stability, pitchfork bifurcation, Hopf oscillatory instability monitoring) and Lyapunov stability monitoring.

**Current code**: `STRATEGY_DIMS = 8` at `crates/roko-orchestrator/src/coordination.rs:497`. `STRATEGY_DIMS_F64 = 8.0` at line 22. `MorphogeneticState` at line 501 has `strategy: [f64; STRATEGY_DIMS]` (line 503), `attributed_returns: [f64; STRATEGY_DIMS]` (line 505), `collective_pheromone: [f64; STRATEGY_DIMS]` (line 507), `collective_size: usize` (line 509). `Default::default()` at line 512 initializes `strategy` to `[1.0/8.0; 8]` (uniform). `MorphogeneticParams` at line 533 has `alpha: f64` (line 536, default 0.05), `beta: f64` (line 538, default 0.15), `mu: f64` (line 540, default 0.01), `baseline: f64` (line 542, default 1/8), `sigma_noise: f64` (line 544, default 0.001), `resource_pressure_scalar: f64` (line 546, default 1.0). `specialization_index()` at line 527 computes normalized entropy of strategy vector. But there is NO `update()` method — params and state exist but dynamics are never applied.

**What to change**:
- Add `pub fn update(&mut self, params: &MorphogeneticParams)` on `MorphogeneticState` implementing:
  ```rust
  pub fn update(&mut self, params: &MorphogeneticParams) {
      let pressure = params.resource_pressure_scalar;
      let size = (self.collective_size as f64).max(1.0);
      let mut rng = rand::rng();
      for i in 0..STRATEGY_DIMS {
          let activation = params.alpha * self.attributed_returns[i] * pressure;
          let inhibition = params.beta * self.collective_pheromone[i] / size;
          let decay = params.mu * (self.strategy[i] - params.baseline);
          let noise = rand_distr::Normal::new(0.0, params.sigma_noise).unwrap().sample(&mut rng);
          self.strategy[i] += activation - inhibition - decay + noise;
          self.strategy[i] = self.strategy[i].max(0.001); // prevent negative
      }
      // Re-normalize to sum to 1.0
      let sum: f64 = self.strategy.iter().sum();
      if sum > 0.0 { for s in &mut self.strategy { *s /= sum; } }
      // Reset accumulators
      self.attributed_returns = [0.0; STRATEGY_DIMS];
      self.collective_pheromone = [0.0; STRATEGY_DIMS];
  }
  ```
- In `orchestrate.rs`, after each task completion (after gate result recording ~line 13888), call `morpho_state.update(&morpho_params)` on the agent's morphogenetic state
- Add `pub fn attribute_return(&mut self, dimension: usize, value: f64)` helper to `MorphogeneticState` for recording returns per dimension

**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:497` — `STRATEGY_DIMS = 8` constant
- `crates/roko-orchestrator/src/coordination.rs:501-530` — `MorphogeneticState` struct, `Default` impl, `specialization_index()` method
- `crates/roko-orchestrator/src/coordination.rs:533-560` — `MorphogeneticParams` struct with all 6 fields and defaults
- `crates/roko-orchestrator/src/coordination.rs:22` — `STRATEGY_DIMS_F64` constant
- `crates/roko-orchestrator/src/coordination.rs:562-573` — `specialization_index()` free function (entropy-based)
- `crates/roko-orchestrator/src/coordination.rs:726-729` — existing specialization test
- `crates/roko-cli/src/orchestrate.rs` — orchestration loop (call `update()` after task completion)
- `docs/13-coordination/07-morphogenetic-specialization.md` — Gierer-Meinhardt update rule, stability analysis
**Accept when**:
- [x] `update()` computes activation/inhibition/decay/noise per dimension -- MorphogeneticState::update() at coordination.rs:583-610 implements full Gierer-Meinhardt dynamics
- [ ] Agent strategy vectors diverge over time in a collective (update() not called from orchestrate.rs)
- [ ] Specialization index increases from 0 to measurable values (update() not called from orchestrate.rs)
- [ ] `cargo test -p roko-orchestrator`
**Verify**:
```bash
grep -n 'fn update' crates/roko-orchestrator/src/coordination.rs
grep -n 'specialization_index' crates/roko-orchestrator/src/coordination.rs
cargo test -p roko-orchestrator -- coordination
```

**Priority**: P1

### COORD-03: Pheromone enrichment in context assembly
- [x] Wire ambient pheromone summary into SystemPromptBuilder

**Spec** (doc 03): Pheromone-enriched context assembly. Agents should sense ambient pheromones via prompt.
**Current code**: `SystemPromptBuilder` layer 3c at `crates/roko-compose/src/system_prompt_builder.rs:66-67` stores `pheromones: Vec<ContextChunk>`. The `with_pheromones()` setter is at `:167-171`. The `pheromone_section()` renderer is at `:1026-1054` (sorts by relevance, renders with `render_pheromone_chunk` at `:1057`). `RoleSystemPromptSpec` at `crates/roko-compose/src/role_prompts.rs:175-176` also has a `pheromones` field and passes it through at `:317-318`. **However**, `PromptBuildOptions` in `crates/roko-cli/src/prompting.rs:12-25` has NO pheromones field, so `build_spec()` at `:27-52` never calls `spec.with_pheromones()`. This means pheromones are structurally supported but never populated in the orchestration path.
**What to change**:
- Add `pub pheromones: Vec<roko_compose::ContextChunk>` to `PromptBuildOptions` in `crates/roko-cli/src/prompting.rs:12` (currently has fields for role, task description, allowed tools, etc. but no pheromones)
- In `build_spec()` at `:27-52`, after existing `spec` configuration, add: `if !options.pheromones.is_empty() { spec = spec.with_pheromones(&options.pheromones); }`
- In `orchestrate.rs`, before each agent dispatch call, query the local pheromone field (a `Vec<Pheromone>` on `PlanRunner` or similar), filter for non-decayed pheromones, convert each to `ContextChunk { label, content, relevance }`, and set `prompt_opts.pheromones = pheromone_chunks`
- The `ContextChunk` conversion should include: pheromone kind as label, intensity/scope/confirmations as content, intensity as relevance score
**Depends on**: COORD-04 (pheromones must be deposited before they can be sensed)
**Reference files**:
- `crates/roko-cli/src/prompting.rs:12-52` -- `PromptBuildOptions` and `build_spec()` (the gap)
- `crates/roko-compose/src/system_prompt_builder.rs:66-67,167-171,1026-1069` -- builder pheromone layer
- `crates/roko-compose/src/role_prompts.rs:175-176,317-318` -- `RoleSystemPromptSpec` pheromone pass-through
- `crates/roko-compose/src/system_prompt_builder.rs:1156-1172` -- existing test `build_with_pheromones_includes_active_signals_layer`
**Accept when**:
- [x] SystemPromptBuilder queries local pheromone field -- active_pheromone_chunks() at orchestrate.rs:5354 converts pheromone_field to ContextChunks, passed to PromptBuildOptions at :16519
- [ ] Active pheromones summarized in prompt (kind, intensity, source) -- build_spec() in prompting.rs never calls spec.with_pheromones(); field populated but not wired through to builder
- [ ] Agent behavior influenced by ambient signals
- [ ] `cargo test -p roko-compose`
**Verify**:
```bash
grep -n 'pheromones' crates/roko-cli/src/prompting.rs
grep -n 'with_pheromones' crates/roko-compose/src/role_prompts.rs
cargo test -p roko-compose -- pheromone
```

**Priority**: P1

### COORD-04: Pheromone deposit in orchestration loop
- [x] Wire gate results and task outcomes to pheromone deposits

**Spec** (doc 00 `00-stigmergy-theory.md`, doc 03 `03-digital-pheromones.md`): Stigmergic loop: deposit -> propagate -> sense -> respond. Gate results are the primary source of pheromone signals in single-agent mode. On gate pass, the agent deposits an `Opportunity` pheromone with intensity proportional to the gate's confidence score. On gate fail, the agent deposits a `Threat` pheromone with high intensity. On repeated same-gate failures (N >= 3), a `Pattern` pheromone is deposited to signal a recurring problem. These pheromones feed into COORD-03 (context assembly) so subsequent agents see the ambient signals.

**Current code**: `Pheromone::new()` at `crates/roko-orchestrator/src/coordination.rs:306-324` accepts `(kind: PheromoneKind, intensity: f64, half_life: Duration, source: String, scope: PheromoneScope)` and returns a `Pheromone`. `PheromoneKind` at `:190-236` has `Threat`:198, `Opportunity`:204, `Pattern`:212, etc. with default half-lives at `:240-254`. `orchestrate.rs` records gate results at `:13888-13895` via `GateResult::from_verdict()` and persists verdicts to `FileSubstrate` at `:13846-13886`, but NEVER constructs or deposits `Pheromone` instances. The stigmergic loop is broken: gate outcomes produce structured results but not pheromone signals.

**What to change**:
- Add `pub pheromone_field: Vec<Pheromone>` field to `PlanRunner` (or create a `PheromoneField` struct on `PlanRunner`)
- After gate result recording in `orchestrate.rs` (~line 13888):
  ```rust
  let pheromone = match &gate_result.verdict {
      Verdict::Pass => Pheromone::new(
          PheromoneKind::Opportunity, 0.8,
          PheromoneKind::Opportunity.default_half_life(),
          format!("gate:{}", gate_name),
          PheromoneScope::Local(collective_id.clone()),
      ),
      Verdict::Fail { .. } => Pheromone::new(
          PheromoneKind::Threat, 0.9,
          PheromoneKind::Threat.default_half_life(),
          format!("gate:{}", gate_name),
          PheromoneScope::Local(collective_id.clone()),
      ),
  };
  plan_runner.pheromone_field.push(pheromone);
  ```
- Add pattern detection: track per-gate failure counts; when `fail_count >= 3` for the same gate, deposit `Pheromone::new(PheromoneKind::Pattern, 0.7, ...)`
- Pheromones in `pheromone_field` are consumed by COORD-03 (context enrichment) before the next agent dispatch

**Reference files**:
- `crates/roko-cli/src/orchestrate.rs:13763-13900` — `run_gate_pipeline()` function where deposits should happen
- `crates/roko-orchestrator/src/coordination.rs:286-351` — `Pheromone` struct, `new()` constructor at :306
- `crates/roko-orchestrator/src/coordination.rs:190-254` — `PheromoneKind` enum with 8 variants and `default_half_life()` at :240
- `crates/roko-orchestrator/src/coordination.rs:256-284` — `PheromoneScope` enum with `Local(CollectiveId)`
- `crates/roko-gate/src/` — `Verdict` enum (Pass/Fail) that triggers deposit decisions
**Accept when**:
- [x] Gate pass -> `Pheromone::new(PheromoneKind::Opportunity, 0.8, ...)` deposited in `pheromone_field` -- orchestrate.rs:14549/14568
- [x] Gate fail -> `Pheromone::new(PheromoneKind::Threat, 0.9, ...)` deposited in `pheromone_field` -- orchestrate.rs:14565/14568
- [x] Same gate failing 3+ times -> `PheromoneKind::Pattern` deposited -- orchestrate.rs:14556-14563 (count >= 3)
- [x] Pheromones stored in `PlanRunner.pheromone_field` (consumed by COORD-03) -- pheromone_field: Vec<Pheromone> at orchestrate.rs:3079
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -n 'Pheromone::new\|PheromoneKind\|pheromone_field' crates/roko-cli/src/orchestrate.rs
cargo test --workspace
```

**Priority**: P1

### COORD-05: Scope promotion gates
- [x] Implement Local->Mesh->Global pheromone promotion

**Spec** (doc 05 `05-pheromone-scope.md`): Scope promotion widens a pheromone's reach when confidence is sufficient. Promotion thresholds: Local->Subnet requires `confirmations >= 3` from distinct agents in the subnet, Subnet->Mesh requires `confirmations >= 5` from agents in at least 2 subnets, Mesh->Global requires `confirmations >= 10` from agents in at least 3 mesh regions. Trust discounting factors applied when reading pheromones from a broader scope: `Local=1.0`, `Subnet=0.90`, `Mesh=0.80`, `Global=0.50`. The Constructal Law connection (Bejan 1997): pheromone scope hierarchy mirrors natural flow systems where information flows from fine-grained local channels to coarser global channels.

**Current code**: `PheromoneScope` at `crates/roko-orchestrator/src/coordination.rs:256-284` has `rank()` (Local=0, Subnet=1, Mesh=2, Global=3) and `is_broader_than()` for comparing scopes. `check_promotion()` at `:411-437` handles **kind** promotion (Pattern->Wisdom->Consensus) using `PromotionConfig` at `:384-409`. But there is NO **scope** promotion logic (Local->Subnet->Mesh->Global). No trust discounting factors implemented. `Pheromone::confirmations` at line 302 tracks confirmation count but no scope-promotion check uses it.

**What to change**:
- Add `ScopePromotionConfig` struct to `crates/roko-orchestrator/src/coordination.rs`:
  ```rust
  pub struct ScopePromotionConfig {
      pub local_to_subnet_confirmations: u32,   // default 3
      pub subnet_to_mesh_confirmations: u32,    // default 5
      pub mesh_to_global_confirmations: u32,    // default 10
  }
  pub const TRUST_DISCOUNT: [f64; 4] = [1.0, 0.90, 0.80, 0.50]; // Local, Subnet, Mesh, Global
  ```
- Add `pub fn check_scope_promotion(pheromone: &Pheromone, config: &ScopePromotionConfig) -> Option<PheromoneScope>` that checks if `pheromone.confirmations` exceeds the threshold for the next scope level
- Add `pub fn trust_discounted_intensity(pheromone: &Pheromone, reader_scope: &PheromoneScope) -> f64` that applies `TRUST_DISCOUNT[scope.rank()]` to `pheromone.intensity`
- Integrate scope promotion into the same path that currently calls `check_promotion()` for kind promotion — after kind promotion, check scope promotion

**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:256-284` — `PheromoneScope` with `rank()`:276 and `is_broader_than()`:280
- `crates/roko-orchestrator/src/coordination.rs:384-437` — `PromotionConfig`:384 + `check_promotion()`:411 (kind promotion pattern to follow)
- `crates/roko-orchestrator/src/coordination.rs:286-351` — `Pheromone` struct with `confirmations`:302
- `crates/roko-orchestrator/src/coordination.rs:773-777` — test showing Local/Subnet/Mesh/Global scope construction
- `crates/roko-orchestrator/src/coordination.rs:363-376` — `pheromone_decay()` + `effective_confirmations()`
- `docs/13-coordination/05-pheromone-scope.md` — scope hierarchy, promotion thresholds, trust discounting
**Depends on**: COORD-01 (Mesh transport must exist for Mesh-scope promotion to be meaningful)
**Accept when**:
- [x] `ScopePromotionConfig` with configurable confirmation thresholds
- [x] `check_scope_promotion()` returns next scope when confirmations exceed threshold
- [x] `trust_discounted_intensity()` applies `TRUST_DISCOUNT[scope.rank()]` on cross-scope reads
- [x] Local pheromone with 3+ confirmations promoted to Subnet scope
- [x] `cargo test -p roko-orchestrator`
**Verify**:
```bash
grep -n 'ScopePromotionConfig\|check_scope_promotion\|trust_discount\|TRUST_DISCOUNT' crates/roko-orchestrator/src/coordination.rs
cargo test -p roko-orchestrator -- coordination
```

**Priority**: P2 (depends on COORD-01)

### COORD-06: Niche competition
- [x] Implement cosine similarity between agent strategy vectors

**Spec** (doc 07 `07-morphogenetic-specialization.md`): Niche competition via cosine similarity detects when two agents are converging on the same strategy, which wastes collective resources. Formula: `cos(a,b) = sum(a_i * b_i) / (||a|| * ||b||)` over the 8 strategy dimensions. Similarity > 0.9 indicates niche overlap — one agent should shift strategy. The doc specifies that niche conflict resolution happens via the inhibition term in the Gierer-Meinhardt dynamics: when two agents detect high similarity, both increase their `collective_pheromone` values for their shared dimensions, which the inhibition term in `update()` then pushes them apart. This is automatic — no central role assignment needed.

**Current code**: `MorphogeneticState.strategy` at `crates/roko-orchestrator/src/coordination.rs:503` is a `[f64; STRATEGY_DIMS]` vector (8 dimensions: depth, breadth, execution, verification, time_horizon, exploration, exploitation, coordination). `specialization_index()` at `:562-573` measures individual diversity via normalized entropy but there is no pairwise comparison. No cosine similarity function exists. `PheromoneAlert` event variant at `crates/roko-runtime/src/heartbeat.rs:572` exists but is never emitted.

**What to change**:
- Add to `crates/roko-orchestrator/src/coordination.rs`:
  ```rust
  pub fn cosine_similarity(a: &[f64; STRATEGY_DIMS], b: &[f64; STRATEGY_DIMS]) -> f64 {
      let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
      let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
      let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
      if norm_a * norm_b < 1e-10 { return 0.0; }
      dot / (norm_a * norm_b)
  }
  pub fn niche_conflicts(
      agents: &[(String, MorphogeneticState)],  // (AgentId, state)
      threshold: f64,                            // default 0.9
  ) -> Vec<(String, String, f64)> {              // (agent_a, agent_b, similarity)
      let mut conflicts = Vec::new();
      for i in 0..agents.len() {
          for j in (i+1)..agents.len() {
              let sim = cosine_similarity(&agents[i].1.strategy, &agents[j].1.strategy);
              if sim > threshold { conflicts.push((agents[i].0.clone(), agents[j].0.clone(), sim)); }
          }
      }
      conflicts
  }
  ```
- In `orchestrate.rs`, after morphogenetic `update()` (see COORD-02), call `niche_conflicts()` with threshold 0.9
- When conflicts detected, emit `PheromoneAlert` event at `crates/roko-runtime/src/heartbeat.rs:572` and boost `collective_pheromone` for the shared dimensions to trigger automatic separation

**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:497-530` — `STRATEGY_DIMS=8`:497, `MorphogeneticState`:501 with `strategy: [f64; 8]`:503
- `crates/roko-orchestrator/src/coordination.rs:562-573` — `specialization_index()` (per-agent entropy, not pairwise)
- `crates/roko-orchestrator/src/coordination.rs:726-729` — existing specialization test (add niche test here)
- `crates/roko-runtime/src/heartbeat.rs:572` — `PheromoneAlert` event variant (emit on conflict)
- `docs/13-coordination/07-morphogenetic-specialization.md` — niche competition, cosine similarity, automatic resolution via inhibition
**Depends on**: COORD-02 (strategy vectors must diverge before competition is meaningful)
**Accept when**:
- [x] `cosine_similarity()` computes correct similarity between 8-dimensional strategy vectors
- [x] `niche_conflicts()` returns pairs with similarity > threshold
- [ ] High similarity (>0.9) emits `PheromoneAlert` event (niche_conflicts() and resolve_niche_conflicts() exist but PheromoneAlert never emitted)
- [x] Conflict triggers increased `collective_pheromone` for shared dimensions (automatic separation)
- [x] `cargo test -p roko-orchestrator`
**Verify**:
```bash
grep -n 'cosine_similarity\|niche_conflict\|NicheConflict' crates/roko-orchestrator/src/coordination.rs
cargo test -p roko-orchestrator -- niche
cargo test -p roko-orchestrator -- coordination
```

**Priority**: P2

### COORD-07: Alpha pheromone paradox
- [x] Implement confirmation-reduces-half-life for Alpha kind

**Spec** (doc 04 `04-pheromone-kinds.md`): Alpha pheromones uniquely exhibit the "Alpha paradox" — confirmations SHORTEN half-life instead of extending it. Formula: `tau_effective = tau_base * max(0.5, 1.0 - confirmations * 0.2)`. Rationale (anti-herding): once everyone sees the alpha signal, it should fade faster to prevent information cascades (Bikhchandani, Hirshleifer & Welch 1992). All other 7 pheromone kinds use the standard formula where confirmations EXTEND half-life: `tau_effective = tau_base * (1.0 + confirmations * 0.5)`.

**Current code**: `Pheromone::effective_half_life()` at `crates/roko-orchestrator/src/coordination.rs:328-331` uses a uniform formula `self.half_life.mul_f64(1.0 + f64::from(self.confirmations) * 0.5)` for ALL kinds — no branching on `self.kind`. `PheromoneKind::Alpha` at line 198 has a 1-hour default half-life at line 248 (`Duration::from_secs(3600)`). `pheromone_decay()` at lines 363-376 also uses the same uniform extension formula inline.

**What to change**:
- Modify `effective_half_life()` at `crates/roko-orchestrator/src/coordination.rs:328-331`:
  ```rust
  pub fn effective_half_life(&self) -> Duration {
      match self.kind {
          PheromoneKind::Alpha => {
              let factor = (1.0 - f64::from(self.confirmations) * 0.2).max(0.5);
              self.half_life.mul_f64(factor)
          }
          _ => self.half_life.mul_f64(1.0 + f64::from(self.confirmations) * 0.5),
      }
  }
  ```
- Update `pheromone_decay()` at lines 363-376 to use `self.effective_half_life()` instead of inline formula
- Add test case: create Alpha pheromone with 3 confirmations, verify `effective_half_life()` < `half_life` (0.4x base)
**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:326-331` -- `effective_half_life()` (needs branching)
- `crates/roko-orchestrator/src/coordination.rs:363-376` -- `pheromone_decay()` (same uniform formula)
- `crates/roko-orchestrator/src/coordination.rs:190-254` -- `PheromoneKind` enum with `Alpha` at `:198`
- `crates/roko-orchestrator/src/coordination.rs:677-688` -- decay test (should add Alpha-specific case)
**Accept when**:
- [x] Alpha pheromones decay faster when confirmed (anti-herding)
- [x] Other kinds keep standard confirmation extension
- [x] `cargo test -p roko-orchestrator`
**Verify**:
```bash
grep -n 'effective_half_life\|Alpha' crates/roko-orchestrator/src/coordination.rs
cargo test -p roko-orchestrator -- decay
cargo test -p roko-orchestrator -- coordination
```

**Priority**: P2

### COORD-08: Permissioned subnet enforcement
- [x] Implement access control for subnet-scoped pheromones

**Spec** (doc 08 `08-permissioned-subnets.md`): Subnets are private mesh scopes with controlled access. Three access models: (1) `Invite` — explicit invitation by existing member, membership stored on-chain or in local config; (2) `Role` — agents with a matching role (e.g., `"verifier"`, `"researcher"`) automatically join; (3) `Reputation` — agents with reputation above threshold in a specific domain gain access. Publishing gates for Subnet->Mesh promotion: a pheromone can only be promoted from Subnet to Mesh scope if it has confirmations from at least `min_distinct_confirmers` distinct subnet members. This prevents a single agent from promoting unvetted signals. Club goods theory (Buchanan 1965): subnets are excludable, non-rivalrous goods — membership provides value, but the value decreases with over-admission.

**Current code**: `SubnetId` at `crates/roko-orchestrator/src/coordination.rs:148-163` has `collective: CollectiveId`:150 and `name: String`:152. Validation via `validate_subnet_name()` at `:172-186` (max 64 chars, alphanumeric + hyphens). `PheromoneScope::Subnet(SubnetId)` at `:264` exists. No `SubnetMembership` struct. No access check on pheromone read/write. No publishing gate. `WisdomGate` at `:626-654` is a similar gate pattern (checks `min_diversity`, `min_confirmations`, `max_staleness_secs`) that can be followed.

**What to change**:
- Add to `crates/roko-orchestrator/src/coordination.rs`:
  ```rust
  pub enum AccessModel { Invite, Role { required_role: String }, Reputation { domain: String, min_score: f64 } }
  pub struct SubnetMembership {
      pub subnet: SubnetId,
      pub members: HashSet<String>,     // AgentIds
      pub access_model: AccessModel,
      pub min_distinct_confirmers: u32, // default 2, for promotion gates
  }
  impl SubnetMembership {
      pub fn can_access(&self, agent_id: &str) -> bool {
          self.members.contains(agent_id)
      }
      pub fn can_promote_to_mesh(&self, pheromone: &Pheromone, confirmer_agents: &[String]) -> bool {
          let distinct: HashSet<_> = confirmer_agents.iter()
              .filter(|a| self.members.contains(a.as_str()))
              .collect();
          distinct.len() as u32 >= self.min_distinct_confirmers
      }
  }
  ```
- Check `SubnetMembership::can_access()` before reading or writing Subnet-scoped pheromones
- Check `can_promote_to_mesh()` before scope promotion from Subnet to Mesh (COORD-05)

**Reference files**:
- `crates/roko-orchestrator/src/coordination.rs:117-186` — `SubnetIdError`:117, `SubnetId`:148, `validate_subnet_name()`:172
- `crates/roko-orchestrator/src/coordination.rs:256-284` — `PheromoneScope::Subnet(SubnetId)`:264
- `crates/roko-orchestrator/src/coordination.rs:626-654` — `WisdomGate` (similar gate pattern: `min_diversity`, `min_confirmations`, `max_staleness_secs`)
- `crates/roko-orchestrator/src/coordination.rs:669-674` — subnet validation test (extend with membership test)
- `docs/13-coordination/08-permissioned-subnets.md` — three access models, publishing gates, club goods theory
**Depends on**: COORD-05 (scope promotion logic for Subnet->Mesh gate)
**Accept when**:
- [x] `SubnetMembership` struct with `members`, `access_model`, `min_distinct_confirmers`
- [x] `can_access()` checked before reading/writing Subnet-scoped pheromones
- [x] `can_promote_to_mesh()` requires min distinct confirming members for Subnet->Mesh promotion
- [x] Three access models (Invite, Role, Reputation) each evaluate membership differently
- [x] `cargo test -p roko-orchestrator`
**Verify**:
```bash
grep -n 'SubnetMembership\|can_access\|can_promote_to_mesh\|AccessModel' crates/roko-orchestrator/src/coordination.rs
cargo test -p roko-orchestrator -- subnet
```

**Priority**: P2

## Verify
```bash
# Core coordination tests
cargo test -p roko-orchestrator -- coordination
# Pheromone rendering in prompts
cargo test -p roko-compose -- pheromone
# C-Factor metrics
cargo test -p roko-learn -- cfactor
# Full workspace build
cargo test --workspace
# Check pheromone wiring from orchestrate.rs
grep -rn 'Pheromone\|pheromone' crates/roko-cli/src/orchestrate.rs | grep -v target/
grep -rn 'pheromones' crates/roko-cli/src/prompting.rs
```
