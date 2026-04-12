# Subscription Configuration

> When Roko runs as a daemon (see `04-daemon-launchd-macos.md` and `05-daemon-systemd-linux.md`),
> it monitors repositories for changes and triggers plan execution automatically. This document
> defines the subscription configuration format in `roko.toml`, the three trigger types (cron,
> file watch, webhook), per-repo overrides, and the subscription lifecycle.

---

## Overview

A subscription is a binding between a repository and a trigger. When the trigger fires, the
daemon executes plans in that repository. Subscriptions are defined in the global
`~/.config/roko/config.toml` and can be overridden by the per-repo `.roko/config.toml`.

The three trigger types:

| Trigger | When it fires | Use case |
|---|---|---|
| **Cron** | On a time schedule (cron expression) | Periodic builds, nightly consolidation |
| **Watch** | When specified files change (fsnotify) | Reactive to PRD edits, code changes |
| **Webhook** | When an HTTP POST arrives at a webhook endpoint | GitHub push events, CI triggers |

Multiple triggers can be combined for a single repository. For example, a repo might have a
cron trigger for nightly full builds and a watch trigger for immediate re-execution when a PRD
changes.

---

## Configuration Format

### Global Config: `~/.config/roko/config.toml`

```toml
# ~/.config/roko/config.toml
#
# Global Roko configuration. Subscriptions defined here are loaded
# by the daemon on startup. Per-repo .roko/config.toml can override
# subscription settings for individual repositories.

[daemon]
# Socket path for IPC (default: platform-dependent)
# macOS: /tmp/roko-daemon.sock
# Linux: $XDG_RUNTIME_DIR/roko-daemon.sock
socket = "/tmp/roko-daemon.sock"

# Log level for the daemon process
log_level = "info"

# Maximum concurrent plan executions across all subscriptions
max_concurrent_runs = 4

# Default agent configuration for subscribed repos
[daemon.defaults]
model = "claude-sonnet-4-6"
max_agents = 4


# ── Subscriptions ───────────────────────────────────────���──────────

[[subscriptions]]
# Repository path (absolute)
repo = "/Users/will/dev/nunchi/roko/roko"

# Cron trigger: run every 30 minutes
[subscriptions.cron]
schedule = "*/30 * * * *"

# Which plans to execute (glob pattern relative to repo root)
plan_dirs = ["plans/"]

# Optional: only run if these files changed since last run
changed_paths = [".roko/prd/**/*.md", "plans/**/*.toml"]


[[subscriptions]]
repo = "/Users/will/dev/project-b"

# File watch trigger: re-run when PRD files change
[subscriptions.watch]
paths = [".roko/prd/"]
debounce_ms = 5000    # Wait 5 seconds after last change before triggering

plan_dirs = ["plans/"]


[[subscriptions]]
repo = "/Users/will/dev/project-c"

# Webhook trigger: HTTP endpoint for external triggers
[subscriptions.webhook]
# The daemon listens on this path (relative to its HTTP server)
path = "/hook/project-c"
# Optional: shared secret for HMAC verification
secret = "${ROKO_WEBHOOK_SECRET_C}"

plan_dirs = ["plans/"]


[[subscriptions]]
repo = "/Users/will/dev/project-d"

# Combined triggers: cron + watch
[subscriptions.cron]
schedule = "0 2 * * *"   # Nightly at 2 AM

[subscriptions.watch]
paths = [".roko/prd/"]
debounce_ms = 3000

plan_dirs = ["plans/"]
```

### Per-Repo Override: `.roko/config.toml`

Each repository can override its subscription settings. The per-repo config is merged on top
of the global config entry for that repo:

```toml
# .roko/config.toml (in the repository root)
#
# This file overrides settings from the global config for this repo.

[agent]
model = "claude-opus-4-6"     # Override model for this repo
max_agents = 8                 # More agents for this large repo

[subscription]
# Override the cron schedule (takes precedence over global)
[subscription.cron]
schedule = "*/15 * * * *"      # Every 15 minutes instead of 30

# Override plan directories
plan_dirs = ["plans/active/"]  # Only run active plans

# Gate configuration for this repo
[subscription.gates]
compile = true
test = true
clippy = true
min_confidence = 0.7
```

### Config Merge Order

For each subscription, settings are resolved in this order (later wins):

```
1. Daemon defaults     ([daemon.defaults] in global config)
2. Global subscription ([[subscriptions]] entry for this repo)
3. Per-repo config     (.roko/config.toml in the repo)
4. Environment vars    (ROKO_* prefix)
```

