# 12-interfaces -- gap checklist

Spec: `docs/12-interfaces/` (25 files, docs 00-23, no doc 20). Code: `crates/roko-cli/`, `crates/roko-serve/`.

Overall: ~50% implemented. CLI foundations solid; TUI views exist but only render in text-mode. Major gaps in scaffolders, plugin system, interactive TUI region navigation, slash commands in REPL, WebSocket named-channel protocol, Spectre creature system, accessibility, and all Phase 2+ visual features.

## Compliant (no action needed)

- CLI subcommands: run, plan, prd, research, status, config, dashboard, serve, daemon, replay, inject (docs 00-01)
- Config layered resolution (4-layer) (doc 04)
- ROSEDUST theme -- palette, color utilities, TUI integration (doc 07)
- TUI widgets: agent grid, plan tree, status/header/phase/token bars, task/plan detail modals, braille, scrollbar (doc 08)
- HTTP API core routes -- status, plans, prds, run, research, agents, learning, config (doc 05)
- Projections -- cohort_health, active_tasks, gate_pipeline, agent_trails with SSE stream and cursors (doc 22)
- REPL mode -- bare `roko` with TTY detection routes to `cmd_repl`; `:help`, `:status`, `:quit` built-ins

## Not applicable (design-only / deferred — no gap item needed)

- UX innovation proposals (doc 18) — aspirational design proposals, not implementable specs
- Rust SDK developer UX (doc 19) — explicitly deferred until external library surface exists
- Rich UX primitives (doc 23) — target-state vocabulary, depends on future projection contracts

## Checklist

### UI-01: `roko new` scaffolders (9 types) not implemented

- [x] Implement code generation subcommand for Synapse traits and plugins

**Spec** (doc 02 `02-roko-new-scaffolders.md`): `roko new <type> <name>` generates compilable boilerplate for 9 types: `domain` (full domain profile with config, gates, templates), `gate` (Gate trait impl with one passing test), `scorer` (Scorer trait impl), `router` (Router trait impl), `policy` (Policy trait impl), `substrate` (Substrate trait impl), `probe` (probe event source), `event-source` (EventSource trait impl), `template` (prompt template). Each generated artifact must compile immediately without manual edits and include at least one passing test. The `domain` type is the most complex — it generates a directory with `mod.rs`, gates, templates, and a `domain.toml` config.

**Current code**: No `New` variant in `Command` enum at `crates/roko-cli/src/main.rs:191`. No scaffold templates anywhere in `crates/roko-cli/src/`. `dispatch_subcommand()` at line 1003 has no `New` arm. Only TUI `DashboardScaffold` in `crates/roko-cli/src/tui/pages/mod.rs:141` exists (unrelated — a TUI page scaffold, not code generation). The six Synapse traits are defined in `crates/roko-core/src/lib.rs`: `Substrate`, `Scorer`, `Gate`, `Router`, `Composer`, `Policy`.

**What to change**:
- Add `New { #[arg] type_name: String, #[arg] name: String }` variant to `Command` enum at `crates/roko-cli/src/main.rs:191`
- Add `Command::New { type_name, name }` arm to `dispatch_subcommand()` at line 1003
- Create `crates/roko-cli/src/scaffold.rs` with `pub fn scaffold(type_name: &str, name: &str) -> Result<()>` that matches on type_name and generates the appropriate Rust module
- For each type, generate a file with: `use roko_core::{TraitName};` import, a struct definition, a `TraitName` impl with stub methods, and a `#[cfg(test)] mod tests` block with one passing test
- Pattern to follow for code generation: `crates/roko-compose/src/templates/` has 9 existing prompt templates that show the template-data separation pattern

**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` — `Command` enum (add `New` variant)
- `crates/roko-cli/src/main.rs:1003` — `dispatch_subcommand()` match (add `Command::New` arm)
- `crates/roko-core/src/lib.rs` — Synapse trait definitions: `Substrate`, `Scorer`, `Gate`, `Router`, `Composer`, `Policy`
- `crates/roko-compose/src/templates/` — existing template patterns (9 templates)
- `docs/12-interfaces/02-roko-new-scaffolders.md` — full spec with example generated code for each type
**Depends on**: None
**Accept when**:
- [x] `roko new gate <name>` generates a compilable gate crate or module with one passing test
- [x] At least 6 of the 9 scaffold types produce compilable output
- [x] Generated code compiles immediately without manual edits
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'roko new\|scaffold\|New.*Command' crates/roko-cli/src/main.rs
cargo test --workspace
```

**Priority**: P1

---

### UI-02: Plugin system (`roko plugin`) not implemented

- [x] Implement plugin discovery, install, and audit subcommands

