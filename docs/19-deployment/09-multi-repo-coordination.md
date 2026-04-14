# Multi-Repo Coordination

> A single Roko daemon can manage multiple repositories simultaneously, each with its own
> subscription schedule, plan directory, agent configuration, and gate pipeline. This document
> covers the isolation model, shared resources, cross-repo knowledge via the Agent Mesh,
> scheduling priorities, and the multi-repo loading algorithm.


> **Implementation**: Specified

---

## Overview

The daemon mode (see `04-daemon-launchd-macos.md` and `05-daemon-systemd-linux.md`) runs as a
single process that manages N repository subscriptions. Each subscription is isolated — a plan
run in repository A does not affect repository B — but they share system resources (CPU, memory,
network) and can optionally share knowledge through the Agent Mesh.

```
                     ┌─────────────────────────────────────┐
                     │         Roko Daemon Process          │
                     │                                      │
                     │  ┌──────────┐  ┌──────────┐        │
                     │  │ Repo A   │  │ Repo B   │  ...   │
                     │  │ cron 30m │  │ watch    │        │
                     │  │ 4 agents │  │ 2 agents │        │
                     │  │ .roko/   │  │ .roko/   │        │
                     │  └──────────┘  └──────────┘        │
                     │        │              │              │
                     │        ▼              ▼              │
                     │  ┌──────────────────────────┐       │
                     │  │    Shared Scheduler       │       │
                     │  │    (max 8 total agents)   │       │
                     │  └──────────────────────────┘       │
                     │        │              │              │
                     │        ▼              ▼              │
                     │  ┌──────────┐  ┌──────────┐        │
                     │  │ Agent    │  │ Agent    │        │
                     │  │ Pool     │  │ Pool     │        │
                     │  └──────────┘  └──────────┘        │
                     └─────────────────────────────────────┘
```

---

## Isolation Model

Each repository subscription maintains strict isolation:

### Filesystem Isolation

- Each repo has its own `.roko/` directory for state, signals, episodes, and gate results
- Plan files are read from the repo's own plan directory
- Executor snapshots are stored per-repo in `.roko/state/executor.json`
- Signal logs are per-repo in `.roko/signals.jsonl`
- Episode logs are per-repo in `.roko/episodes.jsonl`

### Process Isolation

- Agent processes spawned for repo A do not have access to repo B's files
- Each agent receives a working directory set to its repo's root
- The `ProcessSupervisor` in `roko-runtime` tracks agents per repo, ensuring that stopping
  a subscription only kills agents belonging to that subscription

### Configuration Isolation

- Each repo can override model selection, agent count, gate configuration, and other settings
  via its local `.roko/config.toml`
