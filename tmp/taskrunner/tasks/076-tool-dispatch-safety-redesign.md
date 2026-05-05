# Task 076: Tool Dispatch Safety Redesign

```toml
id = 76
title = "Tool dispatch safety redesign: merge denylists, path confinement, env scrubbing, SafetyLayer required, truncated-arg detection"
track = "runner-hardening"
wave = "wave-1"
priority = "high"
blocked_by = []
touches = [
    "crates/roko-agent/src/dispatcher/mod.rs",
    "crates/roko-agent/src/safety/bash.rs",
    "crates/roko-agent/src/safety/mod.rs",
    "crates/roko-std/src/tool/builtin/bash.rs",
]
exclusive_files = [
    "crates/roko-std/src/tool/builtin/bash.rs",
    "crates/roko-agent/src/dispatcher/mod.rs",
]
estimated_minutes = 240
```

## Context

Three audit findings (S12.2, S12.3, S22.4) in tool dispatch that interact. All three must be
fixed together or the individual fixes leave the other gaps open.

**S12.2 — Bash tool has no process confinement.**
`crates/roko-std/src/tool/builtin/bash.rs:102` calls `cmd.current_dir(ctx.worktree())` — CWD
only. A command like `cat /etc/passwd` or `cat ~/.ssh/id_rsa` succeeds because CWD affects
relative paths only. The handler-level denylist (`DEFAULT_DENY_SUBSTRINGS`, lines 54-60) has 5
entries. The safety-layer `BashPolicy::with_defaults()` in
`crates/roko-agent/src/safety/bash.rs:90` has 10 entries covering a strict superset. Two
independent denylists diverge silently as the project evolves — whoever adds a rule to one
typically forgets the other. The spawned process also inherits the full parent environment
including any secrets the server loaded (API keys, `HOME`, `SSH_AUTH_SOCK`, etc.).

Redesign:
- Remove `DEFAULT_DENY_SUBSTRINGS` from the handler entirely. `BashPolicy` in the
  `SafetyLayer` is the single canonical denylist.
- Add optional `allowed_path_prefixes: Vec<String>` to `BashPolicy`. When non-empty, reject
  any command token starting with `/` that falls outside those prefixes. Wire it into
  `check_command_with_policy()`.
- In the bash handler, scrub the child process environment before spawning: strip
  `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`,
  `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, and any env var whose name contains `SECRET`, `TOKEN`,
  `PASSWORD`, or `KEY` (case-insensitive). Use `cmd.env_clear()` then re-add a minimal safe
  set: `PATH`, `HOME`, `TMPDIR`, `TERM`, `LANG`, `LC_ALL`, `USER`, `LOGNAME`.

**S12.3 — SafetyLayer API makes unguarded dispatchers easy to create.**
`ToolDispatcher::new()` at line 108 of `dispatcher/mod.rs` already initializes
`safety: SafetyLayer::with_defaults()`, so production dispatchers are guarded. However, tests
(lines 804-1199) all call `ToolDispatcher::new()` and are silently running under the default
`SafetyLayer` — which denies dangerous commands. Tests that call `bash` with a denied command
will fail for safety reasons, not because of the bug they're testing. The fix is to add a
`#[cfg(test)] new_unguarded()` constructor that makes "no safety intent" explicit for tests
that need it, and add a `SafetyLayer::permissive()` method for composability.

**S22.4 — Truncated tool-call arguments silently salvaged.**
`crates/roko-agent/src/translate/openai.rs:95-104` — when a backend hits its output token
limit mid-JSON, the unparseable `function.arguments` string is wrapped as
`{"__truncated": true, "raw": "..."}`. This synthetic object passes JSON schema validation in
the dispatcher (line 212 of `dispatcher/mod.rs`) because it is still a valid JSON object. The
handler then receives garbage args and fails with an opaque internal error. The model has no
way to know that truncation was the cause or that it should retry with a smaller payload.
The warning is in `tracing::warn!` and invisible without `RUST_LOG=warn`.

