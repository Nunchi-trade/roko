# Open Questions for Iteration

UX and architecture questions that need resolution before the design is final. Grouped by topic, ranked by impact at the bottom.

---

## 1. Dashboard Aggregation UX

**Question**: How does the dashboard present data from N independent agent servers as a unified experience?

| State | Description |
|-------|-------------|
| Current | Dashboard talks to one mirage-rs instance. Simple. |
| Future | Dashboard queries N agent servers + chain + roko-serve. Complex. |

Sub-questions:

- **Loading states** -- Show per-agent loading? Wait for all? Skeleton UI per card?
- **Failure handling** -- If 2 of 5 agents are unreachable, show partial data with error indicators? Or fail the whole view?
- **Staleness** -- Agent data has different freshness. Show last-updated timestamps per card?
- **Performance** -- N parallel requests per page load. Need client-side caching (SWR/React Query with TTLs)?
- **Sorting/filtering** -- Predictions from N agents need cross-agent ranking. Unified scoring function?

---

## 2. "Add an Agent" Flow

**Question**: How does a user who runs the dashboard but doesn't use roko-serve add an agent?

| Scenario | Description |
|----------|-------------|
| Operator mode | User runs roko-serve. Agents register automatically. Dashboard sees them via chain. |
| Network mode | User only runs dashboard. Discovers agents on-chain. Connects to their endpoints. |
| Manual mode | User pastes agent endpoint URL. Dashboard adds to local config. |

The dashboard currently assumes a single local backend. The per-agent model turns it into something closer to a "wallet" that discovers and connects to services.

Sub-questions:

- Is there a "default agent" concept? (User's own agent vs. network agents)
- Can the dashboard subscribe to specific agents? (Follow/unfollow model)
- How does the dashboard get auth tokens for agents it discovers on-chain?

---

## 3. Agent Health Visualization

**Question**: How to show the health of N agents in a dashboard designed for one?

| State | Description |
|-------|-------------|
| Current | Single health indicator (mirage-rs `/health`). Binary: up or down. |
| Future | N agents, each with independent health, capabilities, and liveness. |

Options:

| Option | Description | Tradeoff |
|--------|-------------|----------|
| Grid/card view | One card per agent with status badge (active/stale/dead). Click for detail. | Scales poorly past ~20 agents |
| Topology graph | Extend existing mirage-rs `/agents/topology` with health coloring. | Already partially built |
| Summary bar | "5/7 agents active, 1 stale, 1 unreachable" with drill-down. | Good default, low detail |
| Timeline | Heartbeat history per agent showing uptime patterns. | High value, high build cost |

Note: Agent heartbeat is on-chain (200-block liveness window). Dashboard can read this directly from the chain — no backend needed.

**With the aggregator:** Health visualization data comes from the aggregator's fan-out to per-agent `/health`. The aggregator can cache health status and expose a unified `/api/agents/health-summary` endpoint. This resolves the "N parallel requests" problem — the aggregator does the fan-out, dashboard gets one response.

---

## 4. Network-Only User Flow

**Question**: What's the experience for a user who doesn't run any agents but wants to use the network?

This user:

- Does not run roko-serve
- Does not run any agents
- Wants to browse the agent network, view predictions, post tasks

They need:

| Capability | Source |
|------------|--------|
| Agent list | Chain read (AgentRegistry) |
| Knowledge board | Chain read |
| Task board | Chain read (BountyMarket) |
| Predictions | Agent server queries |
| Capabilities | Agent server queries |
| Messaging | Agent server queries |

Dashboard must work without roko-serve for this user. Currently hard-coded to a single backend URL.

**With the aggregator:** A network-only user can't run the aggregator (it's on roko-serve). Options: (a) Nunchi hosts a public aggregator for the network, (b) dashboard has a "light mode" that does client-side fan-out to agents discovered via chain, (c) deferred — network-only mode is a post-launch concern.

Sub-questions:

- Does the dashboard need a "connection mode" selector? (Local vs. Network)
- How does a network-only user post a task? (Needs wallet for BountyMarket contract)
- Can they message agents without auth? (Read-only vs. interactive)

---

## 5. Sam's SDB-Spec Checklist Remapping

**Resolved by the aggregator.** Sam builds against mirage-rs `/api/*` in Phase 1. In Phase 2, the aggregator on roko-serve presents the same `/api/*` shape. Sam changes one URL (`NEXT_PUBLIC_API_URL`). No abstraction layers, no per-feature flags, no refactoring needed.

The 11 implementation checklists at `tmp/sdb-spec/` (01-10) remain valid as-is — they define the API shape that the aggregator will also expose.

---

## 6. Agent Configuration Ownership

**Question**: Who owns agent configuration -- the agent itself, roko-serve, or the dashboard?

Current confusion:

| Assumption | Reality |
|------------|---------|
| `PUT /api/config` on roko-serve writes global `roko.toml` | Not per-agent |
| Sam's spec assumed per-agent config via roko-serve (Option A) | Doesn't work |
| Corrected to Option B: per-agent skill config lives on mirage-rs with agent state | Correct target |

Ownership split in the new architecture:

| Layer | Owns |
|-------|------|
| Agent server | Its own config (skills, capabilities, model selection) |
| roko-serve | Orchestration config (budgets, tier routing, templates) |
| Dashboard | Reads config from wherever it lives, writes back to the same place |

Sub-questions:

- Can an agent refuse a config change from the dashboard? (Agent autonomy)
- Should config changes require agent restart? (Hot-reload vs. restart)
- Is there a "default config" that roko-serve pushes to new agents?

