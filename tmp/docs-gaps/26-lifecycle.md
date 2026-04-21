# 17-lifecycle -- Gap checklist

Spec: `docs/17-lifecycle/` (13 files). Code: `crates/roko-agent/src/lifecycle.rs`, `crates/roko-cli/src/`, `crates/roko-neuro/`.

Overall: ~23% compliant. Core types (AgentCoreManifest, DeploymentMode, DomainPlugin) exist. Mortality concepts removed. Major gaps in CLI commands, provisioning pipeline, budget enforcement, and knowledge backup/restore.

## Compliant (no action needed)

- Mortality thesis removed -- no death/terminal states (doc 00)
- AgentCoreManifest struct with prompt, mode, domain, schema_version (doc 01)
- DeploymentMode enum (Hosted, SelfHosted) (doc 01)
- DomainPlugin enum (Chain, Coding, Research, Custom) (doc 01)
- Ebbinghaus config -- decay_model, tier multipliers, knowledge type half-lives (doc 10)
- Academic foundations -- reference material (doc 12)

## Checklist

### LIFE-01: Agent creation CLI

- [x] Implement `roko agent create` command flow

**Spec** (doc 01 `docs/17-lifecycle/01-agent-creation.md`): Three-interaction creation pattern:
1. **Describe** — user provides name, domain (`chain`/`coding`/`research`/`general`), natural language prompt, and optional strategy template
2. **Review** — system shows generated `AgentCoreManifest` (prompt, mode, domain, schema_version) and `AgentExtendedManifest` (lineage, budget, neuro, mesh, successor config) for user confirmation
3. **Confirm** — user approves, system writes manifest TOML to disk and runs `validate_manifest()`

CLI entry: `roko agent create [--name <name>] [--domain <domain>] [--template <template>]`. API entry: POST to `/api/agents` on roko-serve. Custody modes: Hosted (managed infra) or SelfHosted (local). Strategy templates: pre-configured bundles of prompt + tools + config (see TOOL-06 in `27-tools.md`).

**Current code** (`crates/roko-agent/src/lifecycle.rs:18`): `AgentCoreManifest` struct with `prompt: String`, `mode: DeploymentMode`, `domain: DomainPlugin`, `schema_version: u32`. `AgentExtendedManifest` at line 121 with `lineage_id: Option<String>`, `generation: u32`, `budget: BudgetConfig`, `neuro: NeuroConfig`, `mesh: MeshConfig`, `successor: SuccessorConfig`. `validate_manifest()` at line 439 checks field validity. `resolve_manifest()` at line 399 resolves defaults. No CLI command `roko agent create` in `crates/roko-cli/src/main.rs`.

**What to change**: Add `AgentCommand::Create { name: Option<String>, domain: Option<String>, template: Option<String> }` variant to CLI in `crates/roko-cli/src/main.rs`. Implement interactive three-step flow. Generate manifest TOML at `.roko/agents/<name>/manifest.toml`. Call `validate_manifest()` before accepting.

**Reference files**:
- `crates/roko-agent/src/lifecycle.rs:18` — `AgentCoreManifest`, `AgentExtendedManifest` at 121, `validate_manifest()` at 439, `resolve_manifest()` at 399
- `crates/roko-cli/src/main.rs` — CLI command registration (add `AgentCommand` subcommand)
- `crates/roko-core/src/config/schema.rs:1289` — `AgentConfig` struct in config
- `docs/17-lifecycle/01-agent-creation.md` — full spec: three-interaction pattern, manifest schema, custody modes
**Depends on**: None
**Accept when**:
- [x] `roko agent create` command exists in CLI
- [x] Prompts for name, domain, strategy (interactive or via flags)
- [x] Generates manifest TOML at `.roko/agents/<name>/manifest.toml`
- [x] Calls `validate_manifest()` before writing
- [x] Supports `--template` flag for strategy templates
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'agent.*create\|AgentCommand' crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1

---

### LIFE-02: Type-state provisioning pipeline

- [x] Implement 8-stage type-state provisioning