---

## Cron Trigger

The cron trigger uses standard 5-field cron expressions:

```
┌───────────── minute (0-59)
│ ┌───────────── hour (0-23)
│ │ ┌───────────── day of month (1-31)
│ │ │ ┌───────────── month (1-12)
│ │ │ │ ┌───────────── day of week (0-6, Sunday=0)
│ │ │ │ │
* * * * *
```

Common patterns:

```toml
# Every 30 minutes
schedule = "*/30 * * * *"

# Every hour at minute 0
schedule = "0 * * * *"

# Nightly at 2 AM
schedule = "0 2 * * *"

# Every weekday at 9 AM
schedule = "0 9 * * 1-5"

# Every 6 hours
schedule = "0 */6 * * *"
```

The daemon uses the `cron` crate to parse expressions and calculate next-fire times. The
scheduler runs in a Tokio task, sleeping until the next scheduled time.

### Changed-Path Filtering

When `changed_paths` is specified, the cron trigger only executes if files matching the glob
patterns have changed since the last successful run:

```toml
[subscriptions.cron]
schedule = "*/30 * * * *"
changed_paths = [".roko/prd/**/*.md", "plans/**/*.toml"]
```

The daemon tracks the last successful run timestamp per subscription in
`~/.local/state/roko/subscriptions.json`. On each cron tick, it checks:

```rust
fn should_run(sub: &Subscription, last_run: SystemTime) -> bool {
    if sub.changed_paths.is_empty() {
        return true; // No filter — always run on schedule
    }

    // Check if any matching files were modified since last run
    for pattern in &sub.changed_paths {
        let repo_root = &sub.repo;
        for entry in glob::glob(&repo_root.join(pattern).to_string_lossy())? {
            if let Ok(path) = entry {
                if path.metadata()?.modified()? > last_run {
                    return true;
                }
            }
        }
    }

    false // No changes — skip this run
}
```

---

## File Watch Trigger

The watch trigger uses the `notify` crate (already a workspace dependency) to monitor filesystem
events in real time:

```toml
[subscriptions.watch]
paths = [".roko/prd/", "src/"]    # Directories or files to watch
debounce_ms = 5000                 # Wait 5s after last change before triggering
recursive = true                   # Watch subdirectories (default: true)
ignore = ["*.swp", "*~", ".git/"]  # Ignore patterns (default: common editor temp files)
```

### Debouncing

File system events often arrive in bursts (e.g., saving a file triggers multiple events). The
debounce timer ensures the daemon waits for the burst to settle before triggering a plan run:

```
Event 1 (file saved)    → Start 5s timer
Event 2 (100ms later)   → Reset timer to 5s
Event 3 (200ms later)   → Reset timer to 5s
... (no more events) ...
Timer expires (5s)       → Trigger plan run
```

The debounce is implemented with a Tokio delay that resets on each new event:

```rust
async fn watch_loop(
    sub: &Subscription,
    notify_rx: mpsc::Receiver<notify::Event>,
    trigger_tx: mpsc::Sender<TriggerEvent>,
) {
    let debounce = Duration::from_millis(sub.watch.debounce_ms);
    let mut timer: Option<tokio::time::Sleep> = None;

    loop {
        tokio::select! {
            Some(_event) = notify_rx.recv() => {
                // Reset debounce timer on each event
                timer = Some(tokio::time::sleep(debounce));
            }
            _ = async { timer.as_mut().unwrap().await }, if timer.is_some() => {
                // Debounce expired — trigger the plan run
                trigger_tx.send(TriggerEvent::Watch {
                    repo: sub.repo.clone(),
                }).await.ok();
                timer = None;
            }
        }
    }
}
```

---

## Webhook Trigger

The webhook trigger starts a lightweight HTTP server (embedded in the daemon) that listens for
POST requests:

```toml
[subscriptions.webhook]
path = "/hook/project-c"
secret = "${ROKO_WEBHOOK_SECRET_C}"
```

### Webhook Server

The daemon starts an Axum HTTP server on a configurable port (default: 9090) that routes
incoming webhook requests to the appropriate subscription:

```rust
async fn webhook_handler(
    Path(repo_name): Path<String>,
    headers: HeaderMap,
    body: Bytes,
    State(state): State<Arc<DaemonState>>,
) -> StatusCode {
    // Find the subscription for this webhook path
    let sub = state.subscriptions.iter()
        .find(|s| s.webhook.as_ref().map(|w| w.path.ends_with(&repo_name)).unwrap_or(false));

    let Some(sub) = sub else {
        return StatusCode::NOT_FOUND;
    };

    // Verify HMAC signature if secret is configured
    if let Some(ref secret) = sub.webhook.as_ref().and_then(|w| w.secret.as_ref()) {
        let signature = headers.get("x-hub-signature-256")
            .and_then(|v| v.to_str().ok());

        if !verify_hmac(secret, &body, signature) {
            return StatusCode::UNAUTHORIZED;
        }
    }

    // Trigger plan run
    state.trigger_tx.send(TriggerEvent::Webhook {
        repo: sub.repo.clone(),
    }).await.ok();

    StatusCode::OK
}
```

### GitHub Integration

The webhook format is compatible with GitHub webhook payloads. To trigger a plan run on push:

1. In the GitHub repo settings, add a webhook:
   - Payload URL: `https://your-daemon-host:9090/hook/project-c`
   - Content type: `application/json`
   - Secret: (same as `ROKO_WEBHOOK_SECRET_C`)
   - Events: Push, Pull Request

2. The daemon receives the webhook, verifies the HMAC signature, and triggers the plan run.

For local development (where the daemon is not publicly accessible), use a tunnel service
(e.g., `ngrok`, `cloudflared tunnel`) to expose the webhook endpoint:

```bash
cloudflared tunnel --url http://localhost:9090
# Gives you a public URL like: https://abc123.trycloudflare.com
```

---

## Environment Variable Interpolation

Subscription config values support `${VAR}` interpolation from environment variables:

```toml
[subscriptions.webhook]
secret = "${ROKO_WEBHOOK_SECRET_C}"

[[subscriptions]]
repo = "${HOME}/dev/project-a"
```

The daemon resolves `${VAR}` at config load time using `std::env::var()`. If a referenced
variable is not set, the daemon logs a warning and uses the literal string (including the
`${...}` syntax) — this makes misconfiguration visible rather than silently failing.

---

## Subscription Lifecycle

### Adding a Subscription

```bash
# Via config file (edit ~/.config/roko/config.toml)
roko config edit

# Via CLI (adds to global config)
roko daemon subscribe --repo ~/dev/project-a --cron "*/30 * * * *"

# Via IPC to running daemon (takes effect immediately)
roko daemon send subscribe --repo ~/dev/project-a --cron "*/30 * * * *"
```

### Removing a Subscription

```bash
# Via CLI (removes from global config)
roko daemon unsubscribe --repo ~/dev/project-a

# Via IPC to running daemon
roko daemon send unsubscribe --repo ~/dev/project-a
```

### Listing Subscriptions

```bash
$ roko daemon send list-subscriptions

Subscriptions:
  /Users/will/dev/project-a
    Triggers: cron (*/30 * * * *)
    Plans:    plans/
    Last run: 2h ago (success, 3 tasks completed)
    Next run: in 12 minutes

  /Users/will/dev/project-b
    Triggers: watch (.roko/prd/)
    Plans:    plans/
    Last run: 15m ago (running, 2/5 tasks complete)

  /Users/will/dev/project-c
    Triggers: webhook (/hook/project-c)
    Plans:    plans/
    Last run: never
```

### Pausing and Resuming

```bash
# Pause all subscriptions (daemon stays running but doesn't trigger)
roko daemon send pause

# Resume all subscriptions
roko daemon send resume

# Pause a specific subscription
roko daemon send pause --repo ~/dev/project-a
```

---

## Subscription State Persistence

The daemon persists subscription state (last run timestamps, run results, next scheduled time)
to `~/.local/state/roko/subscriptions.json`:

```json
{
  "subscriptions": [
    {
      "repo": "/Users/will/dev/project-a",
      "last_run_at": "2026-04-12T08:30:00Z",
      "last_run_status": "success",
      "last_run_tasks": 3,
      "next_run_at": "2026-04-12T09:00:00Z",
      "paused": false
    }
  ]
}
```

This file is updated after each run completes and loaded on daemon startup to resume scheduling
from where it left off.

---

## Current Status

Subscription configuration is at **Tier 3H** priority (P2 — planned). The TOML schema is
designed and documented here. The `notify` crate (for file watching) and `cron` crate (for
scheduling) are both workspace dependencies. The daemon infrastructure (IPC, event loop) is
described in `04-daemon-launchd-macos.md` and `05-daemon-systemd-linux.md`.

Implementation depends on the daemon mode infrastructure being wired first.
