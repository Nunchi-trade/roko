# Daemon Mode: launchd (macOS)

> On macOS, Roko can run as a persistent background daemon managed by launchd. The
> `roko daemon --install` command generates a launchd plist, installs it to
> `~/Library/LaunchAgents/`, and starts the service. This document covers the plist
> generation, lifecycle commands, IPC over Unix socket, log management, and the daemon's
> event subscription model.


> **Implementation**: Specified

---

## Overview

Daemon mode transforms Roko from a CLI tool you run manually into a persistent background
service that:

- **Watches repositories** for changes (file system events, git push webhooks, cron schedules)
- **Triggers plan execution** automatically when PRDs change or on schedule
- **Maintains state** across reboots (launchd restarts it automatically)
- **Accepts commands** via a Unix domain socket IPC interface
- **Streams events** to connected clients (TUI, web dashboard, CI hooks)

On macOS, this uses launchd — the native service manager. The `roko daemon` subcommand handles
all lifecycle operations: install, start, stop, restart, uninstall, status, and log viewing.

---

## The `roko daemon` Subcommand

```
roko daemon [COMMAND]

Commands:
  install      Generate launchd plist and load it (starts on login)
  uninstall    Unload and remove launchd plist
  start        Start the daemon (if installed but not running)
  stop         Stop the daemon
  restart      Stop and start the daemon
  status       Show daemon status (running, PID, uptime, subscriptions)
  logs         Tail daemon logs (stdout + stderr)
  send <cmd>   Send a command to the running daemon via IPC
```

### Installation

```bash
$ roko daemon install

[1/4] Generating launchd plist...
[2/4] Writing to ~/Library/LaunchAgents/dev.nunchi.roko.plist
[3/4] Loading plist (launchctl load)...
[4/4] Verifying daemon started...

Roko daemon installed and running.
  PID:      12345
  Socket:   /tmp/roko-daemon.sock
  Logs:     ~/.local/state/roko/daemon.log
  Config:   ~/.config/roko/config.toml

Daemon will start automatically on login.
To stop: roko daemon stop
To uninstall: roko daemon uninstall
```

---

## launchd Plist

The generated plist lives at `~/Library/LaunchAgents/dev.nunchi.roko.plist`. It uses the
user-level LaunchAgents directory (not system-level LaunchDaemons), so no root access is
required.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- Service identifier -->
    <key>Label</key>
    <string>dev.nunchi.roko</string>

    <!-- Binary and arguments -->
    <key>ProgramArguments</key>
    <array>
        <string>/Users/USERNAME/.cargo/bin/roko</string>
        <string>daemon</string>
        <string>run</string>
        <string>--socket</string>
        <string>/tmp/roko-daemon.sock</string>
    </array>

    <!-- Start on login (persistent daemon) -->
    <key>RunAtLoad</key>
    <true/>

    <!-- Restart on crash (up to 10 times, then back off) -->
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>

    <!-- Throttle restarts: minimum 10 seconds between restart attempts -->
    <key>ThrottleInterval</key>
    <integer>10</integer>

    <!-- Log output -->
    <key>StandardOutPath</key>
    <string>/Users/USERNAME/.local/state/roko/daemon.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/USERNAME/.local/state/roko/daemon.stderr.log</string>

    <!-- Working directory -->
    <key>WorkingDirectory</key>
    <string>/Users/USERNAME</string>

    <!-- Environment variables -->
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>HOME</key>
        <string>/Users/USERNAME</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/Users/USERNAME/.cargo/bin</string>
    </dict>

    <!-- Resource limits -->
    <key>SoftResourceLimits</key>
    <dict>
        <key>NumberOfFiles</key>
        <integer>4096</integer>
    </dict>

    <!-- Nice value: slightly lower priority than interactive processes -->
    <key>Nice</key>
    <integer>5</integer>
</dict>
</plist>
```

### Plist Generation in Rust

The daemon install command generates this plist dynamically, substituting the current user's
home directory, the resolved binary path, and any configuration overrides:

```rust
/// Generate launchd plist content for the daemon.
fn generate_plist(config: &DaemonConfig) -> String {
    let binary_path = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("roko"));
    let home = std::env::var("HOME")
        .unwrap_or_else(|_| String::from("/tmp"));
    let state_dir = format!("{}/.local/state/roko", home);

    // Ensure state directory exists
    std::fs::create_dir_all(&state_dir).ok();

    format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.nunchi.roko</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary}</string>
        <string>daemon</string>
        <string>run</string>
        <string>--socket</string>
        <string>{socket}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardOutPath</key>
    <string>{state_dir}/daemon.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>{state_dir}/daemon.stderr.log</string>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>{log_level}</string>
        <key>HOME</key>
        <string>{home}</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:{home}/.cargo/bin</string>
    </dict>
    <key>SoftResourceLimits</key>
    <dict>
        <key>NumberOfFiles</key>
        <integer>4096</integer>
    </dict>
    <key>Nice</key>
    <integer>5</integer>