**Spec** (docs 00, 01): 5-tier Service Provider Interface. `roko plugin list/install/audit/enable/disable` CLI. Plugin discovery from `plugins/**` directory.
**Current code**: No `Plugin` variant in `Command` enum at `crates/roko-cli/src/main.rs:191`. No `dispatch_subcommand` arm at line 1003. `crates/roko-cli/src/event_sources.rs:9` imports `roko_plugin::CronEventSource` -- plugin crate exists. MCP config passthrough treats all servers identically (no plugin tiers).
**What to change**: Add `Plugin { cmd: PluginCmd }` variant to `Command` enum at `crates/roko-cli/src/main.rs:191`. Add `PluginCmd` sub-enum with `List`, `Install`, `Audit`, `Enable`, `Disable`. Add `Command::Plugin` arm to `dispatch_subcommand()` at line 1003. Implement `plugins/` directory discovery. Wire tier assignment (links to SAFE-06).
**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` (`Command` enum, all existing variants)
- `crates/roko-cli/src/main.rs:1003` (`dispatch_subcommand` match)
- `crates/roko-cli/src/event_sources.rs:9` (`roko_plugin::CronEventSource` import)
- `crates/roko-core/src/config/schema.rs` (config types for roko.toml plugin section)
**Depends on**: SAFE-06 (plugin tier enforcement)
**Accept when**:
- [x] `roko plugin list` prints available plugins discovered from `plugins/` and registry
- [x] `roko plugin install <name>` fetches and registers a plugin
- [x] `roko plugin audit` reports each plugin's requested capabilities and tier assignment
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'plugin\|Plugin' crates/roko-cli/src/main.rs
cargo test --workspace
```

**Priority**: P1

---

### UI-03: `roko explain` progressive help not implemented

- [x] Implement layered topic explanation system with 3-level disclosure

**Spec** (doc 03): `roko explain <topic>` shows Level 1 summary; pressing Enter expands to Level 2 and Level 3. Topics: gates, routing, cognitive, neuro, daimon, dreams, engram, cfactor (minimum 8).
**Current code**: `Explain` variant in `Command` enum dispatches to `cmd_explain()` in main.rs. Topic registry with 10 topics and 3-level content lives in `crates/roko-cli/src/explain.rs`.
**What to change**: Add `Explain { topic: String, #[arg(long)] depth: Option<u8> }` variant to `Command` enum at `crates/roko-cli/src/main.rs:191`. Add `Command::Explain` arm to `dispatch_subcommand()` at line 1003. Create `crates/roko-cli/src/explain.rs` with topic registry (3-level content for at least 8 topics). Implement interactive Enter expansion for depth 2/3.
**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` (`Command` enum, all existing variants)
- `crates/roko-cli/src/main.rs:1003` (`dispatch_subcommand` match)
- `crates/roko-gate/src/lib.rs` (gate types — for "gates" topic content)
- `crates/roko-learn/src/cascade_router.rs` (routing — for "routing" topic content)
**Depends on**: None
**Accept when**:
- [x] `roko explain gates` prints a Level 1 summary (2-3 sentences)
- [x] `--depth 2` or interactive Enter expands to Level 2 detail
- [x] At least 8 topics covered (10 topics: gates, routing, cognitive, neuro, daimon, dreams, engram, cfactor, plans, agents)
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'explain\|Explain' crates/roko-cli/src/main.rs
cargo test --workspace
```

**Priority**: P1

---

### UI-04: Interactive TUI region navigation not wired

- [x] Implement region-based navigation and wire existing views to keyboard shortcuts

**Spec** (doc 09): 29 screens across 6 regions. Views are built (agent list, plan list, config, logs, signals) but the TUI currently renders text-only via `--text` flag; the ratatui interactive path does not implement region switching or the full screen inventory. Regions 2-6 have zero functional interactive screens.
**Current code**: 7 tab views in `crates/roko-cli/src/tui/views/mod.rs:42-59` — `render_tab_content()` dispatches to `dashboard_view`, `plans_view`, `agents_view`, `git_view`, `logs_view`, `config_view`, `context_view` by `Tab` enum. `Tab` at `crates/roko-cli/src/tui/tabs.rs:10` has 7 variants mapped to F1-F7. `FocusZone` at `crates/roko-cli/src/tui/input.rs:38` has 5 zones (`PlanTree`, `TaskProgress`, `AgentOutput`, `CommandOutput`, `RightPanel`). `PageId` at `crates/roko-cli/src/tui/pages/mod.rs:13` has 15 legacy variants (scaffold pages). `App` at `crates/roko-cli/src/tui/app.rs:58` tracks `current_page: PageId` (line 73) and `tui_state: TuiState` (line 63).
**What to change**: Extend `Tab`/view dispatch to include Agent Detail, Plan Detail, Knowledge, Collective, System screens (currently only 7 overview tabs). Add region-based navigation where F-key tabs map to regions and sub-navigation selects screens within a region. Extend `FocusZone` for new views.
**Reference files**:
- `crates/roko-cli/src/tui/views/mod.rs:42-59` (`render_tab_content()` dispatch)
- `crates/roko-cli/src/tui/tabs.rs:10-37` (`Tab` enum, `Tab::ALL`)
- `crates/roko-cli/src/tui/input.rs:38-50` (`FocusZone` enum)
- `crates/roko-cli/src/tui/input.rs:195` (`TuiAction` enum — all navigation actions)
- `crates/roko-cli/src/tui/app.rs:58-80` (`App` struct, `current_page`, `tui_state`)
- `crates/roko-cli/src/tui/pages/mod.rs:13-44` (`PageId` enum, 15 variants)
**Depends on**: None
**Accept when**:
- [x] Region 1 navigation works (F1 cycles Agent List, Plan List, Mesh Status)
- [x] At least 2 Agent Detail screens functional (Output Stream 2.1, Gate Results 2.2)
- [x] At least 2 Plan Detail screens functional (DAG View 3.1, Task Detail 3.2)
- [x] At least 1 System screen functional (Provider Health 6.1)
- [x] Total: at least 12/29 screens interactive (currently ~0 confirmed interactive)
- [x] `cargo test -p roko-cli`
**Verify**:
```bash
ls crates/roko-cli/src/tui/views/
grep -rn 'PageId' crates/roko-cli/src/tui/pages/mod.rs
cargo test -p roko-cli
```

