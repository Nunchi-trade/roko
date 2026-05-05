# 05 — Event Sources and Templates

> Event sources as Trigger Cells and agent templates as Graph templates (Rack pattern).
> Five trigger types fire Graphs. Subscriptions bind triggers to templates. The dispatch
> loop is a React Cell that pattern-matches Pulses and spawns Flows.

**Parent spec**: [13-TRIGGERS.md](../../unified/13-TRIGGERS.md), [14-TOOLS.md](../../unified/14-TOOLS.md), [03-GRAPH.md](../../unified/03-GRAPH.md)

---

## 1. Core Insight

An agent does not run continuously. It is **spawned in response to events**. Something happens
(a file changes, a PR opens, a cron tick fires, a Slack command arrives), and an agent is
instantiated from a template to handle it.

In unified terms:
- Event sources are **Trigger Cells** — they implement the Trigger protocol (`arm / disarm`)
  and publish `TriggerFired` Pulses on Bus when their condition is met.
- Agent templates are **Graph templates** — parameterized Graph definitions (Rack pattern) that
  can be instantiated as Flows when triggered.
- Subscription configuration is a **binding** — the declarative link between a Trigger Cell
  (which fires Pulses) and a Graph template (which spawns Flows).
- The dispatch loop is a **React Cell** — it subscribes to all `trigger:*:fired` Pulses,
  matches against subscriptions, and spawns the bound Graph as a Flow.

This architecture means event handling is not a special subsystem — it is ordinary Cells,
ordinary Bus pub/sub, and ordinary Graph instantiation.

---

## 2. Five Trigger Cell Types

Each Trigger Cell implements the Trigger protocol with a different push mechanism:

### 2.1 Cron Trigger

Fires at scheduled intervals using standard cron expressions.

| Property | Value |
|---|---|
| Push mechanism | Tokio interval timer |
| Configuration | `schedule = "0 9 * * MON-FRI"` |
| Pulse topic | `trigger:cron:{name}:fired` |
| Payload | `{ schedule, fired_at }` |

```rust
/// Cron Trigger Cell — fires a Pulse on schedule.
pub struct CronTrigger {
    pub name: String,
    pub schedule: String,  // Standard cron expression
}

impl TriggerProtocol for CronTrigger {
    async fn arm(&self, binding: &TriggerBinding, bus: Arc<dyn Bus>) -> Result<TriggerHandle> {
        let schedule = self.schedule.clone();
        let topic = format!("trigger:cron:{}:fired", self.name);
        let bus_clone = bus.clone();

        tokio::spawn(async move {
            let scheduler = tokio_cron_scheduler::JobScheduler::new().await.unwrap();
            let job = Job::new_async(&schedule, move |_, _| {
                let bus = bus_clone.clone();
                let topic = topic.clone();
                Box::pin(async move {
                    bus.publish(Pulse {
                        topic,
                        payload: json!({ "schedule": schedule, "fired_at": Utc::now() }),
                    }).await;
                })
            }).unwrap();
            scheduler.add(job).await.unwrap();
            scheduler.start().await.unwrap();
        });

        Ok(TriggerHandle { id: self.id(), state: TriggerState::Armed })
    }
}
```

### 2.2 FileWatch Trigger

Fires when files matching a glob pattern change on the filesystem.

| Property | Value |
|---|---|
| Push mechanism | OS filesystem events (notify crate) |
| Configuration | `watch_path`, `watch_glob`, debounce |
| Pulse topic | `trigger:watch:{name}:fired` |
| Payload | `{ paths: [changed_files], event_kind }` |

