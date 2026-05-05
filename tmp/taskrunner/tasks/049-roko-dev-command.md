# Task 049: Implement `roko dev` Command with PID File Management

```toml
id = 49
title = "Add roko dev command to replace roko-dev-full shell alias"
track = "infrastructure"
wave = "wave-2"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-cli/src/main.rs",
    "crates/roko-cli/src/commands/mod.rs",
    "crates/roko-cli/src/commands/dev.rs",
    "crates/roko-cli/src/commands/server.rs",
    "crates/roko-cli/tests/dev_command.rs",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-core/src/workspace.rs",
]
exclusive_files = []
estimated_minutes = 180
```

## Context

The audit (S1) identified that the current dev workflow uses a shell alias
(`roko-dev-full`) that causes triple process spawns, port conflicts, zombie processes,
and no graceful shutdown. The fix is a proper `roko dev` CLI command.

`roko-fs/src/layout.rs` already defines a `pid_file()` method on `RokoLayout`, confirming PID
file infrastructure was planned but never wired. Current taskrunner path policy prefers
`roko_core::Workspace` for new workspace-bound runtime paths, so add a `Workspace::pid_file()`
or `Workspace::serve_pid_path()` accessor first. Use `RokoLayout::pid_file()` only as a
temporary documented bridge if the Workspace accessor is not available yet.

## Background

Read:
- `crates/roko-cli/src/main.rs` — CLI command registration
- `crates/roko-cli/src/commands/server.rs` — existing `roko serve` / `roko up` commands
- `crates/roko-serve/src/lib.rs` — `effective_port()`, `resolve_bind_with_port_env()`
- `crates/roko-core/src/workspace.rs` — preferred place for the PID path accessor
- `crates/roko-fs/src/layout.rs` — existing `pid_file()` method, temporary reference only

Current call chain to mirror:
- `crates/roko-cli/src/main.rs`: `Command::Serve` resolves the workdir, acquires
  `workspace_lock`, loads config with `resolve_config_for_workdir`, builds
  `RepoRegistry`, creates a `state_hub`, builds `RokoCliRuntime`, bootstraps
  `RokoBootstrap`, applies CLI bind/port/terminal overrides, and constructs
  `roko_serve::ServerBuildConfig`.
- `crates/roko-cli/src/commands/server.rs`: `cmd_up` is the existing example for
  `ServerBuilder::start_background()`, waiting on `tokio::signal::ctrl_c()`,
  calling `state.cancel.cancel()`, and awaiting the server `JoinHandle`.
- `crates/roko-serve/src/lib.rs`: `ServerBuildConfig::effective_bind`,
  `effective_port`, and `effective_addr` are currently private. `roko dev`
  needs the same resolved address for port probing, so either make those methods
  public or add a narrow public helper returning `(bind, port)` from the same
  logic. Do not duplicate the `[server]` vs `[serve]` port fallback or `PORT`
  environment behavior in the CLI.

## What to Change

### 1. Add `roko dev` subcommand

Add a `Dev` variant to the CLI command enum in `main.rs`:

```rust
/// Start the dev environment (serve + optional demo frontend)
Dev {
    /// Skip the demo frontend dev server
    #[arg(long)]
    no_frontend: bool,
},
```

Dispatch it in the main command match as:
`Command::Dev { no_frontend } => commands::dev::cmd_dev(cli, no_frontend).await`.
Also add `pub mod dev;` to `crates/roko-cli/src/commands/mod.rs`.

### 2. Implement dev orchestrator

Create `crates/roko-cli/src/commands/dev.rs`:

The dev command should:
1. **Resolve workdir/config exactly like `Command::Serve`**: use
   `resolve_workdir`, `resolve_config_for_workdir`, `RepoRegistry::load`,
   `RokoCliRuntime::new_with_state_hub`, and `RokoBootstrap` with the same
   `require_workspace`, `require_provider`, and `acquire_lock` values as serve.
2. **Check existing PID file**: read `.roko/serve.pid`. If it contains a live
   process, send SIGTERM to that PID and wait up to 5 seconds; send SIGKILL only
   after that timeout and only for a PID that came from this PID file. If the
   file is missing, malformed, or points at a dead process, remove it.
3. **Probe the resolved listen address**: after stale-PID cleanup and before
   writing the new PID file, attempt to bind the resolved address and immediately
   drop the listener. If the port is still in use and there was no live PID file
   owner, return a clear error that names the address and says it is owned by an
   unknown process.
4. **Write PID file**: atomically write the current `roko dev` process PID to
   `.roko/serve.pid`. `ServerBuilder::start_background()` runs serve in-process
   on a Tokio task, so the PID file should not attempt to identify a separate
   serve child process.
5. **Start `roko serve`**: use `ServerBuilder::start_background()` with the same
   `ServerBuildConfig` built by the serve path.
6. **Optionally start frontend**: `npm run dev` in `demo/demo-app/` if `!no_frontend`
   and `demo/demo-app/package.json` exists. If the directory is missing, log/print
   a skip message and keep serve running.
7. **Signal handling**: On SIGINT/SIGTERM:
   - Send SIGTERM to frontend process
   - Trigger serve cancellation token
   - Wait up to 5 seconds for clean shutdown
   - Remove PID file
   - Exit

Key design:
- Build watching is NOT included (users run `cargo watch` separately or rely on
  `cargo build` in another terminal). This avoids the triple-spawn problem.
