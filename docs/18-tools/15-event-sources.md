# 15 — Event Sources

> Cron, FileWatch, GitHub webhooks, Slack events, generic webhooks.
> How events trigger agent execution via subscriptions.

---

## Overview

Event sources are the entry points for triggering agent execution. An event source converts
an external signal (a cron tick, a file change, a webhook payload, a Slack message) into an
Engram that the dispatch loop routes to the appropriate agent template via subscription
matching.

Event sources operate at **Layer 0 (Runtime)** — they produce events that flow upward through
the architecture. They are part of the `roko-serve` HTTP server that runs alongside agents.

---

## Architecture

```
External Signals
    ├── Cron scheduler (tokio-cron-scheduler)
    ├── File system watcher (notify crate)
    ├── GitHub webhooks (HTTP POST)
    ├── Slack events (Socket Mode / HTTP POST)
    └── Generic webhooks (HTTP POST)
           |
           v
+--- Event Sources (roko-serve) ----------------------------+
|  Convert external signals to SourceEvents                  |
|  Each SourceEvent has: kind, body, source                  |
+--------+---------------------------------------------------+
         |
         v
+--- Dispatch Loop ------------------------------------------+
|  Match SourceEvent.kind against subscription patterns      |
|  Apply filters (ref, action, path_filter)                  |
|  Check concurrency limits and cooldowns                    |
|  Spawn agent with matched template                         |
+--------+---------------------------------------------------+
         |
         v
+--- Agent Execution ----------------------------------------+
|  Template loaded → ToolContext assembled → MCP started      |
|  Agent runs cognitive loop → produces Engrams               |
|  Agent completes → results stored → events emitted          |
+------------------------------------------------------------+
```

---

## The 5 Event Source Types

### 1. Cron Scheduler

Periodic execution on configurable schedules using standard cron expressions.

**Implementation:** `crates/roko-serve/src/scheduler.rs` (using `tokio-cron-scheduler`)

```rust
/// Start cron jobs for all subscriptions with a `schedule` field.
pub async fn start_scheduler(state: Arc<AppState>) -> Result<JobScheduler> {
    let scheduler = JobScheduler::new().await?;

    let subs = state.subscriptions.read().await;
    for sub in subs.iter() {
        if !sub.enabled { continue; }
        let Some(ref schedule) = sub.schedule else { continue; };

        let state_clone = Arc::clone(&state);
        let template_name = sub.agent_template.clone();
        let pattern = sub.pattern.clone();

        match Job::new_async(schedule.as_str(), move |_uuid, _lock| {
            let state = Arc::clone(&state_clone);
            let template = template_name.clone();
            let pat = pattern.clone();
            Box::pin(async move {
                info!(template = %template, schedule = %pat, "cron tick");
                // Emit SourceEvent → dispatch loop → agent spawn
                state.event_bus.emit(ServerEvent::WebhookReceived {
                    kind: "scheduler.cron".into(),
                    signal_hash: signal.hash.clone(),
                    source: "scheduler".into(),
                });
            })
        }) {
            Ok(job) => {
                scheduler.add(job).await?;
                info!(template = %sub.agent_template, schedule = %schedule, "scheduled cron job");
            }
            Err(e) => {
                error!(template = %sub.agent_template, error = %e, "invalid cron expression");
            }
        }
    }

    scheduler.start().await?;
    Ok(scheduler)
}
```

**Subscription example:**

```toml
[[subscription]]
pattern = "scheduler.cron"
agent_template = "pm-health-agent"
schedule = "0 9 * * MON-FRI"   # Weekdays at 9am
```

### 2. File System Watcher

Watches directories for file changes and triggers agents when files matching a glob pattern
are created or modified.

**Implementation:** `crates/roko-serve/src/fswatcher.rs` (using `notify` + `notify-debouncer-mini`)