**Priority**: P1 (core screens), P2 (Knowledge/Collective regions requiring Neuro/Daimon store integration)

---

### UI-05: `recent_episodes` projection not implemented

- [x] Add `recent_episodes` as the fifth named projection

**Spec** (doc 22): Five named projections required: cohort_health, active_tasks, gate_pipeline, agent_trails, recent_episodes.
**Current code**: `crates/roko-serve/src/routes/projections.rs:21-25` defines route handlers. `projection_state_value()` at line 121 matches 7 projection names (`dashboard`, `cohort_health`, `active_tasks`, `gate_pipeline`, `agent_trails`, `alerts`, `plans_list`) but not `recent_episodes` — falls through to `Err(ApiError::not_found(...))` at line 185. `projection_accepts_event()` at line 191 similarly has no `recent_episodes` arm. `DashboardSnapshot` at `crates/roko-core/src/dashboard_snapshot.rs:542` has no episodes field. `DashboardEvent` enum at `crates/roko-core/src/dashboard_snapshot.rs:25` has no episode variant. Episodes live in `.roko/episodes.jsonl`, read via `EpisodeLogger` in `crates/roko-learn/src/episode_logger.rs`.
**What to change**:
- Add `"recent_episodes"` arm to `projection_state_value()` at `crates/roko-serve/src/routes/projections.rs:126` — read from `AppState` layout or add episodes ring to `DashboardSnapshot`
- Add `DashboardEvent::EpisodeRecorded { agent_id, role, episode_id }` variant at `crates/roko-core/src/dashboard_snapshot.rs:25`
- Add `"recent_episodes"` arm to `projection_accepts_event()` at line 191, matching the new event variant
- Wire episode emission from `crates/roko-cli/src/orchestrate.rs` where `EpisodeLogger` writes
**Reference files**:
- `crates/roko-serve/src/routes/projections.rs:121-241` — `projection_state_value()` and `projection_accepts_event()`
- `crates/roko-core/src/dashboard_snapshot.rs:25-90` — `DashboardEvent` enum (needs new variant)
- `crates/roko-core/src/dashboard_snapshot.rs:542-576` — `DashboardSnapshot` struct (no episodes field)
- `crates/roko-learn/src/episode_logger.rs` — `Episode` struct, `EpisodeLogger` for `.roko/episodes.jsonl`
**Depends on**: None
**Accept when**:
- [x] `GET /projections/recent_episodes` returns recent episode records from `.roko/episodes.jsonl`
- [x] `/projections/recent_episodes/stream` delivers SSE deltas as new episodes are written
- [x] `filter` query param supports filtering by agent role
- [ ] `cargo test -p roko-serve`
**Verify**:
```bash
grep -rn 'recent_episodes\|projection_state_value' crates/roko-serve/src/routes/projections.rs
cargo test -p roko-serve
```

**Priority**: P1

---

### UI-06: WebSocket named-channel protocol incomplete

- [x] Implement named channel subscriptions, cursor-based resume, and back-pressure modes