- The PID file prevents multiple `roko dev` instances from fighting over the port.

### 3. PID file management

Add a path accessor to `crates/roko-core/src/workspace.rs` and use it from new
CLI code:

```rust
impl Workspace {
    pub fn serve_pid_file(&self) -> PathBuf {
        self.roko_dir().join("serve.pid")
    }
}
```

Keep read/write/remove helpers in `commands/dev.rs` unless there is already a
core helper pattern to follow. The helpers should use the Workspace accessor and
`roko_core::io::atomic_write`:

```rust
pub fn write_pid_file(workdir: &Path) -> anyhow::Result<()> {
    let pid_path = Workspace::open_or_create(workdir)?.serve_pid_file();
    roko_core::io::atomic_write(&pid_path, std::process::id().to_string().as_bytes())
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

pub fn read_pid_file(workdir: &Path) -> anyhow::Result<Option<u32>> {
    let pid_path = Workspace::open_or_create(workdir)?.serve_pid_file();
    match std::fs::read_to_string(&pid_path) {
        Ok(s) => Ok(s.trim().parse().ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::new(e).context(format!("read {}", pid_path.display()))),
    }
}

pub fn remove_pid_file(workdir: &Path) -> anyhow::Result<()> {
    let pid_path = Workspace::open_or_create(workdir)?.serve_pid_file();
    match std::fs::remove_file(&pid_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("remove {}", pid_path.display()))),
    }
}
```

### 4. Process and port conflict detection

Add small, unit-testable helpers in `commands/dev.rs`:
- `read_pid_file(path) -> io::Result<Option<u32>>`
- `write_pid_file(path, pid: u32) -> io::Result<()>`
- `remove_pid_file(path) -> io::Result<()>`
- `process_is_alive(pid: u32) -> bool` using `libc::kill(pid as i32, 0)` on Unix
- `terminate_pid(pid: u32, timeout: Duration) -> anyhow::Result<()>`
- `probe_addr_available(bind: &str, port: u16) -> anyhow::Result<()>`

Before starting serve, check the resolved bind/port:

```rust
match TcpListener::bind(format!("{bind}:{port}")) {
    Ok(_) => { /* port is free, proceed */ },
    Err(_) => {
        // Check PID file for stale process
        if let Some(pid) = read_pid_file(workdir)? {
            eprintln!("Port {port} in use by PID {pid}. Stopping prior roko dev process...");
            // Send SIGTERM, wait up to 5s, then retry the probe
        } else {
            anyhow::bail!("Port {port} already in use by unknown process. \
                           Kill it or use --port to pick a different port.");
        }
    }
}
```

Use the resolved bind address, not a hard-coded `127.0.0.1`, unless the resolved
bind is `localhost` and you intentionally normalize it to loopback for the
probe. Never kill an unknown process just because it owns the port.

### 5. Tests to Add

Add `crates/roko-cli/tests/dev_command.rs` or unit tests in `commands/dev.rs`
for the mechanical helpers:
- PID write/read/remove roundtrip writes only `.roko/serve.pid`.
- Missing PID file returns `Ok(None)`.
- Malformed PID file returns `Ok(None)` and is removed or ignored consistently.
- A dead PID file is cleaned up without attempting to kill an unrelated process.
- Port probe returns an error while a test `TcpListener` holds the address.

If adding an end-to-end CLI test, spawn `roko dev --no-frontend` as a child,
poll `/api/health`, terminate the child, and assert `.roko/serve.pid` is removed.
Do not use fixed sleeps as the only readiness check.

## What NOT to Do

- Don't include `cargo watch` — that is a separate concern. The dev command manages
  serve + frontend only.
- Don't add SO_REUSEADDR — the PID file approach is cleaner.
- Don't change `roko serve` behavior — `roko dev` is a wrapper that adds PID
  management and frontend spawning.
- Don't make the frontend mandatory — it should be optional via `--no-frontend`.
- Don't kill processes discovered by port scanning. Only terminate a live PID
  read from `.roko/serve.pid`.
- Don't write the PID file before the port probe succeeds.
- Don't leave `.roko/serve.pid` behind on server start failure, frontend start
  failure, Ctrl-C, SIGTERM, or server task exit.

## Wire Target

```bash
rm -f .roko/serve.pid
cargo run -p roko-cli -- dev --no-frontend > /tmp/roko-dev.log 2>&1 &
dev_pid=$!
for i in {1..40}; do
  curl -sf http://127.0.0.1:6677/api/health && break
  sleep 0.25
done
test -f .roko/serve.pid
kill -TERM "$dev_pid"
wait "$dev_pid" || true
test ! -f .roko/serve.pid
```

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`
- [ ] `roko dev --no-frontend` starts serve and creates PID file
- [ ] `curl -sf http://127.0.0.1:6677/api/health` succeeds while `roko dev` is running
- [ ] Ctrl-C/SIGTERM cleans up PID file
- [ ] Running `roko dev` twice terminates the prior PID-file-owned dev process
      or prints a clear unknown-port-owner error; it must not panic
- [ ] `rg -n "cmd_dev|serve_pid_file|ServerBuildConfig::.*effective" crates/roko-cli/src crates/roko-core/src crates/roko-serve/src`
      shows the CLI is wired through the intended helpers

## Status Log

| Time | Agent | Action |
|------|-------|--------|