```rust
/// FileWatch Trigger Cell — fires on filesystem changes.
pub struct FileWatchTrigger {
    pub name: String,
    pub watch_path: PathBuf,
    pub watch_glob: Option<String>,
    pub debounce_ms: u64,  // Collapse rapid changes
}

impl TriggerProtocol for FileWatchTrigger {
    async fn arm(&self, binding: &TriggerBinding, bus: Arc<dyn Bus>) -> Result<TriggerHandle> {
        let (tx, rx) = std::sync::mpsc::channel();
        let debouncer = notify_debouncer_mini::new_debouncer(
            Duration::from_millis(self.debounce_ms), tx
        )?;
        debouncer.watcher().watch(&self.watch_path, RecursiveMode::Recursive)?;

        // Background task: filter events by glob, publish Pulses
        tokio::spawn(async move {
            while let Ok(Ok(events)) = rx.recv() {
                let paths = filter_by_glob(events, &glob_pattern);
                if !paths.is_empty() {
                    bus.publish(Pulse {
                        topic: format!("trigger:watch:{}:fired", name),
                        payload: json!({ "paths": paths }),
                    }).await;
                }
            }
        });

        Ok(TriggerHandle { id: self.id(), state: TriggerState::Armed })
    }
}
```

### 2.3 Webhook Trigger (GitHub)

Fires when GitHub sends an HTTP POST for repository events.

| Property | Value |
|---|---|
| Push mechanism | HTTP handler on `:6677` (Axum route) |
| Configuration | `pattern = "webhook.github.push"` |
| Pulse topic | `trigger:webhook:github:{event}:fired` |
| Payload | Full webhook body + parsed metadata |

Event kinds:
- `webhook.github.push` — code pushed to a branch
- `webhook.github.pull_request` — PR opened/updated/closed
- `webhook.github.pull_request_review` — review submitted
- `webhook.github.issues` — issue opened/closed/labeled

### 2.4 Webhook Trigger (Slack)

Fires when Slack sends events via Socket Mode or HTTP.

| Property | Value |
|---|---|
| Push mechanism | WebSocket (Socket Mode) or HTTP POST |
| Configuration | `pattern = "webhook.slack.slash_command"` |
| Pulse topic | `trigger:webhook:slack:{event}:fired` |
| Payload | Slack event body |

Event kinds:
- `webhook.slack.message` — message in channel
- `webhook.slack.slash_command` — `/command` invoked
- `webhook.slack.interactive` — button/action clicked

### 2.5 Generic Webhook Trigger

Fires for any HTTP POST that does not match a specific platform handler.

| Property | Value |
|---|---|
| Push mechanism | HTTP handler on `:6677` |
| Configuration | `pattern = "webhook.custom"` |
| Pulse topic | `trigger:webhook:custom:fired` |
| Payload | Raw HTTP body + headers |

---

## 3. Agent Templates as Graph Templates

An agent template is a **parameterized Graph definition** — a Rack with macros (configuration
knobs) and slots (connection points). When a trigger fires, the template is instantiated as a
Flow with the trigger's payload as input.

### Template Schema

```rust
/// Agent template: a parameterized Graph definition.
pub struct AgentTemplate {
    /// Unique name (identifier for subscription binding).
    pub name: String,
    /// Human-readable description.
    pub description: String,

    // --- Graph configuration ---
    pub model: String,               // LLM model to use
    pub role: String,                // Agent role (implementer, reviewer, etc.)
    pub max_turns: u32,              // Maximum tool-call turns
    pub system_prompt: String,       // Full system instructions

    // --- Trigger binding ---
    pub triggers: Vec<String>,       // Event patterns this template handles

    // --- Connect Cells to load ---
    pub mcp_servers: Vec<String>,    // MCP servers required

    // --- Verify Pipeline ---
    pub gates: Option<Vec<String>>,  // Gate pipeline after completion

    // --- Flow constraints ---
    pub max_concurrent: Option<u32>, // Max simultaneous instances
    pub cooldown_secs: Option<u64>,  // Min seconds between triggers

    // --- Learning ---
    pub experiment: Option<ExperimentConfig>,  // A/B testing
}
```

### Template as TOML

