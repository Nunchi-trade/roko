# Task 088: ACP Architecture Sweep

```toml
id = 88
title = "Wire effort/thinking to dispatch, remove dead config fields, --global-config flag, workdir negotiation"
track = "ide-acp"
wave = "wave-2"
priority = "high"
blocked_by = [18, 19]
touches = [
    "crates/roko-acp/src/types.rs",
    "crates/roko-acp/src/session.rs",
    "crates/roko-acp/src/handler.rs",
    "crates/roko-acp/src/config.rs",
    "crates/roko-acp/src/runner.rs",
    "crates/roko-acp/src/bridge_events.rs",
    "crates/roko-acp/src/pipeline.rs",
    "crates/roko-cli/src/main.rs",
]
exclusive_files = ["crates/roko-acp/src/handler.rs"]
estimated_minutes = 240
```

## Context

This task is a REDESIGN, not a band-aid. It unifies four distinct ACP problems that were each
identified as dead weight or missing wiring during the infrastructure audit (S27.5, S28.2,
redesign-plan Phase 10.5, Phase 10.9). All four share `session.rs` and `handler.rs` as the
primary touch points, and they block each other in review if done separately.

Sources:
- `tmp/infrastructure-audit.md` §27.5 (workdir), §28.2 (effort theater, dead fields)
- `tmp/redesign-plan.md` Phase 10.5 (--global-config, configSources), Phase 10.9 (dead fields)

## Background

Read these files before making any changes:

1. `crates/roko-acp/src/session.rs` — `SessionConfigState` struct (lines 131–253):
   `temperament` (line 142) and `routing_mode` (line 144) are stored but never used by dispatch.
   `effort` (line 140) is stored and updated by `apply_config_option` but passed as `None` to
   all dispatch call sites. `review_strictness` IS used in `runner.rs` — do not touch it.

2. `crates/roko-acp/src/bridge_events.rs`:
   - Line 391: `routing_mode` copied into episode `extra` — only serialization use, not routing.
   - Line 463: `temperament: None` — explicitly discarded at dispatch.
   - Lines ~3751, 4175, 4215, 4243, 4280: all dispatch call sites pass `effort: None`.

3. `crates/roko-acp/src/runner.rs` — `PipelineConfig` struct (lines 47–57):
   `review_strictness` is actively used (lines 1209–1237) to branch between single/multi-role
   review. `effort` is in `SessionConfigState` but NOT in `PipelineConfig` — it never reaches
   the agent spawn.

4. `crates/roko-acp/src/handler.rs` — `initialize` handler (lines 140–166):
   Returns `InitializeResult` which currently has no `config_sources` or workdir diagnostics.

5. `crates/roko-acp/src/types.rs` — `InitializeResult` struct (lines 168–183):
   No field for surfacing which config files are active.

6. `crates/roko-acp/src/config.rs` — `AcpConfig` struct (all 67 lines):
   Has `workdir`, `config_path`, no `global_config_path`. `load_roko_config()` uses
   `load_config_with_options` — returns `Default` on failure (silently).

7. `crates/roko-cli/src/main.rs`:
   - `Command::Acp` variant (lines 553–566): already has `--workdir` and `--config` flags.
     A `--global-config` flag exists at line 598 already (check before adding). If present,
     wire it through; if not, add it.
   - Two dispatch sites for `Command::Acp`: lines ~1888–1895 (early exit) and ~2409–2414
     (match arm). Both must receive and forward `global_config`.

## What to Change

### 1. ALREADY DONE: `temperament` and `routing_mode` removed from `SessionConfigState`

**Status**: Completed on `wp-arch2`. Both fields have been fully removed from the codebase.
Confirmed via grep: no `temperament` or `routing_mode` struct fields, Default impl entries,
`from_roko_config_with_warnings` assignments, `apply_config_option` match arms, or
`bridge_events.rs` episode-extra serialization remain in `crates/roko-acp/src/session.rs`.

**No action needed** for this section. Skip to section 2.

### 2. Wire `effort` through to dispatch

**Why**: The Thinking/Effort dropdown in the ACP status bar is pure theater (S28.2). The user
selects "Deep" and nothing changes in model behavior because `effort` is stored in
`SessionConfigState` but `None` is passed at every dispatch call site.