</dict>
</plist>"#,
        binary = binary_path.display(),
        socket = config.socket_path.display(),
        state_dir = state_dir,
        home = home,
        log_level = config.log_level,
    )
}
```

---

## Lifecycle Commands

### Install and Load

```rust
/// Install the launchd plist and start the daemon.
fn daemon_install(config: &DaemonConfig) -> Result<()> {
    let plist_path = plist_path()?;

    // Check if already installed
    if plist_path.exists() {
        eprintln!("Daemon already installed at {}", plist_path.display());
        eprintln!("Use 'roko daemon restart' to restart, or 'roko daemon uninstall' first.");
        return Ok(());
    }

    // Generate and write plist
    let content = generate_plist(config);
    std::fs::write(&plist_path, &content)?;

    // Load the plist (starts the daemon)
    let status = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()?;

    if !status.success() {
        anyhow::bail!("launchctl load failed with status {}", status);
    }

    // Wait briefly and verify it started
    std::thread::sleep(std::time::Duration::from_secs(2));
    daemon_status(config)?;

    Ok(())
}

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents/dev.nunchi.roko.plist"))
}
```

### Uninstall

```rust
/// Unload and remove the launchd plist.
fn daemon_uninstall() -> Result<()> {
    let plist_path = plist_path()?;

    if !plist_path.exists() {
        eprintln!("Daemon not installed.");
        return Ok(());
    }

    // Unload the plist (stops the daemon)
    let _ = std::process::Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&plist_path)
        .status();

    // Remove the plist file
    std::fs::remove_file(&plist_path)?;

    eprintln!("Daemon uninstalled.");
    Ok(())
}
```

### Status

```bash
$ roko daemon status

Roko Daemon Status
  State:          running
  PID:            12345
  Uptime:         3d 14h 22m
  Socket:         /tmp/roko-daemon.sock
  Config:         ~/.config/roko/config.toml
  Log (stdout):   ~/.local/state/roko/daemon.stdout.log
  Log (stderr):   ~/.local/state/roko/daemon.stderr.log

Subscriptions:
  ~/dev/project-a    cron: */30 * * * *    last: 2h ago (success)
  ~/dev/project-b    watch: .roko/prd/     last: 15m ago (running)
  ~/dev/project-c    webhook: POST /hook   last: never
```

---

## IPC: Unix Domain Socket

The daemon exposes a Unix domain socket at `/tmp/roko-daemon.sock` for command-and-control from
the CLI, TUI, or other local processes. The protocol is newline-delimited JSON:

```rust
/// Commands the daemon accepts over IPC.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum DaemonCmd {
    /// Get daemon status
    Status,
    /// Trigger a plan run for a specific repo
    RunPlan { repo_path: PathBuf, plan_dir: Option<String> },
    /// Add a repository subscription
    Subscribe { repo_path: PathBuf, schedule: SubscriptionSchedule },
    /// Remove a repository subscription
    Unsubscribe { repo_path: PathBuf },
    /// List all subscriptions
    ListSubscriptions,
    /// Stream events (daemon sends events until client disconnects)
    StreamEvents { filter: Option<EventFilter> },
    /// Graceful shutdown
    Shutdown,
}

/// Responses from the daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum DaemonResponse {
    Ok { message: String },
    Error { message: String },
    Status(DaemonStatus),
    Subscriptions(Vec<Subscription>),
    Event(DaemonEvent),
}
```

### IPC Server Implementation

```rust
/// Start the IPC server on a Unix domain socket.
async fn start_ipc_server(
    socket_path: &Path,
    state: Arc<DaemonState>,
) -> Result<()> {
    // Remove stale socket file
    let _ = std::fs::remove_file(socket_path);

    let listener = tokio::net::UnixListener::bind(socket_path)?;

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            handle_ipc_connection(stream, state).await;
        });
    }
}
```

### CLI IPC Client

```bash
# Send a command to the running daemon
$ roko daemon send status
# Connects to /tmp/roko-daemon.sock, sends {"type":"Status"}, prints response