**Spec** (doc 06): Named channel prefixes (`projection:*`, `topic:*`, `engram-stream:*`). Client provides cursor on reconnect to resume from last-seen event. Back-pressure modes: AtMostOnce, Coalesce, ResumeRequired, configurable per subscription.
**Current code**: `crates/roko-serve/src/routes/ws.rs:21-22` — single `/ws` route. `ClientMsg` struct at line 32 has only `subscribe: Vec<String>` field (no `cursor`, no `back_pressure`). `handle_ws()` at line 38 replays from seq 0 (line 43: `state.event_bus.replay_from(0)`) with no cursor support. `matches_filter()` at line 115 matches on serde `type` tag, not on channel prefixes. `EventBus` at `crates/roko-serve/src/event_bus.rs:1` has `replay_from(seq)` method that could support cursor-based resume.
**What to change**:
- Extend `ClientMsg` at `crates/roko-serve/src/routes/ws.rs:32` — add `cursor: Option<u64>` and `back_pressure: Option<String>` fields
- In `handle_ws()` at line 43, use `cursor` value instead of hardcoded `0` for `replay_from()`
- Update `matches_filter()` at line 115 to parse channel prefixes (`projection:*`, `topic:*`, `engram-stream:*`) with glob matching
- Add back-pressure mode state per subscription (skip, coalesce, or buffer)
**Reference files**:
- `crates/roko-serve/src/routes/ws.rs:21-139` — full WebSocket handler
- `crates/roko-serve/src/event_bus.rs:1-50` — `EventBus`, `Envelope`, `replay_from(seq)`
- `crates/roko-serve/src/routes/subscriptions.rs` — subscription management pattern
- `crates/roko-serve/src/routes/sse.rs` — SSE stream (parallel pattern for cursor)
**Depends on**: None
**Accept when**:
- [x] Client `subscribe` message accepts channel patterns (`projection:gate_pipeline`, `topic:agent.*`)
- [x] Client can send `cursor` in subscribe message; server replays from that sequence number
- [x] Back-pressure mode (`at_most_once`, `coalesce`, `resume_required`) accepted per channel
- [ ] `cargo test -p roko-serve`
**Verify**:
```bash
grep -rn 'subscribe\|channel\|cursor\|back_pressure' crates/roko-serve/src/routes/ws.rs
cargo test -p roko-serve
```

**Priority**: P1

---

### UI-07: Slash commands in REPL not implemented

- [x] Wire slash commands into the interactive REPL mode

**Spec** (docs 01, 21): `/edit`, `/run`, `/plan`, `/explain`, `/learn`, `/replay`, `/tune`, `/undo` available in REPL and TUI interactive modes. Current REPL only supports colon-prefixed built-ins (`:help`, `:status`, `:quit`).
**Current code**: `crates/roko-cli/src/repl.rs:12` — `ReplCommand` enum has 4 variants: `Quit`, `Help`, `Status`, `Prompt(String)`. `parse_input()` at line 47 only matches `:quit`/`:help`/`:status`; everything else becomes `Prompt`. `ReplMode::run()` at line 63 loops through input, collecting commands but not dispatching prompts to agents. `cmd_repl()` at `crates/roko-cli/src/main.rs:1148` creates `ReplMode` with a session ID and calls `run()` — discards returned commands.
**What to change**:
- Add slash variants to `ReplCommand` at `crates/roko-cli/src/repl.rs:12` — e.g. `SlashPlan(Vec<String>)`, `SlashReplay(String)`, `SlashExplain(String)`, etc.
- Extend `parse_input()` at line 47 to detect `/`-prefixed lines and parse slash command + args
- In `cmd_repl()` at `crates/roko-cli/src/main.rs:1148`, actually dispatch slash commands to the corresponding async handlers (`cmd_plan`, `cmd_replay`, etc.)
**Reference files**:
- `crates/roko-cli/src/repl.rs:12-55` (`ReplCommand` enum, `parse_input()`)
- `crates/roko-cli/src/repl.rs:63-130` (`ReplMode::run()` loop)
- `crates/roko-cli/src/main.rs:1148-1160` (`cmd_repl()` — creates ReplMode, discards commands)
- `crates/roko-cli/src/main.rs:1003` (`dispatch_subcommand` — reference for handler names)
**Depends on**: UI-03 (for `/explain` dispatch)
**Accept when**:
- [x] Slash commands parsed and dispatched in `ReplMode::run`
- [x] `/plan` triggers `cmd_plan` equivalent in-session
- [x] `/replay <hash>` triggers signal DAG walk
- [x] `/explain <topic>` prints topic summary (links to UI-03)
- [x] At least 6 of 8 slash commands functional (/plan, /explain, /replay, /run, /research, /learn, /tune, /status)
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'ReplCommand\|slash\|Slash' crates/roko-cli/src/repl.rs
cargo test -p roko-cli
```

**Priority**: P1

---

### UI-08: Bare `roko` does not detect workspace or offer resumption

- [x] Implement workspace detection and recent-session resumption in `cmd_repl`

**Spec** (doc 00): Running `roko` with no subcommand should detect the nearest `.roko/` directory, show an intent classifier with recent work, and offer to resume the last interrupted session.
**Current code**: `crates/roko-cli/src/main.rs:978-1001` — `dispatch()` checks for subcommand, then falls through to `cmd_repl()` at line 1000 when no subcommand and stdin is TTY. `cmd_repl()` at line 1148 creates `ReplMode::new(session_id)` and calls `run()` — prints bare `roko> ` prompt. No `.roko/` detection. `ReplMode::run()` at `crates/roko-cli/src/repl.rs:63` prints `"roko repl (session: ...)"` banner but no workspace context. `RokoLayout` in `crates/roko-fs/src/layout.rs` has methods for locating `.roko/` directories. Executor snapshots persist at `.roko/state/executor.json`.
**What to change**:
- In `cmd_repl()` at `crates/roko-cli/src/main.rs:1148`, use `RokoLayout::discover()` or walk up from cwd to find `.roko/`
- Print workspace path in the REPL banner at `crates/roko-cli/src/repl.rs:71`
- Enumerate recent plans from `.roko/prd/` and show last 3
- Check `.roko/state/executor.json` for interrupted sessions and offer resumption prompt
**Reference files**:
- `crates/roko-cli/src/main.rs:978-1001` (`dispatch()` fallthrough)
- `crates/roko-cli/src/main.rs:1148-1160` (`cmd_repl()`)
- `crates/roko-cli/src/repl.rs:63-130` (`ReplMode::run()` — banner and loop)
- `crates/roko-fs/src/layout.rs` (`RokoLayout` — workspace discovery)
- `crates/roko-cli/src/workspace_paths.rs` (workspace path resolution)
**Depends on**: None
**Accept when**:
- [x] `cmd_repl` detects nearest `.roko/` directory and prints workspace path
- [x] Recent plans and PRDs summarized on startup (last 3)
- [x] Last interrupted executor snapshot offered for resumption (`roko plan run --resume ...`)
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'cmd_repl\|workspace\|\.roko' crates/roko-cli/src/main.rs
cargo test -p roko-cli
```

