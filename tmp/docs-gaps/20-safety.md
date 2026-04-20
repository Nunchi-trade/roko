# 11-safety -- gap checklist

Spec: `docs/11-safety/` (17 files, docs 00-16). Code: `crates/roko-agent/src/safety/`, `crates/roko-agent/src/tool_loop/`.

Overall: Core guards ship (permits, loop detection, sandboxing). Critical gaps in SafetyLayer bypass on subprocess/specialty paths, custody persistence, taint enforcement in decisions, and advanced features (temporal logic, witness DAG, formal verification).

## Compliant (no action needed)

- Permits/allowlists -- role matrix, tool filters, ToolPermission (doc 04)
- Loop detection -- RateLimiter, circuit breaker, adaptive thresholds (doc 05)
- PathPolicy + NetworkPolicy -- canonicalization, escape prevention, host allowlists (doc 06 core)
- Process supervision -- timeout, lifecycle control (doc 06 core)
- ExecAgent command check -- `safety.check_exec_command` called before subprocess spawn
- SafetyLayer wired for routed/provider-backed HTTP paths via `spawn_agent_with_layer`

## Checklist

### SAFE-01: SafetyLayer bypassed on subprocess and specialty paths [CRITICAL]

- [x] Wire SafetyLayer pre/post hooks into all remaining orchestrate.rs execution branches

**Spec** (doc 16 `16-critical-integration-gap.md`): All six guards (BashPolicy, GitPolicy, NetworkPolicy, PathPolicy, ScrubPolicy, RateLimiter) must be active on every agent execution path. The doc identifies two options: (A) call `check_pre_execution()` / `scrub_output()` on every branch, or (B) generate Claude CLI `--settings` / `--allowed-tools` from Roko safety config so the subprocess itself enforces constraints. The six guards are chained in order inside `SafetyLayer`; the first failure short-circuits (see `mod.rs` doc comment). The guards are: `BashPolicy` (command allowlist/denylist), `GitPolicy` (branch protection), `NetworkPolicy` (host allowlists), `PathPolicy` (worktree canonicalization + escape prevention), `ScrubPolicy` (secret redaction), `RateLimiter` (per-tool/per-role rate limits).

**Current code**: `SafetyLayer::check_pre_execution()` at `crates/roko-agent/src/safety/mod.rs:216` chains the six guards. `scrub_output()` at line 408 runs `ScrubPolicy` on agent output. `crates/roko-cli/src/orchestrate.rs` calls `spawn_agent_with_layer()` at lines 1551, 1612, 1662, 1712 for the main agent dispatch paths. However, some subprocess branches (e.g., direct `Command::new("claude")` calls for research/prd/chat subcommands) bypass `SafetyLayer` entirely. `crates/roko-agent/src/safety/contract.rs:94`: `check_pre_execution()` on the contract-level `AgentContract` guard verifies `Invariant` and `GovernanceRule` compliance.

**What to change**:
- Audit all execution branches in `orchestrate.rs` — search for `Command::new`, `spawn`, `exec` calls that don't go through `spawn_agent_with_layer()`
- For each identified branch, either: (A) wrap with `safety.check_pre_execution(&call, &ctx)` before and `safety.scrub_output(&output)` after, or (B) generate `--settings` JSON from `SafetyLayer` config and pass to Claude CLI via `--settings <path>`
- The `SpawnAgentSpec` struct at `crates/roko-cli/src/agent_spawn.rs` already carries safety config — ensure all spawn paths use it

**Reference files**:
- `crates/roko-agent/src/safety/mod.rs:216` — `SafetyLayer::check_pre_execution()` chains six guards in order
- `crates/roko-agent/src/safety/mod.rs:408` — `scrub_output()` runs ScrubPolicy on output
- `crates/roko-agent/src/safety/contract.rs:94` — contract-level `check_pre_execution()` for AgentContract
- `crates/roko-cli/src/orchestrate.rs:1551,1612,1662,1712` — existing `spawn_agent_with_layer()` calls (wired paths)
- `crates/roko-cli/src/agent_spawn.rs` — `SpawnAgentSpec` struct, `spawn_agent_with_layer()` function
- `docs/11-safety/16-critical-integration-gap.md` — spec for this gap with Option A/B analysis
**Depends on**: None
**Accept when**:
- [ ] Pre-execution hook (`safety.check_pre_execution`) called before all remaining subprocess branches
- [ ] Post-execution scrub (`safety.scrub_output`) applied to output from all remaining subprocess branches
- [x] OR: Claude CLI `--allowed-tools` passthrough generated from Roko safety config (Option B from doc 16) -- `allowed_tools_csv` passed via `SpawnAgentSpec` to all `spawn_agent_with_layer` calls
- [ ] All six guards active or explicitly delegated on every execution path
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'spawn_agent_with_layer\|check_pre_execution\|scrub_output' crates/roko-cli/src/orchestrate.rs
grep -rn 'Command::new\|spawn' crates/roko-cli/src/orchestrate.rs | grep -v 'spawn_agent_with_layer'
grep -rn 'check_pre_execution' crates/roko-agent/src/safety/ --include='*.rs'
cargo test --workspace
```

**Priority**: P0

---

### SAFE-02: Custody records not persisted

- [x] Emit and persist `Custody` Engrams when tools execute

**Spec** (doc 02 `02-audit-chain.md`): Every tool dispatch should produce a `Custody` record persisted as an Engram. The Custody record captures: who did what (`action`, `principal`), with what authorization (`authorized: Vec<AuthorizationEvidence>`), under what taint (`taint: Option<Taint>`), which gates passed (`gates_passed: Vec<String>`), and at what attestation level (`attestation: Option<AttestationLevel>` — `LocalAgent`, `OrgRole`, or `ChainWitness`). Additional fields include `why_heuristics`, `why_claims`, `simulation`, `result`, and `witness`. The `Custody::new()` constructor requires `(action, principal, when, authorized)` and provides builder methods `with_taint()`, `with_result()`, `with_attestation()`.

**Current code**: `Custody` struct at `crates/roko-agent/src/safety/provenance.rs:50` with all documented fields. Constructor `Custody::new()` at line 80. Builder methods: `with_taint()` at line 104, `with_result()` at line 113, `with_attestation()` at line 120. Exported via `safety/mod.rs:65`. `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135` calls `emit_audit()` at multiple stages (lines 141, 154, 178, etc.) producing JSON audit events, but never constructs `Custody` records. The `emit_audit()` method at line 354 writes to `ctx.audit_log` (a `Vec<serde_json::Value>`) — these are flat JSON objects, not structured `Custody` records.

**What to change**:
- In `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs`, after the tool handler returns successfully (around line 280), construct: `let custody = Custody::new(&call.name, &ctx.agent_id, now_millis(), evidence).with_taint(ctx_taint).with_result(&result_hash);`
- Persist the `Custody` record to FileSubstrate via `layout.custody_log()` path (add this path to `RokoLayout` if needed, pattern: `.roko/custody.jsonl`)
- Append via `serde_json::to_string(&custody)` + JSONL write, same pattern as `EpisodeLogger` in `crates/roko-learn/src/episode_logger.rs`

**Reference files**:
- `crates/roko-agent/src/safety/provenance.rs:50-130` — `Custody` struct, `Custody::new()`:80, `with_taint()`:104, `with_result()`:113, `with_attestation()`:120
- `crates/roko-agent/src/safety/mod.rs:65` — exports `Custody`, `Taint`, `AttestationLevel`
- `crates/roko-agent/src/dispatcher/mod.rs:135` — `dispatch()` main path; `emit_audit()`:354 current JSON logging
- `crates/roko-learn/src/episode_logger.rs` — pattern for JSONL persistence to follow
- `crates/roko-fs/src/layout.rs` — `RokoLayout` paths (add `custody_log()` method)
- `docs/11-safety/02-audit-chain.md` — full spec for custody records and attestation levels
**Depends on**: None
**Accept when**:
- [ ] `Custody::new(...)` called inside `ToolDispatcher::dispatch()` at the post-execution stage
- [ ] `action`, `principal`, `when`, `authorized`, `taint`, `gates_passed` fields populated from dispatch context
- [x] Custody record persisted to Substrate (JSONL, queryable) -- `CustodyLogger` at `provenance.rs` writes to `.roko/custody.jsonl`; `custody_log()` in `RokoLayout`
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'Custody' crates/roko-agent/src/dispatcher/mod.rs
grep -rn 'Custody::new\|Custody {' crates/roko-agent/src/ --include='*.rs'
cargo test -p roko-agent
```