```toml
name = "pr-review-agent"
description = "Automated PR review with codebase context"
model = "claude-sonnet-4-20250514"
role = "reviewer"
max_turns = 12
max_concurrent = 3

triggers = ["webhook.github.pull_request"]
mcp_servers = ["github"]

[experiment]
name = "review-depth"
variants = ["concise", "thorough"]
metric = "review_resolution_rate"

system_prompt = """
You are an expert code reviewer...
"""
```

### Template Instantiation

When a trigger fires and matches a template, the runtime:
1. Loads the template definition
2. Resolves MCP servers (start processes, discover tools)
3. Assembles ToolContext (merge built-in + MCP tools, filter by role)
4. Creates the agent Flow (Graph instance with RunId)
5. Injects the trigger payload as the initial input Signal
6. Runs the cognitive loop (up to `max_turns`)
7. On completion: runs gate pipeline (if configured)
8. Records episode, emits completion Pulse

---

## 4. Subscription Configuration

Subscriptions are the binding between Trigger Cells and Graph templates. They are declared in
`.roko/subscriptions.toml`:

```toml
# .roko/subscriptions.toml

[[subscription]]
# Required: which Trigger Pulse topic to match
pattern = "webhook.github.push"

# Required: which Graph template to instantiate
agent_template = "auto-plan-agent"

# Optional: additional filter conditions (AND logic)
filter = { ref = "refs/heads/main" }

# Optional: file path glob filter
path_filter = ".roko/prd/**"

# Optional: cron schedule (for cron triggers)
schedule = "0 9 * * MON-FRI"

# Optional: concurrency limit
max_concurrent = 1

# Optional: minimum seconds between triggers
cooldown_secs = 300

# Optional: enable/disable
enabled = true
```

### Filter Matching Logic

All filter conditions must match (AND logic). Path filters match against changed files:

```rust
/// Subscription matching: all conditions must pass.
fn matches(sub: &Subscription, event: &TriggerEvent) -> bool {
    // Pattern match (required)
    if sub.pattern != event.source.kind() { return false; }

    // Filter match (all fields must match)
    if let Some(ref filter) = sub.filter {
        for (key, expected) in filter {
            let actual = event.payload.get(key);
            if !value_matches(actual, expected) { return false; }
        }
    }

    // Path filter (at least one changed file must match glob)
    if let Some(ref path_filter) = sub.path_filter {
        let glob = glob::Pattern::new(path_filter).unwrap();
        let paths = event.payload.get("paths").and_then(|p| p.as_array());
        if let Some(paths) = paths {
            if !paths.iter().any(|p| glob.matches(p.as_str().unwrap_or(""))) {
                return false;
            }
        }
    }

    true
}
```

---

## 5. Dispatch Loop as React Cell

The dispatch loop is a **React Cell** — it watches Pulses on Bus and emits Signals (agent
spawn commands) in response. It is event-driven, source-agnostic, and stateless between
dispatches.

```rust
/// Dispatch loop: React Cell watching trigger:*:fired Pulses.
pub async fn dispatch_loop(state: Arc<AppState>) {
    // Subscribe to all trigger events
    let mut rx = state.bus.subscribe("trigger:*:fired");

    while let Ok(pulse) = rx.recv().await {
        let event: TriggerEvent = serde_json::from_value(pulse.payload)?;
        let subs = state.subscriptions.read().await;

        for sub in subs.iter() {
            if !sub.enabled { continue; }
            if !matches(sub, &event) { continue; }

            // Concurrency check
            if let Some(max) = sub.max_concurrent {
                let running = state.running_flows(&sub.agent_template).await;
                if running >= max { continue; }
            }

            // Cooldown check
            if let Some(cooldown) = sub.cooldown_secs {
                let last = state.last_trigger_time(&sub.agent_template);
                if last.elapsed() < Duration::from_secs(cooldown) { continue; }
            }

            // Spawn the Graph template as a Flow
            state.spawn_flow(&sub.agent_template, event.payload.clone()).await;

            // Publish dispatch notification
            state.bus.publish(Pulse {
                topic: format!("dispatch.{}.spawned", sub.agent_template),
                payload: json!({
                    "template": sub.agent_template,
                    "trigger": event.source.kind(),
                    "trace_id": event.trace_id,
                }),
            }).await;
        }
    }
}
```

