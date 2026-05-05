# Daemon and Subscription System

> The daemon is a **Hot Flow Graph that owns Trigger Cells**. It transforms Roko from a
> tool you invoke into a persistent runtime that watches, schedules, and executes
> autonomously. The subscription system defines what triggers execution and how multiple
> repositories are coordinated under shared resource constraints.

---

## The Daemon as Hot Flow

A Hot Flow is a Graph that stays resident between firings, re-evaluating on each tick or
external stimulus. The daemon is exactly this: a long-running process whose Graph contains
Trigger Cells (cron, filesystem watch, webhook) that fire execution sub-Graphs when
conditions are met.

```
Daemon Hot Flow Graph:
  ┌─────────────────────────────────────────────────────────┐
  │                                                         │
  │  [CronTrigger]──┐                                      │
  │  [FsWatchTrigger]──┼──>[Scheduler]──>[PlanRunner]──>   │
  │  [WebhookTrigger]──┘       │              │            │
  │                            │              ▼            │
  │                       [Verify: limits]  [Agent Pool]   │
  │                                           │            │
  │  [IPC Server]◄────────────────────────────┘            │
  │  [Health Check Loop]                                   │
  │                                                         │
  └─────────────────────────────────────────────────────────┘
```

The daemon is the **production runtime**. It is not a development convenience. Every
deployment shape except "laptop" (interactive CLI) benefits from daemon-mode operation.

---

## Daemon Lifecycle as Type-State

The daemon has a well-defined lifecycle expressed as type-state transitions:

```rust
/// Daemon lifecycle states. Each transition is explicit and logged.
///
/// Uninstalled -> Installed -> Running -> Draining -> Stopped
///                    ^                       |
///                    └───────────────────────┘ (restart)
pub enum DaemonState {
    /// No service definition exists on this host.
    Uninstalled,
    /// Service definition installed (plist/unit file) but process not running.
    Installed,
    /// Process running, accepting IPC commands and processing triggers.
    Running {
        pid: u32,
        started_at: SystemTime,
        subscriptions: Vec<ActiveSubscription>,
    },
    /// Graceful shutdown in progress: draining tasks, saving state.
    Draining {
        deadline: Instant,
        remaining_tasks: usize,
    },
    /// Process exited cleanly.
    Stopped,
}
```

Lifecycle commands map to state transitions:

| Command | Transition | Platform Action |
|---|---|---|
| `roko daemon install` | Uninstalled -> Installed -> Running | Write plist/unit, load, start |
| `roko daemon start` | Installed -> Running | `launchctl load` / `systemctl start` |
| `roko daemon stop` | Running -> Draining -> Stopped | SIGTERM, drain, exit |
| `roko daemon restart` | Running -> Draining -> Stopped -> Running | Stop then start |
| `roko daemon uninstall` | Any -> Uninstalled | Unload, remove plist/unit |

---

## IPC: The Connect Protocol

The daemon exposes a Unix domain socket for local command-and-control. This is a Connect
Cell implementing a JSON-RPC protocol over the socket.

```rust
/// IPC protocol: newline-delimited JSON over Unix domain socket.
/// The daemon is the server; CLI commands, TUI, and external tools are clients.

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum DaemonRequest {
    /// Query daemon health and subscription state.
    Status,
    /// Trigger an immediate plan run for a repository.
    RunPlan { repo: PathBuf, plan_dir: Option<String> },
    /// Add a new subscription (persisted to config).
    Subscribe { repo: PathBuf, trigger: TriggerConfig },
    /// Remove a subscription.
    Unsubscribe { repo: PathBuf },
    /// List all subscriptions with their current state.
    ListSubscriptions,
    /// Pause/resume a subscription or all subscriptions.
    Pause { repo: Option<PathBuf> },
    Resume { repo: Option<PathBuf> },
    /// Subscribe to real-time event stream (long-lived connection).
    StreamEvents { filter: Option<EventFilter> },
    /// Graceful shutdown.
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    Ok { message: String },
    Error { code: u32, message: String },
    Status(StatusPayload),
    Subscriptions(Vec<SubscriptionInfo>),
    Event(DaemonEvent),
}
```

Socket paths are platform-dependent:

| Platform | Socket Path | Rationale |
|---|---|---|
| macOS | `/tmp/roko-daemon.sock` | No per-user runtime dir by default |
| Linux | `/run/user/$UID/roko-daemon.sock` | XDG_RUNTIME_DIR, tmpfs, auto-cleaned |