**Spec** (doc 02 `docs/17-lifecycle/02-provisioning.md`): Rust type-state pattern enforces stage progression at compile time. Eight stages:
1. `Agent<Unvalidated>` — manifest loaded from disk, not yet checked
2. `Agent<Validated>` — `validate_manifest()` passed, fields are sane
3. `Agent<ResourcesAllocated>` — compute resources reserved (VM or local process slot)
4. `Agent<NeuroInitialized>` — Neuro store created or loaded from backup
5. `Agent<RoutingConfigured>` — model routing configured (CascadeRouter, provider registry)
6. `Agent<ToolsLoaded>` — tool profile activated, MCP servers discovered
7. `Agent<MeshRegistered>` — registered with Agent Mesh (if mesh.enabled)
8. `Agent<Running>` — heartbeat started, gamma loop active

Each transition method consumes `self` and returns the next state, e.g.:
```rust
impl Agent<Unvalidated> {
    fn validate(self) -> Result<Agent<Validated>, ProvisioningError>;
}
impl Agent<Validated> {
    fn allocate_resources(self) -> Result<Agent<ResourcesAllocated>, ProvisioningError>;
}
```
Invalid transitions are compile-time errors (cannot call `allocate_resources()` on `Agent<Unvalidated>`).

Three deployment paths: (a) `roko run` — inline provisioning within CLI, (b) `roko serve` — HTTP-driven provisioning via control plane, (c) programmatic — Rust API for embedding.

**Current code** (`crates/roko-agent/src/lifecycle.rs:454`): Type-state marker structs **already defined**: `Unvalidated` (454), `Validated` (458), `ResourcesAllocated` (462), `NeuroInitialized` (466), `RoutingConfigured` (470), `ToolsLoaded` (474), `MeshRegistered` (478). `ProvisioningError` at line 426 with variants. **Partially built** — markers exist but no `Agent<S>` generic struct or transition methods connecting them.

**What to change**: Add `pub struct Agent<S> { manifest: AgentExtendedManifest, _state: PhantomData<S> }` in `crates/roko-agent/src/lifecycle.rs`. Implement `impl Agent<State>` for each of the 8 states with a transition method that performs the stage's work and returns `Result<Agent<NextState>, ProvisioningError>`. Wire into `roko run` and `roko serve` paths.