### Source-Agnostic Routing

The dispatch loop does not know or care whether the Pulse came from a cron timer, a file
watcher, or a GitHub webhook. All triggers produce the same `TriggerEvent` shape on Bus.
This means:
- Adding a new trigger type requires no changes to the dispatch loop
- Subscriptions can match any trigger type with the same filter syntax
- The same template can be triggered by multiple event sources

---

## 6. The 18 Agent Templates

Roko ships 18 pre-built Graph templates organized by repository context:

### Collaboration Repository (6 templates)

| Template | Trigger | Role | Purpose |
|---|---|---|---|
| `doc-lifecycle-agent` | github.push | operator | Manage document status transitions |
| `digest-agent` | cron (Monday 9am) | researcher | Weekly change digest to Slack |
| `meeting-agent` | github.push | researcher | Extract actions from call notes |
| `sync-agent` | cron + slack command | operator | Cross-repo document sync |
| `conflict-detector-agent` | github.push | researcher | Detect contradicting claims |
| `freshness-agent` | cron (daily) | operator | Flag stale documents |

### Knowledge-Base Repository (5 templates)

| Template | Trigger | Role | Purpose |
|---|---|---|---|
| `pm-board-agent` | cron + github.push | operator | PM board sync and validation |
| `enrich-agent` | github.push | researcher | Cross-reference enrichment |
| `triage-agent` | github.issues/PR | planner | Auto-triage with labels and workstreams |
| `pm-health-agent` | cron (weekday 9am) | operator | Health reports, blocked task alerts |
| `action-tracker-agent` | cron (daily 8am) | researcher | Action item reconciliation |

### Roko Repository (7 templates)

| Template | Trigger | Role | Purpose |
|---|---|---|---|
| `pr-review-agent` | github.pull_request | reviewer | Automated code review |
| `slack-notify-agent` | agent.completed/failed | operator | Structured Slack notifications |
| `auto-plan-agent` | github.push + prd.published | planner | Generate plans from PRDs |
| `code-implementer-agent` | prd.plan_approved | implementer | Execute tasks, push PRs |
| `gate-fixer-agent` | agent.gate_failed | implementer | Auto-fix gate failures |
| `prd-ingestion-agent` | github.push | operator | Sync PRDs across repos |
| `review-response-agent` | github.pull_request_review | implementer | Respond to review comments |

---

## 7. Multi-Repository Configuration

The `roko-serve` HTTP control plane watches subscriptions from multiple repositories:

```toml
# roko-serve.toml — multi-repo configuration
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

All templates and subscriptions from all repos are loaded into a single dispatch loop. The React
Cell matches events against all subscriptions regardless of source repository.

### Priority Queue

When multiple triggers fire simultaneously, the dispatch loop prioritizes:

1. **Webhooks** (real-time, external) — highest priority
2. **File watchers** (local changes, time-sensitive) — medium priority
3. **Cron** (scheduled, can wait) — lowest priority

This ensures that interactive events (someone opened a PR) are handled before batch operations
(Monday morning digest).

---

## 8. A/B Experiments on Templates

Templates support A/B testing via the `experiment` field:

```toml
[experiment]
name = "review-depth"
variants = ["concise", "thorough"]
metric = "review_resolution_rate"
```

When the template is instantiated:
1. The experiment system selects a variant (Thompson sampling)
2. The variant modifies the template parameters (e.g., different system prompt, max_turns)
3. The resulting Flow is tagged with the variant
4. After completion, the metric is measured and attributed to the variant
5. Over time, the better variant gets selected more often

This is the predict-publish-correct pattern applied to template configuration:
- **Predict**: variant selection is a prediction about which configuration works better
- **Publish**: the selected variant and outcome are published as Pulses
- **Correct**: metric comparison updates the selection policy (bandit algorithm)

---

## 9. Template Lifecycle

```
[1] Define (TOML)
       │
       v