---

## Three Trigger Cell Types

Trigger Cells are the ingress points of the daemon Graph. Each watches a different stimulus
and emits a `TriggerPulse` on the Bus when conditions are met.

### CronTrigger

Fires on a time schedule using standard 5-field cron expressions.

```rust
/// CronTrigger Cell: fires at scheduled intervals.
pub struct CronTrigger {
    schedule: CronExpression,
    /// Optional: only fire if files matching these globs changed since last run.
    changed_paths: Vec<GlobPattern>,
    /// Tracks last successful execution time for change detection.
    last_run: Option<SystemTime>,
}

impl Trigger for CronTrigger {
    async fn next_fire(&self) -> Instant {
        self.schedule.next_from(Utc::now())
    }

    async fn should_fire(&self, repo: &Path) -> bool {
        if self.changed_paths.is_empty() {
            return true; // No filter: always fire on schedule
        }
        // Check if any matching files were modified since last run
        self.changed_paths.iter().any(|pattern| {
            glob(repo.join(pattern))
                .any(|path| path.modified() > self.last_run.unwrap_or(UNIX_EPOCH))
        })
    }
}
```

Configuration:

```toml
[subscriptions.cron]
schedule = "*/30 * * * *"              # Every 30 minutes
changed_paths = [".roko/prd/**/*.md"]  # Only if PRDs changed
```

### FsWatchTrigger

Fires when filesystem events occur in watched directories. Uses debouncing to coalesce
rapid changes (e.g., editor save bursts).

```rust
/// FsWatchTrigger Cell: fires on filesystem changes with debouncing.
pub struct FsWatchTrigger {
    paths: Vec<PathBuf>,
    debounce: Duration,
    recursive: bool,
    ignore: Vec<GlobPattern>,
    /// Internal: the notify watcher handle.
    watcher: RecommendedWatcher,
}

impl FsWatchTrigger {
    /// Debounce algorithm:
    /// - On first event: start timer (debounce duration)
    /// - On subsequent events: reset timer
    /// - When timer expires with no new events: fire
    async fn watch_loop(&mut self, bus: &Bus) {
        let mut timer: Option<Sleep> = None;

        loop {
            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    if self.should_ignore(&event) { continue; }
                    timer = Some(tokio::time::sleep(self.debounce));
                }
                _ = async { timer.as_mut().unwrap().await }, if timer.is_some() => {
                    bus.publish(TriggerPulse::FsChanged {
                        paths: self.paths.clone(),
                    }).await;
                    timer = None;
                }
            }
        }
    }
}
```

Configuration:

```toml
[subscriptions.watch]
paths = [".roko/prd/", "plans/"]
debounce_ms = 5000
recursive = true
ignore = ["*.swp", "*~", ".git/"]
```

### WebhookTrigger

Fires when an HTTP POST arrives at a configured endpoint. The daemon runs a lightweight
HTTP server (separate from roko-serve) for webhook ingress.

```rust
/// WebhookTrigger Cell: fires on authenticated HTTP POST.
pub struct WebhookTrigger {
    path: String,           // e.g., "/hook/my-project"
    secret: Option<String>, // HMAC-SHA256 verification secret
    port: u16,              // Webhook listener port (default: 9090)
}

impl WebhookTrigger {
    async fn handle(&self, headers: &HeaderMap, body: &[u8]) -> bool {
        // Verify HMAC signature if secret is configured
        if let Some(ref secret) = self.secret {
            let signature = headers.get("x-hub-signature-256")
                .and_then(|v| v.to_str().ok());
            if !verify_hmac_sha256(secret, body, signature) {
                return false; // Reject: bad signature
            }
        }
        true // Accept: fire trigger
    }
}
```

Configuration:

```toml
[subscriptions.webhook]
path = "/hook/my-project"
secret = "${ROKO_WEBHOOK_SECRET}"  # Resolved from environment
```

---

## Scheduler: Priority Queue with Verify Pre-Conditions

When multiple Trigger Cells fire simultaneously, the Scheduler Cell dequeues them according
to priority and resource availability. Concurrency limits act as Verify pre-conditions --
the Scheduler checks available capacity before allowing a run to proceed.