**Priority**: P2

---

### UI-09: Unified verb set not consistent across surfaces

- [x] Map all CLI surfaces to 9 canonical verbs: ask, plan, do, watch, inspect, replay, learn, tune, connect

**Spec** (docs 01, 21): Nine verbs available and behaviorally consistent across CLI, TUI, Chat, and (future) Web surfaces. Current CLI has `run`, `research`, `status`, `replay` but no `watch`, `inspect`, `tune`, or `connect` subcommands.
**Current code**: `Command` enum at `crates/roko-cli/src/main.rs:191-368` has ~20 variants: `Init`, `Run`, `Status`, `Replay`, `Dream`, `Config`, `Secret`, `Agent`, `Inject`, `Plan`, `Prd`, `Research`, `Chat`, `Neuro`, `Subscription`, `EventSources`, `Provider`, `Model`, `Experiment`, `Deploy`, `Update`, `Completions`, `Daemon`, `Dashboard`, `Serve`, `Worker`. Missing canonical verbs: `watch` (Dashboard alias), `inspect` (Replay alias), `tune` (threshold editor), `connect` (agent mesh), `ask`, `do`, `learn`. Clap supports `#[command(visible_alias = "...")]` for aliases (already used for `Secret`/`Config`).
**What to change**:
- Add `visible_alias` annotations: `Dashboard` gets `visible_alias = "watch"`, `Replay` gets `visible_alias = "inspect"`
- Add new variants `Tune`, `Connect`, `Ask`, `Do`, `Learn` to `Command` enum at `crates/roko-cli/src/main.rs:191` with appropriate dispatch at line 1003
- For REPL slash commands (UI-07), map `/watch`, `/inspect`, etc. to same handlers
**Reference files**:
- `crates/roko-cli/src/main.rs:191-368` (`Command` enum — all 20+ variants)
- `crates/roko-cli/src/main.rs:1003` (`dispatch_subcommand` — match arms)
- `crates/roko-cli/src/main.rs:236` (`Secret` variant — example of `visible_alias`)
- `crates/roko-cli/src/repl.rs:12-55` (`ReplCommand` — slash commands use same verbs)
**Depends on**: None
**Accept when**:
- [x] All 9 verbs available as top-level CLI subcommands (aliases accepted)
- [x] `roko watch` equivalent to `roko dashboard`
- [x] `roko inspect <hash>` equivalent to `roko replay <hash>`
- [x] `roko tune` opens adaptive threshold editor
- [ ] Verb behavior consistent: same verb in CLI, TUI, and Chat produces equivalent result
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'Command\|watch\|inspect\|tune\|connect' crates/roko-cli/src/main.rs | head -20
cargo test --workspace
```

**Priority**: P2

---

### UI-10: Web portal (deferred)

- [ ] Implement 5-page first-party web UI

**Spec** (doc 13): SvelteKit or React. Five pages: Dashboard, Agent Detail, Plan Explorer, Knowledge Browser, Settings. HTTP API provides the substrate.
**Current code**: Not started. HTTP API at `crates/roko-serve/src/routes/` (~20 route files: `agents.rs`, `plans.rs`, `prds.rs`, `status.rs`, `learning.rs`, `config.rs`, `projections.rs`, `providers.rs`, `research.rs`, `run.rs`, `sse.rs`, `ws.rs`, `templates.rs`, `webhooks.rs`, `deployments.rs`, `diagnosis.rs`, `aggregator.rs`, `subscriptions.rs`, `middleware.rs`) provides all required data endpoints on `:6677`.
**What to change**: Create SvelteKit or React frontend. Five pages consuming existing HTTP API routes.
**Reference files**:
- `crates/roko-serve/src/routes/` (all route files)
- `crates/roko-serve/src/routes/mod.rs` (router assembly)
**Depends on**: HTTP API stabilization
**Accept when**:
- [ ] Web UI serves on a port
- [ ] Dashboard page renders agent/plan status
- [ ] At least 3 of 5 pages functional
- [ ] `cargo test -p roko-serve`
**Verify**:
```bash
ls crates/roko-serve/src/routes/
cargo test -p roko-serve
```

**Priority**: P2 (Phase 2+, explicitly deferred -- HTTP API must stabilize first)

---

### UI-11: Agent onboarding flow not implemented

- [x] Implement `roko init` interactive setup with domain-profile selection and workspace detection

**Spec** (doc 14 `14-agent-onboarding-flow.md`): Three onboarding paths — Minimal (under 30s: `roko plugin install @roko/coding-profile && roko init && roko ask "..."`), Standard (~2min with guided choices), Full (~5min with explicit config). The `roko init` command should: auto-detect project domain from file patterns (Cargo.toml -> Rust, package.json -> TypeScript, etc.), offer matching domain-profile bundle, create `.roko/` directory with resumable `roko.toml`, check plugins/MCP servers opportunistically, generate a Spectre creature identity from project hash. The minimal path target is "first useful output in under 30 seconds." The onboarding should also offer to resume the last interrupted session from `.roko/state/executor.json`.

**Current code**: `roko init` exists as `Command::Init` at `crates/roko-cli/src/main.rs:191` and creates `.roko/` directory and `roko.toml`, but does NOT auto-detect project domain, does NOT offer domain-profile selection, does NOT check for interrupted sessions, and does NOT run interactive setup. The `RokoLayout` in `crates/roko-fs/src/layout.rs` handles `.roko/` directory structure. No domain-profile bundle system exists.

**What to change**:
- Extend `cmd_init()` at `crates/roko-cli/src/main.rs` to add interactive prompts: detect project files, suggest domain profile, configure model provider
- Add `--profile <name>` flag to `Command::Init` for non-interactive setup (`roko init --profile coding`)
- Add project-domain auto-detection: scan cwd for `Cargo.toml` (Rust), `package.json` (TypeScript), `go.mod` (Go), `requirements.txt` (Python)
- Check `.roko/state/executor.json` for interrupted sessions and offer resumption

**Reference files**:
- `crates/roko-cli/src/main.rs` — `Command::Init`, `cmd_init()` handler
- `crates/roko-fs/src/layout.rs` — `RokoLayout` for `.roko/` paths
- `crates/roko-core/src/config/schema.rs` — `RokoConfig` for `roko.toml` generation
- `docs/12-interfaces/14-agent-onboarding-flow.md` — three onboarding paths with timing targets
**Depends on**: UI-02 (plugin system for domain-profile bundles)
**Accept when**:
- [x] `roko init` detects project domain and suggests matching profile (auto-detection for Rust, TypeScript, Go, Python, Ruby, Java; --profile flag)
- [ ] Interactive prompts for model provider and gate configuration
- [x] Interrupted session offered for resumption
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'cmd_init\|Init' crates/roko-cli/src/main.rs
cargo test -p roko-cli
```