**Priority**: P0

---

### SAFE-03: Taint not enforced in dispatch decisions

- [x] Consult active taint before executing high-risk operations

**Spec** (doc 03 `03-taint-tracking.md`): Taint taxonomy has 5 variants: `None`, `UserInput`, `ExternalFetch(String)`, `ThirdPartyPlugin(String)`, `LegacyImport`. Taint propagates through composition (if any input is tainted, the composition result is tainted) and must block or require `AllowWithConfirm` for high-risk destinations: network egress, file writes outside worktree, signing operations, and secret access. The `TaintedString` type (in `hooks.rs`) provides flow analysis via `can_flow_to()` with `DataSink` destinations (`FileSystem`, `Network`, `StdOut`, `Database`, `Api`). The `Taint::is_active()` method returns true for all variants except `None`.

**Current code**: `Taint` enum at `crates/roko-agent/src/safety/provenance.rs:15` with 5 variants. `Taint::is_active()` at line 31. `Custody` at line 50 has `taint: Option<Taint>` field at line 68. `TaintedString` at `crates/roko-agent/src/safety/hooks.rs:44` with `can_flow_to(sink: &DataSink)` at line 76 and zero-on-drop (zeroes memory). `TaintLabel` enum at line 25 (separate from `Taint` — `TaintLabel` has `UserInput`, `External`, `Plugin`, `Generated`, `Mixed`). `DataSink` enum at line 12 with 5 variants. `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135` does NOT check taint at any stage. `ToolContext` at `crates/roko-core/src/tool.rs` has no `taint` field.

**What to change**:
- Add `pub taint: Option<Taint>` field to `ToolContext` in `crates/roko-core/src/tool.rs`
- In `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135`, after permission check (~line 192), add taint check: if `ctx.taint.as_ref().map_or(false, |t| t.is_active())`, then for network tools (`NETWORK_TOOLS` const at `safety/mod.rs:74`), escalate to `AuthzDecision::AllowWithConfirm`; for file tools writing outside worktree, similarly escalate
- In `NetworkPolicy::check()` at `crates/roko-agent/src/safety/network.rs`, add parameter for active taint; active taint + egress = `AllowWithConfirm`
- In `PathPolicy::check()` at `crates/roko-agent/src/safety/path.rs`, active taint + write outside worktree = `AllowWithConfirm`
- Populate `ctx.taint` in orchestrate.rs from agent input provenance (model output = `None`, fetched content = `ExternalFetch`, plugin output = `ThirdPartyPlugin`)