```rust
/// Scheduling priority: webhook > watch > cron.
/// Within same priority: FIFO by queue time.
#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
pub enum TriggerPriority {
    Webhook = 0, // External event, time-sensitive
    Watch = 1,   // File change, user is actively editing
    Cron = 2,    // Scheduled, can wait
}

/// Scheduler Cell: dequeues trigger events under resource constraints.
pub struct Scheduler {
    queue: BinaryHeap<QueuedRun>,
    /// Global concurrency limit (Verify pre-condition).
    max_concurrent_runs: usize,
    /// Global agent limit (Verify pre-condition).
    max_total_agents: usize,
    /// Per-repo agent limits.
    per_repo_limits: HashMap<PathBuf, usize>,
    /// Semaphore enforcing the global agent limit.
    agent_semaphore: Arc<Semaphore>,
}

impl Scheduler {
    /// Verify pre-condition: can this run proceed?
    fn can_schedule(&self, run: &QueuedRun) -> bool {
        let current_agents = self.max_total_agents
            - self.agent_semaphore.available_permits();
        let repo_agents = self.active_agents_for(&run.repo);
        let repo_limit = self.per_repo_limits
            .get(&run.repo)
            .copied()
            .unwrap_or(self.max_total_agents);

        current_agents < self.max_total_agents
            && repo_agents < repo_limit
    }
}
```

Configuration:

```toml
[daemon]
max_concurrent_runs = 4   # Max simultaneous plan runs across all repos
max_total_agents = 8      # Max total agent processes globally
```

---

## Multi-Repo Coordination

The daemon manages N repository subscriptions. Each is isolated in filesystem, process, and
configuration space. They share only system resources (CPU, memory, API keys).

### Isolation Model

```
Per-Repo Isolated:                    Globally Shared:
  .roko/ state directory               System CPU/memory
  Plan files                            LLM API keys
  Executor snapshots                    Network/rate limits
  Signal/episode logs                   Agent semaphore pool
  Gate results                          Daemon process itself
  Local config overrides
```

### Config Merge: Three Layers

Each subscription resolves its configuration through a three-layer merge:

```
Layer 1: [daemon.defaults]          -- baseline for all repos
Layer 2: [[subscriptions]] entry    -- per-repo in global config
Layer 3: .roko/config.toml          -- repo owner's local overrides (wins)
```

```toml
# Global config: ~/.config/roko/config.toml
[daemon.defaults]
model = "claude-sonnet-4-6"
max_agents = 4

[[subscriptions]]
repo = "/Users/will/dev/project-a"
max_agents = 6  # Override: this repo gets more agents

# Per-repo: /Users/will/dev/project-a/.roko/config.toml
[agent]
model = "claude-opus-4-6"  # Final override: repo owner chooses model
```

---

## Agent Mesh: Cross-Repo Knowledge via Bus Federation

By default, repos are isolated. When Agent Mesh is enabled, repos in the same **group** can
share Signals across a federated Bus.

```toml
[[subscriptions]]
repo = "/path/to/repo-a"

[subscriptions.mesh]
enabled = true
group = "nunchi-projects"
share_kinds = ["Insight", "Heuristic", "Warning"]
min_confidence = 0.7
```

The Mesh is a Bus federation pattern:
1. After a successful plan run, qualifying Signals are published to the group Bus partition
2. Before starting a run, the daemon queries the group Bus for relevant Signals from peers
3. HDC similarity search (threshold 0.526) identifies cross-domain structural analogies
4. Imported Signals carry provenance tagging indicating their source repo

This enables cross-domain insight resonance: a heuristic learned in one project surfaces
when relevant in another.

---

## Secret Management: Hierarchical Store

Secrets follow a layered resolution strategy that works across all deployment shapes. The
daemon resolves secrets at startup and injects them into the agent environment.

```rust
/// Secret resolution order (first match wins):
/// 1. CLI flags (--api-key)
/// 2. Environment variables (ANTHROPIC_API_KEY)
/// 3. Config files with ${VAR} interpolation
/// 4. OS keychain (macOS Keychain / Linux Secret Service)
/// 5. External secret store (Vault, AWS Secrets Manager)
/// 6. Fail with actionable error message
pub fn resolve_secret(name: &str) -> Result<String> {
    // Layer 1: CLI flag (set by caller)
    // Layer 2: Environment
    if let Ok(val) = std::env::var(name) {
        return Ok(val);
    }
    // Layer 2b: _FILE indirection (Docker/K8s pattern)
    if let Ok(path) = std::env::var(format!("{name}_FILE")) {
        return Ok(std::fs::read_to_string(path)?.trim().to_string());
    }
    // Layer 3: Config interpolation already resolved during config load
    // Layer 4: OS keychain
    if let Ok(val) = keyring::Entry::new("roko", name)?.get_password() {
        return Ok(val);
    }
    // Layer 5: External store (if configured)
    // Layer 6: Fail
    anyhow::bail!(
        "Secret '{name}' not found. Set via environment, keychain, or config.\n\
         Run 'roko config set-secret {name}' to store it."
    )
}
```

