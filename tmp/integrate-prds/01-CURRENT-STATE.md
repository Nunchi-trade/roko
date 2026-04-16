# Current State: Docs vs Code

## The Docs (Target Architecture)

Location: `/Users/will/dev/nunchi/roko/roko/docs/`

375+ documents across 22 sections describing the full "Synapse Architecture":

- **1 universal data type**: Engram (currently `Signal` in code)
- **6 synapse traits**: Substrate, Scorer, Gate, Router, Composer, Policy
- **5-layer taxonomy**: Runtime → Framework → Scaffold → Harness → Orchestration
- **3 cognitive cross-cuts**: Neuro (knowledge), Daimon (affect), Dreams (consolidation)
- **9-step CoALA cognitive loop**: Perceive → Evaluate → Attend → Integrate → Act → Verify → Persist → Adapt → Meta-Cognize
- **3 cognitive speeds**: Gamma (~5-15s), Theta (~75s), Delta (hours)

### Key Doc Sections

| Section | Path | What |
|---------|------|------|
| Architecture | `docs/00-architecture/` | Naming glossary, Engram type, synapse traits, universal loop |
| Orchestration | `docs/01-orchestration/` | Plan DAG, executor, task dispatch |
| Agents | `docs/02-agents/` | Type taxonomy, extensibility |
| Neuro | `docs/06-neuro/` | Tiered knowledge, HDC, decay |
| Daimon | `docs/09-daimon/` | PAD vectors, behavioral states, compute modulation |
| Dreams | `docs/10-dreams/` | NREM replay, REM imagination, integration |
| Heartbeat | `docs/16-heartbeat/` | Three-speed cognitive loop |
| Coordination | `docs/13-coordination/` | Multi-agent stigmergy |

### Authoritative Naming Glossary

`/Users/will/dev/nunchi/roko/roko/docs/00-architecture/01-naming-and-glossary.md`

---

## The Code (Current State)

~177K LOC across 18+ crates. The core self-hosting loop works end-to-end.

### Crate Inventory

| Crate | Status | Notes |
|-------|--------|-------|
| `roko-core` | Stable kernel | Signal + 6 traits. 376 tests |
| `roko-agent` | Wired | 5 LLM backends, MCP, tool loop, safety. 346 tests |
| `roko-gate` | Wired | 11 gates, 6-rung pipeline. 200 tests |
| `roko-orchestrator` | Wired | Plan DAG, parallel executor. 158 tests |
| `roko-learn` | Wired | Episodes, bandits, routing, experiments. 101 tests |
| `roko-compose` | Wired | 7-layer prompt builder, 9 templates. 23 tests |
| `roko-conductor` | Built, not wired | 10 watchers, circuit breaker |
| `roko-neuro` | Built, not wired | Knowledge store, tiers, HDC |
| `roko-daimon` | Built, not wired | PAD vectors, affect engine |
| `roko-dreams` | Scaffold | Depends on roko-golem stubs |
| `roko-fs` | Stable | FileSubstrate, GC |
| `roko-std` | Stable | NoOp impls, 19 builtin tools |
| `roko-cli` | Main entry | All subcommands |
| `roko-index` | Built | Parser + graph + HDC |
| `roko-plugin` | Built | Event sources, feedback |
| `roko-serve` | Built | REST + WebSocket API; PRD promote/plan and research wiring |
| **bardo-runtime** | **Needs rename** | Event bus, process supervisor |
| **bardo-primitives** | **Needs rename** | HDC vectors, tier routing |
| **roko-golem** | **Needs dissolution** | Placeholder scaffold with dead code |
| `roko-chain` | Built | Chain abstractions |
| `roko-lang-*` | Built | Rust/TypeScript/Go language support |

### What Works End-to-End

```
roko prd idea → prd draft new → prd draft promote → prd plan → plan run → dashboard
```

All steps are CLI commands. The plan-execute-gate-persist loop is wired.

### What's Built But Not Wired

- Neuro (knowledge management) — `roko-neuro` has full implementation, not called from orchestrator; score axes and knowledge tiers are already in code
- Daimon (affect engine) — `roko-daimon` has full PAD implementation, not modulating dispatch
- Conductor (watchers) — `roko-conductor` has 10 anomaly detectors, not monitoring execution
- Safety layer — built in `roko-agent`, partially integrated

### What's Scaffold Only

- Dreams (offline consolidation) — struct stubs, depends on roko-golem
- Interactive TUI — ratatui in deps but text-only dashboard
- Heartbeat (autonomous operation) — specified in docs, no code
- Agent Mesh (multi-agent coordination) — specified in docs, no code

---

## The Gap

### Naming Mismatches

| What | Docs Say | Code Says |
|------|----------|-----------|
| Universal data type | Engram | Signal |
| Architecture branding | Synapse Architecture | "1 noun + 6 verbs" |
| Runtime crate | roko-runtime | bardo-runtime |
| Primitives crate | roko-primitives | bardo-primitives |
| Knowledge subsystem | Neuro | roko-neuro (tiers/HDC implemented; still not queried everywhere) |
| Scaffold crate | dissolved | roko-golem exists with dead code |
| Agent groups | Collective / Mesh | (not referenced) |
| Behavioral states | 6 cyclical, no terminus | mortality.rs exists in roko-golem |
| Token name | KORAI / DAEJI | (not in code) |
| Config file | roko.toml | roko.toml (correct) |

### Structural Mismatches

1. `roko-golem` exists as a monolithic scaffold crate but docs say it should be dissolved:
   - Daimon → `roko-daimon` (already exists as standalone)
   - Dreams → `roko-dreams` (exists but imports from roko-golem)
   - Grimoire → `roko-neuro` (already exists as standalone)
   - Chain Witness → `roko-chain` (already exists)
   - Hypnagogia → `roko-dreams` (still in roko-golem)
   - Mortality → **DELETE** (concept removed)
   - ScaffoldEngine trait → **DELETE** (no umbrella trait)

2. `roko-dreams` depends on `roko-golem` for re-exports — needs to be self-contained

3. `roko-serve` route stubs are largely gone: plan execution, plan generation, PRD draft, and research routes all run through the shared runtime. PRD/research episode logging in `crates/roko-cli/src/agent_exec.rs` is also wired. Direct `roko run`, orchestrate fallback paths, and raw `ExecAgent` subprocess fallback now all enter the same scoped safety surface and share more prompt/config assembly. The remaining runtime gap is broader backend universality: some native/provider-specific paths still bypass the shared `ToolDispatcher` chain.

4. Workspace metadata no longer references `bardo.run` email or `wpank/bardo` repo. Remaining naming work is concentrated in conceptual/documentation cleanup, not Cargo metadata.

### Conceptual Mismatches

- Death/mortality framing completely removed in docs — code still has `mortality.rs`
- Docs describe 6 cyclical behavioral states (no terminal state) — code has mortality clocks
- Docs describe Engram with Ebbinghaus decay — code has Signal with simpler decay
- Docs describe 3 cognitive speeds (Gamma/Theta/Delta) — code has no speed differentiation