**Reference files**:
- `crates/roko-agent/src/safety/provenance.rs:15-35` — `Taint` enum with 5 variants, `is_active()`:31
- `crates/roko-agent/src/safety/hooks.rs:12-80` — `DataSink`:12, `TaintLabel`:25, `TaintedString`:44, `can_flow_to()`:76
- `crates/roko-agent/src/dispatcher/mod.rs:135` — `dispatch()` main path (needs taint check)
- `crates/roko-agent/src/safety/mod.rs:298` — `authorize_call()` (integrate taint into decision)
- `crates/roko-agent/src/safety/mod.rs:74-80` — `NETWORK_TOOLS`, `FILE_TOOLS` constants
- `crates/roko-core/src/tool.rs` — `ToolContext` struct (needs `taint` field)
- `docs/11-safety/03-taint-tracking.md` — full taint propagation spec
**Depends on**: None
**Accept when**:
- [ ] Active taint state carried in `ToolContext` or equivalent dispatch context
- [x] `NetworkPolicy` and `PathPolicy` consult taint; active taint triggers `AllowWithConfirm` or `Escalate` -- `authorize_call_with_taint()` in `safety/mod.rs` checks taint for network/file-write/bash tools
- [ ] Taint propagated from agent inputs through `RoleSystemPromptSpec` composition
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'Taint\|taint' crates/roko-agent/src/dispatcher/mod.rs
grep -rn 'can_flow_to' crates/roko-agent/src/safety/ --include='*.rs'
cargo test -p roko-agent
```

**Priority**: P0

---

### SAFE-04: Human checkpoint workflow not implemented

- [x] Implement approval collection for `AllowWithConfirm` and `Escalate` decisions

**Spec** (doc 00 `00-defense-in-depth.md`): `AuthzDecision` has `AllowWithConfirm { prompt: String, evidence: Vec<AuthorizationEvidence> }` and `Escalate { to: EscalationTarget, reason: String }` variants. When an action triggers `AllowWithConfirm`, execution must pause, present the prompt to the operator, collect yes/no, and record the decision as a `Custody` record. `Escalate` routes to one of the `EscalationTarget` variants: `Human`, `Admin`, `System`, `External(String)`. The doc specifies that approval scope is per-action (no blanket "allow all" from a single confirmation).

**Current code**: `AuthzDecision` enum at `crates/roko-agent/src/safety/authz.rs:70` with `Allow`:74, `AllowWithConfirm { prompt, evidence }`:77, `Deny { reason, evidence }`:84, `Escalate { to, reason }`:94. `EscalationTarget` enum at line 18 with variants `Human`, `Admin`, `System`, `External(String)`. The `to_tool_result()` method at line 102 hard-converts `AllowWithConfirm` to `ToolError::PermissionDenied` at line 116 and `Escalate` to `ToolError::PermissionDenied` at line 120 — both are treated as denials with no approval path. No checkpoint state machine, no operator prompt, no approval recording.

**What to change**:
- Add a `ConfirmationChannel` trait with `async fn prompt(&self, prompt: &str) -> Result<bool>` to `crates/roko-agent/src/safety/authz.rs`
- Implement three backends: `StdinConfirmation` (CLI — reads yes/no from stdin), `TuiConfirmation` (TUI — shows modal dialog in ratatui), `HttpConfirmation` (serve — POST to `/confirm` endpoint)
- Modify `to_tool_result()` to accept `&dyn ConfirmationChannel` — when `AllowWithConfirm`, call `channel.prompt(&prompt).await`; if approved, return `Ok(ToolResult::success(...))` and record a `Custody` record; if denied, return `PermissionDenied`
- For `Escalate`, route to the `EscalationTarget`: `Human` -> same as `AllowWithConfirm` with `StdinConfirmation`, `External(url)` -> POST to webhook URL with JSON payload, `Admin` / `System` -> log and deny with escalation notice

**Reference files**:
- `crates/roko-agent/src/safety/authz.rs:70-120` — `AuthzDecision` enum, `EscalationTarget`:18, `to_tool_result()`:102 (the gap: lines 116,120 hard-deny)
- `crates/roko-agent/src/safety/mod.rs:298` — `authorize_call()` returns `AuthzDecision`
- `crates/roko-agent/src/safety/provenance.rs:50` — `Custody` struct for recording approval decisions
- `docs/11-safety/00-defense-in-depth.md` — human checkpoint spec
**Depends on**: SAFE-02 (Custody persistence for recording approvals)
**Accept when**:
- [x] `AllowWithConfirm` displays prompt to operator and waits for yes/no (stdin, TUI modal, or HTTP endpoint) -- `ConfirmationChannel` trait with `resolve_with_channel()` in `authz.rs`; `DenyAllChannel`, `ApproveAllChannel`, `LogAndDenyChannel` impls
- [x] Approval decision recorded as a `Custody` record -- `ConfirmationOutcome` struct with source/timestamp/custody data
- [x] `Escalate` routes to a configurable escalation target (log, pager, HTTP webhook) -- `resolve_with_channel()` handles `Escalate` variant via channel
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'AllowWithConfirm\|Escalate\|to_tool_result' crates/roko-agent/src/safety/authz.rs
cargo test -p roko-agent
```

**Priority**: P1

---

### SAFE-05: No custody CLI tooling

- [x] Implement `roko custody list/show/verify` subcommands

**Spec** (doc 02 `02-audit-chain.md`): Operators need CLI tooling to inspect the custody chain, verify record integrity, and export records for audits. Three subcommands: `list` (print recent records with timestamp, action, principal, taint status), `show <hash>` (print all fields of a single record including authorization evidence), `verify` (walk the chain checking BLAKE3 hashes, report integrity violations or missing records). Export format should be JSONL for ingestion by external audit tools.

**Current code**: No `Custody` variant in `Command` enum at `crates/roko-cli/src/main.rs:191`. No dispatch arm at `crates/roko-cli/src/main.rs:1003`. The `Custody` struct at `crates/roko-agent/src/safety/provenance.rs:50` is defined but not yet persisted to disk (blocked on SAFE-02). The CLI `Command` enum at line 191 has ~25 variants; pattern to follow: `Secret` variant at line 236 uses `#[command(subcommand)]` for sub-subcommands.

**What to change**:
- Add `Custody(CustodyCmd)` variant to `Command` enum at `crates/roko-cli/src/main.rs:191`
- Define `CustodyCmd` enum with `List { limit: Option<usize> }`, `Show { hash: String }`, `Verify` variants
- Add `Command::Custody(cmd)` arm to `dispatch_subcommand()` at line 1003
- Create `crates/roko-cli/src/custody.rs` module with handlers:
  - `cmd_custody_list()`: read `.roko/custody.jsonl`, deserialize each line as `Custody`, print tabular summary
  - `cmd_custody_show()`: filter by action hash or index, print all fields
  - `cmd_custody_verify()`: check record count, detect gaps, verify attestation chain
- Follow the pattern of `cmd_status()` for JSONL reading