```rust
/// Start file watchers for all subscriptions with `watch_path`.
pub async fn start_watchers(state: Arc<AppState>) -> Result<Vec<Debouncer<RecommendedWatcher>>> {
    let mut watchers = Vec::new();

    let subs = state.subscriptions.read().await;
    for sub in subs.iter() {
        if !sub.enabled { continue; }
        let Some(ref watch_path) = sub.watch_path else { continue; };

        // Debounce: collapse rapid changes into single events
        let debounce_ms = sub.cooldown_secs.unwrap_or(5) * 1000;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut debouncer = new_debouncer(Duration::from_millis(debounce_ms), tx)?;
        debouncer.watcher().watch(watch_path, RecursiveMode::Recursive)?;

        // Process debounced events in background task
        tokio::spawn(async move {
            loop {
                match rx.recv() {
                    Ok(Ok(events)) => {
                        let mut changed: HashSet<PathBuf> = HashSet::new();
                        for event in events {
                            if event.kind == DebouncedEventKind::Any {
                                // Apply glob filter
                                if let Some(ref pattern) = glob_pattern {
                                    if let Ok(glob) = glob::Pattern::new(pattern) {
                                        let relative = event.path
                                            .strip_prefix(&watch_path_clone)
                                            .unwrap_or(&event.path);
                                        if !glob.matches_path(relative) { continue; }
                                    }
                                }
                                changed.insert(event.path);
                            }
                        }
                        if changed.is_empty() { continue; }

                        // Emit SourceEvent → dispatch loop → agent spawn
                        state_clone.event_bus.emit(ServerEvent::WebhookReceived {
                            kind: "watcher.fs_change".into(),
                            signal_hash: signal.hash.clone(),
                            source: "fswatcher".into(),
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        watchers.push(debouncer);
    }

    Ok(watchers)
}
```

**Subscription example:**

```toml
[[subscription]]
pattern = "watcher.fs_change"
agent_template = "doc-lifecycle-agent"
watch_path = "/Users/will/dev/nunchi/collaboration/docs"
watch_glob = "**/*.md"
cooldown_secs = 300  # Don't re-trigger within 5 minutes
```

### 3. GitHub Webhooks

GitHub sends HTTP POST requests for repository events. The `roko-serve` HTTP server receives
these and converts them to SourceEvents.

**Event kinds:**
- `webhook.github.push` — code pushed to a branch
- `webhook.github.pull_request` — PR opened, updated, closed
- `webhook.github.pull_request_review` — PR review submitted
- `webhook.github.issues` — issue opened, closed, labeled

**Subscription examples:**

```toml
[[subscription]]
pattern = "webhook.github.push"
agent_template = "doc-lifecycle-agent"
filter = { ref = "refs/heads/main" }
path_filter = "docs/**/*.md"

[[subscription]]
pattern = "webhook.github.pull_request"
agent_template = "pr-review-agent"
filter = { action = ["opened", "synchronize"] }
max_concurrent = 3
```

### 4. Slack Events

Slack events arrive via Socket Mode (WebSocket) or HTTP webhooks:

**Event kinds:**
- `webhook.slack.message` — message posted in a channel
- `webhook.slack.slash_command` — slash command invoked (e.g., `/sync`)
- `webhook.slack.interactive` — interactive component action

**Subscription example:**

```toml
[[subscription]]
pattern = "webhook.slack.slash_command"
agent_template = "sync-agent"
filter = { command = "/sync" }
```

### 5. Generic Webhooks

Any HTTP POST to the webhook endpoint that doesn't match a specific platform:

**Subscription example:**

```toml
[[subscription]]
pattern = "webhook.custom"
agent_template = "generic-handler"
filter = { source = "my-service" }
```

---

## Subscription Configuration

Subscriptions are declared in `subscriptions.toml` files located in each repository's
`.roko/` directory:

```toml
# .roko/subscriptions.toml

[[subscription]]
# Required: Event pattern to match
pattern = "webhook.github.push"

# Required: Which agent template to spawn
agent_template = "auto-plan-agent"

# Optional: Filter conditions (must all match)
filter = { ref = "refs/heads/main" }

# Optional: File path filter (glob pattern)
path_filter = ".roko/prd/**"

# Optional: Cron schedule (for scheduler.cron events)
schedule = "0 9 * * MON-FRI"

# Optional: Maximum concurrent agent instances from this subscription
max_concurrent = 1

# Optional: Cooldown between triggers (seconds)
cooldown_secs = 300

# Optional: Whether this subscription is enabled
enabled = true
```