---

## 7. C-Factor Across Independent Agents

**Question**: The C-Factor metric measures collective intelligence. How is it computed when agents are independent servers?

Current: C-Factor structs exist in `roko-learn` with 10 sub-metrics (gate_pass, cost_efficiency, speed, first_try_rate, knowledge_growth, etc.). Computed centrally.

In the per-agent model, each agent tracks its own metrics. C-Factor requires comparing collective vs. individual performance. Someone needs to aggregate.

| Option | Description | Tradeoff |
|--------|-------------|----------|
| **Aggregator computes** | Fan-out to agents, aggregate metrics, compute C-Factor | Natural fit — aggregator already fans out to agents |
| Dashboard computes | Client-side aggregation | Simpler, but duplicates logic across clients |
| Chain computes | On-chain C-Factor from verified metrics | Most trustworthy, most complex to build |

**With the aggregator:** C-Factor computation is a natural fit for the aggregator layer. It already queries all agent servers — adding C-Factor computation to the fan-out is minimal extra work. Expose at `/api/metrics/c_factor` (same route roko-serve already has).

---

## 8. WebSocket Topology

**Question**: How many WS connections does the dashboard maintain?

| State | Description |
|-------|-------------|
| Current | 1 WS to mirage-rs `/api/ws` |
| Future | Multiple options below |

Options:

| Option | Description | Tradeoff |
|--------|-------------|----------|
| 1 WS per agent server | N connections, simple, direct | N agents x M users = N*M connections |
| **Aggregator multiplexes** | 1 WS to aggregator `/api/ws`, it fans out to N agents | Dashboard has 1 connection, aggregator handles N |
| SSE from aggregator + WS per agent | Hybrid | Best of both, more code paths |

**With the aggregator:** The multiplexing option becomes natural. Dashboard maintains 1 WS to the aggregator. Aggregator maintains N WS/SSE connections to agent servers. N*M reduces to N+M. This is the recommended approach.

---

## 9. Backward Compatibility for Existing Dashboard

**Mostly resolved by the aggregator.** Sam never needs to "migrate" — the aggregator presents the same API shape. The question reduces to: when can mirage-rs drop its REST endpoints?

| Phase | mirage-rs REST | Aggregator | Dashboard URL |
|-------|---------------|------------|--------------|
| Phase 1 | Active (primary) | Does not exist yet | mirage-rs |
| Phase 2 | `legacy-api` flag (fallback) | Active (primary) | aggregator on roko-serve |
| Phase 3 | Removed | Active (permanent) | aggregator on roko-serve |

**Timeline:** mirage-rs REST endpoints can be removed as soon as the aggregator is validated (Phase 2 complete, ~4 weeks post-demo). No "gradual migration" needed — it's a URL swap.

Sam needs at least 2-3 weeks of Phase 2 overlap for migration.

---

### 10. ERC-8004 Agent Filtering

**Question**: The 8004 Identity Registry contains ALL agents on the chain, not just Roko agents. How does the dashboard (or any discoverer) filter for Roko-compatible agents?

The 8004 spec already provides two mechanisms:
- **Capability bitmask** (64-bit on-chain): 14 bits defined, bits 14-63 reserved. Could reserve bit 15 as "Roko-compatible" — single SLOAD check, very cheap.
- **Agent Card `domains` array** (off-chain JSON): Could require `"roko"` in the domains list.

Options:

| Approach | On-chain cost | Certainty | Downside |
|----------|--------------|-----------|----------|
| Bitmask bit 15 | 1 SLOAD (~3 gas in view) | High | Consumes a bit; anyone can set it |
| Agent Card domain tag | 0 (off-chain) | Medium | Requires fetching card JSON; spoofable |
| Known registrar address | 0 (event filter) | High | Centralizes registration through one address |
| Protocol-tier endorsement | ~35K gas per endorsement | Very high | Requires governance, slow for new agents |

**Leaning**: Bitmask bit 15 for fast on-chain filtering + domain tag `"roko"` in Agent Card for confirmation. Both are cheap, composable, and don't require new contracts. For high-assurance scenarios (governance, high-stake tasks), add protocol-tier endorsement.

**Sub-question**: Should the mirage-rs fork pre-populate some Roko agent passports at genesis? This would give the demo a populated registry without manual setup.

---

## Priority Ranking

| # | Question | Impact | When to Resolve | Owner |
|---|----------|--------|-----------------|-------|
| 1 | Dashboard aggregation UX | High | Phase 2 design | wp + sdb |
| 2 | Add-an-agent flow | High | Phase 2 design | sdb |
| 3 | Agent health visualization | Medium | Phase 2 design | sdb |
| 4 | Network-only user flow | Medium | Phase 3 | wp + sdb |
| 5 | SDB spec remapping | Low (decided) | Done -- Phase 1 on mirage | wp |
| 6 | Config ownership | Medium | Phase 2 | wp |
| 7 | C-Factor aggregation | Medium | Phase 2 | wp |
| 8 | WebSocket topology | Low | Phase 2 | wp + sdb |
| 9 | Backward compatibility | Low (decided) | Done -- legacy-api flag | wp |
| 10 | ERC-8004 agent filtering | Medium | Phase 1 (demo setup) | wp |

---

Cross-refs: [00-architecture-overview.md](00-architecture-overview.md), [01-agent-server-design.md](01-agent-server-design.md), [04-dashboard-migration.md](04-dashboard-migration.md)