$ roko daemon send run-plan --repo ~/dev/my-project
# Triggers plan execution for the specified repo

$ roko daemon send subscribe --repo ~/dev/my-project --cron "*/30 * * * *"
# Adds a cron-based subscription
```

---

## Log Management

Daemon logs go to XDG state directory:

- **stdout**: `~/.local/state/roko/daemon.stdout.log` — structured info/debug output
- **stderr**: `~/.local/state/roko/daemon.stderr.log` — errors and warnings

The `roko daemon logs` command tails both:

```bash
$ roko daemon logs
# Equivalent to: tail -f ~/.local/state/roko/daemon.stdout.log

$ roko daemon logs --stderr
# Equivalent to: tail -f ~/.local/state/roko/daemon.stderr.log

$ roko daemon logs --lines 100
# Show last 100 lines
```

### Log Rotation

launchd does not provide built-in log rotation. The daemon implements its own:

- Maximum log file size: 10MB
- On reaching the limit, rename `daemon.stdout.log` to `daemon.stdout.log.1` and start fresh
- Keep at most 3 rotated files (`.log.1`, `.log.2`, `.log.3`)
- Total maximum disk usage for logs: ~40MB

Alternatively, on macOS 13+, configure the system log via `os_log` for integration with
Console.app and the unified logging system.

---

## Daemon Startup Sequence

When the daemon starts (either via `launchctl load` or `roko daemon start`), it follows a
13-step initialization:

```
 1. Parse CLI args and resolve config paths
 2. Load global config (~/.config/roko/config.toml)
 3. Initialize logging (file + optional stderr)
 4. Create or connect to Unix domain socket
 5. Load subscription list from config
 6. For each subscription:
    a. Validate repo path exists
    b. Load repo-local config (.roko/config.toml)
    c. Initialize file watcher (if watch mode)
    d. Initialize cron scheduler (if cron mode)
    e. Initialize webhook listener (if webhook mode)
 7. Start IPC server (accept commands on socket)
 8. Start event bus (internal pub/sub for daemon events)
 9. Start health check loop (periodic self-assessment)
10. Start adaptive clock (Gamma/Theta/Delta frequencies)
11. Log startup complete with PID and socket path
12. Enter main event loop (process subscriptions, IPC commands, events)
13. On SIGTERM/SIGINT: graceful shutdown (drain running tasks, save state, exit)
```

---

## Graceful Shutdown

When the daemon receives SIGTERM (from `launchctl unload` or `roko daemon stop`):

1. **Stop accepting new tasks** — mark all subscriptions as paused
2. **Drain running tasks** — wait up to 30 seconds for in-progress plan runs to complete
3. **Save state** — write subscription states and any pending events to disk
4. **Close IPC socket** — clean up the socket file
5. **Exit cleanly** — exit code 0

If tasks do not complete within the 30-second drain period, the daemon force-kills them and
exits. launchd will not restart the daemon after a clean exit (only after crash exits, per the
`SuccessfulExit = false` KeepAlive configuration).

---

## Environment Variables in launchd

launchd runs daemons in a minimal environment — it does not source `~/.zshrc` or `~/.bashrc`.
Environment variables that the daemon needs must be set in the plist's `EnvironmentVariables`
dictionary.

The `roko daemon install` command detects environment variables from the current shell and
includes relevant ones in the plist:

- `ANTHROPIC_API_KEY` — if set, included in the plist
- `OPENAI_API_KEY` — if set, included in the plist
- `RUST_LOG` — always included (defaults to `info`)
- `HOME` — always included (launchd may not set it)
- `PATH` — always included with `~/.cargo/bin` appended

For sensitive keys (API keys), the daemon can alternatively read them from the macOS Keychain
at runtime via the `keyring` crate, avoiding storage in the plist file. See
`10-secret-management.md` for the full key resolution strategy.

---

## Current Status

Daemon mode is at **Tier 3H** priority in the implementation plan (P2 — planned but not yet in
the critical path). The `roko daemon` subcommand structure is defined in the CLI but the
implementation is scaffold-only:

- The `DaemonCmd` enum and IPC protocol are designed but not wired
- The plist generation function is implemented but not tested end-to-end
- File watching uses `notify` crate (already a workspace dependency) but is not connected to
  the daemon event loop
- Cron scheduling uses `cron` crate but is not integrated

The prerequisite for daemon mode is completing the subscription configuration system
(see `08-subscription-configuration.md`) which defines how repositories are registered and
how their schedules are configured in `roko.toml`.
