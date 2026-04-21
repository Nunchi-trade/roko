# 18-tools -- Gap checklist

Spec: `docs/18-tools/` (18 files). Code: `crates/roko-core/src/tool/`, `crates/roko-std/`, `crates/roko-agent/src/mcp/`, `crates/roko-mcp-*/`.

Overall: ~11% compliant. Built-in tools (16) and MCP architecture ship. MCP servers are scaffolds. Chain domain, plugin SDK, safety hook chain, and tool profiles are all unbuilt.

## Compliant (no action needed)

- 16 built-in tools fully implemented (doc 01)
- MCP integration architecture -- JSON-RPC, tool discovery, config passthrough (doc 09)
- ToolDef pattern with permissions and risk tiers (doc 00 partial)
- Capability<T> token system exists (doc 04 partial)

## Checklist

### TOOL-01: MCP server implementations

- [x] Implement at least GitHub and Slack MCP servers

**Spec** (docs 10-13):
- **roko-mcp-github** (doc 10 `docs/18-tools/10-mcp-github.md`): 17 tools across 4 groups: 6 PR tools (get/create/review/merge/update/list), 6 issue tools (get/create/update/list/comment/search), 4 repo tools (get_file/search_code/list_branches/get_tree), 1 CI tool (get_check_status). Full JSON Schema for each. Rate limiting per endpoint.
- **roko-mcp-slack** (doc 11 `docs/18-tools/11-mcp-slack.md`): 8 tools: post_message, update_message, reply_thread, add_reaction, upload_file, get_channel_history, get_thread, lookup_user. Socket Mode + HTTP Mode. Rate limits per method. Block Kit support.
- **roko-mcp-scripts** (doc 12 `docs/18-tools/12-mcp-scripts.md`): Config-driven tool wrappers via `scripts.toml`. 6 collaboration scripts, 5 knowledge-base scripts. Executor with timeout/isolation.
- **roko-mcp-stdio** (doc 13 `docs/18-tools/13-mcp-stdio.md`): Scaffold crate: `McpToolHandler` trait, `McpServerBuilder`, JSON-RPC protocol handler, tool registry, error codes.

**Current code**:
- `crates/roko-mcp-github/src/main.rs`: ~2400 LOC, 19 tools implemented (all functional), rate limiting with exponential backoff.
- `crates/roko-mcp-slack/src/main.rs`: 9 functional tools: post_message, reply, get_thread, react, list_channels, lookup_user, dm, get_channel_history, update_message.
- `crates/roko-mcp-scripts/src/main.rs`: Functional script runner with directory scanning, description parsing, timeout/isolation, env allowlist.
- `crates/roko-mcp-stdio/src/lib.rs`: JSON-RPC stdio transport library used by all MCP servers.
- `crates/roko-mcp-code/src/lib.rs`: Code-intelligence MCP server (working reference implementation).

**What to change**:
1. Audit roko-mcp-github: verify which tools exist, add missing ones (especially search_code, list_branches, get_tree)
2. Implement core Slack tools in roko-mcp-slack: post_message, get_channel_history, reply_thread (minimum viable)
3. Implement scripts.toml parsing in roko-mcp-scripts with at least 2 script definitions
4. Ensure all servers follow JSON-RPC 2.0 over stdio (MCP protocol)

**Reference files**:
- `crates/roko-mcp-github/src/main.rs` — existing implementation (~1100 LOC, audit for completeness)
- `crates/roko-mcp-slack/src/main.rs` — scaffold (implement core tools)
- `crates/roko-mcp-scripts/src/main.rs` — scaffold (implement scripts.toml parser)
- `crates/roko-mcp-stdio/src/lib.rs` — stdio transport library
- `crates/roko-mcp-code/src/lib.rs` — working MCP server (pattern reference for GitHub/Slack)
- `docs/18-tools/10-mcp-github.md` — full spec: 17 tools with JSON Schema
- `docs/18-tools/11-mcp-slack.md` — full spec: 8 tools, Socket Mode, Block Kit
**Depends on**: None
**Accept when**:
- [x] roko-mcp-github has at least 10 of 17 specified tools functional
- [x] roko-mcp-slack has at least 3 functional tools (post_message, get_channel_history, reply_thread)
- [x] roko-mcp-scripts parses scripts.toml and exposes at least 2 tools
- [x] All servers respond to JSON-RPC `tools/list` request
- [x] `cargo test -p roko-mcp-github` passes
**Verify**:
```bash
grep -rn 'fn handle_\|"tools/list"\|tool_name' crates/roko-mcp-github/src/main.rs | head -20
grep -rn 'fn handle_\|post_message' crates/roko-mcp-slack/src/main.rs | head -10
cargo test -p roko-mcp-github
```
**Priority**: P1