**Design**: Do NOT add effort to `ModelCallRequest` in this task — that requires roko-core and
roko-agent changes that are out of scope. Instead, wire effort into the Claude CLI `--thinking`
flag path that already exists in `bridge_events.rs`. The effort-to-flag mapping is:
- `"low"` → skip `--thinking` flag
- `"medium"` → `--thinking auto`
- `"high"` → `--thinking extended` (or `--thinking high` depending on CLI version)
- `"max"` → `--thinking extended` with highest budget

In `crates/roko-acp/src/bridge_events.rs`, at each call site where `thinking_level: None` or
`effort: None` is passed to the dispatch struct, replace `None` with the session's `effort`
value from `config_state.effort`. Trace the field name — if it is `thinking_level`, map:
- `"low"` → `None`
- `"medium"` → `Some("auto".to_owned())`
- `"high"` | `"max"` → `Some("extended".to_owned())`

If the dispatch struct has no effort/thinking field yet, add one: `pub effort: Option<String>`
to whatever struct is passed from `bridge_events.rs` into the agent spawn. Do not modify
`ModelCallRequest` in roko-core (that is a larger task).

Document the remaining gap in `.roko/GAPS.md`: effort is passed to Claude CLI `--thinking` but
not yet to OpenAI-compat `reasoning_effort` or Gemini `thinking_budget`.

### 3. ALREADY DONE: `--global-config` flag and `AcpConfig.global_config_path`

**Status**: Completed on `wp-arch2`. Confirmed via grep:
- `crates/roko-cli/src/main.rs` line 600: `global_config: Option<PathBuf>` flag exists.
- Both dispatch sites (lines ~1892 and ~2413) forward `global_config` into
  `AcpConfig { global_config_path: global_config.clone(), ... }`.
- `crates/roko-acp/src/config.rs` line 18: `pub global_config_path: Option<PathBuf>` field exists.
- `with_global_config()` builder method (line 42), Default impl (line 35), and
  `load_roko_config()` merge logic (lines 85-91) are all present.

**No action needed** for this section. Skip to section 4.

### 4. ALREADY EXISTS — verify/fix only: `configSources` in `initialize` response

**Status**: Implemented on `wp-arch2`. Confirmed via grep:
- `crates/roko-acp/src/types.rs` line 185: `pub config_sources: Vec<String>` exists on
  `InitializeResult`.
- `crates/roko-acp/src/config.rs` line 54: `pub fn config_sources(&self) -> Vec<String>` exists.
- `crates/roko-acp/src/handler.rs` line 97: `sessions.config_sources = config.config_sources()`
  is called during init; line 184 populates the response field.
- `crates/roko-acp/src/session.rs` line 872: `SessionManager` stores `config_sources`.

**Remaining work** (from "Current Branch Status" gaps above):
- `configSources` currently reports prefixed strings (`global:`, `config:`, `workspace:`,
  `env:`), includes configured paths even when missing on disk, orders `ROKO_CONFIG` after
  workspace/explicit config, and does not report the implicit canonical global config loaded
  by core `merge_global`.
- `SessionManager.config_sources` is not recomputed after live reload.

These are the only items to address — do not reimplement the base structure.

### 5. Workdir negotiation — warn on missing `roko.toml` and escalate to global

**Why**: When Zed opens a project with no `roko.toml`, `roko acp --workdir .` silently uses
empty config, which falls through to static fallback (Anthropic/Sonnet hardcode) (S27.5).

In `crates/roko-acp/src/handler.rs`, in `run_acp_server()`, after config is loaded:

```rust
let workdir_toml = config.workdir.join("roko.toml");
if !workdir_toml.exists() {
    tracing::warn!(
        workdir = %config.workdir.display(),
        global_config = ?config.global_config_path,
        "workdir has no roko.toml — using global config only. \
         Add roko.toml or pass --global-config ~/.roko/config.toml in Zed settings."
    );
}
```

Optionally include a `no_workspace_config: bool` field in `InitializeResult` so the editor
can surface a "configure roko" prompt when true. This is optional — the `configSources` empty
array already signals the problem.

Do NOT attempt to "fix" the workdir automatically by walking up the directory tree. The workdir
is set by the editor and changing it silently would confuse users.

## What NOT to Do

