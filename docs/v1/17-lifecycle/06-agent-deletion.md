# Agent Deletion

> **Layer**: L0 Runtime (process lifecycle, supervision) + L1 Framework (resource release)
>
> **Prerequisites**: `docs/17-lifecycle/02-provisioning.md` (provisioning pipeline — deletion is its reverse), `docs/17-lifecycle/05-knowledge-backup-export.md` (backup before deletion)
>
> **Synapse traits**: Substrate (Neuro store flushed and closed), Policy (deletion event emitted to Mesh), Router (model routing torn down), Composer (pending context discarded)


> **Implementation**: Specified

---

## Overview

Agent deletion in Roko is **always user-initiated**. There is no natural death, no stochastic termination, no vitality-driven shutdown. The user decides when an agent should stop running, and the deletion process follows a clean, predictable shutdown sequence.

Deletion is the second step of the four-step knowledge transfer process:

```
BACKUP → DELETE → CREATE → RESTORE
```

The operator should back up the agent's Neuro before deleting (see `05-knowledge-backup-export.md`). Deletion without backup is allowed but produces a confirmation warning.

---

## Deletion Command

```bash
# Clean shutdown with confirmation prompt
roko delete

# Clean shutdown without confirmation (CI/CD pipelines)
roko delete --yes

# Force kill (no graceful shutdown, skips pending work)
roko delete --force

# Delete with automatic pre-deletion backup
roko delete --backup

# Delete and archive all data (Neuro, logs, episodes)
roko delete --archive ./archives/agent-V1St/
```

---

## Clean Shutdown Sequence

The clean shutdown reverses the provisioning pipeline. Each step undoes the corresponding provisioning step, in reverse order.

```
1. Signal shutdown to cognitive loop                    ~instant
   - Current turn completes (if in progress)
   - No new turns started
   - Pending tool invocations complete or timeout (10s max)

2. Flush Neuro store                                   ~3-5s
   - All in-memory Engrams written to persistent storage
   - Decay state snapshot persisted
   - Tier assignments persisted
   - If --backup flag: automatic backup created

3. Deregister from Mesh                                ~1-2s
   - Send deregistration message to Mesh relay
   - Close outbound WebSocket connection
   - Peers notified of departure

4. Release tool handles                                ~instant
   - Close open file handles
   - Disconnect from external services
   - Cancel pending HTTP requests

5. Shut down model routing                             ~instant
   - Close inference provider connections
   - Flush routing metrics to disk

6. Release compute resources                           ~instant (self-hosted)
   - Stop health server                                ~2-5s (hosted: VM destruction)
   - Release allocated memory
   - For hosted: control plane destroys VM

7. Archive episode log                                 ~1-2s
   - Flush `.roko/episodes.jsonl` to disk
   - Flush `.roko/learn/efficiency.jsonl` to disk
   - If --archive flag: copy all data to archive location

8. Mark agent as deleted in state                      ~instant
   - Update `.roko/state/executor.json`
   - Write deletion record with timestamp and reason
```

**Total shutdown time**: 5-15 seconds for clean shutdown.
**Force shutdown time**: ~instant (SIGKILL, no graceful steps).

---

## Shutdown Budget

The clean shutdown has a 30-second budget for all steps. If any step exceeds its allocated time, it is skipped and the next step proceeds. This prevents a hung Mesh connection or stuck tool invocation from blocking the entire shutdown.

| Step | Budget | Fallback if exceeded |
|------|--------|---------------------|
| Complete current turn | 10s | Abort turn, discard partial results |
| Flush Neuro | 5s | Write what's flushed, log warning |
| Deregister from Mesh | 3s | Close socket without clean deregistration |
| Release tools | 2s | Force close all handles |
| Shut down routing | 1s | Drop connections |
| Release compute | 5s (hosted) | Force destroy VM |
| Archive episodes | 3s | Write what's archived |
| Mark deleted | 1s | Write to stderr and exit |

---

## Deletion Confirmation

Without `--yes` or `--force`, deletion prompts for confirmation:

```
$ roko delete

WARNING: About to delete agent agent-V1StGXR8_Z5j

  Agent name:    my-monitoring-agent
  Uptime:        14 days, 6 hours
  Neuro entries: 12,847 Engrams
  Last backup:   2026-04-10T08:00:00Z (2 days ago)

  ⚠ No backup exists from today. Consider running:
    roko neuro backup

  Type 'delete' to confirm: █
```

If no backup exists at all, the warning is more prominent:

```
  ⚠ NO BACKUP EXISTS for this agent's Neuro store.
    Deletion will permanently lose all 12,847 Engrams.
    Run `roko neuro backup` first, or use `roko delete --backup`
    to create an automatic backup before deletion.
```

---

## What Deletion Destroys

Deletion destroys the **agent process and its runtime state**. It does NOT destroy:

- **Neuro backups**: All backups in `.roko/backups/` are preserved
- **Episode logs**: `.roko/episodes.jsonl` remains on disk
- **Efficiency logs**: `.roko/learn/efficiency.jsonl` remains on disk
- **Configuration**: `roko.toml`, `STRATEGY.md` remain on disk
- **Mesh-synced data**: Backups synced to Mesh relay are preserved (per relay retention policy)

The philosophy: deletion removes the running agent, not its history. An operator can always create a new agent and restore knowledge from a backup. The backup/restore cycle is the replacement for legacy "succession" — but it is explicit, user-controlled, and does not involve death.

---

## Chain Domain: Wallet Settlement

For chain-domain agents, deletion includes wallet settlement:

### Delegation Mode (Recommended)

No settlement needed. The delegation grant simply expires. The operator retains full control of their funds in their own wallet. This is the cleanest deletion path — the agent held a disposable session key, and the key is zeroized from memory at shutdown.

### Embedded Mode (Privy)

Remaining funds are swept to the operator's wallet:

1. Query balance of agent's Privy server wallet
2. Initiate sweep transaction to operator's address
3. Wait for transaction confirmation (timeout: 60s)
4. If sweep fails: log failure, schedule retry in reconciliation job

### LocalKey Mode

The delegation grant expires. Key material is zeroized from memory. The encrypted keystore file remains on disk for potential future use.

---

## On-Chain Deregistration

For chain-domain agents with ERC-8004 identity:

1. Call `AgentRegistry.deregister(agent_id)` on Korai chain
2. Wait for transaction confirmation
3. ERC-8004 identity marked as deregistered (not burned — historical record preserved)

Deregistration is optional but recommended. An underegistered agent identity accumulates KORAI demurrage on any staked tokens, and peer agents may attempt to communicate with a non-existent agent.

---

## Graceful vs. Force Deletion

| Aspect | Graceful (`roko delete`) | Force (`roko delete --force`) |
|--------|--------------------------|-------------------------------|
| Current turn | Completes | Aborted |
| Neuro flush | Yes | No |
| Mesh deregistration | Yes | No |
| Wallet settlement | Yes | No |
| Episode archival | Yes | No |
| Data on disk | Preserved | Preserved |
| Time | 5-15s | ~instant |
| When to use | Normal operation | Hung process, emergency |

Force deletion should be used only when the agent process is unresponsive. All data on disk (backups, logs, config) is preserved regardless of deletion mode.

---

## Post-Deletion State

After deletion, the agent directory contains:

```
.roko/
├── backups/
│   └── agent-V1StGXR8_Z5j/
│       └── 2026-04-12T14-30-00Z.neuro     # Preserved backup
├── episodes.jsonl                           # Preserved episode log
├── learn/
│   ├── efficiency.jsonl                     # Preserved efficiency log
│   ├── cascade-router.json                  # Preserved routing data
│   └── gate-thresholds.json                 # Preserved gate data
├── state/
│   └── executor.json                        # Updated: status = "deleted"
├── neuro/
│   └── (empty or flushed data)             # Neuro store (flushed, not destroyed)
roko.toml                                    # Preserved config
STRATEGY.md                                  # Preserved strategy
PLAYBOOK.md                                  # Preserved heuristics
```

This directory is a complete record of the agent's existence. A new agent can be created in the same directory, and it will coexist with the historical data.

---

## Irreversibility

Deletion of the agent **process** is irreversible. Once deleted, the agent cannot be "undeleted" — you create a new agent. This is intentional: it prevents the operator from treating deletion as a pause mechanism (use `roko pause` for that) and ensures that knowledge transfer through backup/restore is explicit.

Deletion of the agent's **data** requires separate action (`rm -rf .roko/`). This two-step design prevents accidental loss of valuable knowledge.

---

## Cross-References

- `docs/17-lifecycle/05-knowledge-backup-export.md` — Back up before deleting
- `docs/17-lifecycle/07-new-agent-creation.md` — Create a fresh agent after deletion
- `docs/17-lifecycle/08-selective-restore.md` — Restore knowledge into the new agent
- `docs/17-lifecycle/02-provisioning.md` — Provisioning (deletion reverses this)