For the daemon specifically, secrets must be available without interactive prompts:
- **macOS (launchd)**: Environment variables in the plist, or keychain access at runtime
- **Linux (systemd)**: `EnvironmentFile=-~/.config/roko/daemon.env` for API keys
- **Container**: Standard environment variable injection
- **Fly.io**: `fly secrets set` (encrypted at rest, injected as env vars)

---

## Platform-Specific Connector Cells

The daemon integrates with platform service managers through Connector Cells that abstract
the OS-specific lifecycle.

### macOS: launchd Connector

```rust
/// launchd Connector Cell: manages the daemon plist lifecycle.
pub struct LaunchdConnector {
    plist_path: PathBuf, // ~/Library/LaunchAgents/dev.nunchi.roko.plist
    label: String,       // "dev.nunchi.roko"
}

impl LaunchdConnector {
    pub fn install(&self, config: &DaemonConfig) -> Result<()> {
        let plist = generate_launchd_plist(config);
        std::fs::write(&self.plist_path, plist)?;
        Command::new("launchctl").args(["load", "-w"]).arg(&self.plist_path).status()?;
        Ok(())
    }

    pub fn uninstall(&self) -> Result<()> {
        Command::new("launchctl").args(["unload", "-w"]).arg(&self.plist_path).status()?;
        std::fs::remove_file(&self.plist_path)?;
        Ok(())
    }
}
```

Key plist settings:
- `RunAtLoad = true` -- start on login
- `KeepAlive.SuccessfulExit = false` -- restart on crash, not on clean exit
- `ThrottleInterval = 10` -- minimum seconds between restarts
- `Nice = 5` -- slightly lower priority than interactive processes

### Linux: systemd Connector

```rust
/// systemd Connector Cell: manages the daemon unit file lifecycle.
pub struct SystemdConnector {
    unit_path: PathBuf, // ~/.config/systemd/user/roko.service
    unit_name: String,  // "roko.service"
}

impl SystemdConnector {
    pub fn install(&self, config: &DaemonConfig) -> Result<()> {
        let unit = generate_systemd_unit(config);
        std::fs::write(&self.unit_path, unit)?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", &self.unit_name])?;
        systemctl(&["start", &self.unit_name])?;
        Ok(())
    }
}
```

Key unit settings:
- `Restart=on-failure` with exponential backoff (10s -> 300s max)
- `WatchdogSec=60` -- systemd kills if no heartbeat in 60s
- Security hardening: `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`
- `EnvironmentFile=-~/.config/roko/daemon.env` -- optional secret injection

### Watchdog Integration

On Linux, the daemon pings systemd's watchdog from its main event loop:

```rust
/// Ping the systemd watchdog every tick. If we miss the WatchdogSec
/// window (60s), systemd considers us hung and restarts us.
async fn daemon_tick(state: &mut DaemonState) {
    // Process triggers, IPC commands, health checks...
    process_pending_triggers(state).await;
    process_ipc_commands(state).await;

    // Ping watchdog (no-op on macOS)
    #[cfg(target_os = "linux")]
    sd_notify::notify(false, &[sd_notify::NotifyState::Watchdog]).ok();
}
```

---

## Daemon Startup Sequence

```
 1. Parse CLI args, resolve config paths
 2. Load global config (~/.config/roko/config.toml)
 3. Initialize logging (file + optional journald)
 4. Create or bind Unix domain socket
 5. Load subscription list from config
 6. For each subscription:
    a. Validate repo path exists and is a git repo
    b. Load per-repo config (.roko/config.toml)
    c. Merge config layers (defaults -> global -> local)
    d. Initialize Trigger Cells (cron/watch/webhook)
 7. Start IPC server (accept commands on socket)
 8. Start webhook HTTP server (if any webhook subscriptions)
 9. Start Bus (internal Pulse transport)
10. Start health check loop (periodic self-assessment)
11. Notify service manager: ready (sd_notify::Ready on Linux)
12. Enter main event loop (Trigger -> Schedule -> Execute)
13. On SIGTERM: Draining -> save state -> close socket -> exit(0)
```