- Do NOT remove `review_strictness` — it is actively used in `runner.rs` lines 1209–1237.
- Do NOT add `effort` to `ModelCallRequest` in `roko-core` — that is a separate larger task
  touching roko-agent provider backends.
- Do NOT modify `SessionNewParams` or `SessionLoadParams` — those belong to task 018 (ACP
  session params).
- Do NOT change the JSON-RPC protocol version or add required (non-optional) fields to
  `InitializeResult` — this would break existing Zed extensions.
- Do NOT add `temperament` or `routing_mode` to config options in the UI — they are being
  removed. If they need to come back, they need actual wiring first.
- Do NOT walk up ancestor directories to find `roko.toml` in the ACP handler — use the workdir
  as given, warn if missing, let the user configure the path.

## Current Branch Status - 2026-05-05

Status: **mostly implemented, with follow-up gaps**.

Implemented on `wp-arch2`:
- Removed ACP `SessionConfigState.temperament` and `routing_mode`; `routing_mode` is no
  longer serialized into ACP episode extras.
- Added and wired `roko acp --global-config`; both CLI ACP launch sites forward it into
  `AcpConfig.global_config_path`.
- Added explicit ACP global-config merge for providers/models and selected agent defaults.
  Local/editor config keeps precedence; inherited providers/models fill missing keys.
- Added `InitializeResult.configSources` and populates it from ACP config source metadata.
- Added best-effort ACP config live watch. ACP reloads config on the next inbound request and
  revalidates existing session provider/model state.
- Wired session `effort` from config/session updates into ACP dispatch by setting effective
  `agent.default_effort` before `ModelCallService` dispatch and passing it to Anthropic
  dispatch/routing.
- Removed silent config-update no-ops for unknown option IDs and invalid provider/model/effort
  values.
- Related IDE batches are present: `session/new` model/provider/effort params, IndexMap
  provider/model ordering, MCP status notifications, `discoveryTimeoutMs`, provider `ready`,
  bare-mode command categories, and max-output surfacing.

Remaining gaps:
- `configSources` currently reports prefixed strings (`global:`, `config:`, `workspace:`,
  `env:`), includes configured paths even when missing, orders `ROKO_CONFIG` after
  workspace/explicit config, and does not report the implicit canonical global config loaded
  by core `merge_global`.
- ACP live watch watches explicit `--global-config`, explicit `--config` or
  `{workdir}/roko.toml`, and `ROKO_CONFIG`; it does not watch implicit
  `~/.roko/config.toml` unless passed via `--global-config`.
- Config reload is request-driven only. It does not proactively send `config_option_update`
  or refreshed `configSources` notifications to IDE clients.
- `SessionManager.config_sources` is not recomputed after live reload.
- Missing-workdir warning exists, but should include `global_config_path` and clearer guidance
  for Zed users.
- Effort is now dispatched through provider config / `AgentOptions`; do not describe it as an
  ACP-local hardcoded `low -> none`, `medium -> auto`, `high/max -> extended` mapping unless
  that mapping is added explicitly.

## Worker 17 Mechanical Notes

### Current branch snapshot (2026-05-05)

The code has moved past parts of the "Remaining gaps" list above. Verify the
current state before changing anything:

- `SessionConfigState` in `crates/roko-acp/src/session.rs` has
  `effort`, `review_strictness`, `workflow`, etc. It no longer has
  `temperament` or `routing_mode`.
- `bridge_events.rs` captures `session.config_state.effort` as
  `session_effort`, passes it into both
  `run_anthropic_cognitive_task(...)` and
  `run_openai_compat_cognitive_task(...)`, and uses
  `config_with_session_effort()` to set `config.agent.default_effort`.
- `ModelCallRequest` is still built without an explicit effort field in
  `model_call_request_from_acp_messages()`. That is intentional for this task;
  effort flows through `RokoConfig.agent.default_effort` and provider options.
- `AcpConfig::config_sources()` now includes only existing files and includes
  the implicit canonical global config as `default:<path>` when no explicit
  `--global-config` is set.
- `handler.rs::run_acp_server_with_transport()` updates
  `sessions.config_sources` after request-driven config reload and sends
  `config_option_update` notifications for active sessions.
- The missing-workdir warning already mentions whether explicit or implicit
  global config is being used and gives Zed-oriented guidance.