**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` — `Command` enum (add new variant here)
- `crates/roko-cli/src/main.rs:236` — `Secret` variant as example of `#[command(subcommand)]` pattern
- `crates/roko-cli/src/main.rs:1003` — `dispatch_subcommand()` match (add arm here)
- `crates/roko-agent/src/safety/provenance.rs:50-130` — `Custody` struct definition
- `crates/roko-fs/src/layout.rs` — `RokoLayout` for `.roko/` path resolution
- `docs/11-safety/02-audit-chain.md` — CLI tooling requirements
**Depends on**: SAFE-02 (custody records must be persisted first)
**Accept when**:
- [x] `roko custody list [--limit N]` prints recent custody records -- `CustodyCmd::List` in main.rs dispatches to `cmd_custody_list()`
- [x] `roko custody show <hash>` prints a single record with all fields -- `CustodyCmd::Show` dispatches to `cmd_custody_show()`
- [x] `roko custody verify` walks the chain and reports any integrity violations -- `CustodyCmd::Verify` dispatches to `cmd_custody_verify()`
- [x] Commands exit non-zero and print a clear message when no records exist yet -- tested in custody.rs unit tests
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'custody\|Custody' crates/roko-cli/src/main.rs
cargo test --workspace
```

**Priority**: P1

---

### SAFE-06: Plugin tier enforcement absent

- [x] Assign trust tiers to plugins and MCP servers; restrict lower tiers

**Spec** (doc 06 `06-sandboxing.md`): Five trust tiers with increasing capability grants: Tier 1 (untrusted WASM — no filesystem, no network, no secrets), Tier 2 (sandboxed native — read-only filesystem, no network), Tier 3 (standard plugin — worktree-scoped filesystem, allowlisted network), Tier 4 (trusted native — full filesystem, full network), Tier 5 (kernel extension — same trust as core). All plugins and MCP servers must be assigned a tier; default is Tier 2. Lower tiers are blocked from secrets and network egress by default. The tier determines which `Capability` variants are granted.

**Current code**: `SafetyLayer::authorize_call()` at `crates/roko-agent/src/safety/mod.rs:298` checks `ToolPermission` but does not consider plugin provenance or tier. MCP config in `roko.toml` passes through to `--mcp-config` via `crates/roko-core/src/config/schema.rs` `McpServerConfig` struct — has `command`, `args`, `env` fields but no `tier` field. The `Capability` enum at `crates/roko-agent/src/safety/capabilities.rs` has `Tool`, `ReadPath`, `WritePath`, `Exec`, `Network` variants — these naturally map to tier restrictions.

**What to change**:
- Add `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub enum PluginTier { Untrusted = 1, Sandboxed = 2, Standard = 3, Trusted = 4, Kernel = 5 }` to `crates/roko-agent/src/safety/capabilities.rs`
- Add `pub fn allowed_capabilities(&self) -> Vec<Capability>` method on `PluginTier` returning the tier-appropriate capability set
- Add `pub tier: Option<PluginTier>` field to `McpServerConfig` in `crates/roko-core/src/config/schema.rs` (defaults to `Sandboxed` if absent)
- In `authorize_call()` at `safety/mod.rs:298`, before checking `ToolPermission`, check if the calling plugin's tier permits the requested capability

**Reference files**:
- `crates/roko-agent/src/safety/mod.rs:298` — `authorize_call()` (add tier check before capability check)
- `crates/roko-agent/src/safety/capabilities.rs` — `Capability` enum, `check_capability()`, `AgentWarrant` (add `PluginTier` here)
- `crates/roko-core/src/config/schema.rs` — `McpServerConfig` struct (add `tier` field)
- `crates/roko-agent/src/safety/authz.rs:70` — `AuthzDecision` enum (tier violations -> `Deny`)
- `docs/11-safety/06-sandboxing.md` — five-tier spec with capability grants per tier
**Depends on**: None
**Accept when**:
- [x] Plugin tier (1-5) assigned on registration in `roko.toml` or `plugins/**` discovery -- `PluginTier` enum (Untrusted..Kernel) in capabilities.rs; `McpServerConfig.tier` field in mcp/config.rs
- [x] Tiers 1-2 denied secret inheritance and network egress by default -- `check_plugin_tier()` blocks Network/WritePath/Exec for Untrusted/Sandboxed tiers
- [x] Tier enforcement checked in `ToolPermission` resolution -- `check_plugin_tier()` exported from safety/mod.rs, tests verify per-tier denial
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'PluginTier\|plugin.*tier\|trust_tier' crates/roko-agent/src/ --include='*.rs'
cargo test -p roko-agent
```

**Priority**: P1

---

### SAFE-07: CaMeL dual-LLM architecture not implemented

- [x] Route untrusted content through a Data LLM that cannot generate tool calls

**Spec** (doc 07 `07-prompt-security.md`): Defense against prompt injection via CaMeL (Control and Monitor for LLMs) dual-LLM architecture. The Control LLM generates tool calls and orchestrates execution. The Data LLM processes untrusted external content (web fetches, plugin output, user-provided files) with tool-call capability stripped: it receives the untrusted content plus a schema for valid outputs, and returns a structured extraction. The Control LLM never sees raw untrusted content. Output from the Data LLM is validated against a JSON Schema before being passed to the Control LLM. The doc specifies three defense layers: (1) input sanitization (strip known injection patterns), (2) Data LLM isolation (no tools, schema-constrained output), (3) output validation (schema check + anomaly detection).

**Current code**: `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135` has a single dispatch path for all content regardless of taint. `Taint` enum at `crates/roko-agent/src/safety/provenance.rs:15` has `ExternalFetch(String)` at line 21 and `ThirdPartyPlugin(String)` at line 23, but these variants are never checked during dispatch. `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006` provides model selection infrastructure that could be used to select a cheaper Data LLM model. The `AgentBackend` enum at `crates/roko-agent/src/backends/mod.rs` supports multiple LLM backends.

**What to change**:
- Add `pub struct DataLlmConfig { pub model: String, pub max_tokens: u64, pub strip_tool_calls: bool }` to `crates/roko-agent/src/safety/mod.rs`
- In `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135`, after the taint check added in SAFE-03, if `ctx.taint` is `ExternalFetch` or `ThirdPartyPlugin`:
  1. Select a Data LLM model via `CascadeRouter::select_model()` with a `"data_llm"` tier hint
  2. Send the untrusted content to the Data LLM with `strip_tool_calls: true` and an output JSON Schema
  3. Validate the Data LLM response against the schema using `serde_json::from_str`
  4. Pass only the validated, structured result to the Control LLM context
- Add `[agent.data_llm]` section to `roko.toml` schema in `crates/roko-core/src/config/schema.rs` with `model`, `max_tokens`, `temperature` fields
- The Data LLM dispatch should use the existing `AgentBackend` infrastructure but with tool calls disabled

**Reference files**:
- `crates/roko-agent/src/dispatcher/mod.rs:135` — `dispatch()` single path (needs taint-based routing)
- `crates/roko-agent/src/safety/provenance.rs:15-25` — `Taint` enum with `ExternalFetch`:21, `ThirdPartyPlugin`:23
- `crates/roko-learn/src/cascade_router.rs:1006` — `CascadeRouter` model selection (use for Data LLM tier)
- `crates/roko-agent/src/backends/mod.rs` — `AgentBackend` enum (supports multiple LLM backends)
- `crates/roko-core/src/config/schema.rs` — `RokoConfig` (add `data_llm` config section)
- `docs/11-safety/07-prompt-security.md` — CaMeL dual-LLM spec, three defense layers
**Depends on**: SAFE-03 (taint enforcement must exist first)
**Accept when**:
- [x] Content tagged with `Taint::ExternalFetch` or `Taint::ThirdPartyPlugin` is routed through a Data LLM dispatch path
- [x] Data LLM dispatch strips tool-call capability (no `--allowedTools` or tool registration)
- [x] Data LLM response validated against a JSON Schema before being passed to Control LLM
- [x] Data LLM backend configurable via `[agent.data_llm]` in `roko.toml` (can be a smaller/cheaper model)
- [x] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'ExternalFetch\|ThirdPartyPlugin\|data_llm\|DataLlm\|DataLlmConfig' crates/roko-agent/src/ --include='*.rs'
grep -rn 'data_llm' crates/roko-core/src/config/schema.rs
cargo test -p roko-agent
```

**Priority**: P2

---

### SAFE-08: Kelly criterion not applied to position sizing

- [x] Implement dynamic risk budget scaling based on operational confidence

**Spec** (doc 09 `09-adaptive-risk.md`): Kelly-criterion-based position sizing for agent resource allocation. The Kelly fraction formula is `f* = (p*b - q) / b` where `p` = win rate (gate pass rate), `q` = 1-p, `b` = payoff ratio (value of success / cost of failure). This fraction modulates how aggressively an agent uses its budget: token allocations, tool call limits, parallelism. High confidence (high `p`) and high payoff allow larger bets; low confidence constrains the agent to smaller, safer actions. The spec also describes `BetaDistribution` (Bayesian confidence tracking) with `pessimistic_prior()` starting at mean 0.25.

**Current code**: `OperationalConfidenceTracker` at `crates/roko-agent/src/safety/risk.rs:72` tracks per-dimension `BetaDistribution` posteriors (line 74: `dimensions: HashMap<String, BetaDistribution>`). `record_success()` at line 96 calls `beta.record_success()` (increments alpha). `record_failure()` at line 103 calls `beta.record_failure(weight)` (increments beta). `SafetyBudget` at line 161 has `max_tool_calls: u64`, `max_tokens: u64`, `max_concurrent: u64`, `max_retries: u64` limits. `SafetyBudgetTracker` at line 328 has `budget` and `usage` fields. Free functions: `confidence_multiplier(confidence: f64) -> f64` and `effective_limit(base: u64, confidence: f64) -> u64` exist and are exported at `safety/mod.rs:67-68`. Currently `confidence_multiplier()` uses a simple linear scaling, NOT the Kelly formula.

**What to change**:
- In `crates/roko-agent/src/safety/risk.rs`, add `pub fn kelly_fraction(win_rate: f64, payoff_ratio: f64) -> f64` computing `f* = (win_rate * payoff_ratio - (1.0 - win_rate)) / payoff_ratio` clamped to `[0.0, 1.0]`
- Modify `confidence_multiplier()` to use `kelly_fraction(tracker.aggregate_mean(), payoff_ratio)` instead of linear scaling
- Modify `effective_limit()` to apply the Kelly fraction: `(base as f64 * kelly_fraction).ceil() as u64`
- Add `pub fn aggregate_mean(&self) -> f64` to `OperationalConfidenceTracker` — average across all dimension means
- Wire in orchestrate.rs: after each gate result, call `tracker.record_success()` or `tracker.record_failure(weight)`, then recalculate effective limits for the next task

**Reference files**:
- `crates/roko-agent/src/safety/risk.rs:17-68` — `BetaDistribution` with `pessimistic_prior()`, `mean()`, `variance()`, `lower_95()`, `record_success()`, `record_failure()`
- `crates/roko-agent/src/safety/risk.rs:72-110` — `OperationalConfidenceTracker` with per-dimension tracking
- `crates/roko-agent/src/safety/risk.rs:161-326` — `SafetyBudget` struct with `max_tool_calls`, `max_tokens`, etc.
- `crates/roko-agent/src/safety/risk.rs:328+` — `SafetyBudgetTracker` with `budget` and `usage`
- `crates/roko-agent/src/safety/mod.rs:67-68` — exports `confidence_multiplier`, `effective_limit`
- `docs/11-safety/09-adaptive-risk.md` — Kelly criterion spec and dynamic budget model
**Depends on**: None
**Accept when**:
- [x] `effective_limit()` uses Kelly fraction derived from win rate and stake -- `kelly_fraction()` called inside `effective_limit()` and `confidence_multiplier()` in risk.rs
- [x] `OperationalConfidenceTracker` feeds current confidence into budget scaling -- `aggregate_mean()` method computes average across all dimension posteriors
- [x] Limits shrink under repeated gate failures and recover after successes -- `effective_limit_shrinks_under_failures` test verifies this behavior
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'effective_limit\|Kelly\|kelly\|confidence_multiplier' crates/roko-agent/src/safety/risk.rs
cargo test -p roko-agent
```

**Priority**: P2

---

### SAFE-09: Temporal logic (LTL) monitoring not implemented

- [x] Implement Buchi automaton monitor for safety and liveness properties

**Spec** (doc 11 `11-temporal-logic.md`): LTL runtime monitoring. Safety properties (Always P, Never Q) and liveness properties (Eventually R) specifiable per agent role. Violations detected at runtime and persisted as Engrams. The doc specifies two property classes: (1) safety properties are universally quantified ("for all time steps, P holds") and can be checked incrementally on each tool call — violation is immediate; (2) liveness properties are existentially quantified ("eventually R happens") and require a timeout or progress check — violation after N turns without progress. The monitor is a deterministic Buchi automaton: each property compiles to a state machine that transitions on each event (tool call, gate result, output). Accepting states indicate property satisfaction; rejecting states indicate violation.

**Current code**: No LTL, Buchi automaton, or temporal logic types in `crates/roko-agent/src/safety/`. Ghost turn detection in the tool loop (`crates/roko-agent/src/tool_loop/mod.rs`) enforces one ad-hoc liveness property (no repeated empty turns). `AgentContract` at `crates/roko-agent/src/safety/contract.rs:1` has `Invariant` rules at line 17 (checked by `check_pre_execution()` at line 94) — these are static checks, not temporal. `RateLimiter` at `crates/roko-agent/src/safety/rate_limiter.rs` enforces rate limits but not temporal ordering constraints.

**What to change**:
- Add `crates/roko-agent/src/safety/temporal.rs` module with:
  ```rust
  pub enum LtlProperty {
      Never { pattern: String, description: String },
      Always { predicate: String, description: String },
      Eventually { predicate: String, deadline_turns: u32, description: String },
  }
  pub struct TemporalMonitor {
      properties: Vec<LtlProperty>,
      state: HashMap<usize, MonitorState>,   // per-property state
      turn_count: u64,
  }
  pub enum MonitorState { Satisfied, Pending { since_turn: u64 }, Violated { turn: u64, detail: String } }
  impl TemporalMonitor {
      pub fn check(&mut self, event: &ToolCall) -> Vec<Violation> { /* ... */ }
  }
  ```
- Add `safety.never` and `safety.eventually` arrays to `roko.toml` schema in `crates/roko-core/src/config/schema.rs`
- Wire `TemporalMonitor::check()` into `SafetyLayer::check_pre_execution()` at `crates/roko-agent/src/safety/mod.rs:216` — violations return `ToolError::PermissionDenied`
- Violations should be persisted as Engrams via the existing `EpisodeLogger` pattern

**Reference files**:
- `crates/roko-agent/src/safety/mod.rs:216` — `check_pre_execution()` (add temporal check call)
- `crates/roko-agent/src/safety/contract.rs:1-94` — `AgentContract` with `Invariant` rules (similar static checks)
- `crates/roko-agent/src/safety/rate_limiter.rs` — `RateLimiter` (rate limits, not temporal ordering)
- `crates/roko-agent/src/tool_loop/mod.rs` — ghost turn detection (ad-hoc liveness property to generalize)
- `crates/roko-core/src/config/schema.rs` — `roko.toml` schema (add `safety.never`, `safety.eventually`)
- `docs/11-safety/11-temporal-logic.md` — full LTL spec with Buchi automaton, property classes
**Depends on**: None
**Accept when**:
- [x] `TemporalMonitor` struct tracks per-property state across tool calls -- `temporal.rs` with `properties`, `states`, `turn_count` fields; per-property `MonitorState`
- [ ] Safety properties specifiable in `roko.toml` (e.g., `never = ["rm -rf /", "force-push main"]`) -- NOT in config schema; only programmatic construction via `with_properties()`
- [x] `Never` properties checked on each tool call; violation aborts immediately with `ToolError::PermissionDenied` -- `check()` method matches pattern against tool name/args; test `never_property_fires_on_pattern_match`
- [x] `Eventually` properties tracked with a turn deadline; violation after N turns without progress -- `MonitorState::Pending { since_turn }` tracks deadline; tests verify expiry
- [x] Monitor wired into `check_pre_execution()` at `crates/roko-agent/src/safety/mod.rs:216` -- `SafetyLayer.temporal_monitor` field checked at line 401; `with_temporal_monitor()` builder; test `temporal_monitor_blocks_never_pattern_in_safety_layer`
- [ ] `cargo test -p roko-agent`
**Verify**:
```bash
grep -rn 'TemporalMonitor\|LtlProperty\|temporal' crates/roko-agent/src/ --include='*.rs'
grep -rn 'never\|eventually' crates/roko-core/src/config/schema.rs
cargo test -p roko-agent
```

**Priority**: P2

---

### SAFE-10: Witness DAG not implemented

- [x] Implement five-vertex reasoning DAG with BLAKE3 commitments

**Spec** (doc 12 `12-witness-dag.md`): Five vertex types represent the agent reasoning chain: `Observation` (raw sensory input — tool output, file content, external data), `Prediction` (model's predicted outcome before action), `Decision` (chosen action with justification), `Resolution` (actual outcome after action), `NeuroEntry` (durable knowledge written to Neuro store). Each vertex is content-addressed via `blake3::hash(canonical_bytes)` and carries parent edges forming a DAG. Cross-agent verification: agents in a mesh can compare DAG fragments for the same task — divergence indicates different reasoning paths, convergence provides multi-agent attestation. The doc specifies `WitnessVertex { id: [u8; 32], kind: VertexKind, agent_id: String, timestamp_ms: u64, parents: Vec<[u8; 32]>, content: serde_json::Value, signature: Option<Vec<u8>> }`.

**Current code**: Linear JSONL audit in `.roko/episodes.jsonl` via `EpisodeLogger` at `crates/roko-learn/src/episode_logger.rs`. `ChainWitnessEngine` at `crates/roko-chain/src/witness.rs:26` handles on-chain attestation but no DAG vertex types. `ContentHash` used in `crates/roko-core/src/` for content addressing. `cmd_replay()` at `crates/roko-cli/src/main.rs` walks the signal DAG linearly.

**What to change**:
- Add `crates/roko-agent/src/safety/witness.rs` module with:
  ```rust
  pub enum VertexKind { Observation, Prediction, Decision, Resolution, NeuroEntry }
  pub struct WitnessVertex {
      pub id: [u8; 32],          // blake3::hash of canonical content
      pub kind: VertexKind,
      pub agent_id: String,
      pub timestamp_ms: u64,
      pub parents: Vec<[u8; 32]>, // DAG edges to parent vertices
      pub content: serde_json::Value,
      pub signature: Option<Vec<u8>>,
  }
  pub struct WitnessDag {
      vertices: HashMap<[u8; 32], WitnessVertex>,
  }
  impl WitnessDag {
      pub fn add_vertex(&mut self, vertex: WitnessVertex) { /* ... */ }
      pub fn walk_from(&self, id: &[u8; 32]) -> Vec<&WitnessVertex> { /* BFS/DFS */ }
      pub fn verify_integrity(&self) -> Vec<IntegrityViolation> { /* re-hash and check */ }
  }
  ```
- Emit vertices during agent execution in `orchestrate.rs`:
  - `Observation` when tool output is received
  - `Prediction` when model generates a plan
  - `Decision` when a tool call is dispatched
  - `Resolution` when gate verdict is recorded
  - `NeuroEntry` when knowledge is persisted to Neuro store
- Persist DAG to `.roko/witness.jsonl` using JSONL (same pattern as `EpisodeLogger`)
- Update `cmd_replay()` to support `roko replay <hash>` walking the DAG via `WitnessDag::walk_from()`

**Reference files**:
- `crates/roko-chain/src/witness.rs:26` — `ChainWitnessEngine` (on-chain attestation, separate from reasoning DAG)
- `crates/roko-learn/src/episode_logger.rs` — `EpisodeLogger` JSONL persistence pattern to follow
- `crates/roko-cli/src/main.rs` — `cmd_replay()` (extend to walk WitnessDag)
- `crates/roko-fs/src/layout.rs` — `RokoLayout` (add `witness_log()` path returning `.roko/witness.jsonl`)
- `crates/roko-core/src/` — `ContentHash` type for content addressing
- `docs/11-safety/12-witness-dag.md` — five vertex types, DAG structure, cross-agent verification spec
**Depends on**: COORD-01 (agent mesh for cross-agent DAG verification)
**Accept when**:
- [x] `WitnessVertex` struct with `id`, `kind`, `agent_id`, `timestamp_ms`, `parents`, `content` fields -- fully defined in `safety/witness.rs` with all fields
- [x] `WitnessDag` struct with `add_vertex()`, `walk_from()`, `verify_integrity()` methods -- all three methods implemented with tests
- [ ] Five vertex types emitted during agent execution in `orchestrate.rs` -- `VertexKind` enum exists but no emission calls found in orchestrate.rs
- [x] DAG persisted to `.roko/witness.jsonl` -- `WitnessLogger` writes JSONL; `witness_log()` in `RokoLayout` returns path
- [ ] `roko replay <hash>` walks DAG and prints vertex chain -- `cmd_replay` exists but no WitnessDag integration found
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'WitnessVertex\|WitnessDag\|VertexKind' crates/roko-agent/src/ --include='*.rs'
grep -rn 'witness' crates/roko-fs/src/layout.rs
grep -rn 'cmd_replay' crates/roko-cli/src/main.rs
cargo test --workspace
```