---

## Graceful Shutdown Protocol

When the daemon receives SIGTERM (from `launchctl unload`, `systemctl stop`, or
`roko daemon stop`):

```rust
/// Graceful shutdown: drain tasks, save state, exit clean.
async fn graceful_shutdown(state: &mut DaemonState, deadline: Duration) {
    // Phase 1: Stop accepting new work
    state.accepting = false;
    // Readiness probe returns false from this point

    // Phase 2: Drain running tasks (bounded by deadline)
    let drain_result = tokio::time::timeout(
        deadline,
        drain_all_tasks(&mut state.active_runs),
    ).await;

    if drain_result.is_err() {
        // Deadline exceeded: force-kill remaining agents
        state.supervisor.force_shutdown_all().await;
    }

    // Phase 3: Persist state
    save_subscription_state(&state.subscriptions).await;
    save_executor_snapshots(&state.active_runs).await;

    // Phase 4: Close connections
    state.ipc_socket.shutdown().await;
    state.webhook_server.shutdown().await;

    // Phase 5: Exit cleanly (exit 0 = don't restart)
    std::process::exit(0);
}
```

---

## What This Enables

1. **Autonomous operation**: Roko watches repositories and executes plans without human
   invocation. The daemon IS the production runtime.

2. **Multi-project orchestration**: A single daemon manages many repos with isolation
   guarantees and shared resource governance.

3. **Cross-repo learning**: Agent Mesh enables insights to flow between projects,
   compounding knowledge across the entire workspace.

4. **Platform-native integration**: The daemon feels like a first-class system service on
   both macOS and Linux, with proper restart, logging, and health monitoring.

5. **Event-driven execution**: Webhooks close the loop from external events (GitHub push)
   to automated plan execution without polling.

---

## Feedback Loops

- **Trigger-to-execution latency**: Measured per trigger type. If webhook latency drifts,
  the Scheduler adjusts priority weights (Loop pattern).

- **Subscription health tracking**: Each subscription tracks consecutive failures. After 3+
  failures, the daemon publishes a warning Pulse and optionally pauses the subscription.

- **Agent pool utilization**: The Scheduler observes pool utilization over time. If
  consistently saturated, it publishes a capacity warning suggesting `max_total_agents`
  increase.

---

## Open Questions

1. **Hot config reload**: Should the daemon watch its own config file and reload
   subscriptions without restart? Current answer: yes for subscription changes, no for
   shape changes (shape requires process restart).

2. **Distributed daemon**: Can multiple daemon instances form a cluster? Current answer: no.
   Use the "clustered" shape with roko-serve instead. The daemon is a single-machine concept.

3. **Subscription dependencies**: Can subscription A depend on subscription B (e.g., "run
   project B only after project A succeeds")? Not currently modeled. Would require a
   meta-scheduler Graph.

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Daemon event loop and Hot Flow Graph | `crates/roko-cli/src/daemon.rs` | Scaffolded |
| CronTrigger Cell implementation | `crates/roko-cli/src/daemon/cron_trigger.rs` | Not started |
| FsWatchTrigger Cell implementation | `crates/roko-cli/src/daemon/watch_trigger.rs` | Not started |
| WebhookTrigger Cell implementation | `crates/roko-cli/src/daemon/webhook_trigger.rs` | Not started |
| IPC server (Unix socket, JSON-RPC) | `crates/roko-cli/src/daemon/ipc.rs` | Not started |
| Scheduler with priority queue | `crates/roko-cli/src/daemon/scheduler.rs` | Not started |
| launchd plist generation | `crates/roko-cli/src/daemon/launchd.rs` | Scaffolded |
| systemd unit generation + watchdog | `crates/roko-cli/src/daemon/systemd.rs` | Not started |
| Subscription config schema + merge | `crates/roko-core/src/config/subscription.rs` | Not started |
| Secret resolution hierarchy | `crates/roko-core/src/config/secrets.rs` | Partial |
| Agent Mesh Bus federation | `crates/roko-runtime/src/mesh.rs` | Not started |
| Subscription state persistence | `crates/roko-cli/src/daemon/state.rs` | Not started |
| Graceful shutdown protocol | `crates/roko-runtime/src/supervisor.rs` | Partial |