Redesign: detect `__truncated` at the top of `ToolDispatcher::dispatch()`, before validation,
and return a clear human-readable `ToolError::Other` that names the tool, the truncation cause,
and the raw fragment length. Do NOT modify `translate/openai.rs` — the sentinel is intentional
and the detection belongs in the dispatcher.

Sources:
- `tmp/infrastructure-audit.md` — S12.2 (§12.2), S12.3 (§12.3), S22.4 (§22.4)

## Background

Read these files before starting:

1. `crates/roko-std/src/tool/builtin/bash.rs` — handler-level `DEFAULT_DENY_SUBSTRINGS` (line
   54) and the check loop (lines 90-94); where `cmd.current_dir()` is set (line 102); how the
   subprocess is spawned (lines 100-114).
2. `crates/roko-agent/src/safety/bash.rs` — `BashPolicy` struct (line 65),
   `BashPolicy::with_defaults()` (line 89), `check_command_with_policy()` (line 152). Note the
   10-entry default denylist vs. the 5-entry handler denylist.
3. `crates/roko-agent/src/safety/mod.rs` — `SafetyLayer` struct (line 184), `bash_policy`
   field (line 186), `with_defaults()` (line 244), where the bash policy is invoked
   (`check_pre_execution()` at lines 360 and 539).
4. `crates/roko-agent/src/dispatcher/mod.rs` — `ToolDispatcher` struct (line 85), `new()`
   constructor (line 108), `dispatch()` pipeline (line 207). Note that `safety` is already
   `SafetyLayer` (not `Option<SafetyLayer>`).
5. `crates/roko-agent/src/translate/openai.rs` — `parse_calls()`, the truncation salvage path
   (lines 95-104): `serde_json::json!({ "__truncated": true, "raw": args_str })`.

Find all `ToolDispatcher::new()` construction sites:
```bash
grep -rn "ToolDispatcher::new\|\.with_safety" crates/ --include='*.rs' | grep -v target/
```

Confirm which env vars the bash handler currently inherits (it inherits all of the parent):
```bash
grep -n "env_clear\|env_remove\|env(" crates/roko-std/src/tool/builtin/bash.rs
# Should return nothing — env scrubbing is missing
```

## What to Change

### 1. Remove handler-level denylist from the bash builtin

In `crates/roko-std/src/tool/builtin/bash.rs`:

Delete the `DEFAULT_DENY_SUBSTRINGS` constant (lines 54-60) and the check loop (lines 90-94).
Replace them with a doc comment explaining that command safety is the `SafetyLayer`'s
responsibility:

```rust
// Command-level safety (denylist, path confinement) is enforced by the
// SafetyLayer's `BashPolicy` before this handler is invoked. No second-
// tier check here — a single authoritative policy avoids divergence.
```

### 2. Add env var scrubbing to the bash handler subprocess

In `crates/roko-std/src/tool/builtin/bash.rs`, in the `execute()` method, replace the bare
`cmd.current_dir(ctx.worktree())` setup block with:

```rust
let mut cmd = tokio::process::Command::new("bash");
cmd.arg("-c").arg(&command);
cmd.current_dir(ctx.worktree());
cmd.kill_on_drop(true);

// Scrub secrets from the child environment. Inheriting the full parent
// env exposes API keys, SSH agent sockets, and other credentials to
// agent-controlled commands.
cmd.env_clear();
let safe_env_keys = ["PATH", "HOME", "TMPDIR", "TEMP", "TMP", "TERM",
                     "LANG", "LC_ALL", "USER", "LOGNAME", "SHELL"];
for key in &safe_env_keys {
    if let Ok(val) = std::env::var(key) {
        cmd.env(key, val);
    }
}
```

This is defense-in-depth: even if the denylist fails to catch a command, leaked env vars cause
less damage.

### 3. Add `allowed_path_prefixes` to `BashPolicy`

In `crates/roko-agent/src/safety/bash.rs`:

Add a new field to `BashPolicy`:
```rust
/// Restrict absolute paths referenced in commands to these prefix
/// directories. Empty by default (no confinement). When non-empty,
/// any command token that looks like an absolute path and falls outside
/// all listed prefixes is rejected.
pub allowed_path_prefixes: Vec<String>,
```