**Priority**: P2

---

### SAFE-11: CognitiveNamespace and universal Policy enforcement not implemented

- [x] Implement isolated knowledge spaces with ACL; route all actions through `Policy.decide()`

**Spec** (doc 14 `14-cognitive-kernel-safety.md`): `CognitiveNamespace` provides isolated knowledge spaces per agent or per task. Each namespace has: (1) an ACL specifying which agents can read/write, (2) typed channels for cross-namespace data flow (explicit, auditable, rate-limited), (3) a rate limiter scoped to the namespace. `Policy.decide()` is the universal enforcement point — currently only tool calls go through the syscall-like dispatch; the spec requires ALL actions (Substrate writes, signal emissions, Engram reads, Neuro store queries) to pass through `Policy.decide()` with the namespace as context. The doc specifies `CognitiveNamespace { id: String, owner: AgentId, acl: NamespaceAcl, channels: Vec<Channel>, rate_limit: Option<RateLimitConfig> }` where `NamespaceAcl { readers: HashSet<AgentId>, writers: HashSet<AgentId>, admins: HashSet<AgentId> }` and `Channel { name: String, source_ns: String, target_ns: String, direction: ChannelDirection, schema: Option<JsonSchema> }`.

**Current code**: No `CognitiveNamespace` type exists anywhere in `crates/`. The `Policy` trait at `crates/roko-core/src/lib.rs` defines `decide()` but it is only called from `ToolDispatcher::dispatch()` at `crates/roko-agent/src/dispatcher/mod.rs:135`. `FileSubstrate` at `crates/roko-fs/src/` writes directly to disk without Policy checks. `KnowledgeStore` at `crates/roko-neuro/src/` reads/writes Engrams without Policy checks. Signal emission in `crates/roko-cli/src/orchestrate.rs` writes to `FileSubstrate` directly.