- `InitializeResult.config_sources` currently has
  `skip_serializing_if = "Vec::is_empty"` in `types.rs`. If the acceptance
  criterion is that `"configSources"` is always present, even as `[]`, remove
  that skip attribute and update the initialize serialization test.

### Mechanical verification/fix order

1. Run the greps below first. If they match the snapshot above, do not rewrite
   already-implemented sections.
2. In `session.rs`, only modify `SessionConfigState` if `temperament` or
   `routing_mode` have reappeared as session fields or config options. Do not
   remove `roko_core::config::AgentConfig.temperament`; that is a core provider
   construction setting, not the dead ACP status-bar option.
3. In `bridge_events.rs`, verify every production dispatch path takes
   `&session_effort` or config with `default_effort` set. Ignore `effort: None`
   occurrences inside test setup structs unless a production call also passes
   `None`.
4. In `config.rs`, preserve the existing source ordering:
   explicit global -> project/explicit config -> `ROKO_CONFIG` -> implicit
   `~/.roko/config.toml`. Do not include missing paths.
5. In `handler.rs`, preserve request-driven reload. If adding proactive reload
   notifications, keep them optional and do not block JSON-RPC message handling
   on file watching.
6. If changing `AcpConfig::load_roko_config()`, note that it currently
   `unwrap_or_default()`s loader errors. Replacing that with a hard startup
   error changes editor launch behavior and must return a JSON-RPC-visible
   startup failure, not a panic or silent process exit.

### Tests to add or update

- `crates/roko-acp/src/config.rs`: tests for `config_sources()` covering
  explicit global, workspace config, `ROKO_CONFIG`, implicit global, and missing
  files not being reported.
- `crates/roko-acp/src/handler.rs`: initialize response includes
  `configSources`, and a reload updates `sessions.config_sources` before the
  next initialize/session response path that reads it.
- `crates/roko-acp/src/bridge_events.rs`: a focused unit test that sets
  `session.config_state.effort = "high"` and asserts the config handed to
  dispatch has `agent.default_effort == "high"` or that the generated
  routing/thinking context carries `"high"`.
- Existing tests asserting episode extras must keep verifying that
  `routing_mode` is absent.

### Grep checks

```bash
rg -n 'temperament|routing_mode' crates/roko-acp/src/session.rs
# Expected: no SessionConfigState fields or config-option arms.

rg -n 'routing_mode' crates/roko-acp/src/bridge_events.rs
# Expected: tests/comments only, no episode-extra serialization.

rg -n 'session_effort|config_with_session_effort|default_effort|effort: None' \
  crates/roko-acp/src/bridge_events.rs
# Expected: production dispatch uses session_effort/config_with_session_effort;
# effort: None may remain in tests only.

cargo run -p roko-cli -- acp --help | rg -- '--global-config'
```

## Wire Target

```bash
# Verify --global-config flag exists and is accepted
cargo run -p roko-cli -- acp --help | grep global-config

# Test initialize response includes configSources
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}' | \
  cargo run -p roko-cli -- acp --workdir . 2>/dev/null | python3 -m json.tool
# Expect: "configSources" array in the response

# Test missing roko.toml warning
cargo run -p roko-cli -- acp --workdir /tmp 2>&1 | grep -i "no roko.toml"
# Expect: warning line mentioning workdir has no roko.toml

# Verify temperament and routing_mode fields are gone
grep -n 'temperament\|routing_mode' crates/roko-acp/src/session.rs
# Expect: no struct field declarations (comments OK if explanatory)
```

## Verification

- [ ] `cargo build --workspace` — clean build
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `temperament` field is absent from `SessionConfigState` and its `Default` impl
- [ ] `routing_mode` field is absent from `SessionConfigState` and its `Default` impl
- [ ] `routing_mode` is NOT copied into episode `extra` in `bridge_events.rs`
- [ ] `effort` from `config_state` is forwarded to agent spawn (not `None`)
- [ ] `roko acp --global-config /path` starts without error
- [ ] `initialize` JSON response includes `"configSources"` key (may be empty array)
- [ ] ACP startup in a directory without `roko.toml` logs a `warn!` line
- [ ] No new `unwrap()` calls in the changed paths

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | wp-arch2 audit | Mostly implemented. Captured remaining `configSources`, live-watch, reload-notification, warning-detail, and effort-mapping gaps above. |
