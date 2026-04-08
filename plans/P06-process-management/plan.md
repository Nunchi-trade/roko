---
plan: P06-process-management
depends_on: []
parallel_with: []
crates_touched: [roko-agent, roko-cli]
estimated_tasks: 7
estimated_parallel_width: 2
estimated_minutes: 70
---

# P06: Wire Process Management into Agents and CLI

## Context

`crates/roko-agent/src/process/` contains fully-implemented primitives that
are **never called** by the agents that need them:

| Module | Primitive | Status |
|--------|-----------|--------|
| `registry.rs` | `register_spawned_pid`, `cleanup_orphaned_agents`, `reap_orphaned_children` | implemented, unwired |
| `kill.rs` | `kill_tree` (SIGTERM→SIGKILL escalation) | implemented, unwired |
| `group.rs` | `set_process_group` (setpgid) | implemented, unwired |
| `stderr.rs` | `classify_benign_stderr`, `benign_stderr_warn_once` | implemented, unwired |
| `env.rs` | `AgentEnv`, `apply_agent_env` | implemented, unwired |

`roko-core` has a `GracefulShutdown` coordinator (`crates/roko-core/src/shutdown.rs`)
that is never registered in the CLI entrypoint.

Per-process resource limits (CPU/memory caps via `setrlimit`) do not exist yet.

This plan wires all existing primitives into `ExecAgent`, `ClaudeCliAgent`, and
the CLI `main()`, then adds the missing resource-limit layer.

## What already exists (do NOT reimplement)

- `crates/roko-agent/src/process/registry.rs` — global PID set + disk persistence
- `crates/roko-agent/src/process/kill.rs` — `kill_tree` with configurable grace periods
- `crates/roko-agent/src/process/group.rs` — `set_process_group` / `kill_process_group`
- `crates/roko-agent/src/process/stderr.rs` — warn-once benign stderr suppression
- `crates/roko-agent/src/process/env.rs` — `AgentEnv` / `apply_agent_env`
- `crates/roko-core/src/shutdown.rs` — `GracefulShutdown` drain coordinator
- `crates/roko-agent/tests/process_integration.rs` — integration test for `kill_tree`

## Tasks summary

| # | Title | Files |
|---|-------|-------|
| T1 | Wire `set_process_group` + `register_spawned_pid` into `ExecAgent::run` | `exec.rs` |
| T2 | Wire `kill_tree` timeout path in `ExecAgent::run` | `exec.rs` |
| T3 | Wire `classify_benign_stderr` + `benign_stderr_warn_once` into `ExecAgent::run` | `exec.rs` |
| T4 | Wire process group, PID registry, benign stderr into `ClaudeCliAgent::run` | `claude_cli_agent.rs` |
| T5 | Add `ResourceLimits` struct + `apply_resource_limits` to `process/` | `process/limits.rs`, `process/mod.rs` |
| T6 | Wire orphan reaper + `GracefulShutdown` hook into CLI `main()` | `roko-cli/src/main.rs` |
| T7 | Add integration test: orphan reaper cleans up PID file on restart | `roko-agent/tests/` |

## References

- `crates/roko-agent/src/exec.rs` — `ExecAgent` (needs T1–T3)
- `crates/roko-agent/src/claude_cli_agent.rs` — `ClaudeCliAgent` (needs T4)
- `crates/roko-agent/src/process/mod.rs` — process module re-exports (needs T5)
- `crates/roko-cli/src/main.rs` — CLI entrypoint (needs T6)
- `crates/roko-agent/tests/process_integration.rs` — existing integration test
- `crates/roko-core/src/shutdown.rs` — `GracefulShutdown`
- Mori ref: `apps/mori/src/agent/connection.rs:2624–2655` (PID registry + stderr)
- Mori ref: `apps/mori/src/agent/connection.rs:826` (`classify_known_warning`)