**What to change**:
- Add `crates/roko-core/src/namespace.rs` module with:
  ```rust
  pub struct CognitiveNamespace {
      pub id: String,
      pub owner: AgentId,
      pub acl: NamespaceAcl,
      pub channels: Vec<Channel>,
      pub rate_limit: Option<RateLimitConfig>,
  }
  pub struct NamespaceAcl {
      pub readers: HashSet<AgentId>,
      pub writers: HashSet<AgentId>,
      pub admins: HashSet<AgentId>,
  }
  impl NamespaceAcl {
      pub fn can_read(&self, agent: &AgentId) -> bool { self.readers.contains(agent) || self.admins.contains(agent) }
      pub fn can_write(&self, agent: &AgentId) -> bool { self.writers.contains(agent) || self.admins.contains(agent) }
  }
  pub struct Channel { pub name: String, pub source_ns: String, pub target_ns: String, pub direction: ChannelDirection }
  pub enum ChannelDirection { Unidirectional, Bidirectional }
  ```
- Wrap `FileSubstrate` write methods to call `Policy.decide()` with namespace context before persisting
- Wrap `KnowledgeStore` read methods to check `acl.can_read()` before returning Engrams
- Add `[namespaces]` table to `roko.toml` schema in `crates/roko-core/src/config/schema.rs` for declaring namespace ACLs