**Priority**: P2

---

### UI-12: A2UI generative interfaces protocol not implemented

- [ ] Implement JSONL A2UI emission and rendering in TUI/CLI

**Spec** (doc 15 `15-generative-interfaces-a2ui.md`, marked "Scaffold/P3"): Agents emit structured JSONL UI descriptions via the A2UI protocol. 12 component types: `table`, `progress`, `chart`, `status`, `code`, `callout`, `tree`, `kv`, `diagram`, `form`, `markdown`, `image`. Each line is `{"a2ui": "<type>", ...}`. The TUI renders as Unicode (tables with `─│┌┐└┘`, charts with braille, status with check/cross symbols). CLI renders as text. Web renders as HTML components. A2UI output is sandboxed — cannot escape viewport or access other agents' data. ROSEDUST design language inherited automatically. A2UI is optional — agents work fine without it.

**Current code**: No A2UI parsing or rendering anywhere in the codebase. Agent output in the tool loop is treated as plain text. The TUI at `crates/roko-cli/src/tui/` renders predefined views but has no dynamic component rendering from agent output.

**What to change**:
- Define `A2uiComponent` enum with 12 variants in `crates/roko-core/src/` or `crates/roko-cli/src/`
- Add JSONL parser that detects `{"a2ui": "..."}` lines in agent output stream
- In TUI `agents_view` at `crates/roko-cli/src/tui/views/`, render detected A2UI components as Unicode widgets
- In CLI output path, render A2UI components as formatted text