---

### TOOL-02: Safety hook chain completion

- [x] Implement HallucinationDetector and ResultFilter hooks

**Spec** (doc 04 `docs/18-tools/04-safety-hooks.md`): The safety system uses a 7-hook sequential chain. Each hook implements `SafetyHook` and can block, modify, or pass through tool calls. The chain runs in order:
1. **PolicyCage** — role-based authorization (does this agent's role permit this tool?)
2. **AllowlistGuard** — tool-level allowlist/denylist filtering
3. **SpendingLimiter** — cost budget check (per-turn and daily limits)
4. **RateLimiter** — per-tool rate limiting (requests/minute)
5. **RevmSimulator** — EVM simulation for chain tools (skip for non-chain)
6. **HallucinationDetector** — validate tool call plausibility (e.g., file path exists, API endpoint valid, parameter ranges sane)
7. **ResultFilter** — sanitize tool output (strip secrets, truncate large responses, label tainted strings)

Also specified: **Capability<T> 8-step flow** for compile-time safety:
1. Tool requests capability → 2. Hook chain validates → 3. Capability token issued → 4. Tool executes with token → 5. Result captured → 6. Result filtered → 7. Audit logged → 8. Token consumed.

`TaintedString` type wraps untrusted data with `TaintLabel` (UserInput, ExternalApi, FileContent, WebContent). All external data enters as tainted; sanitization removes taint.

**Current code** (`crates/roko-agent/src/safety/`):
- `hooks.rs:192` — `SafetyHook` trait defined, `TaintLabel` and `TaintedString` types
- `mod.rs` — `SafetyLayer` struct with wiring
- `authz.rs` — authorization checks (partial PolicyCage)
- `capabilities.rs:12` — `Capability` enum, `AgentWarrant`
- `rate_limit.rs:80` — `RateLimiter` implementation
- `risk.rs` — risk assessment
- `scrub.rs` — output scrubbing (partial ResultFilter)
- `contract.rs` — contract safety (partial RevmSimulator)
**Missing**: `HallucinationDetector` struct, complete `ResultFilter` struct, hooks not chained sequentially in a pipeline. `AllowlistGuard` and `SpendingLimiter` not implemented as `SafetyHook` impls.

**What to change**:
1. Implement `HallucinationDetector` struct in `crates/roko-agent/src/safety/hallucination.rs` — checks: file path exists, API endpoint resolves, parameter ranges sane, tool name matches registry
2. Implement `ResultFilter` struct in `crates/roko-agent/src/safety/result_filter.rs` — strips secrets (regex patterns for API keys, tokens), truncates large responses (>100KB), wraps external data in `TaintedString`
3. Wrap existing `authz.rs`, `rate_limit.rs`, `scrub.rs` as `SafetyHook` implementations
4. Wire all 7 hooks into a `Vec<Box<dyn SafetyHook>>` sequential chain in `ToolDispatcher`

**Reference files**:
- `crates/roko-agent/src/safety/hooks.rs:192` — `SafetyHook` trait, `TaintLabel`, `TaintedString`
- `crates/roko-agent/src/safety/` — all files (authz, capabilities, rate_limit, risk, scrub, contract)
- `crates/roko-agent/src/dispatcher/mod.rs` — `ToolDispatcher` (integration point for hook chain)
- `docs/18-tools/04-safety-hooks.md` — full spec: 7-hook chain, Capability<T> flow, TaintedString, audit trail
**Depends on**: None
**Accept when**:
- [x] `HallucinationDetector` validates tool call plausibility (file exists, params sane)
- [x] `ResultFilter` sanitizes output (strip secrets, truncate, taint external data)
- [x] All 7 hooks implement `SafetyHook` trait
- [x] Hooks chained sequentially in `ToolDispatcher` as `Vec<Box<dyn SafetyHook>>`
- [x] Hook chain runs before every tool invocation
- [x] Audit trail logged for each hook decision
- [x] `cargo test -p roko-agent` passes — tokio tests in hook_chain.rs validate full chain with HallucinationDetector+ResultFilter, short-circuit on reject, and secret detection
**Verify**:
```bash
grep -rn 'HallucinationDetector\|ResultFilter\|hook_chain\|SafetyHook' crates/roko-agent/src/ --include='*.rs'
cargo test -p roko-agent
```
**Priority**: P1

---

### TOOL-03: Tool profiles for domain bundling

- [x] Implement domain-specific tool profiles alongside existing role profiles

**Spec** (doc 05 `docs/18-tools/05-tool-profiles.md`): Domain profile bundles activate subsets of tools based on the agent's domain (Coding, Chain, Research). 13 chain reference profiles with distinct tool sets. Profile composition: `effective_profile = role_profile ∩ domain_profile ∩ config_overrides`. Configuration hierarchy: CLI flags > env vars > config > defaults.

**Current code** (`crates/roko-std/src/roles.rs:21`):
- `RoleToolProfileKind` enum with 5 role types: Implementer, Researcher, Reviewer, Strategist, Scribe
- `RoleToolProfile` struct at line 36 with `kind: RoleToolProfileKind`, `allowed_tools: Option<&'static [&'static str]>`, `denied_tools: &'static [&'static str]`
- 5 role-specific profile constants: `IMPLEMENTER_TOOL_PROFILE` (allow all, line 120), `RESEARCHER_TOOL_PROFILE` (read-only, blocks write/edit/bash, line 124), `REVIEWER_TOOL_PROFILE` (read + comment, blocks write/edit, line 131), `STRATEGIST_TOOL_PROFILE` (read + plan mgmt, blocks destructive, line 138), `SCRIBE_TOOL_PROFILE` (read + write, blocks exec, line 159)
- `ROLE_TOOL_PROFILES` array at line 163 collecting all 5
- `DomainPlugin` enum at `crates/roko-agent/src/lifecycle.rs:54` with Chain, Coding, Research, Custom variants

**Role-based profiles exist and work. Domain-based profiles (what extra tools a Chain vs Coding agent gets) do not exist. No profile composition (intersection of role + domain). No config-driven profile loading.**

**What to change**:
1. Add `DomainToolProfile` struct in `crates/roko-std/src/roles.rs`:
   ```rust
   pub struct DomainToolProfile {
       pub domain: DomainPlugin,
       pub extra_tools: &'static [&'static str],  // tools added for this domain
       pub excluded_tools: &'static [&'static str], // tools removed for this domain
   }
   ```
2. Define domain profiles: `CODING_DOMAIN_PROFILE` (add all 16 builtins), `CHAIN_DOMAIN_PROFILE` (add chain tools, DeFi tools), `RESEARCH_DOMAIN_PROFILE` (add web_search, web_fetch, research tools)
3. Add `fn compose_profile(role: &RoleToolProfile, domain: &DomainToolProfile, overrides: &ToolOverrides) -> EffectiveProfile` that computes the intersection
4. Add `[tools.profile]` config section parsing in `crates/roko-core/src/config/schema.rs` for custom allow/deny overrides
5. Wire profile composition into tool registration in `crates/roko-agent/src/dispatcher/mod.rs`

**Reference files**:
- `crates/roko-std/src/roles.rs:21` — `RoleToolProfileKind`, `RoleToolProfile` at 36, 5 role constants at 120-169
- `crates/roko-std/src/tool/builtin/mod.rs` — 16 builtin tools (tool names referenced by profiles)
- `crates/roko-agent/src/lifecycle.rs:54` — `DomainPlugin` enum (Chain, Coding, Research, Custom)
- `crates/roko-agent/src/dispatcher/mod.rs` — tool registration (wire profile filtering here)
- `docs/18-tools/05-tool-profiles.md` — full spec: 13 chain profiles, profile composition, config hierarchy
**Depends on**: None
**Accept when**:
- [x] `DomainToolProfile` struct defined alongside `RoleToolProfile`
- [x] At least 3 domain profiles: Coding, Chain, Research
- [x] `compose_profile()` computes role ∩ domain ∩ overrides
- [x] `[tools.profile]` config section parsed from roko.toml
- [ ] Profile filtering applied during tool registration in dispatcher — `compose_profile` not called from roko-agent dispatcher
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'DomainToolProfile\|compose_profile\|CODING_DOMAIN\|CHAIN_DOMAIN' crates/roko-std/src/ --include='*.rs'
grep -rn 'tools.*profile' crates/roko-core/src/config/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1

---

### TOOL-04: Plugin SDK with 5-tier SPI

- [x] Implement TOML-based manifest loading and `roko plugin install` CLI

**Spec** (doc 14): Five tiers: prompts, profile bundles, declarative tools, native traits, WASM extensions. Manifest-first loading from TOML files.

**Current code** (`crates/roko-plugin/src/lib.rs`):
- `PluginManifest` struct at line 617 with `name: String`, `version: String`, `event_sources: Vec<Box<dyn EventSource>>`, `feedback_collectors: Vec<Box<dyn FeedbackCollector>>`
- `PluginBuilder` at line 629 with fluent API: `.new(name)`, `.event_source(src)`, `.feedback_collector(col)`, `.build()`
- `EventSource` trait at line 133 with `fn poll()`, `fn kind()`. Concrete impls: `CronEventSource` (line 146, with cron scheduling), `FileWatchEventSource` (line 80, with notify-based watching)
- `EventSourceKind` enum at line 67 (Cron, FileWatch, WebhookPush, GitCommit, ExternalApi)
- `FeedbackCollector` trait at line 395 with `fn collect()`
**TOML manifest loading implemented in `crates/roko-plugin/src/manifest.rs`.** `PluginManifestFile` struct with `PluginMeta`, `PromptTemplate` (T1), `ToolProfileBundle` (T2), `DeclarativeTool` (T3), `TriggerDef`, `PluginDependency`. `load_manifest()`, `parse_manifest()`, `discover_plugins()`, `validate_manifest()` all functional. `roko plugin install|list|audit` CLI commands wired in `crates/roko-cli/src/main.rs`.

**What to change**:
1. Define TOML plugin manifest format (name, version, tiers, dependencies)
2. Implement `PluginLoader` that reads TOML manifest and constructs `PluginManifest`
3. Add `roko plugin install <path>` CLI command
4. Add Tier 1 (prompts) — register prompt templates from manifest
5. Add Tier 2 (profiles) — register tool profile bundles from manifest
**Reference files**:
- `crates/roko-plugin/src/lib.rs:617` — `PluginManifest` struct, `PluginBuilder` at 629, `EventSource` trait at 133, `CronEventSource` at 146, `FileWatchEventSource` at 80
- `crates/roko-serve/Cargo.toml` (depends on roko-plugin)
- `crates/roko-cli/Cargo.toml` (depends on roko-plugin)
- `docs/18-tools/14-plugin-sdk.md` — full 5-tier SPI spec
- `docs/18-tools/16-plugin-loading.md` — manifest-first loading spec
**Depends on**: TOOL-03 (tool profiles for Tier 2)
**Accept when**:
- [x] TOML plugin manifest format defined and parseable
- [x] `roko plugin install <path>` reads manifest and registers plugin
- [x] Tier 1 (prompts): prompt templates loadable from manifest
- [x] Tier 2 (profiles): tool profile bundles loadable from manifest
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'PluginManifest\|PluginBuilder\|PluginLoader' crates/roko-plugin/src/ --include='*.rs'
grep -rn 'plugin.*install\|PluginCommand' crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1

---

### TOOL-05: Chain domain tools

- [x] Implement core DeFi tool subset using Alloy

**Spec** (doc 03 `docs/18-tools/03-chain-domain-tools.md`): 423+ DeFi tools organized as one domain plugin. Two-layer model:
- **Layer 1 (Chain primitives)**: RPC calls via Alloy — `eth_getBalance`, `eth_sendTransaction`, `eth_call`, `eth_getLogs`, `eth_getBlockByNumber`. These are domain-agnostic EVM operations.
- **Layer 2 (Protocol adapters)**: Protocol-specific ABI calls — Uniswap (swap, add/remove liquidity, get pool info), Aave (supply, borrow, repay, get health factor), ERC-20 (approve, transfer, balanceOf).

10 core tools minimum:
1. `balance` — `eth_getBalance` for native token, `balanceOf()` for ERC-20
2. `transfer` — `eth_sendTransaction` for native, ERC-20 `transfer()`
3. `approve` — ERC-20 `approve()` for spending allowance
4. `swap` — Uniswap V3 `exactInputSingle()` or V2 `swapExactTokensForTokens()`
5. `add_liquidity` — Uniswap V3 `mint()` or V2 `addLiquidity()`
6. `remove_liquidity` — Uniswap V3 `decreaseLiquidity()` + `collect()`
7. `get_pool_info` — query pool state (reserves, fee, tick, liquidity)
8. `get_position` — query LP position details
9. `simulate_tx` — dry-run via Revm/mirage-rs fork without broadcasting
10. `gas_estimate` — `eth_estimateGas` with buffer

**Current code** (`crates/roko-chain/src/`): Extensive type stubs across `phase2.rs` (~2700 LOC), `identity_economy_*.rs` files. Alloy dependency exists in workspace `Cargo.toml` (alloy-core, alloy-primitives, alloy-sol-types). `crates/roko-chain/Cargo.toml` depends on `alloy-primitives`. No DeFi tool implementations — all chain code is type definitions and Phase 2 stubs, not functional tools.

**What to change**:
1. Create `crates/roko-chain/src/tools/mod.rs` module for chain tools
2. Implement each tool as a struct implementing the `ToolDef` pattern (see `crates/roko-std/src/tool/builtin/` for reference)
3. Use `alloy` provider for RPC calls (connect to configured RPC URL from `roko.toml`)
4. Add `simulate_tx` using Revm or mirage-rs for dry-run simulation
5. Register chain tools in `CHAIN_DOMAIN_PROFILE` (see TOOL-03)

**Reference files**:
- `crates/roko-chain/src/` — type stubs, `phase2.rs`, alloy primitives
- `crates/roko-std/src/tool/builtin/` — reference for tool implementation pattern (NAME, DESCRIPTION, handler fn)
- `crates/roko-core/src/tool/` — `ToolDef` type, `ToolContext`, `ToolResult`
- `crates/roko-std/src/tool/builtin/mod.rs` — `StaticToolRegistry` with `TOOL_COUNT = 16` (chain tools extend this)
- `docs/18-tools/03-chain-domain-tools.md` — full spec: 423+ tools, two-layer model, profile-adapter mapping
**Depends on**: None (Alloy dependency already in workspace)
**Accept when**:
- [x] `crates/roko-chain/src/tools/` module exists with at least 10 tool implementations
- [x] Tools use Alloy provider for EVM interaction
- [x] `balance`, `transfer`, `approve` work for ERC-20 tokens
- [x] `simulate_tx` performs dry-run via Revm/mirage-rs
- [x] Tools registered as `ToolDef` structs
- [x] `cargo test -p roko-chain` passes
**Verify**:
```bash
ls crates/roko-chain/src/tools/ 2>/dev/null
grep -rn 'fn swap\|fn approve\|fn balance\|fn transfer\|fn simulate_tx' crates/roko-chain/src/ --include='*.rs'
grep -rn 'ToolDef\|impl.*Tool' crates/roko-chain/src/ --include='*.rs' | grep -v target/
cargo test -p roko-chain
```
**Priority**: P2 (Tier 6 chain work)

---

### TOOL-06: Agent templates

- [x] Codify strategy templates for common agent patterns

**Spec** (doc 15-16 `docs/18-tools/15-16-agent-templates.md`): 18 agent templates organized in 3 categories:
- **6 collaboration templates**: doc-lifecycle (meeting notes → PRD), digest (channel summarization), meeting (live transcription), sync (status report), conflict-detector (goal conflicts), freshness (stale doc detection)
- **5 knowledge-base templates**: pm-board (Kanban tracking), enrich (PR context enrichment), triage (issue classification), pm-health (project health dashboard), action-tracker (meeting action items)
- **7 roko templates**: pr-review (automated code review), slack-notify (deployment notifications), auto-plan (PRD → plan generation), code-implementer (task execution), gate-fixer (failed gate remediation), prd-ingestion (external PRD import), review-response (PR feedback response)

Each template bundles: system prompt, tool profile (role + domain), trigger configuration (cron/watch/webhook), config presets (budget, model, MCP servers).

**Current code**: `crates/roko-compose/src/templates/` has 9 role-based prompt template files (implementer, researcher, reviewer, strategist, scribe, orchestrator, base, dev, system_prompt_builder). `crates/roko-std/src/roles.rs` has 5 role tool profiles. `crates/roko-agent/src/lifecycle.rs` has `AgentCoreManifest` with `prompt`, `mode`, `domain` fields. **No unified `AgentTemplate` struct combining prompt + tools + config + triggers.**

**What to change**:
1. Define `AgentTemplate` struct in `crates/roko-agent/src/` or `crates/roko-compose/src/`:
   ```rust
   pub struct AgentTemplate {
       pub name: &'static str,
       pub description: &'static str,
       pub system_prompt: &'static str,
       pub role: RoleToolProfileKind,
       pub domain: DomainPlugin,
       pub trigger: Option<TriggerConfig>,
       pub budget_preset: BudgetConfig,
       pub mcp_servers: Vec<String>,
   }
   ```
2. Create at least 6 template constants covering the most useful patterns: `PR_REVIEW`, `CODE_IMPLEMENTER`, `AUTO_PLAN`, `GATE_FIXER`, `DOC_LIFECYCLE`, `SLACK_NOTIFY`
3. Wire templates into `roko agent create --template <name>` (depends on LIFE-01)
4. Add `roko templates list` CLI command to discover available templates

**Reference files**:
- `crates/roko-compose/src/templates/` — existing prompt templates (9 files, role-based)
- `crates/roko-std/src/roles.rs` — role tool profiles (compose with templates)
- `crates/roko-agent/src/lifecycle.rs` — `AgentCoreManifest`, `AgentExtendedManifest`
- `docs/18-tools/15-16-agent-templates.md` — full spec: 18 templates with system prompts, triggers, subscription configs
**Depends on**: LIFE-01 (agent creation CLI), TOOL-03 (tool profiles)
**Accept when**:
- [x] `AgentTemplate` struct defined with prompt, role, domain, trigger, budget fields
- [x] At least 6 templates defined as constants
- [ ] `roko agent create --template <name>` uses template to populate manifest — not found in CLI
- [ ] `roko templates list` shows available templates — not found in CLI (only HTTP API via roko-serve)
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'AgentTemplate\|PR_REVIEW\|CODE_IMPLEMENTER\|GATE_FIXER' crates/ --include='*.rs' | grep -v target/
grep -rn 'templates.*list' crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

### TOOL-07: Tool testing framework

- [x] Implement property-based and LLM evaluation tests for tools

**Spec** (doc 07 `docs/18-tools/07-tool-testing.md`): Four testing layers:
1. **SessionShim**: mock LLM session that replays recorded tool call sequences. `MockSession { turns: Vec<MockTurn> }` where `MockTurn { input: String, expected_tools: Vec<ToolCall>, response: String }`.
2. **Unit tests**: deterministic input/output per tool. Already partially implemented.
3. **Property-based tests** (proptest): fuzz tool inputs and verify invariants:
   - `read_file`: any valid path returns `Ok` or `FileNotFound`, never panics
   - `edit_file`: `old_string` found ↔ `Ok`, not found ↔ `Err(NotFound)`, edit is idempotent (applying twice = same result)
   - `grep`: regex pattern always valid or returns `InvalidRegex`, match count <= line count
   - `bash`: timeout always respected, output length bounded
4. **LLM evaluation tests** (66 tests): present task description to LLM, check if it selects the correct tool. Categories: file operations (12), search (8), code editing (10), web (6), planning (8), chain (12), research (10).
5. **Red-team tests**: OWASP Agentic Top 10 attack patterns + DeFi-specific attacks.

**Current code**: Basic unit tests exist in `crates/roko-std/src/tool/builtin/*.rs` (each tool file has `#[cfg(test)]` module). `crates/roko-std/src/tool/mock_dispatcher.rs` provides `MockToolDispatcher` for deterministic testing. No `proptest`/`quickcheck` dependency. No LLM evaluation test harness. No red-team test suite.

**What to change**:
1. Add `proptest = "1"` to `crates/roko-std/Cargo.toml` `[dev-dependencies]`
2. Write property-based tests for `read_file`, `edit_file`, `grep` in `crates/roko-std/tests/property_tests.rs`:
   ```rust
   proptest! {
       #[test]
       fn edit_file_idempotent(old in ".*", new in ".*") {
           // applying edit twice with same old_string produces same result
       }
       #[test]
       fn grep_never_panics(pattern in ".*") {
           // grep with any pattern returns Ok or Err, never panics
       }
   }
   ```
3. Add LLM evaluation test harness in `crates/roko-std/tests/eval_tests.rs` with test cases as TOML fixtures
4. Add red-team test cases for command injection via `bash` tool

**Reference files**:
- `crates/roko-std/src/tool/builtin/` — 16 builtin tool implementations (test targets)
- `crates/roko-std/src/tool/mock_dispatcher.rs` — `MockToolDispatcher` for deterministic testing
- `crates/roko-std/Cargo.toml` — add proptest dev-dependency
- `docs/18-tools/07-tool-testing.md` — full spec: 4 layers, 66 eval tests, red-team attack patterns
**Depends on**: None
**Accept when**:
- [x] `proptest` dependency added to roko-std
- [x] Property-based tests for at least `read_file`, `edit_file`, `grep`
- [x] LLM evaluation test harness with at least 10 test cases
- [x] Red-team test for command injection via bash tool
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'proptest' crates/roko-std/Cargo.toml
ls crates/roko-std/tests/property_tests.rs crates/roko-std/tests/eval_tests.rs 2>/dev/null
cargo test -p roko-std -- property
```
**Priority**: P2

---

### TOOL-08: Event source triggers

- [x] Wire Cron, FileWatch, and Webhook event triggers

**Spec** (doc 15): 5 event sources (Cron, FileWatch, GitHub webhooks, Slack, generic webhooks).
**Current code**: `crates/roko-serve/src/scheduler.rs` has scheduling infrastructure. `crates/roko-serve/src/fswatcher.rs` has file-watcher infrastructure. `crates/roko-plugin/` has event source SDK. Not wired as trigger sources.
**What to change**: Wire scheduler.rs as Cron trigger. Wire fswatcher.rs as FileWatch trigger. Add webhook receiver route to roko-serve for GitHub/generic webhooks.
**Reference files**:
- `crates/roko-serve/src/scheduler.rs` (scheduling)
- `crates/roko-serve/src/fswatcher.rs` (file watching)
- `crates/roko-plugin/` (event source SDK)
- `crates/roko-serve/src/routes/` (route registration)
**Depends on**: None
**Accept when**:
- [x] Cron trigger fires on schedule
- [x] FileWatch trigger fires on file change
- [x] At least one webhook source functional
- [x] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'Cron\|FileWatch\|webhook' crates/roko-serve/src/ --include='*.rs' | head -15
grep -rn 'EventSource\|Trigger' crates/roko-plugin/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

### TOOL-09: Wallet management (chain domain)

- [ ] Implement WalletHandle abstraction and custody mode selection

**Spec** (doc 06 `docs/18-tools/06-wallet-management.md`): Three custody modes for chain agents:
1. **Delegation (Enclave)** — keys in secure hardware (HSM/TEE). Agent uses session keys (ERC-7715) with time-limited, value-bounded authorization. Config: `custody = "delegation"`, `session_key_ttl = "24h"`, `max_value_per_session = "10000 USD"`.
2. **Embedded (ERC-4337)** — account abstraction via smart contract wallet. Providers: Privy, ZeroDev, Safe. On-chain policy enforcement via PolicyCage contract. UserOperation validation. Config: `custody = "embedded"`, `provider = "privy"`.
3. **Local Key (Dev)** — plain private key, dev/test only. Config: `custody = "local_key"`.

`WalletHandle` abstraction normalizes all modes to Alloy's `Signer` trait. 7 wallet providers: MetaMask Snap, Privy, ZeroDev, Safe, Turnkey, Fireblocks, local.

Also: identity NFT custody (ERC-8004 agent identity), credential lifecycle.

**Current code**: `crates/roko-chain/src/` has chain type stubs. No `WalletHandle` struct. No custody mode selection. Alloy dependency exists in workspace Cargo.toml.

**What to change**: Implement `WalletHandle` enum with three variants wrapping different Alloy `Signer` implementations. Add `[wallet]` config section parsing. Start with LocalKey variant for dev.

**Reference files**:
- `crates/roko-chain/src/` — chain crate (implementation target)
- `crates/roko-core/src/config/schema.rs` — config schemas (add `[wallet]` section)
- `docs/18-tools/06-wallet-management.md` — full spec: 3 custody modes, WalletHandle, 7 providers, ERC-7715 session keys
**Depends on**: TOOL-05 (chain domain tools use WalletHandle)
**Accept when**:
- [ ] `WalletHandle` enum with Delegation/Embedded/LocalKey variants
- [ ] `[wallet]` config section parsed from roko.toml
- [ ] At least LocalKey variant functional for dev
- [ ] WalletHandle normalizes to Alloy `Signer` trait
- [ ] `cargo test -p roko-chain` passes
**Verify**:
```bash
grep -rn 'WalletHandle\|custody\|wallet.*mode' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2 (chain domain)

---

### TOOL-10: Service integration architecture

- [x] Implement three-layer integration for external platforms

**Spec** (doc 08 `docs/18-tools/08-service-integrations.md`): Three-layer architecture for connecting to external platforms:
1. **Layer 1: Event Reception** — webhook endpoints (GitHub, Slack), polling adapters (Linear, chain events), WebSocket streams (chain data, Slack Socket Mode)
2. **Layer 2: Agent Execution** — Event -> Engram conversion, template matching (which agent handles this?), agent spawn with ToolContext
3. **Layer 3: MCP Tool Adapters** — github.* tools (via roko-mcp-github), slack.* tools (via roko-mcp-slack), scripts.* tools (via roko-mcp-scripts)

Chain domain services: MetaMask Snap (wallet), Uniswap Trading API (DEX), Venice (inference), Bankr (banking), AgentCash (payments). Operations adapters: Slack (messaging), GitHub (code), Linear (project management).

**Structural vs decorative classification**: structural integrations (MetaMask, Uniswap API) are required for core functionality; decorative integrations (bounty program, social) are optional enhancements.

**Current code**: Full three-layer implementation exists.
- Layer 1: `crates/roko-serve/src/routes/webhooks.rs` — GitHub, Slack, generic webhook endpoints with HMAC signature verification.
- Layer 2: `crates/roko-serve/src/dispatch.rs` — `dispatch_loop()` matches Engram signals to subscriptions via `SubscriptionRegistry::find_matching()`, dispatches to agent templates.
- Layer 3: `roko-mcp-github` (19 tools), `roko-mcp-slack` (9 tools), `roko-mcp-scripts` (dynamic tools).
- Registry: `crates/roko-serve/src/integrations.rs` — `IntegrationRegistry` catalogs all integrations with structural/decorative classification.
- API: `GET /api/integrations`, `GET /api/integrations/:name` expose the catalog.

**Reference files**:
- `crates/roko-serve/src/routes/` — existing route modules (add webhook routes)
- `crates/roko-mcp-github/src/main.rs` — GitHub MCP server (Layer 3)
- `crates/roko-mcp-slack/src/main.rs` — Slack MCP server (Layer 3)
- `docs/18-tools/08-service-integrations.md` — full spec: 3 layers, service catalog, structural vs decorative
**Depends on**: TOOL-01 (MCP servers for Layer 3), TOOL-08 (event source triggers)
**Accept when**:
- [x] Webhook receiver route exists for at least GitHub
- [x] Incoming webhooks converted to Engrams
- [x] Engrams matched to agent templates for dispatch
- [x] `cargo test -p roko-serve` passes
**Verify**:
```bash
grep -rn 'webhook\|Webhook\|/webhooks/' crates/roko-serve/src/ --include='*.rs'
cargo test -p roko-serve
```
**Priority**: P2

---

## Verify

```bash
cargo test -p roko-std
cargo test -p roko-agent
cargo test --workspace
```