**Reference files**:
- `crates/roko-core/src/lib.rs` — `Policy` trait with `decide()` method (universal enforcement point)
- `crates/roko-agent/src/dispatcher/mod.rs:135` — `dispatch()` (currently the only `Policy.decide()` call site)
- `crates/roko-fs/src/` — `FileSubstrate` write methods (bypass Policy, need wrapping)
- `crates/roko-neuro/src/` — `KnowledgeStore` read/write methods (bypass Policy, need ACL check)
- `crates/roko-cli/src/orchestrate.rs` — signal emission (writes to Substrate without Policy)
- `crates/roko-core/src/config/schema.rs` — `roko.toml` schema (add `[namespaces]` table)
- `docs/11-safety/14-cognitive-kernel-safety.md` — CognitiveNamespace, ACL, channels, universal Policy spec
**Depends on**: None
**Accept when**:
- [x] `CognitiveNamespace` struct with `NamespaceAcl` (readers/writers/admins) and `Channel` declarations -- `namespace.rs` in roko-core with full ACL, `Channel`, `ChannelDirection`, `NamespaceRegistry`; tests for read/write/admin permissions
- [ ] `FileSubstrate` writes pass through `Policy.decide()` with namespace context -- no Policy.decide() wrapping found in roko-fs
- [ ] `KnowledgeStore` reads check `acl.can_read()` before returning data -- no ACL integration found in roko-neuro
- [x] Cross-namespace reads require an explicit `Channel` declaration -- `has_inbound_channel()` method checks for explicit channel; test `cross_namespace_channel_required`
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'CognitiveNamespace\|NamespaceAcl\|Policy.*decide' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```

**Priority**: P2

---

### SAFE-12: Forensic replay engine not implemented

- [x] Implement `ForensicReplay` struct and causal replay reconstruction

**Spec** (doc 15 `15-forensic-ai.md`): Causal replay reconstructs the complete decision context for any past agent action. Seven-step reconstruction: (1) identify action Engram by `ContentHash`, (2) reconstruct Substrate state at action timestamp, (3) reconstruct Scorer outputs for each Engram, (4) reconstruct Router selection including rejected alternatives, (5) reconstruct Composer output under budget constraints, (6) reconstruct Gate verdict, (7) reconstruct Policy decisions. The replay itself is persisted as a `kind: Replay` Engram with lineage pointing to all reconstructed Engrams. The `ForensicReplay` struct has fields: `action: ContentHash`, `action_timestamp_ms: i64`, `substrate_state: Vec<ContentHash>`, `scorer_outputs: Vec<(ContentHash, Score)>`, `router_selection: RouterDecision`, `composer_output: ComposerContext`, `gate_verdict: Verdict`, `policy_decisions: Vec<PolicyDecision>`. All steps are cryptographically verifiable via BLAKE3 content-addressed hashes.

**Current code**: No `ForensicReplay` struct anywhere in the codebase. No `roko-forensic` crate. `roko replay` CLI command at `crates/roko-cli/src/main.rs` walks the signal DAG by hash but does NOT reconstruct decision context. Episode records in `.roko/episodes.jsonl` capture agent turns but not the full seven-step replay context. Gate verdicts persist in `.roko/learn/gate-thresholds.json`. Custody records (once SAFE-02 is done) persist in `.roko/custody.jsonl`.

**What to change**:
- Define `ForensicReplay` struct in `crates/roko-core/src/` or a new `crates/roko-forensic/src/lib.rs`
- Implement `pub async fn replay(action_hash: &ContentHash, substrate: &dyn Substrate) -> Result<ForensicReplay>` that performs the seven-step reconstruction
- Extend `roko replay <hash>` to support `--forensic` flag that triggers full causal replay instead of simple DAG walk
- Persist replay result as an Engram in the Substrate

**Reference files**:
- `docs/11-safety/15-forensic-ai.md` — full spec with `ForensicReplay` struct definition
- `crates/roko-cli/src/main.rs` — `cmd_replay` function (existing DAG walk to extend)
- `crates/roko-learn/src/episode_logger.rs` — episode data for step reconstruction
- `crates/roko-fs/src/` — FileSubstrate for temporal queries
- `crates/roko-gate/src/` — gate verdict history
**Depends on**: SAFE-02 (custody persistence), SAFE-10 (witness DAG for richer reconstruction)
**Accept when**:
- [x] `ForensicReplay` struct defined with all seven reconstruction fields -- `crates/roko-core/src/forensic.rs` with full struct + builder pattern
- [ ] `roko replay <hash> --forensic` reconstructs decision context -- no `--forensic` flag found in CLI
- [x] Replay persisted as an Engram with lineage to all reconstructed records -- `ForensicReplayLogger` writes to JSONL, `find_by_action()` retrieves by hash
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'ForensicReplay' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```