- API keys are shared (they come from the daemon's environment), but everything else is per-repo

### What Is NOT Isolated

- **CPU and memory**: All repos share the same system resources. The scheduler enforces limits.
- **API keys**: LLM provider keys are daemon-level, shared across all subscriptions.
- **Network**: All agent processes share the same network interface and provider rate limits.
- **The daemon process itself**: If the daemon crashes, all subscriptions stop.

---

## Multi-Repo Loading Algorithm

When the daemon starts, it loads subscriptions from the global config and resolves per-repo
overrides:

```rust
/// Load and merge all subscriptions from global config + per-repo overrides.
fn load_subscriptions(global_config: &GlobalConfig) -> Vec<ResolvedSubscription> {
    let defaults = &global_config.daemon.defaults;

    global_config.subscriptions.iter().map(|sub| {
        let repo_root = PathBuf::from(&sub.repo);

        // Load per-repo config if it exists
        let repo_config = repo_root.join(".roko/config.toml");
        let local_overrides = if repo_config.exists() {
            toml::from_str(&std::fs::read_to_string(&repo_config).unwrap_or_default())
                .unwrap_or_default()
        } else {
            LocalConfig::default()
        };

        // Merge: defaults → global subscription → local overrides
        ResolvedSubscription {
            repo: repo_root,
            model: local_overrides.agent.model
                .or(sub.model.clone())
                .unwrap_or_else(|| defaults.model.clone()),
            max_agents: local_overrides.agent.max_agents
                .or(sub.max_agents)
                .unwrap_or(defaults.max_agents),
            plan_dirs: local_overrides.subscription.plan_dirs
                .or(sub.plan_dirs.clone())
                .unwrap_or_else(|| vec!["plans/".to_string()]),
            triggers: merge_triggers(&sub, &local_overrides),
            gates: merge_gates(&defaults.gates, &sub.gates, &local_overrides.gates),
        }
    }).collect()
}
```

The merge order ensures that:
1. Daemon defaults provide sensible baseline values
2. Global subscription entries customize per-repo settings
3. Per-repo `.roko/config.toml` has final say (the repo owner controls their own config)

---

## Shared Scheduler

The daemon enforces a global limit on concurrent agent processes across all subscriptions. This
prevents a single repo from consuming all system resources:

```toml
# ~/.config/roko/config.toml
[daemon]
max_concurrent_runs = 4    # Max simultaneous plan runs across all repos
max_total_agents = 8       # Max total agent processes across all repos
```

### Scheduling Priority

When multiple subscriptions trigger simultaneously and the agent pool is full, the scheduler
uses a priority queue:

```rust
#[derive(Debug, Clone)]
struct QueuedRun {
    repo: PathBuf,
    trigger: TriggerType,
    priority: RunPriority,
    queued_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RunPriority {
    /// Webhook triggers get highest priority (external event, time-sensitive)
    Webhook = 0,
    /// Watch triggers get medium priority (file change, user is actively editing)
    Watch = 1,
    /// Cron triggers get lowest priority (scheduled, can wait)
    Cron = 2,
}
```

Within the same priority level, runs are ordered by queue time (FIFO). The scheduler dequeues
runs as agent slots become available:

```rust
async fn scheduler_loop(state: Arc<DaemonState>) {
    loop {
        // Wait for an agent slot to free up
        let permit = state.agent_semaphore.acquire().await.unwrap();

        // Dequeue the highest-priority run
        if let Some(run) = state.run_queue.lock().pop_front_by_priority() {
            tokio::spawn(async move {
                execute_plan_run(run, permit).await;
            });
        } else {
            // No queued runs — wait for a trigger event
            state.trigger_notify.notified().await;
        }
    }
}
```

### Per-Repo Agent Limits

Each subscription can specify its own `max_agents` limit, which is a ceiling within the
global limit:

```toml
# Global: max 8 agents total
[daemon]
max_total_agents = 8

# Repo A: up to 4 agents
[[subscriptions]]
repo = "/path/to/repo-a"
max_agents = 4

# Repo B: up to 2 agents
[[subscriptions]]
repo = "/path/to/repo-b"
max_agents = 2
```

If repo A is using 4 agents and repo B needs 2, the scheduler can accommodate both (4 + 2 = 6
≤ 8). If repo A is using 6 agents (exceeding its own limit of 4, which means it was configured
with a global-only limit), repo B must wait.

---

## Cross-Repo Knowledge Sharing

### Isolated by Default

By default, each repository's knowledge (Engrams in `.roko/signals.jsonl`, episodes in
`.roko/episodes.jsonl`, learned patterns in `.roko/learn/`) is strictly isolated. Repo A
cannot read repo B's knowledge.

### Shared via Agent Mesh

When the Agent Mesh is enabled, repositories can share knowledge through the peer-to-peer
Engram sharing protocol. This is configured per subscription:

```toml
[[subscriptions]]
repo = "/path/to/repo-a"

[subscriptions.mesh]
enabled = true
# Share insights and heuristics (but not raw signals)
share_kinds = ["Insight", "Heuristic", "Warning"]
# Only share Engrams with confidence > 0.7
min_confidence = 0.7
# Mesh group: repos in the same group can share
group = "nunchi-projects"
```

When mesh sharing is enabled:
1. After a successful plan run, the daemon exports qualifying Engrams (matching `share_kinds`
   and `min_confidence`) to the Agent Mesh
2. Before starting a plan run, the daemon queries the Agent Mesh for relevant Engrams from
   other repos in the same group
3. Imported Engrams are stored with provenance indicating their source repo

This enables **cross-domain insight resonance** — the HDC-based structural analogy detection
(threshold 0.526) can identify patterns that transfer between repositories. For example, a
heuristic learned in a web API project ("always validate input at the boundary") can be
surfaced when working on a CLI project.

### Collective Intelligence (C-Factor)

When multiple repos share through the Agent Mesh, the C-Factor metric tracks whether the
collective outperforms the sum of individual repos:

```
C-Factor = Collective Performance / Sum(Individual Performances)
```

When C-Factor > 1.0, the Mesh is providing superlinear value — insights from one repo are
improving work in other repos. The daemon tracks this metric in
`~/.local/state/roko/mesh-metrics.json`.

See the Collective Intelligence documentation for the full C-Factor formulation and the
diagnostic signals (turn-taking equality, knowledge flow rate, cross-domain transfer, emergent
coordination) inspired by Woolley et al. (Science 330(6004), 2010).