**Reference files**:
- `crates/roko-agent/src/lifecycle.rs:454` — marker structs (`Unvalidated` through `MeshRegistered`), `ProvisioningError` at 426, `validate_manifest()` at 439
- `crates/roko-agent/src/dispatcher/mod.rs` — dispatcher setup (used in RoutingConfigured -> ToolsLoaded transition)
- `crates/roko-cli/src/orchestrate.rs` — current inline provisioning flow (to be refactored into type-state)
- `docs/17-lifecycle/02-provisioning.md` — full spec: 8 stages, deployment paths, warm pool
**Depends on**: LIFE-01 (agent creation produces manifest to provision)
**Accept when**:
- [x] `Agent<S>` generic struct defined with `PhantomData<S>`
- [x] Each stage has a transition method consuming `self` and returning next state
- [x] Invalid transitions are compile-time errors
- [x] Each stage performs its required work (validation, resource allocation, neuro init, etc.)
- [x] Wired into at least one deployment path (`roko run`)
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'Agent<\|impl Agent<\|PhantomData' crates/roko-agent/src/lifecycle.rs
cargo test --workspace
```
**Priority**: P1

---

### LIFE-03: Budget system with cost tracking

- [x] Implement budget configuration, tracking, and graceful degradation

**Spec** (doc 04 `docs/17-lifecycle/04-funding-and-budgets.md`): Budget model with 5 resource types:
1. **Inference tokens** — per-LLM-call, tracked by dispatcher
2. **Compute time** — wall-clock seconds (managed infra only)
3. **Tool invocations** — per-call (usually free locally, x402-gated remotely)
4. **On-chain gas** — gas units x price (chain domain only)
5. **Mesh operations** — per-query/sync (x402-gated for public mesh)

Budget config in `roko.toml`:
```toml
[budget]
max_daily_inference_usd = 10.0
max_total_usd = 1000.0         # optional lifetime cap
max_tokens_per_turn = 8192
warning_at = 0.7               # warn at 70%
critical_at = 0.9              # critical at 90%
degradation = "cascade"        # "cascade" | "pause" | "notify-only"
```

Per-turn cost tracking via `TurnCostRecord` (model, input/output tokens, cache tokens, cost_usd, latency_ms). Written to `.roko/learn/efficiency.jsonl`.

Multi-level guardrails: 70% warning, 90% critical, 100% hard stop. Degradation cascade: (1) downgrade T2->T1 model, (2) reduce context budget, (3) increase T0 threshold (fewer LLM calls), (4) pause non-essential tasks, (5) notify operator, (6) hard stop.

Four funding sources: direct config, KORAI token staking, x402 payment protocol, operator top-up.

**Current code**: `BudgetConfig` at `crates/roko-core/src/config/schema.rs:1969` with `max_plan_usd`, `max_turn_usd`, `prompt_token_budget`. `AgentBudget` at line 1382 with `max_tokens_per_turn`, `max_cost_usd_cents_per_turn`. `BudgetConfig` at `crates/roko-agent/src/lifecycle.rs:337` with per-turn/daily/lifetime limits and `BudgetDegradationMode` enum (line 371) with `Cascade`, `Pause`, `NotifyOnly` variants. **Config structs exist. No `BudgetTracker` runtime enforcement. No degradation cascade logic. No per-turn cost accumulation.**

**What to change**:
1. Add `BudgetTracker` struct in `crates/roko-agent/src/` with `accumulated_daily_usd: f64`, `accumulated_total_usd: f64`, `fn record_turn_cost(&mut self, cost: &TurnCostRecord)`, `fn check_budget(&self) -> BudgetStatus`
2. Wire `BudgetTracker::check_budget()` call into dispatcher before each LLM invocation
3. Implement degradation cascade: when `BudgetStatus::Warning`, downgrade model tier; when `BudgetStatus::Critical`, pause non-essential; when `BudgetStatus::Exhausted`, hard stop
4. Emit budget events to efficiency log for dashboard visibility

**Reference files**:
- `crates/roko-core/src/config/schema.rs:1969` — `BudgetConfig` with limits
- `crates/roko-agent/src/lifecycle.rs:337` — `BudgetConfig` with degradation mode, `BudgetDegradationMode` at 371
- `crates/roko-agent/src/dispatcher/mod.rs` — dispatch loop (insert budget check before LLM call)
- `crates/roko-learn/src/efficiency.rs` — `TurnCostRecord`, efficiency event logging
- `docs/17-lifecycle/04-funding-and-budgets.md` — full spec: 5 resource types, TurnCostRecord fields, degradation cascade, funding sources
**Depends on**: None
**Accept when**:
- [x] `BudgetTracker` struct accumulates per-turn costs
- [x] Budget limits configurable in `roko.toml` `[budget]` section
- [x] Per-turn cost recorded via `TurnCostRecord` to efficiency log
- [x] `check_budget()` called before each LLM invocation in dispatcher
- [x] Warning at 70%, critical at 90%, hard stop at 100% (configurable thresholds)
- [x] Degradation cascade: downgrade model -> reduce context -> increase T0 threshold -> pause -> notify -> stop
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'BudgetTracker\|budget_check\|fn enforce_budget\|BudgetStatus' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P1

---

### LIFE-04: Knowledge backup CLI

- [x] Implement `roko neuro backup` command

**Spec** (doc 05 `docs/17-lifecycle/05-knowledge-backup-export.md`): `roko neuro backup <path>` exports the Neuro knowledge store to a portable archive. Archive format (tar.gz) contains:
- `manifest.json` — agent ID, schema version, export timestamp, Engram count, total size
- `engrams/` — serialized Engrams with scores, tiers, provenance, decay state
- `hdc/` — HDC vectors (10,240-bit BSC) for similarity search
- `metadata.json` — knowledge type distribution, tier distribution, age histogram

Optional **genomic bottleneck compression**: export only top-N Engrams by score (e.g., top 1000), discarding low-confidence knowledge. This simulates biological information bottleneck at reproduction — only the most valuable knowledge transfers.

**Current code**: `crates/roko-neuro/src/knowledge_store.rs` has knowledge store with `iter_engrams()`, `get_by_hash()`, tier management. No `backup` subcommand in CLI. No archive format defined. No export function.

**What to change**: Add `NeuroCommand::Backup { path: PathBuf, top_n: Option<usize> }` to CLI in `crates/roko-cli/src/main.rs`. Implement `fn backup_neuro(store: &KnowledgeStore, path: &Path, top_n: Option<usize>) -> Result<()>` that creates tar.gz archive with manifest, Engrams, HDC vectors, and metadata.

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs` — `KnowledgeStore` with `iter_engrams()`, tier management
- `crates/roko-neuro/src/lib.rs` — knowledge types, tier constants
- `crates/roko-cli/src/main.rs` — CLI command registration
- `docs/17-lifecycle/05-knowledge-backup-export.md` — full spec: archive format, genomic bottleneck, manifest fields
**Depends on**: None
**Accept when**:
- [x] `roko neuro backup <path>` CLI command exists
- [x] Creates backup directory with manifest.json, knowledge.jsonl, confirmations
- [x] Optional `--top-n <N>` flag for genomic bottleneck compression
- [x] Archive is self-describing (manifest includes schema version and Engram count)
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'neuro.*backup\|NeuroCommand\|fn backup' crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1