Add a constructor helper and update `with_defaults()` to set it empty:
```rust
// In BashPolicy::with_defaults():
Self {
    deny_patterns,
    allow_prefixes: Vec::new(),
    max_command_len: 8192,
    allowed_path_prefixes: Vec::new(), // add this
}
```

Add the confinement check function and wire it into `check_command_with_policy()`:

```rust
/// Check that all absolute-path tokens in `command` start with one of
/// the allowed prefixes. Only tokens that look like plain paths (start
/// with `/`, contain no shell metacharacters `$`, `` ` ``, `|`, `;`,
/// `&`, `(`, `)`) are inspected. Shell syntax parsing is intentionally
/// NOT attempted — the denylist covers the worst cases; this is an
/// additional depth-of-defense layer.
pub fn check_path_confinement(command: &str, prefixes: &[String]) -> Result<(), ToolError> {
    if prefixes.is_empty() {
        return Ok(());
    }
    let metachar = |c: char| matches!(c, '$' | '`' | '|' | ';' | '&' | '(' | ')');
    for token in command.split_ascii_whitespace() {
        if token.starts_with('/') && !token.contains(metachar) {
            // Strip trailing punctuation that isn't part of the path
            let path = token.trim_end_matches(|c: char| matches!(c, ':' | ',' | ')' | ']'));
            if !prefixes.iter().any(|p| path.starts_with(p.as_str())) {
                return Err(ToolError::CommandNotAllowed(format!(
                    "absolute path `{path}` is outside the allowed prefixes"
                )));
            }
        }
    }
    Ok(())
}
```

In `check_command_with_policy()`, add after the denylist scan:
```rust
// Path confinement check (only fires when allowed_path_prefixes is set).
check_path_confinement(command, &policy.allowed_path_prefixes)?;
```

### 4. Add `SafetyLayer::permissive()` and `ToolDispatcher::new_unguarded()`

In `crates/roko-agent/src/safety/mod.rs`, add a `permissive()` constructor that creates a
layer with empty denylists and open policies — for use in tests that must bypass safety to
exercise handler behavior. Use the current `SafetyLayer` field shape; it is not the older
`hooks: Vec<_>`/optional-contract shape:

```rust
/// A [`SafetyLayer`] that passes all checks. For test use only.
///
/// Do NOT use in production code. Every production dispatcher should be
/// constructed with [`SafetyLayer::with_defaults()`] or an explicit policy.
#[cfg(test)]
#[must_use]
pub fn permissive() -> Self {
    use self::bash::BashPolicy;
    use self::git::GitPolicy;
    use self::network::NetworkPolicy;
    use self::path::PathPolicy;
    use self::scrub::ScrubPolicy;
    Self {
        bash_policy: BashPolicy {
            deny_patterns: Vec::new(),
            allow_prefixes: Vec::new(),
            max_command_len: usize::MAX,
            allowed_path_prefixes: Vec::new(),
        },
        git_policy: GitPolicy {
            protected_branches: Vec::new(),
            allow_force_push_on: Vec::new(),
            block_force_push: false,
            block_hard_reset_on_protected: false,
            block_branch_delete_protected: false,
        },
        network_policy: NetworkPolicy {
            allow_schemes: Vec::new(),
            allow_hosts: Vec::new(),
            deny_hosts: Vec::new(),
            block_private_networks: false,
        },
        path_policy: PathPolicy {
            deny_symlinks: false,
            prevent_escapes: false,
        },
        scrub_policy: ScrubPolicy {
            extra_patterns: Vec::new(),
            disable_defaults: true,
        },
        rate_limiter: None,
        safety_budget: None,
        role: "test".to_string(),
        contract: AgentContract::permissive("test"),
        warrant: None,
        role_tools: std::collections::HashMap::new(),
        role_overrides: std::collections::HashMap::new(),
        temporal_monitor: None,
    }
}
```

`role_tools`, `role_overrides`, and `temporal_monitor` are private to the `safety` module, so
the constructor belongs inside the existing `impl SafetyLayer`. Do not add public setters just
for this test helper. `GitPolicy::permissive()` and `NetworkPolicy::allow_all()` do not
currently exist; use the inline structs above. Do not modify `safety/git.rs` or
`safety/network.rs` just to add helpers for this constructor.

In `crates/roko-agent/src/dispatcher/mod.rs`, add a test-only constructor that makes the
"no confinement" intent explicit:

```rust
/// Construct a dispatcher that skips safety enforcement (test-only).
///
/// Use this in unit tests that need to call handlers directly without
/// interference from the default `BashPolicy` or network allowlist.
/// Production code must use [`ToolDispatcher::new`], which initializes
/// with [`SafetyLayer::with_defaults()`].
#[cfg(test)]
#[must_use]
pub fn new_unguarded(
    registry: Arc<dyn ToolRegistry>,
    resolver: Arc<dyn HandlerResolver>,
) -> Self {
    Self {
        registry,
        resolver,
        max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
        safety: SafetyLayer::permissive(),
        tool_cache: None,
        hook_chain: None,
        tool_selector: None,
    }
}
```

Migrate existing dispatcher unit tests that are about validation, permissions, truncation,
batching, cache behavior, or handler behavior from `ToolDispatcher::new()` to
`ToolDispatcher::new_unguarded()`. Keep or add tests that intentionally verify default safety
on `ToolDispatcher::new()` using the normal constructor. Do NOT change production sites.

### 5. Add `__truncated` detection at the top of `ToolDispatcher::dispatch()`

In `crates/roko-agent/src/dispatcher/mod.rs`, add a step **0** at the very top of the
`dispatch()` method, before the existing step **1. Validate args**:

```rust
// 0. Detect translator-salvaged truncated args (translate/openai.rs).
//    When the model hits its output token limit mid-JSON, the translator
//    wraps the unparseable fragment as {"__truncated": true, "raw": "..."}.
//    Return a clear error so the model can retry with a smaller payload.
if call.arguments.get("__truncated").and_then(|v| v.as_bool()) == Some(true) {
    let raw_fragment = call
        .arguments
        .get("raw")
        .and_then(|v| v.as_str())
        .unwrap_or("<empty>");
    let err = ToolError::Other(format!(
        "tool `{}` received truncated arguments ({} chars) — the model hit its output \
         token limit mid-call. Retry with a smaller payload or split into multiple calls. \
         Truncated fragment: {:.120}",
        call.name,
        raw_fragment.len(),
        raw_fragment,
    ));
    tracing::warn!(
        tool = %call.name,
        raw_len = raw_fragment.len(),
        "truncated tool-call arguments detected at dispatcher"
    );
    Self::emit_audit(
        ctx,
        &call,
        "args",
        "truncated",
        &json!({ "raw_len": raw_fragment.len(), "tool": call.name }),
    );
    Self::emit_terminal_audit(ctx, &call, &ToolResult::err(err.clone()), timeout_ms);
    return ToolResult::err(err);
}
```

The error text must be specific enough for the model to act on it. Do NOT return a generic
"invalid arguments" error — the model must know it was a truncation issue.

## Implementation Notes from Code Inspection

Actual runtime call chain for the safety redesign:

`roko-cli` / provider adapter -> `roko_agent::provider::build_tool_dispatcher()` ->
`ToolDispatcher::new(...).with_safety(current_safety_layer_or_default)` ->
`ToolLoop::run_inner()` -> `ToolDispatcher::dispatch()` -> `SafetyLayer::check_pre_execution()`
-> `bash::check_command_with_policy()` -> `roko-std` bash handler.

Mechanical details to avoid drift:

1. `ToolDispatcher::new()` already stores `safety: SafetyLayer`, not `Option<SafetyLayer>`.
   Update stale comments in `dispatch()` that still say "if a SafetyLayer is attached".
2. `BashPolicy` struct literals in `crates/roko-agent/src/safety/bash.rs` tests must gain
   `allowed_path_prefixes: Vec::new()`.
3. Add focused bash-policy tests:
   - empty `allowed_path_prefixes` allows `cat /tmp/file` subject to the denylist;
   - `allowed_path_prefixes = [worktree]` allows an absolute path under that prefix;
   - the same policy rejects `/etc/passwd`;
   - tokens containing shell metacharacters are not parsed as paths by this helper.
4. Add a `roko-std` bash-handler unit test that sets `OPENAI_API_KEY` and a fake
   `MY_SECRET_TOKEN`, runs `bash` with `env`, and verifies neither variable appears while
   allowed keys such as `PATH` are still present.
5. Add a dispatcher test that sends a `ToolCall` with
   `{"__truncated": true, "raw": "partial-json"}` and asserts the returned
   `ToolError::Other` names the tool and includes the raw fragment length. The test should
   verify validation was bypassed by using a registry/schema that would otherwise reject the
   synthetic object.

## What NOT to Do

- Do not try to parse shell ASTs or fully tokenize commands for path extraction. The simple
  whitespace-split heuristic is intentional — it is conservative and avoids false positives
  on complex expressions. The denylist already covers the highest-risk patterns.
- Do not change `translate/openai.rs`. The `__truncated` sentinel is correct and intentional.
  Detection belongs in the dispatcher, not the translator.
- Do not add env scrubbing via `SafetyLayer` or `BashPolicy` — it belongs in the handler's
  subprocess setup where `Command` is built. Keeping it there means it is unconditional and
  cannot be accidentally bypassed by omitting a safety layer.
- Do not remove `BashPolicy` from `SafetyLayer` or change its position in the pre-execution
  chain.
- Do not change `ToolDispatcher::new()` — it already initializes with
  `SafetyLayer::with_defaults()`. Only add `new_unguarded()` under `#[cfg(test)]`.