**Reference files**:
- `docs/12-interfaces/15-generative-interfaces-a2ui.md` — full A2UI spec with 12 component types and schema
- `crates/roko-cli/src/tui/views/` — TUI views where rendering would be added
- `crates/roko-cli/src/tui/widgets/` — existing TUI widgets to extend
**Depends on**: None
**Accept when**:
- [ ] A2UI JSONL lines detected in agent output
- [ ] At least 4 component types render in TUI (table, progress, status, code)
- [ ] Non-A2UI lines still render as plain text (backward compatible)
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'a2ui\|A2ui\|A2UI' crates/ --include='*.rs' | grep -v target/
cargo test -p roko-cli
```

**Priority**: P3 (design-only, deferred)

---

### UI-13: IDE integration (MCP + ACP) not started

- [ ] Expose Roko commands as MCP tools for IDE integration

**Spec** (doc 20 `20-ide-integration-strategy.md`): Three-phase strategy: (1) MCP server now (~500 LOC, works in VS Code Copilot, Cursor, Continue, any MCP client), (2) ACP agent next (`roko acp` for full agent lifecycle in Zed, JetBrains, Neovim, Emacs), (3) VS Code extension later only if ACP insufficient. Phase 1 MCP server exposes Roko commands (plan run, status, gate results, agent chat) as MCP tools. The existing `crates/roko-mcp-code/` provides code-intelligence tools; the IDE integration needs an additional MCP server for orchestration commands.

**Current code**: `crates/roko-mcp-code/src/lib.rs` implements code-intelligence MCP tools (search_code, get_symbol_context, etc.) — this is the code index MCP, NOT the orchestration MCP. No orchestration MCP server exists. No ACP implementation.

**What to change**:
- Create `crates/roko-mcp-orchestrator/` crate (or add to existing `roko-mcp-code`) with MCP tools: `plan_run`, `plan_status`, `gate_results`, `agent_status`, `prd_list`, `research_topic`
- Add `roko mcp` CLI subcommand that starts the orchestration MCP server on stdio
- Phase 2: Add `roko acp` subcommand for Agent Communication Protocol (JSON-RPC 2.0 over stdio)

**Reference files**:
- `docs/12-interfaces/20-ide-integration-strategy.md` — 3-phase strategy with evaluation matrix
- `crates/roko-mcp-code/src/lib.rs` — existing MCP server pattern to follow
- `crates/roko-cli/src/main.rs` — CLI command registration
**Depends on**: None (MCP server is independent)
**Accept when**:
- [ ] MCP server starts and registers at least 5 orchestration tools
- [ ] Works with Claude Code, VS Code Copilot, or Cursor as MCP client
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'mcp.*orchestrat\|roko-mcp-orch' crates/ --include='*.rs' --include='*.toml' | grep -v target/
cargo test --workspace
```

**Priority**: P2

---

### UI-14: Spectre creature system not implemented (docs 10-12)

- [ ] Implement procedural creature generation and behavioral state animation

**Spec** (docs 10-12): Each agent has a procedurally generated Spectre creature. Doc 10 (`10-spectre-creature-visualization.md`): Creature generated deterministically from agent hash (`blake3::hash(agent_id)`) producing a dot-cloud geometry with spring physics. Six behavioral states (Focused, Exploring, Resting, Agitated, Collaborative, Dreaming) control animation parameters (tendril count, glow intensity, eye behavior, movement speed). Doc 11 (`11-spectre-rendering-per-interface.md`): Per-renderer implementations — TUI ASCII rasterization (braille characters + ROSEDUST colors), CLI inline (single-line ASCII), Web Portal WebGL (3D dot cloud), API JSON state endpoint. Doc 12 (`12-spectre-as-collective-display.md`): Multi-agent Spectre visualization — filament connections between collaborating agents, pheromone field visual overlay, breathing synchronization via Kuramoto coupling (`d(theta_i)/dt = omega_i + K/N * sum(sin(theta_j - theta_i))`), C-Factor encoded as collective harmony.

**Current code**: No Spectre creature types anywhere in `crates/`. The TUI has widgets at `crates/roko-cli/src/tui/widgets/` but no creature rendering widget. No `SpectreState`, `SpectreGeometry`, or `CreatureGenerator` structs.

**What to change**:
- Add `crates/roko-cli/src/tui/widgets/spectre.rs` with `SpectreWidget` for TUI ASCII rendering
- Add `SpectreState` enum with 6 behavioral states to `crates/roko-core/src/`
- Add `CreatureGenerator::from_hash(hash: &[u8; 32]) -> SpectreGeometry` for deterministic generation
- Wire into agents view in TUI to render creature next to agent status