---

### LIFE-05: Knowledge restore CLI

- [x] Implement `roko neuro restore` with confidence decay

**Spec** (doc 08 `docs/17-lifecycle/08-selective-restore.md`): `roko neuro restore <path>` imports knowledge from a backup archive with selective filtering and generational confidence decay.

**0.85^N confidence decay**: each generation hop (backup -> restore) multiplies all Engram confidence scores by 0.85. First restore: 0.85x. Second-generation restore: 0.72x. This prevents unbounded confidence inheritance.

**Quarantine/validate/adopt pipeline**:
1. **Quarantine** — imported Engrams start at `Transient` tier, isolated from active context
2. **Validate** — run imported heuristics against recent outcomes, check for contradictions with existing knowledge
3. **Adopt** — Engrams that pass validation are promoted to `Working` tier; those that fail are discarded

**Selective restore options**: filter by knowledge type (`--types insight,warning`), by domain, by minimum confidence, by age. Cross-agent restore supported (restore from a different agent's backup).

**Current code**: `crates/roko-neuro/src/knowledge_store.rs` has knowledge store with tier management and `insert_engram()`. No restore CLI command. No 0.85^N confidence decay logic. No quarantine pipeline.

**What to change**: Add `NeuroCommand::Restore { path: PathBuf, types: Option<Vec<String>>, min_confidence: Option<f64> }` to CLI. Implement `fn restore_neuro(store: &mut KnowledgeStore, archive: &Path, generation: u32, filters: &RestoreFilters) -> Result<RestoreReport>` that:
1. Reads tar.gz archive, parses manifest
2. Applies 0.85^generation confidence decay to all Engrams
3. Filters by type/domain/confidence as requested
4. Inserts at Transient tier (quarantine)
5. Returns `RestoreReport` with counts (imported, filtered, quarantined)

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs` — `KnowledgeStore`, `insert_engram()`, tier management
- `crates/roko-neuro/src/lib.rs:64` — tier constants, `CAUSAL_LINK_HALF_LIFE_DAYS`
- `crates/roko-cli/src/main.rs` — CLI command registration
- `docs/17-lifecycle/08-selective-restore.md` — full spec: 0.85^N decay, quarantine/validate/adopt, filter options, cross-agent restore
**Depends on**: LIFE-04 (backup format must exist to restore from)
**Accept when**:
- [x] `roko neuro restore <path>` CLI command exists
- [x] 0.85^N confidence decay applied (N = generation count from --generation flag)
- [x] Restored Engrams start at Transient tier (quarantine)
- [x] Filter options: `--types`, `--min-confidence`
- [x] RestoreReport printed with import/filter/quarantine counts
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'neuro.*restore\|confidence_decay\|fn restore' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P1

---

### LIFE-06: Agent deletion

- [x] Implement `roko agent delete` with clean shutdown

**Spec** (doc 06 `docs/17-lifecycle/06-agent-deletion.md`): `roko delete [--force]` performs an 8-step clean shutdown with 30s per-step budget. Steps:
1. **Stop processing** — cancel current task, drain message queue
2. **Flush pending** — complete in-flight tool calls, write pending Engrams
3. **Backup knowledge** — auto-invoke `roko neuro backup` to `.roko/backups/<timestamp>/`
4. **Deregister from mesh** — notify peers, remove from collective
5. **Release resources** — free compute slots, close connections
6. **Archive signals** — compress `.roko/signals.jsonl` and `.roko/episodes.jsonl`
7. **Clean state** — remove `.roko/state/executor.json` and transient files
8. **Confirm** — emit deletion event, write `DELETED` marker

`--force` skips ordered shutdown for immediate termination (kills process, no backup). Each step has a 30s timeout — if a step stalls, it's skipped and the next step proceeds.

**Current code**: No `roko delete` command in CLI. `crates/roko-cli/src/daemon.rs` has `DaemonState` enum and `daemon_start()`/stop logic but no agent deletion flow. `crates/roko-runtime/src/process.rs` has `ProcessSupervisor` for agent shutdown signals.

**What to change**: Add `roko delete [--force]` command to CLI. Implement 8-step shutdown function with per-step 30s timeout using `tokio::time::timeout`. Auto-invoke LIFE-04 backup before step 7. Wire `ProcessSupervisor::shutdown()` for step 1.

**Reference files**:
- `crates/roko-cli/src/daemon.rs` — daemon lifecycle, `DaemonState` enum (pattern reference)
- `crates/roko-agent/src/lifecycle.rs` — manifest, provisioning stages
- `crates/roko-runtime/src/process.rs` — `ProcessSupervisor` for agent shutdown signaling
- `docs/17-lifecycle/06-agent-deletion.md` — full 8-step spec with per-step timeout and force option
**Depends on**: LIFE-04 (knowledge backup for auto-backup step 3)
**Accept when**:
- [x] `roko agent delete` command exists in CLI
- [x] Performs ordered 8-step shutdown
- [x] Each step has 30s timeout (stalled steps are skipped)
- [x] Knowledge automatically backed up before cleanup (step 3)
- [x] `--force` skips ordered shutdown for immediate termination
- [x] Deletion event emitted for audit trail (DELETED marker)
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'delete\|Delete\|shutdown.*agent' crates/roko-cli/src/ --include='*.rs' | head -20
cargo test --workspace
```
**Priority**: P1

---

### LIFE-07: Configuration hot-reload

- [x] Implement file watching for STRATEGY.md changes

**Spec** (doc 03 `docs/17-lifecycle/03-configuration-and-operator-model.md`): Four config files with different hot-reload semantics:
- `roko.toml` — **partial hot-reload**: `[budget]`, `[tools.profile]`, `[heartbeat]` sections reload; `[agent]`, `[inference]` require restart
- `STRATEGY.md` — **full hot-reload**: changes reflected immediately (strategy goals, tactics, risk bounds)
- `PLAYBOOK.md` — read-only to operator, written by agent (Dream integration)
- `hermes.yaml` — read-only to operator, written by agent (Mesh topology)

Operator freedom hierarchy (5 levels): (1) suggest via STRATEGY.md, (2) constrain via budget/tools, (3) override via CLI flags, (4) intervene via dashboard, (5) kill via `roko delete`.

**Current code**:
- `crates/roko-core/src/config/hot_reload.rs` — `ConfigSection` enum with `is_hot_reloadable()`, `config_diff()`, `apply_hot_reload()`, `parse_strategy_md()`, `StrategyDocument` type. Fully classifies sections as hot-reloadable (`[budget]`, `[tools]`, `[learning]`, `[gates]`, `[conductor]`, `[routing]`) vs restart-required (`[agent]`, `[providers]`, `[models]`, `[serve]`).
- `crates/roko-serve/src/config_watcher.rs` — Background polling watcher that detects `roko.toml` and `STRATEGY.md` changes every 2s with debouncing. Calls `reload_config_from_disk()` and `reload_strategy_from_disk()`.
- `crates/roko-serve/src/routes/config.rs` — `reload_config_from_disk()` uses `hot_reload::config_diff()` + `apply_hot_reload()`, emits `ConfigReloaded` event. `reload_strategy_from_disk()` parses STRATEGY.md and emits `StrategyReloaded` event.
- `crates/roko-serve/src/events.rs` — `ConfigReloaded` and `StrategyReloaded` server event variants.
- Config watcher started automatically in both `ServerBuilder::start()` and `run_server_with_state()`.

**Reference files**:
- `crates/roko-core/src/config/schema.rs:42` — `RokoConfig` struct, `load_config()` function
- `crates/roko-serve/src/fswatcher.rs` — existing `notify`-based file watcher (reuse for config watching)
- `crates/roko-cli/src/orchestrate.rs` — config consumer (needs `Arc<RwLock<RokoConfig>>` instead of owned config)
- `docs/17-lifecycle/03-configuration-and-operator-model.md` — full spec: 4 config files, hot-reload semantics, operator freedom hierarchy
**Depends on**: None
**Accept when**:
- [x] File watcher detects `STRATEGY.md` and `roko.toml` changes
- [x] Hot-reloadable fields updated in running agent without restart
- [x] Non-hot-reloadable fields log a warning suggesting restart
- [x] Config-changed event emitted for dashboard
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'hot_reload\|config_reload\|watch.*config\|fswatcher' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2

---

### LIFE-08: Successor agent creation

- [x] Implement Clean, Same Strategy, and Lineage successor patterns

**Spec** (doc 07 `docs/17-lifecycle/07-new-agent-creation.md`): Three successor patterns:
1. **Clean** — new agent inherits nothing; fresh Neuro, fresh Daimon, new ID. Like a factory reset.
2. **Same Strategy** — new agent inherits the parent's `STRATEGY.md`, `AgentCoreManifest.prompt`, and tool profile, but gets fresh Neuro (no knowledge transfer). Preserves operational intent without accumulated biases.
3. **Full Lineage** — new agent inherits everything transferable: runs `roko neuro backup` on parent, creates new agent, runs `roko neuro restore` on new agent with 0.85^generation confidence decay. Maximum knowledge continuity.

**Anti-proletarianization** (Stiegler 2010): after restore, the successor's knowledge must diverge by >= 0.15 (measured via cosine distance of HDC fingerprints) within the first `exploration_boost_duration` iterations. If it doesn't, it means the agent is merely replaying inherited patterns without learning — a failure mode.

**Current code** (`crates/roko-agent/src/lifecycle.rs`):
- `AgentExtendedManifest` at line 121 with `lineage_id: Option<String>` (line 148), `generation: u32` (line 149)
- `SuccessorConfig` at line 382 with `initial_exploration_boost: f64 = 0.2`, `exploration_boost_duration: u64 = 100`
- `resolve_manifest()` at line 399: when `generation > 0`, auto-applies `SuccessorConfig::default()` (line 418-420)
- `validate_manifest()` at line 439 validates field constraints
**No `create_successor()` function. No `SuccessorMode` enum. No CLI `--successor` flag. No divergence measurement.**

**What to change**:
1. Define `SuccessorMode` enum in `crates/roko-agent/src/lifecycle.rs`:
   ```rust
   pub enum SuccessorMode {
       Clean,          // inherits nothing
       SameStrategy,   // inherits prompt + tool profile only
       FullLineage,    // inherits everything via backup/restore
   }
   ```
2. Add `fn create_successor(parent: &AgentExtendedManifest, mode: SuccessorMode) -> Result<AgentExtendedManifest>` that:
   - Generates new agent ID
   - Sets `lineage_id = parent.lineage_id.or(Some(parent_id))`
   - Sets `generation = parent.generation + 1`
   - Copies prompt/profile for SameStrategy, triggers backup/restore for FullLineage
   - Applies `SuccessorConfig::default()` (exploration boost)
3. Add `roko agent create --successor <parent_id> --mode <clean|strategy|lineage>` CLI variant
4. For FullLineage: auto-invoke LIFE-04 backup and LIFE-05 restore with generation-aware 0.85^N decay

**Reference files**:
- `crates/roko-agent/src/lifecycle.rs:121` — `AgentExtendedManifest` with `lineage_id` at 148, `generation` at 149, `SuccessorConfig` at 382, `resolve_manifest()` at 399
- `crates/roko-cli/src/main.rs` — CLI command registration (add --successor flag)
- `docs/17-lifecycle/07-new-agent-creation.md` — full spec: 3 successor patterns, anti-proletarianization, exploration boost
**Depends on**: LIFE-01 (agent creation), LIFE-04/05 (knowledge backup/restore for lineage transfer)
**Accept when**:
- [x] `SuccessorMode` enum with Clean/SameStrategy/FullLineage
- [x] `create_successor()` function produces new manifest with incremented generation
- [ ] `roko agent create --successor <id>` CLI command works
- [x] SameStrategy copies prompt and tool profile
- [x] FullLineage triggers backup/restore with 0.85^N decay
- [x] `SuccessorConfig` exploration boost applied (0.2 boost for 100 iterations)
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'SuccessorMode\|create_successor\|SuccessorConfig\|lineage_id' crates/roko-agent/src/lifecycle.rs
grep -rn 'successor' crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

### LIFE-09: Knowledge transfer via Agent Mesh

- [x] Implement live knowledge sharing between running agents

**Spec** (doc 09 `docs/17-lifecycle/09-knowledge-transfer-via-mesh.md`): While backup/restore handles knowledge transfer between deleted agents and successors, the Agent Mesh enables **live knowledge sharing between running agents**. Three sharing modes:
1. **Collective sync** — bidirectional delta sync between agents in the same Collective
2. **P2P Engram sharing** — direct agent-to-agent knowledge transfer
3. **Public knowledge feeds** — subscribe to curated Engram streams from other Collectives

**Protocol**: version-vector-based delta sync (Lamport 1978, Fidge 1988). Each agent maintains a `VersionVector = HashMap<String, u64>` tracking highest sequence number received from each peer. `SyncDelta` contains only unseen Engrams. `SharedEngram` wraps a `BackupEngram` with sequence number and attestation.

**Bloom filter discovery**: before requesting full Engram content, agents exchange Bloom filters to discover novel knowledge without redundant transfers.

**Daimon-driven sharing thresholds**: high arousal -> lower threshold (share more), high dominance -> higher threshold (share selectively). Behavioral state modulates: Struggling shares more, Focused shares less.

Config: `[mesh.sharing]` with `share_types`, `min_share_confidence`, `received_confidence_discount = 0.7`, `max_received_per_hour = 100`, `sync_interval_secs = 300`.

**Current code**: No mesh knowledge sharing implementation exists. The mesh concept is referenced in config schemas but not wired. No `SyncDelta`, `SharedEngram`, or `VersionVector` types.

**What to change**: Implement mesh sharing protocol in `crates/roko-neuro/src/` or a new mesh module. Define `SyncDelta`, `SharedEngram`, `VersionVector` types. Implement delta sync loop. Wire Daimon PAD state into sharing threshold computation.

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs` — knowledge store (source and destination of shared Engrams)
- `crates/roko-agent/src/lifecycle.rs` — `MeshConfig` field in `AgentExtendedManifest`
- `crates/roko-core/src/config/schema.rs` — `[mesh]` config section
- `docs/17-lifecycle/09-knowledge-transfer-via-mesh.md` — full spec: version-vector sync, Bloom filter, Daimon-driven thresholds, sharing protocol
**Depends on**: LIFE-01 (agent creation), BEAT-06 (CorticalState PAD fields for threshold modulation)
**Accept when**:
- [x] `SyncDelta`, `SharedEngram`, `VersionVector` types defined (lifecycle.rs)
- [x] Delta sync loop sends only unseen Engrams (via `roko neuro sync`)
- [x] Version-vector-based discovery reduces redundant transfers
- [x] Received Engrams discounted by `received_confidence_discount` (0.7x)
- [x] Rate limited to `max_received_per_hour` (configurable via --max-send)
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'SyncDelta\|SharedEngram\|VersionVector\|mesh.*sharing' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2

---

### LIFE-10: Knowledge demurrage — periodic runner

- [x] Implement periodic demurrage cycle runner and wire into heartbeat

**Spec** (doc 11 `docs/17-lifecycle/11-knowledge-demurrage.md`): Knowledge demurrage applies Gesell's Freigeld principle to knowledge. Two levels:
1. **Knowledge-level demurrage** — periodic confidence reduction on un-validated Engrams via `DemurrageConfig`:
   - `validation_interval: u64 = 250` (iterations between checks, ~2.9h at 1 iter/40s)
   - `decay_per_interval: f64 = 0.03` (3% confidence loss per missed interval)
   - `archive_threshold: f64 = 0.1` (below this, Engram archived to cold storage)
   - `domain_multipliers: HashMap<String, f64>` (volatile domains decay faster: gas_patterns=2.0x, price_direction=1.5x, protocol_behavior=0.5x)
2. **Token-level demurrage** — 1% annual demurrage on KORAI tokens (chain domain only, mainnet)

The demurrage cycle runs every `validation_interval` iterations: scan all Engrams, reduce confidence by `decay_per_interval * domain_multiplier` for those not re-validated, archive Engrams below `archive_threshold`. This incentivizes circulation — use knowledge or lose it.

**Current code** (`crates/roko-agent/src/lifecycle.rs:1428`):
- `DemurrageConfig` struct at line 1430 with `validation_interval: u64`, `decay_per_interval: f64`, `archive_threshold: f64`, `domain_multipliers: HashMap<String, f64>` — **matches spec exactly**
- `Default` impl at line 1441 sets `validation_interval=250`, `decay_per_interval=0.03`, `archive_threshold=0.1`, domain multipliers: gas_patterns=2.0, price_direction=1.5, volatility_regime=1.0, yield_trends=0.8, protocol_behavior=0.5
- `DemurrageReport` at line 1461 with `entries_processed`, `entries_archived`, `total_confidence_lost`, `average_confidence_after`
- `apply_demurrage()` function at line 1473 applies decay to a single `BackupEngram`
- `AgentExtendedManifest.demurrage: Option<DemurrageConfig>` at line 210-211 (initialized to `Some(DemurrageConfig::default())`)
**Config struct, report struct, and per-Engram decay function exist. Missing: `DemurrageCycle` runner that periodically scans `KnowledgeStore` and applies `apply_demurrage()` to all Engrams. Not wired into Theta/Delta heartbeat loop.**

**What to change**:
1. Add `DemurrageCycle` struct in `crates/roko-neuro/src/` or `crates/roko-runtime/src/` that:
   - Tracks iteration count
   - Every `validation_interval` iterations, calls `apply_demurrage()` on all un-validated Engrams
   - Archives Engrams below `archive_threshold` to cold storage
   - Returns `DemurrageReport` for observability
2. Wire `DemurrageCycle` as a Theta or Delta heartbeat consumer (BEAT-01 or BEAT-02)
3. Emit demurrage events to `.roko/learn/efficiency.jsonl`

**Reference files**:
- `crates/roko-agent/src/lifecycle.rs:1430` — `DemurrageConfig` (fully defined), `DemurrageReport` at 1461, `apply_demurrage()` at 1473
- `crates/roko-neuro/src/knowledge_store.rs` — `KnowledgeStore`, tier management, Engram iteration
- `crates/roko-neuro/src/lib.rs:64` — existing tier half-life constants (Ebbinghaus, complementary to demurrage)
- `docs/17-lifecycle/11-knowledge-demurrage.md` — full spec: demurrage cycle algorithm, domain multipliers, archiving
**Depends on**: None (config + function exist; only the runner + wiring needed)
**Accept when**:
- [x] `DemurrageConfig` struct defined with all specified fields (exists at lifecycle.rs:1430)
- [x] `apply_demurrage()` function applies per-Engram decay (exists at lifecycle.rs:1473)
- [x] `DemurrageCycle` runner periodically scans `KnowledgeStore`
- [x] Engrams below `archive_threshold` moved to cold storage
- [x] Demurrage events emitted for observability
- [x] Wired into Theta/Delta heartbeat consumer
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'DemurrageConfig\|DemurrageCycle\|apply_demurrage\|DemurrageReport' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2

---

## Verify

```bash
cargo test --workspace
```
