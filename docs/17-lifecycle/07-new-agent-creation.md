# New Agent Creation (Post-Deletion)

> **Layer**: L0 Runtime + L1 Framework (same as initial creation, see `01-agent-creation.md`)
>
> **Prerequisites**: `docs/17-lifecycle/06-agent-deletion.md` (agent deletion), `docs/17-lifecycle/01-agent-creation.md` (creation flow)
>
> **Synapse traits**: Substrate (fresh Neuro store initialized, ready for restore), Router (fresh model routing), Gate (fresh gate pipeline)


> **Implementation**: Specified

---

## Overview

Creating a new agent after deleting a previous one follows the same creation flow documented in `01-agent-creation.md`. This document covers the specific considerations that apply when the new agent is intended to receive knowledge from a predecessor via backup/restore.

This is the third step of the four-step knowledge transfer process:

```
BACKUP → DELETE → CREATE → RESTORE
```

---

## Same Flow, Different Context

The creation flow is identical to first-time creation:

```bash
# Option A: Same config, fresh agent
roko init --config roko.toml

# Option B: New config from scratch
roko init --prompt "New strategy: focus on stablecoin yields instead of LP management"

# Option C: From template
roko init --template stablecoin-yield
```

The new agent receives:
- A new agent ID (`agent-{nanoid(12)}`)
- A fresh Neuro store (empty)
- A fresh Daimon state (neutral PAD vector)
- A fresh PLAYBOOK.md (empty, will be populated by Dream integration)
- Fresh Mesh connections (if enabled)
- For chain domain: a new wallet, a new ERC-8004 identity

The new agent is **not** the predecessor with a new name. It is a genuinely new agent. This mirrors Arendt's concept of natality (Arendt 1958) — every new agent is a moment of beginning, carrying inherited context but not inherited identity.

---

## Three Successor Patterns

When creating an agent to replace a deleted one, the operator faces a deliberate choice among three patterns:

### Pattern A: Clean Start (No Inheritance)

The operator creates a fresh agent with no knowledge from the predecessor. This is appropriate when:

- The predecessor's strategy was fundamentally wrong
- Market conditions have changed so dramatically that old knowledge is harmful
- The operator wants to test a completely different approach

```bash
roko init --prompt "Completely new approach: passive index tracking"
```

The predecessor's backup exists on disk but is not restored. The new agent learns everything from scratch.

### Pattern B: Same Strategy, Fresh Knowledge

The operator creates a fresh agent with the same strategy but no inherited knowledge. This is appropriate when:

- The predecessor's knowledge has become stale (knowledge plateau)
- The operator wants the same goals but fresh learning
- The predecessor had accumulated corrupt or adversarial Engrams

```bash
roko init --config roko.toml   # Same config as predecessor
# No restore step — agent starts with empty Neuro
```

### Pattern C: Lineage Continuation (Selective Restore)

The operator creates a fresh agent and restores selected knowledge from the predecessor's backup. This is the pattern that replaces legacy "succession."

```bash
roko init --config roko.toml
roko neuro restore ./backups/agent-V1St-2026-04-12.neuro --confidence-decay 0.85
```

See `08-selective-restore.md` for the full restore specification.

---

## The Operator's Decision

The operator's choice among these three patterns is the replacement for the legacy system's automatic succession. In the old architecture, succession was triggered by death — the owner was notified and decided whether to continue the lineage. In the new architecture, the same decision happens — but it is triggered by the operator's deliberate act of deletion, not by an artificial death clock.

The decision framework mirrors the legacy system's three options:

| Legacy option | New equivalent | When to use |
|--------------|----------------|-------------|
| No successor (lineage ends) | Pattern A: Clean start | Strategy was wrong |
| New strategy (clean start) | Pattern B: Same strategy, fresh knowledge | Knowledge was stale |
| Continue lineage (inheritance enabled) | Pattern C: Selective restore | Knowledge is valuable |

The difference: the legacy system framed this as a mortality event ("your agent died, do you want to create a successor?"). The new system frames it as a resource management decision ("you've backed up and deleted the old agent, what do you want the new one to know?").

The research that motivated the legacy design — Rogers' Paradox on social learning (Rogers 1988), Enquist's critical social learners (Enquist et al. 2007), the Baldwin Effect on capacity-to-learn transfer (Baldwin 1896, Hinton & Nowlan 1987) — still applies. It just applies to the operator's conscious decision rather than an automatic system event.

---

## Identity and Continuity

### No Identity Preservation

The new agent receives a new ID, new wallet (chain domain), and new ERC-8004 identity (chain domain). There is no concept of "the same agent" across deletion and recreation. This is intentional and mirrors Parfit's argument in _Reasons and Persons_ (1984): what matters in survival is not numerical identity but psychological continuity and connectedness. The new agent has psychological connectedness to its predecessor through shared knowledge (if restored) — Parfit's Relation R — but it is not numerically identical.

### Lineage Tracking

For operators who want to track knowledge evolution across multiple agents, Roko provides optional lineage tracking:

```toml
[agent]
lineage_id = "my-trading-lineage"  # Shared across successor agents
generation = 3                      # Incremented on each creation
```

Lineage tracking is metadata only — it does not affect agent behavior. It provides the operator with a history of:
- How many agents have been created in this lineage
- What knowledge was backed up and restored at each generation
- Whether later generations improved on earlier ones (via efficiency metrics)

This replaces the legacy `LineageRecord` struct but without death causes, mortality metrics, or ratchet scores based on death testament quality. The metrics that matter are:
- Gate pass rate (is the agent producing correct outputs?)
- Cost efficiency (is the agent spending budget wisely?)
- Knowledge growth (is the Neuro store growing with validated Engrams?)
- Task completion (is the agent achieving its strategy goals?)

---

## Elevated Initial Exploration

When an agent is created with `generation > 0` (indicating it has a predecessor), the Daimon defaults to slightly elevated exploration temperature for the first 100 cognitive loop iterations:

```toml
[agent.successor]
initial_exploration_boost = 0.2    # +0.2 to exploration temperature
exploration_boost_duration = 100   # First 100 iterations
```

This ensures that a successor agent explores the current environment independently before settling into inherited strategy patterns. It is the computational equivalent of the Baldwin Effect — the successor has capacity to learn faster (thanks to inherited knowledge) but must still learn independently (thanks to elevated exploration).

The legacy system implemented this as "the first 100 ticks run at elevated exploration temperature (+0.2 above configured baseline)" — the mechanism is identical, but triggered by lineage metadata rather than death/rebirth.

---

## Anti-Proletarianization

Stiegler (2010, 2018) defined proletarianization as the process by which knowledge, formalized by a technique, escapes the individual who thereby loses it. A successor agent that merely executes inherited knowledge without developing its own understanding is a proletarianized agent.

The new architecture prevents proletarianization through:

1. **Confidence decay on restore** (0.85^N per generation): Inherited knowledge is not trusted at face value
2. **Elevated initial exploration**: The agent is architecturally biased toward independent learning
3. **PLAYBOOK.md non-transfer**: The predecessor's machine-evolved heuristics are available for reference but not automatically loaded as active heuristics — the new agent must develop its own
4. **Divergence tracking**: If the operator uses lineage tracking, the system records how much the new agent's learned knowledge diverges from the restored knowledge — low divergence is a warning sign

---

## Domain-Specific Successor Considerations

### Chain Domain (roko-chain)

When an agent operates in the blockchain domain, successor creation involves additional wallet and identity steps:

- **New wallet**: The successor receives a fresh wallet via the configured `CustodyMode` (Delegation, Embedded, or LocalKey). The predecessor's wallet is settled during deletion (see `06-agent-deletion.md`)
- **New ERC-8004 identity**: A new Korai Passport (soulbound ERC-721) is minted with fresh `capabilityList`, `reputationTracks`, and zero `slashHistory`
- **Reputation reset**: The successor starts with base reputation. Reputation is earned, not inherited. This prevents reputation laundering where an operator deletes a slashed agent and recreates it to escape penalties
- **Domain stakes**: Any domain stakes from the predecessor are returned during deletion. The successor must re-stake independently

### Coding Domain (roko-coding)

- **Codebase context**: Not inherited via backup/restore. The successor must re-index the target codebase independently (codebase structure changes over time)
- **Tool configurations**: MCP server configurations from `roko.toml` carry forward if the same config file is used
- **Language-specific patterns**: If restored, coding heuristics undergo the same 0.85^N confidence decay. Language-level patterns (Rust idioms, TypeScript conventions) are relatively stable and survive decay well

### Research Domain (roko-research)

- **Citation networks**: Inherited citation graphs are valuable and decay slowly (academic papers don't become stale as quickly as market data)
- **Research methodology heuristics**: Restored with confidence decay, but research methodology is relatively stable across generations
- **Topic expertise**: Domain-specific knowledge decays at the domain's natural rate (see `10-ebbinghaus-for-knowledge-not-agents.md` for domain decay rates)

---

## Lifecycle Position

New agent creation is the third step of the four-step knowledge transfer lifecycle:

| Step | Command | Layer | Description |
|------|---------|-------|-------------|
| 1. Backup | `roko neuro backup` | L1 Framework | Serialize knowledge state |
| 2. Delete | `roko delete` | L0 Runtime | Clean shutdown, free resources |
| 3. **Create** | **`roko init`** | **L0 Runtime + L1** | **New agent, fresh state** |
| 4. Restore | `roko neuro restore` | L1 Framework | Selective knowledge import |

Steps 1-2 and 3-4 are paired, but all four steps are independently executable. An operator can back up without deleting, delete without creating, or create without restoring. The lifecycle is modular and user-directed.

---

## Cross-References

- `docs/17-lifecycle/01-agent-creation.md` — Full creation flow specification
- `docs/17-lifecycle/08-selective-restore.md` — How to restore knowledge into the new agent
- `docs/17-lifecycle/05-knowledge-backup-export.md` — How the backup was created
- `docs/17-lifecycle/06-agent-deletion.md` — How the predecessor was deleted