- Do not gate env scrubbing behind a `BashPolicy` flag. Env scrubbing should be unconditional
  whenever the bash handler spawns a subprocess.

## Wire Target

```bash
# Verify handler-level denylist is gone:
grep -n "DEFAULT_DENY_SUBSTRINGS" crates/roko-std/src/tool/builtin/bash.rs
# Must return nothing

# Verify env scrubbing is present:
grep -n "env_clear\|safe_env_keys" crates/roko-std/src/tool/builtin/bash.rs
# Must show the env_clear() call and safe_env_keys array

# Verify __truncated detection is wired:
grep -n "__truncated" crates/roko-agent/src/dispatcher/mod.rs
# Must show the new step-0 block

# Verify path confinement field exists:
grep -n "allowed_path_prefixes" crates/roko-agent/src/safety/bash.rs
# Must show the struct field and check function

# Verify new_unguarded exists and is cfg(test):
grep -n "new_unguarded\|cfg(test)" crates/roko-agent/src/dispatcher/mod.rs | head -10
# Must show the test-only constructor

# Build and test:
cargo build -p roko-agent -p roko-std
cargo test -p roko-agent -p roko-std -- --include-ignored
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `grep -n "DEFAULT_DENY_SUBSTRINGS" crates/roko-std/src/tool/builtin/bash.rs` — returns nothing
- [ ] `grep -n "env_clear" crates/roko-std/src/tool/builtin/bash.rs` — shows env scrubbing
- [ ] `grep -n "__truncated" crates/roko-agent/src/dispatcher/mod.rs` — shows step-0 detection block
- [ ] `grep -n "allowed_path_prefixes" crates/roko-agent/src/safety/bash.rs` — shows field + check function
- [ ] `grep -n "new_unguarded" crates/roko-agent/src/dispatcher/mod.rs` — shows `#[cfg(test)]` constructor
- [ ] `grep -n "SafetyLayer::permissive" crates/roko-agent/src/safety/mod.rs` — shows `#[cfg(test)]` method
- [ ] All production `ToolDispatcher::new()` call sites compile unchanged
- [ ] Test-only `ToolDispatcher::new()` sites that are not explicitly verifying default safety migrated to `new_unguarded()`
- [ ] Status Log documents: (a) path-extraction heuristic decision, (b) which env vars were
      chosen for the safe set and why, (c) that `SafetyLayer::permissive()` uses inline
      permissive `GitPolicy`/`NetworkPolicy` structs without adding unrelated helpers

## Status Log

| Time | Agent | Action |
|------|-------|--------|