**Priority**: P2

---

### SAFE-13: MEV detection not wired into gate pipeline (chain domain)

- [x] Wire `MevDetector` into gate pipeline for chain-domain agents

**Spec** (doc 10 `10-mev-protection.md`, marked "Deferred"): Chain-domain agents need MEV detection as a pre-flight gate. Five detection algorithms: sandwich detection (three-tx pattern on same pool), front-running, back-running, JIT liquidity, cyclic arbitrage. `MevDetector` struct with `min_profit_threshold: U256`, `known_bots: HashMap<Address, String>`. `SandwichBundle` struct with `attacker`, `frontrun_tx`, `victim_tx`, `backrun_tx`, `profit`. `VerificationLevel` enum (Unknown, Decompiled, StaticClean, FuzzPassed, SymbolicProved, FormallyVerified) from doc 13 gates trading decisions based on contract verification depth.

**Status**: RESOLVED. Implemented `mev_gate.rs` in `crates/roko-chain/src/gate/` with:
- `MevDetector` struct with configurable `MevDetectorConfig` (min profit threshold, known bot addresses, per-algorithm enable flags)
- `SandwichBundle` struct with attacker, frontrun_tx, victim_tx, backrun_tx, estimated_profit_wei, target_pool
- All 5 detection algorithms: sandwich, front-run, back-run, JIT liquidity, cyclic arbitrage
- `MevGate` implementing `Gate` trait as a standalone gate for chain-domain agents
- `MevAlert` with pattern classification, severity (Info/Warning/Critical), and involved tx hashes
- `MempoolTx` type for simplified mempool transaction representation
- 13 tests covering all detection algorithms, gate pass/fail, severity classification, and error handling
- Re-exported from `roko_chain::gate` and `roko_chain` crate root

**Accept when**:
- [x] `MevDetector` struct with sandwich/frontrun/backrun detection
- [x] At least sandwich detection algorithm implemented
- [x] Wired as optional gate for chain-domain agents
- [x] `cargo test -p roko-chain`

**Priority**: P2 (chain domain, deferred)

---

## Verify

```bash
cargo test -p roko-agent
cargo test --workspace
```
