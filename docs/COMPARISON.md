# Roko vs. Alternatives

> How Roko compares to other agent frameworks. Updated 2026-04-12.

---

## Positioning

Roko is a **cognitive architecture for self-developing agents**, not a prompt orchestration
framework. The difference: prompt orchestration chains LLM calls in sequences; Roko gives
agents memory, affect, learning, verification, decay, and offline consolidation — the
machinery to improve themselves over time.

---

## Feature Comparison

| Capability | **Roko** | **LangChain** | **CrewAI** | **SWE-Agent** | **AutoGPT** |
|-----------|---------|-------------|----------|-------------|-----------|
| **Language** | Rust | Python | Python | Python | Python |
| **Architecture model** | Cognitive (1 noun + 6 traits) | DAG/chain | Role-based crews | Single agent + ACI | Loop-based |
| **Universal data type** | Engram (content-addressed, decaying, scored) | Varies per chain | Messages | Observations | Messages |
| **Content addressing** | BLAKE3 hash on every datum | No | No | No | No |
| **Verification pipeline** | 11 gates, 6-rung pipeline, adaptive thresholds | Optional callbacks | No built-in | SWE-bench evaluation | No built-in |
| **Knowledge management** | 6 types × 4 tiers, HDC similarity, decay | Vector store (external) | No built-in | No built-in | No built-in |
| **Affect/emotion model** | PAD vectors, 6 behavioral states, somatic markers | No | No | No | No |
| **Offline learning** | NREM replay, REM imagination, hypnagogia | No | No | No | No |
| **Model routing** | CascadeRouter (T0→T1→T2), adaptive | Manual selection | Manual selection | Fixed model | Manual selection |
| **Multi-agent coordination** | Pheromone-based stigmergy, mesh gossip | Agent executor | Crew delegation | Single agent | No built-in |
| **Self-development** | Reads own PRDs, generates plans, executes, validates | No | No | No | No |
| **Session persistence** | Snapshot + resume, append-only JSONL | Checkpointers (optional) | No built-in | No built-in | No built-in |
| **Safety model** | Role auth, pre/post checks, taint tracking, capability tokens | No built-in | No built-in | Container sandbox | No built-in |
| **Test suite** | 3,761 tests across 36 workspace members | Varies | Minimal | SWE-bench | Minimal |
| **Token budget management** | VCG attention auction, per-section bidding | Manual truncation | No built-in | Context window | No built-in |
| **Temporal dynamics** | 4 decay models, knowledge half-lives, Ebbinghaus curves | No | No | No | No |

---

## Architectural Differentiators

### 1. One Noun, Six Verbs

Most frameworks have many types: tasks, messages, tools, observations, actions, memories.
Roko has exactly one data type (Engram) and six trait operations. This enables
universal composability — any Scorer scores any Engram, any Substrate stores any Engram,
any Gate verifies any Engram. Components compose freely.

### 2. Everything Decays

In Roko, knowledge is not permanent. Every Engram has a decay model (exponential, linear,
stepped, or asymptotic) and a half-life determined by its validation tier. Transient
knowledge (unverified) decays in hours. Persistent knowledge (cross-validated by multiple
agents) decays over months. This prevents stale information from poisoning decisions —
a problem that grows worse as agent systems run longer.

### 3. Affect-Driven Cognition

Roko agents have emotional states (PAD vectors: Pleasure, Arousal, Dominance) that
modulate behavior: which model tier to use, how much context to assemble, when to explore
vs. exploit. The Daimon affect engine implements somatic markers (Damasio 1994) — fast
gut-feeling pattern matches that bypass expensive deliberation. This is not anthropomorphism;
it's a proven decision-making optimization from cognitive science.

### 4. Offline Learning (Dreams)

When idle, Roko agents enter a dream cycle: NREM replay (Mattar-Daw utility-based episode
selection), REM imagination (Pearl SCM counterfactual reasoning), and integration staging
(knowledge promotion through validation tiers). This consolidation loop transforms raw
experience into validated knowledge — something no other agent framework does.

### 5. Self-Verification at Every Step

Roko's 6-rung gate pipeline (syntax → compile → test → lint → diff → semantic) verifies
every agent output before it enters the knowledge base. Adaptive thresholds (EMA per rung)
learn expected pass rates and flag anomalies. Gate verdicts are themselves Engrams that
re-enter the cognitive loop — "verification as cognition."

### 6. Self-Development

Roko is designed to develop itself. The workflow is concrete and works today:
`prd idea` → `prd draft` → `research enhance-prd` → `prd plan` → `plan run` → gate →
persist → resume. The system reads its own PRDs, generates implementation plans, executes
them via Claude agents, validates results through the gate pipeline, and persists outcomes.

---

## When to Use What

| If you need... | Use |
|---------------|-----|
| Quick LLM prototyping | LangChain |
| Role-based team simulation | CrewAI |
| Automated code fixing against benchmarks | SWE-Agent |
| A cognitive architecture for self-improving agents | **Roko** |
| Agents that learn and consolidate over time | **Roko** |
| Production Rust with strong verification | **Roko** |
| Python ecosystem compatibility | LangChain or CrewAI |

---

## Limitations

Roko is honest about its limitations:

- **Not production-ready for all subsystems**: The self-hosting loop ships. Daimon, Neuro,
  Dreams, and Coordination are built or scaffolded but not yet wired into the runtime. See
  [`STATUS.md`](STATUS.md) for the full breakdown.
- **Rust-only**: No Python SDK. If your stack is Python, this is a barrier.
- **Steep learning curve**: 22 documentation sections, 36 workspace members, cognitive science concepts.
  The architecture is powerful but not simple.
- **Single-developer origin**: Roko was built by one person migrating a prior 108K LOC system.
  Community and ecosystem are nascent.
- **Chain features deferred**: The Korai chain integration, identity/economy layer, and
  on-chain attestation are specified but not yet deployed.