---

## Multi-Repo Status

The `roko daemon status` command shows all subscriptions and their current state:

```bash
$ roko daemon status

Roko Daemon
  PID:        12345
  Uptime:     3d 14h 22m
  Agents:     5/8 (3 available)
  Queue:      1 pending

Subscriptions:
  ┌─────────────────────────────────────────────────────────────────┐
  │ /Users/will/dev/project-a                                       │
  │   Trigger:  cron (*/30 * * * *)                                │
  │   Status:   running (3/5 tasks, 2 agents)                      │
  │   Last:     2h ago (success)                                   │
  │   Next:     in 12 minutes                                      │
  │   Mesh:     enabled (group: nunchi-projects, shared: 14)       │
  ├─────────────────────────────────────────────────────────────────┤
  │ /Users/will/dev/project-b                                       │
  │   Trigger:  watch (.roko/prd/)                                 │
  │   Status:   idle                                               │
  │   Last:     15m ago (success, 3 tasks)                         │
  │   Mesh:     enabled (group: nunchi-projects, shared: 7)        │
  ├─────────────────────────────────────────────────────────────────┤
  │ /Users/will/dev/project-c                                       │
  │   Trigger:  webhook (/hook/project-c)                          │
  │   Status:   queued (waiting for agent slot)                    │
  │   Last:     never                                              │
  │   Mesh:     disabled                                           │
  └─────────────────────────────────────────────────────────────────┘
```

---

## Error Handling and Resilience

### Per-Repo Failure Isolation

A failure in one repo does not affect other repos:

- If a plan run in repo A fails (gate failure, agent crash), the daemon logs the failure,
  records it in the subscription state, and continues managing other repos
- The failed repo's next scheduled run proceeds normally (the daemon does not disable
  subscriptions on failure)
- Persistent failures (3+ consecutive failures) trigger a warning in `roko daemon status`
  and an optional notification (configurable webhook)

### Resource Exhaustion Protection

If a repo's plan run consumes excessive resources:

- **Memory**: The `ProcessSupervisor` monitors agent memory usage. If an agent exceeds the
  configured limit (default: 2GB per agent), it is killed.
- **Time**: Each plan run has a configurable timeout (default: 1 hour). If the timeout expires,
  running agents are killed and the run is marked as timed out.
- **Disk**: The daemon monitors `.roko/` directory size. If it exceeds the configured limit
  (default: 1GB), old signals and episodes are garbage-collected.

```toml
# Per-subscription resource limits
[[subscriptions]]
repo = "/path/to/repo-a"

[subscriptions.limits]
max_run_duration = "1h"
max_agent_memory_mb = 2048
max_roko_dir_size_mb = 1024
```

---

## Git Operations for Multi-Repo

When the daemon triggers a plan run for a subscription, it performs these git operations:

```rust
/// Prepare a repository for a plan run.
async fn prepare_repo(sub: &ResolvedSubscription) -> Result<()> {
    let repo = &sub.repo;

    // 1. Check that the repo exists and is a git repo
    if !repo.join(".git").exists() {
        anyhow::bail!("Not a git repository: {}", repo.display());
    }

    // 2. Stash any uncommitted changes (safety net)
    git_stash(repo).await?;

    // 3. Pull latest changes (if remote is configured)
    if git_has_remote(repo).await? {
        git_pull(repo).await?;
    }

    // 4. Create a worktree for isolated execution (optional)
    if sub.use_worktree {
        let worktree_path = repo.join(".roko/worktrees")
            .join(format!("run-{}", chrono::Utc::now().timestamp()));
        git_worktree_add(repo, &worktree_path).await?;
    }

    Ok(())
}
```

The worktree option (`use_worktree = true` in subscription config) runs plan execution in a
separate git worktree, ensuring that the main working tree is not affected by agent-generated
changes. This is critical for repos where the user is actively working — the daemon's changes
happen in a parallel worktree and can be reviewed before merging.

---

## Current Status

Multi-repo coordination is at **Tier 3H** priority (P2 — planned), dependent on the daemon
mode infrastructure. The subscription configuration format is designed and documented. The
shared scheduler, per-repo isolation, and Agent Mesh integration require the daemon event loop
to be wired.

The `ProcessSupervisor` in `roko-runtime` already supports tracking multiple agent groups,
which maps directly to the per-repo isolation model. The `roko-orchestrator` DAG executor
already supports multiple concurrent plan runs via the `PlanRunner` struct.