[2] Discover (scan .roko/templates/ across repos)
       │
       v
[3] Validate (check required fields, trigger patterns, MCP server refs)
       │
       v
[4] Bind (match against subscriptions)
       │
       v
[5] Wait (Trigger Cell armed, dispatch loop listening)
       │
       v
[6] Fire (TriggerEvent Pulse received, subscription matches)
       │
       v
[7] Instantiate (resolve MCP, assemble ToolContext, create Flow)
       │
       v
[8] Execute (cognitive loop runs up to max_turns)
       │
       v
[9] Gate (Verify Pipeline: compile, test, clippy, diff)
       │
       v
[10] Record (episode logged, metrics published, completion Pulse)
```

---

## What This Enables

1. **Declarative automation** — define what triggers what, in TOML. No imperative code needed
   to wire events to agent execution.
2. **Source-agnostic dispatch** — the same dispatch loop handles cron, webhooks, file changes,
   and internal events. Adding a new trigger type is adding a new Cell, not modifying the
   dispatch logic.
3. **Multi-repo orchestration** — a single `roko-serve` instance manages automation across
   multiple repositories with shared dispatch and priority.
4. **Experimental templates** — A/B testing on template configuration enables continuous
   improvement of agent behavior without manual tuning.
5. **Composable triggers** — because triggers are Cells and subscriptions are data, complex
   trigger conditions (AND, debounce, cooldown, concurrency) compose naturally.

---

## Feedback Loops

- **Trigger frequency → cooldown adjustment**: if a trigger fires too frequently (noisy file
  watcher), the cooldown adapts upward. If it fires rarely but is high-value, cooldown tightens.
- **Template success rate → experiment convergence**: A/B experiments converge on winning variants
  via Thompson sampling. Losing variants get less traffic over time.
- **Concurrency utilization → limit tuning**: if `max_concurrent` is never reached, the limit
  may be too high (wasted slots). If it is always reached, requests queue and latency increases.
- **Gate failure rate per template → template revision**: templates whose Flows frequently fail
  gates are flagged for system prompt revision.

---

## Open Questions

1. **Trigger composition** — can multiple triggers be AND-ed (fire only when both conditions
   are true simultaneously)? Current model is OR (any matching trigger fires independently).
2. **Template inheritance** — can templates inherit from a base template and override specific
   fields? (e.g., all roko-repo templates share common system prompt preamble.)
3. **Dynamic template creation** — can an agent create new templates at runtime, or are
   templates exclusively operator-defined?
4. **Cross-repo trigger correlation** — when a file change in repo A should trigger an agent
   in repo B context, how is the context switch handled?
5. **Trigger backpressure** — when triggers fire faster than agents can complete, what is the
   queuing and shedding strategy?

---

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Cron scheduler (tokio-cron-scheduler) | `crates/roko-serve/src/scheduler.rs` | Shipped |
| File system watcher (notify + debouncer) | `crates/roko-serve/src/fswatcher.rs` | Shipped |
| GitHub webhook handler | `crates/roko-serve/src/routes/` | Shipped |
| Slack event handler | `crates/roko-serve/src/routes/` | Planned |
| Generic webhook handler | `crates/roko-serve/src/routes/` | Shipped |
| Dispatch loop (React Cell) | `crates/roko-serve/src/state.rs` | Shipped |
| Subscription parser (TOML) | `crates/roko-serve/src/` | Shipped |
| Template loader + validator | `crates/roko-serve/src/` | Shipped |
| Multi-repo configuration | `crates/roko-serve/src/` | Shipped |
| A/B experiment integration | `crates/roko-learn/src/` | Shipped |
| Priority queue for concurrent triggers | `crates/roko-serve/src/` | Planned |
| Template inheritance | (not yet designed) | Open question |
| Trigger composition (AND logic) | (not yet designed) | Open question |
| PRD publish subscriber (internal trigger) | `crates/roko-serve/src/` | Shipped |
