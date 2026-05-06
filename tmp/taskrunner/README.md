# Task Runner — Parallel Agent Execution System

## Overview

Run up to 20 agents in parallel to implement, test, wire, and verify tasks.
Each agent gets a worktree, a task file with full context, and clear exit criteria.

## How It Works

```
┌─────────────────────────────────────────────────────────────┐
│                        WAVE 0                               │
│  Foundation tasks (config, core types)                      │
│  Parallelism: up to 5 agents (shared dependencies)          │
├─────────────────────────────────────────────────────────────┤
│                     GATE: cargo build + test + clippy        │
├─────────────────────────────────────────────────────────────┤
│                        WAVE 1                               │
│  Independent fixes (IDE, terminal, runner, v2 quick wins)   │
│  Parallelism: up to 20 agents (no file conflicts)           │
├─────────────────────────────────────────────────────────────┤
│                     GATE: cargo build + test + clippy        │
├─────────────────────────────────────────────────────────────┤
│                     AUDIT WAVE                              │
│  Fresh agents verify wiring, run CLI commands, check output │
├─────────────────────────────────────────────────────────────┤
│                        WAVE 2                               │
│  Next batch of work...                                      │
└─────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
tmp/taskrunner/
├── README.md              ← You're here
├── dag.toml               ← Master DAG: tasks, dependencies, waves
├── STATUS.toml            ← Live state: claimed/done/verified
├── tasks/                 ← One .md file per task (full agent context)
│   ├── 001-*.md
│   └── ...
├── waves/                 ← Wave definitions (which tasks run together)
│   ├── wave-0.toml
│   └── ...
├── audits/                ← Post-wave audit reports
├── logs/                  ← Per-agent execution logs
├── scripts/               ← Automation scripts
│   ├── next.sh            ← Find next claimable task
│   ├── claim.sh           ← Claim a task for an agent
│   ├── complete.sh        ← Mark task status
│   ├── gate.sh            ← Run wave gate (build+test+clippy)
│   ├── status.sh          ← Show overall progress
│   ├── spawn.sh           ← Spawn an agent with a task in a worktree
│   ├── merge.sh           ← Merge completed worktree back
│   └── audit.sh           ← Spawn audit agent for verification
├── templates/             ← Templates for task files
│   ├── task.md            ← Task template
│   ├── audit.md           ← Audit template
│   └── agent-prompt.md    ← Prompt preamble for any agent
├── worktrees/             ← Gitignored; worktree tracking
└── AGENT.md               ← Instructions for agents (copy into agent context)
```

## Quick Start

```bash
# See what's available to work on
./scripts/status.sh

# Find next unclaimed task
./scripts/next.sh

# Spawn an agent on a task (creates worktree, prints agent prompt)
./scripts/spawn.sh 003

# After agent completes, merge its worktree
./scripts/merge.sh 003

# After a wave of tasks complete, run the gate
./scripts/gate.sh wave-1

# Spawn audit agents to verify wiring
./scripts/audit.sh wave-1
```

## Status Progression

A task is NOT done until all stages pass:

```
pending → claimed → implemented → tested → wired → verified → done
                                                        ↑
                                               audit agent checks this
```

| Status | Meaning |
|--------|---------|
| pending | Not started |
| claimed | Agent is working on it |
| implemented | Code written, compiles |
| tested | Unit/integration tests pass |
| wired | Called from a runtime path (not just tests) |
| verified | Exercised via CLI command, output confirmed |
| done | Audit agent confirmed wiring + verification |

## Rules

1. **One task per worktree.** Never have an agent work on multiple tasks.
2. **Wave gates are mandatory.** No wave N+1 work starts until wave N gate passes.
3. **Audit waves verify wiring.** A fresh agent (no context from implementation)
   reads the task's wire target and verification commands, runs them, confirms.
4. **STATUS.toml is the source of truth.** Task .md files are immutable context.
5. **No task is done without a wire target.** If you can't name a CLI command that
   exercises the code, the task isn't ready to be created.