**Reference files**:
- `crates/roko-cli/src/tui/widgets/` — existing TUI widgets (add `spectre.rs`)
- `crates/roko-cli/src/tui/views/` — agent views (render Spectre alongside agent data)
- `docs/12-interfaces/10-spectre-creature-visualization.md` — generation algorithm, 6 behavioral states
- `docs/12-interfaces/11-spectre-rendering-per-interface.md` — per-renderer specs
- `docs/12-interfaces/12-spectre-as-collective-display.md` — multi-agent visualization, Kuramoto coupling
**Depends on**: None
**Accept when**:
- [ ] `SpectreState` enum with 6 behavioral states derived from agent metrics
- [ ] Deterministic creature generation from agent hash
- [ ] TUI ASCII rendering via braille characters
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'Spectre\|SpectreState\|SpectreWidget' crates/ --include='*.rs' | grep -v target/
cargo test -p roko-cli
```

**Priority**: P3 (target-state visual feature)

---

### UI-15: Sonification system not implemented (doc 16)

- [ ] Implement ambient audio engine with behavioral state presets

**Spec** (doc 16 `16-sonification-reframed.md`): Eno-mandate ambient sonification. Five musical layers: drone (pad chord reflecting collective mood), rhythm (gate pass/fail pulses), melody (agent activity arpeggios), texture (pheromone field noise), accent (event stingers). Eight behavioral state presets (Focused, Exploring, Resting, Agitated, Collaborative, Dreaming, Stalled, Recovering) each select a harmonic vocabulary and tempo. Audio is optional, ambient, and can be fully disabled. No lifecycle audio — audio reflects current state, not lifecycle phases.

**Current code**: No sonification code anywhere in `crates/`. No audio dependencies. The TUI and CLI are visual-only.

**What to change**: This is Phase 3+ work. Stub `crates/roko-cli/src/audio.rs` with `AudioEngine` trait and `NullAudioEngine` default. Wire behavioral state events to audio engine interface. Actual audio synthesis deferred.

**Reference files**:
- `docs/12-interfaces/16-sonification-reframed.md` — five layers, eight presets, Eno mandate
**Depends on**: UI-14 (behavioral states shared with Spectre)
**Accept when**:
- [ ] `AudioEngine` trait defined with `play_preset(state: BehavioralState)` method
- [ ] `NullAudioEngine` default implementation (silent, always available)
- [ ] At least one real backend (e.g., `cpal`-based) that plays a drone
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'AudioEngine\|sonif\|audio' crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-cli
```

**Priority**: P3 (optional, deferred)

---

### UI-16: Accessibility compliance not implemented (doc 17)

- [x] Implement WCAG 2.1 AA accessibility targets for TUI and CLI

**Spec** (doc 17 `17-accessibility-and-current-status.md`): WCAG 2.1 AA compliance targets. Key requirements: keyboard navigation for all TUI interactions (no mouse-only features), screen reader support (semantic labels on TUI regions), reduced motion mode (disable animations, static Spectre), high-contrast color mode (alternative to ROSEDUST for accessibility), minimum 4.5:1 contrast ratios for text, aria-equivalent labels for TUI widgets. Port allocation documented for deployment accessibility.

**Current code**: TUI at `crates/roko-cli/src/tui/` has keyboard input handling in `input.rs` but no screen reader integration, no reduced motion mode, no high-contrast theme. `RosedustTheme` at `crates/roko-cli/src/tui/theme.rs` provides the dark theme but no light/high-contrast alternatives.

**What to change**:
- Add `high_contrast` theme variant to `crates/roko-cli/src/tui/theme.rs` alongside ROSEDUST
- Add `--reduced-motion` CLI flag to disable TUI animations
- Add `--high-contrast` CLI flag to switch to accessible color scheme
- Ensure all TUI widgets have semantic labels for potential screen reader integration

**Reference files**:
- `crates/roko-cli/src/tui/theme.rs` — `RosedustTheme` (add high-contrast variant)
- `crates/roko-cli/src/tui/input.rs` — keyboard input (verify all features keyboard-accessible)
- `crates/roko-cli/src/tui/app.rs` — `App` struct (add `reduced_motion`, `high_contrast` flags)
- `docs/12-interfaces/17-accessibility-and-current-status.md` — WCAG 2.1 AA targets
**Depends on**: None
**Accept when**:
- [x] High-contrast theme available via `--high-contrast` flag (also `ROKO_HIGH_CONTRAST` env var)
- [x] Reduced motion mode via `--reduced-motion` flag (also `ROKO_REDUCED_MOTION` env var)
- [ ] All TUI features accessible via keyboard alone
- [ ] `cargo test -p roko-cli`
**Verify**:
```bash
grep -rn 'high_contrast\|reduced_motion\|accessibility' crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-cli
```

**Priority**: P2

---

## Verify

```bash
cargo test -p roko-cli
cargo test -p roko-serve
```
