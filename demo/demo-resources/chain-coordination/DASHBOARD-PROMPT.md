# Chain Intelligence Layer — Command Center Integration

## Context

The mirage-rs devnet is running locally (`127.0.0.1:8545`, chain ID 31337, 50ms blocks) with live on-chain data:
- **7 knowledge entries** (insights + warnings + observations from 3 agents)
- **5 pheromones** (2 threats, 2 opportunities, 1 wisdom) with decaying intensity
- **4 registered agents** with heartbeats (sentinel, quant, oracle, test)
- **47+ agents** in the Solidity AgentRegistry contract
- **ISFR rate** live at the ISFR contract
- **Block height** incrementing at ~20 blocks/sec

Three new roko-serve routes proxy chain data:
- `GET /api/chain/status` → `{ connected, block_number, chain_id, wallet }`
- `GET /api/chain/agents` → `{ registered_count }` from on-chain AgentRegistry
- `GET /api/chain/bounties` → `{ total_jobs, jobs: [{id, state, state_label}] }`

Mirage-rs REST endpoints provide rich data:
- `GET /api/stats` → aggregate counts (insights, pheromones, tasks, predictions)
- `GET /api/knowledge/entries` → all knowledge entries with kind, state, author, content, weight, confirmations, challenges
- `GET /api/pheromones/summary` → by-kind aggregation with intensity stats
- `GET /api/pheromones?limit=20` → individual pheromones with content, kind, intensity, age
- `GET /api/agents` → chain-extension-registered agents with roles, stats, heartbeat timestamps
- `POST /api/pheromones/query` body `{query, k}` → semantic similarity search over pheromones
- `GET /api/knowledge/search?query=X&k=5` → semantic search over knowledge entries

The `.env` is already configured:
```
VITE_CHAIN_URL=http://127.0.0.1:8545
VITE_CHAIN_ID=31337
VITE_DEVNET_AUTOPILOT=true
VITE_ROKO_URL=http://127.0.0.1:6677
```

The `mirage-contracts.ts` addresses are corrected (daeji/roleRegistry swapped for local deploy order).

---

## Task: Build the "Chain Intelligence" Section in Command Center

Replace the empty **Section 01 COLLECTIVE** area (currently showing "No on-chain agents registered" and the PhosphorLog) with a rich, data-dense **Chain Intelligence** panel that surfaces the live knowledge graph, pheromone field, and agent coordination data from mirage-rs.

This should be the visual centerpiece of the Command Center — the section that makes someone lean forward and say "what is this?" It's not filler; every pixel shows real data from the running chain.

### Design Language (BARDO / ROSEDUST)

Follow the existing design system in `tokens.css` and the Bardo Terminal Foundation PRD:

- **Base palette**: `--rd-bg-void` (#060608), surfaces (#0C0A0E → #1C1C22), text (#EDE9E3/#CFC7BF/#9E9890)
- **Accents**: rose-bright (#D8A0B8) for live/active state, bone-hot (#F0E0B8) for the single most important element, success (#70887A), warning (#AA8855), error (#AA5555)
- **Typography**: Serif (Fraunces) for section headers, Mono (JetBrains Mono) for all data values, Sans (Space Grotesk) for body text
- **Components**: Use `ChassisFrame` for contained modules, `SectionLabel` for numbered sections, `PhosphorLog` aesthetic for streaming data
- **Depth**: Glass-over-void — layered transparency with backdrop blur. At least 50% void space. Never more than 5% of area at max brightness.
- **Motion**: Everything is alive. Elements driven by continuously changing variables. Luxury easing `cubic-bezier(0.22, 1, 0.36, 1)`. No snapping, no bouncing.

### Component Breakdown

Build this as **three side-by-side ChassisFrame modules** within Section 01, using the F2 Triptych compositional pattern (three equal columns):

#### Module 1: `KnowledgeGraph` (left panel)

**Data source**: `GET ${MIRAGE_BASE}/api/knowledge/entries` (poll 10s) + `GET ${MIRAGE_BASE}/api/stats`

A live-updating knowledge panel showing the agent collective's shared understanding:

- **Header**: "KNW KNOWLEDGE LAYER" in chassis label style
- **Stats strip** at top: total entries, active count, confirmed count, challenged count — mono font, rose-bright for non-zero
- **Entry list** below: Each entry shows:
  - Kind badge: colored pill — `insight` (blue #2563eb), `warning` (red #AA5555), `observation` (amber #AA8855), `heuristic` (green), `causal_link` (emerald), `strategy_fragment` (purple), `anti_knowledge` (gray)
  - Author in mono (e.g. `roko-sentinel-alpha`) — dimmed
  - Content truncated to 2 lines, full on hover
  - Weight indicator: thin horizontal bar (0-1 scale), rose-bright fill
  - Confirmations/challenges as small counters: `↑3 ↓1`
  - State badge: `active` (green dot), `confirmed` (double green), `challenged` (amber pulse), `decaying` (fading opacity)
- **Bottom**: Semantic search input — when typed, calls `/api/knowledge/search?query=X&k=5` and highlights matching entries with similarity score

The entries should appear with a stagger animation (50ms per item) and new entries should slide in from the top with a phosphor glow that fades over 2 seconds.

#### Module 2: `PheromoneField` (center panel)

**Data source**: `GET ${MIRAGE_BASE}/api/pheromones/summary` (poll 5s) + `GET ${MIRAGE_BASE}/api/pheromones?limit=20`

A visualization of the stigmergic signal field — the ambient intelligence layer where agents deposit decaying chemical signals:

- **Header**: "STG STIGMERGY FIELD" in chassis label style
- **Radial intensity gauge** at top center: Three concentric rings representing the three pheromone kinds:
  - Outer ring: **threat** — red/rose gradient, arc length proportional to total threat intensity
  - Middle ring: **opportunity** — amber/gold gradient, arc length proportional to total opportunity intensity
  - Inner ring: **wisdom** — teal/green gradient, arc length proportional to total wisdom intensity
  - Center: total pheromone count in bone-hot mono font
  - All rings animate smoothly as intensity decays in real-time (the pheromones have half-lives, so intensity drops between polls)
  - Ring segments glow with `box-shadow` proportional to max_intensity for that kind

- **Signal list** below the gauge: Individual pheromones as compact cards:
  - Left edge: colored vertical bar (threat=red, opportunity=amber, wisdom=teal)
  - Intensity value in mono: `0.946` with brightness proportional to value
  - Content (1 line, ellipsis)
  - Age indicator: `2m ago`, `45s ago` — fades as pheromone decays
  - Half-life indicator: small clock icon + `1h`, `5m`, `24h`

- **Bottom strip**: Aggregate stats — "2 THREAT · 2 OPPORTUNITY · 1 WISDOM" with colored dots, total intensity as a subtle sparkline over the last N polls

The entire panel should breathe — a subtle 0.5% brightness oscillation on the background at the frequency of new pheromone deposits. When a new pheromone appears, its entry pulses rose-bright for 1 second.

#### Module 3: `AgentTopology` (right panel)

**Data source**: `GET ${MIRAGE_BASE}/api/agents` (poll 10s) + agent heartbeats + `GET ${MIRAGE_BASE}/api/agents/{id}/stats`

The live agent collective — who's alive, what they're doing, and how they relate:

- **Header**: "COL AGENT COLLECTIVE" in chassis label style
- **Agent cards** as compact rows:
  - Status dot: green (heartbeat < 30s ago), amber (30s-2min), red (>2min), gray (never)
  - Agent ID in mono: `roko-sentinel-alpha`
  - Role badge: `defi-watcher` / `quantitative-analyst` / `oracle-monitor` — colored by role family
  - Stats mini-bar: insights posted, tasks completed, challenges given — tiny inline sparkline
  - Last heartbeat: relative time in dimmed mono

- **Coordination graph** (if space allows): Small force-directed graph showing agents as nodes, with edges representing shared knowledge (if agent A confirmed agent B's insight). Node size = total activity. Edge opacity = recency.

- **Aggregate stats** at bottom:
  - Total agents (from both chain extension registry AND on-chain Solidity registry)
  - Online/offline split
  - Total insights posted across collective
  - Total tasks completed

### Integration Points

1. **New service file**: Create `src/services/mirage-knowledge.ts` with typed fetch helpers:
   ```typescript
   fetchKnowledgeEntries(): Promise<KnowledgeEntry[]>
   fetchPheromones(limit?: number): Promise<Pheromone[]>
   fetchPheromoneSummary(): Promise<PheromoneSummary>
   fetchChainAgents(): Promise<ChainAgent[]>
   searchKnowledge(query: string, k?: number): Promise<SearchResult[]>
   ```
   Base URL: `MIRAGE_BASE` from constants.ts (resolves to `http://127.0.0.1:8545`)

2. **New hooks**: `useKnowledgeLayer()`, `usePheromoneField()`, `useChainAgents()` — React Query with appropriate refetch intervals

3. **Types**: Add to `types/api.ts`:
   ```typescript
   interface KnowledgeEntry {
     id: string; kind: string; state: string; author: string;
     content: string; weight: number; confirmations: number;
     challenges: number; created_at?: number;
   }
   interface Pheromone {
     id: number; kind: string; content: string; intensity: number;
     half_life_secs: number; created_at?: number;
   }
   interface PheromoneSummary {
     by_kind: Record<string, { count: number; total_intensity: number; avg_intensity: number; min_intensity: number; max_intensity: number }>;
     total_count: number; total_intensity: number;
   }
   interface ChainAgent {
     id: string; role: string; owner: string; registered_at: number;
     last_heartbeat_block: number; last_heartbeat_ts: number;
     stats: { confirmations_given: number; challenges_given: number; insights_posted: number; tasks_completed: number; tasks_failed: number; };
   }
   ```

4. **Wire into CommandCenter.tsx**: Replace the current FleetCensus + ActivityFeed in Section 01 with the three new modules. Keep the PhosphorLog as a collapsed/expandable sub-element within the ActivityFeed section, or move it to a fourth narrow column.

### Live Data Currently Available

Here's what's on-chain right now — all of this should be visible in the new panels:

**Knowledge Entries (7):**
| Kind | Author | Content (truncated) |
|------|--------|---------------------|
| insight | roko-sentinel-alpha | ETH funding rate negative for 72h — short squeeze precursor |
| warning | roko-sentinel-alpha | Aave V3 USDC utilization 94.2% — liquidation cascade risk |
| insight | roko-quant-beta | Cross-exchange basis Binance/dYdX widened to 45bps |
| warning | roko-oracle-gamma | Chainlink ETH/USD deviation 0.5% for 12 blocks |
| insight | roko-oracle-gamma | MakerDAO proposal #47 — DAI stability fee to 0% |
| observation | roko-quant-beta | UniV3 ETH/USDC TVL up 340% in 48h — whale accumulation |
| observation | roko-sentinel-alpha | Curve 3pool USDT imbalance at 41% vs 33% target |

**Pheromones (5):**
| Kind | Intensity | Half-Life | Content (truncated) |
|------|-----------|-----------|---------------------|
| threat | 0.95 | 1h | Flash loan attack vector on Compound V3 cUSDC |
| opportunity | 0.88 | 5m | MEV sandwich: 142 ETH on pending UniV3 swap |
| wisdom | 0.72 | 24h | Funding rate reversion: 73% win rate, 2.1x Sharpe |
| threat | 0.65 | 2h | MakerDAO governance #47 — oracle manipulation risk |
| opportunity | 0.81 | 6h | Morpho Blue ETH market 4.8% APY vs Aave 2.1% |

**Agents (4):**
| ID | Role | Status |
|----|------|--------|
| roko-sentinel-alpha | defi-watcher | heartbeat sent |
| roko-quant-beta | quantitative-analyst | heartbeat sent |
| roko-oracle-gamma | oracle-monitor | heartbeat sent |
| test-agent | tester | registered |

### Quality Bar

This isn't a placeholder panel. Every element must show real, queryable data. The knowledge entries should be readable. The pheromone intensities should visibly decay between polls. The agent heartbeats should show genuine liveness. A viewer should be able to look at this panel and understand what the agent collective knows, what threats it's tracking, and who's online.

The aesthetic target is the intersection of:
- **Bloomberg Terminal** data density (every pixel is information)
- **Teenage Engineering** industrial design (the ChassisFrame screws, LED pulses)
- **Serial Experiments Lain** atmosphere (CRT glow, phosphor trails, dark depth)
- **The existing BARDO ROSEDUST** system already in tokens.css

Build it so it looks like it's always been there.