### Filter Matching

Filters match against the SourceEvent's body fields. All filter conditions must match (AND
logic):

```rust
fn matches_filter(event: &SourceEvent, filter: &Filter) -> bool {
    filter.iter().all(|(key, expected)| {
        event.body.get(key)
            .map(|actual| match expected {
                FilterValue::String(s) => actual.as_str() == Some(s.as_str()),
                FilterValue::Array(arr) => arr.iter().any(|s| actual.as_str() == Some(s.as_str())),
            })
            .unwrap_or(false)
    })
}
```

### Path Filter

Path filters use glob patterns to match against changed file paths. Only events affecting
files matching the pattern trigger the subscription:

```rust
fn matches_path_filter(event: &SourceEvent, path_filter: &str) -> bool {
    let glob = glob::Pattern::new(path_filter).unwrap();
    event.body.get("paths")
        .and_then(|p| p.as_array())
        .map(|paths| paths.iter().any(|p| {
            glob.matches(p.as_str().unwrap_or(""))
        }))
        .unwrap_or(true) // No paths in event → match everything
}
```

---

## Dispatch Loop

The dispatch loop is the central routing mechanism that connects event sources to agent
templates:

```rust
pub async fn dispatch_loop(state: Arc<AppState>) {
    let mut rx = state.event_bus.subscribe();

    while let Ok(event) = rx.recv().await {
        let subs = state.subscriptions.read().await;

        for sub in subs.iter() {
            if !sub.enabled { continue; }
            if sub.pattern != event.kind { continue; }
            if !matches_filter(&event, &sub.filter) { continue; }
            if !matches_path_filter(&event, &sub.path_filter) { continue; }

            // Check concurrency limits
            if let Some(max) = sub.max_concurrent {
                let running = state.running_agents(sub.agent_template).await;
                if running >= max { continue; }
            }

            // Check cooldown
            if let Some(cooldown) = sub.cooldown_secs {
                if state.last_trigger(sub.agent_template).elapsed() < Duration::from_secs(cooldown) {
                    continue;
                }
            }

            // Spawn agent
            state.spawn_agent(sub.agent_template, event.body.clone()).await;
        }
    }
}
```

The dispatch loop is **source-agnostic** — it doesn't care whether the event came from a cron
tick, a file watcher, or a GitHub webhook. All events flow through the same routing logic.

---

## Multi-Repository Configuration

`roko-serve` can watch subscriptions from multiple repositories simultaneously:

```toml
# roko-serve.toml
[[repos]]
path = "/Users/will/dev/nunchi/collaboration"
subscriptions = ".roko/subscriptions.toml"
templates = ".roko/templates/"

[[repos]]
path = "/Users/will/dev/nunchi/knowledge-base"
subscriptions = ".roko/subscriptions.toml"
templates = ".roko/templates/"

[[repos]]
path = "/Users/will/dev/nunchi/roko/roko"
subscriptions = ".roko/subscriptions.toml"
templates = ".roko/templates/"
```

Templates and subscriptions from all repos are loaded into a single dispatch loop. Events
are matched against all subscriptions regardless of source repository.

---

## EventSource as roko-plugin Trait

The built-in event sources (cron, file watcher) also serve as reference implementations of
the `EventSource` trait from `roko-plugin`:

```rust
// Built-in implementations (in roko-serve)
pub struct CronEventSource { /* wraps tokio-cron-scheduler */ }
pub struct FsWatchEventSource { /* wraps notify debouncer */ }

// Both implement the EventSource trait
impl EventSource for CronEventSource { /* ... */ }
impl EventSource for FsWatchEventSource { /* ... */ }
```

Third-party plugins can implement `EventSource` for custom event types (chain block events,
email inbox, RSS feeds, etc.) and register them with the runtime.
