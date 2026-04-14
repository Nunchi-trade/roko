# UX Innovation Proposals

> Seven frontier UX innovations that transform how operators interact with cognitive agents — from conversational development to knowledge cartography. Each proposal grounds speculative design in published research, concrete Rust structs, and TUI mockups.

> **Implementation**: Not yet built

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [08-tui-main-layout.md](./08-tui-main-layout.md), [15-generative-interfaces-a2ui.md](./15-generative-interfaces-a2ui.md), [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md)
**Key sources**: Roko subsystem docs (Daimon, Neuro, Dreams, Learning, Orchestration), academic literature 2023–2026

---

## Abstract

This document proposes seven UX innovations that extend Roko's interface layer beyond monitoring dashboards into **cognitive partnership interfaces** — interfaces where the operator doesn't just observe agents, but thinks alongside them. Each innovation is grounded in:

1. A **problem statement** identifying the gap in current agent UX
2. **Research foundations** citing 2024–2026 academic work
3. A **design specification** with Rust struct definitions
4. A **TUI mockup** showing terminal-native rendering
5. **Integration wiring** connecting to existing Roko subsystems
6. **Test criteria** for validation

The seven proposals:

| # | Innovation | Core Idea | Key Subsystem |
|---|---|---|---|
| 1 | [Conversational Development](#1-conversational-development) | Natural language → PRD → plan → execute | Orchestration + Compose |
| 2 | [Time-Travel Debugging](#2-time-travel-debugging) | Rewind agent decisions via Witness DAG + episodes | Learning + Safety |
| 3 | [Dream Journal Interface](#3-dream-journal-interface) | Review what agents learned during offline consolidation | Dreams + Neuro |
| 4 | [Agent Garden](#4-agent-garden) | Visualize agents as living organisms with health/growth | Spectre + Daimon |
| 5 | [Pair Programming with Affect](#5-pair-programming-with-affect) | Agents express cognitive state through Daimon | Daimon + Compose |
| 6 | [Collaborative Planning](#6-collaborative-planning) | Human + agents co-create and co-edit plans | Orchestration + A2UI |
| 7 | [Knowledge Map](#7-knowledge-map) | Interactive visualization of entire Neuro knowledge base | Neuro + HDC |

---

## Design Principles (Shared)

All seven innovations inherit these principles from the existing interface architecture:

1. **ROSEDUST everywhere**: Dark-only palette, glass morphism, void-black background. No innovation introduces a new visual language.
2. **Progressive disclosure**: Overview first, detail on demand. Every innovation has a 200ms-glance summary and a minutes-of-study detail mode.
3. **TUI-primary**: The terminal is the primary development interface. Web Portal versions exist as complements, never replacements.
4. **Data-grounded**: Every visual element traces to a data source (Tufte's data-ink ratio principle). If it can't be grounded, it doesn't render.
5. **Engram-native**: Every interaction produces Engrams. Conversations, debug sessions, dream reviews, plan edits — all persisted as content-addressed signals.
6. **Keyboard-first**: All interactions accessible via keyboard. Mouse is optional.

---

## 1. Conversational Development

> Natural language to PRD-plan-execute cycle — the operator describes intent, and roko orchestrates the full self-hosting loop conversationally.

### Problem Statement

Roko's self-hosting workflow requires 6+ CLI commands in sequence (`prd idea` → `prd draft` → `research enhance-prd` → `prd plan` → `plan run` → `status`). Each command is well-designed, but the transitions between them are manual. The operator must decide *when* to move between phases, *which* commands to invoke, and *how* to handle failures — all via discrete CLI invocations.

This creates a cognitive overhead gap: the operator's intent ("add retry logic to agent dispatch") is simple, but the execution requires understanding the PRD lifecycle, plan generation, and orchestration subsystems. A conversational interface collapses this gap by accepting natural language intent and orchestrating the full cycle.

### Research Foundations

**ChatDev** (Qian et al., ACL 2024) demonstrated that multi-agent chat chains can complete entire software development processes in under 7 minutes at sub-dollar cost. Their "communicative dehallucination" — agents cross-checking via structured conversation — achieves higher quality than single-agent approaches. Roko's existing multi-agent orchestration (ParallelExecutor + PlanRunner) provides the computational substrate; what's missing is the conversational *interface* to it.

**SWE-agent** (Yang et al., NeurIPS 2024) introduced the Agent-Computer Interface (ACI) concept — purpose-built interfaces between LLM agents and development environments. Their finding that interface design matters as much as model capability validates investing in how the operator communicates with Roko, not just how Roko executes.

**CoRE** (Xu et al., 2024) showed that natural language can serve as a programming language when structured with control flow syntax. This suggests that Roko's conversational interface should not be free-form chat but structured dialogue with explicit phase transitions.

The commercial validation is overwhelming: Cursor reached 2M+ users by early 2026, and the "vibe coding" paradigm — iterative natural language development — became the dominant developer UX pattern.

### Design

The Conversational Development interface wraps Roko's self-hosting workflow in a dialogue-driven shell. The operator converses with a meta-agent that maps intent to CLI operations, handles phase transitions automatically, and escalates decisions that require human judgment.

#### State Machine

```
                    ┌──────────────┐
          ┌────────▶│   IDEATION   │◀──── operator types intent
          │         └──────┬───────┘
          │                │ auto-detect: is this an idea, bug, or task?
          │         ┌──────▼───────┐
          │         │   DRAFTING   │◀──── meta-agent generates PRD
          │         └──────┬───────┘
          │                │ operator approves / edits
          │         ┌──────▼───────┐
    failure│         │  RESEARCHING │◀──── research enhance-prd
    replan │         └──────┬───────┘
          │                │ auto: sufficient context?
          │         ┌──────▼───────┐
          │         │   PLANNING   │◀──── prd plan <slug>
          │         └──────┬───────┘
          │                │ operator reviews plan
          │         ┌──────▼───────┐
          │         │  EXECUTING   │◀──── plan run
          │         └──────┬───────┘
          │                │ gates pass / fail
          │         ┌──────▼───────┐
          └─────────│  REVIEWING   │──────▶ DONE
                    └──────────────┘
```

#### Rust Structs

```rust
/// A conversational development session that wraps the PRD-plan-execute lifecycle.
///
/// Each session maps a natural-language intent to a sequence of orchestration
/// operations, with the operator confirming key transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationalSession {
    /// Unique session identifier (BLAKE3 hash of creation timestamp + intent)
    pub id: String,
    /// Original natural-language intent from the operator
    pub intent: String,
    /// Current phase of the conversational workflow
    pub phase: ConversationalPhase,
    /// PRD slug generated from the intent (None until DRAFTING)
    pub prd_slug: Option<String>,
    /// Plan directory path (None until PLANNING)
    pub plan_dir: Option<PathBuf>,
    /// Conversation turns (operator + meta-agent exchanges)
    pub turns: Vec<ConversationTurn>,
    /// Decisions the operator has made (phase approvals, edits)
    pub decisions: Vec<OperatorDecision>,
    /// Session creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last activity timestamp
    pub updated_at: DateTime<Utc>,
    /// Session configuration overrides
    pub config: ConversationalConfig,
}

/// Phases of the conversational development lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationalPhase {
    /// Operator is describing intent; meta-agent classifies
    Ideation,
    /// Meta-agent is drafting a PRD from the intent
    Drafting,
    /// Research agent is gathering context for the PRD
    Researching,
    /// Plan generation from the finalized PRD
    Planning,
    /// Plan execution with gate validation
    Executing,
    /// Reviewing results, operator confirms completion
    Reviewing,
    /// Session complete
    Done,
    /// Session paused (can resume)
    Paused,
}

/// A single turn in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    /// Who produced this turn
    pub speaker: Speaker,
    /// Turn content (natural language)
    pub content: String,
    /// Optional structured data (PRD draft, plan preview, gate results)
    pub structured: Option<serde_json::Value>,
    /// A2UI components emitted with this turn
    pub a2ui_components: Vec<serde_json::Value>,
    /// CLI commands executed as a result of this turn
    pub commands_executed: Vec<String>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    /// The human operator
    Operator,
    /// The meta-agent orchestrating the conversation
    MetaAgent,
    /// A subsystem reporting status (gates, research, etc.)
    System,
}

/// An explicit operator decision at a phase transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorDecision {
    /// What phase transition was approved
    pub transition: (ConversationalPhase, ConversationalPhase),
    /// Operator's optional comment or edit
    pub comment: Option<String>,
    /// Whether the operator approved or requested changes
    pub approved: bool,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Configuration for conversational sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationalConfig {
    /// Whether to auto-advance through phases without confirmation
    /// Default: false (confirm each phase transition)
    pub auto_advance: bool,
    /// Maximum turns before escalating to operator
    /// Default: 50
    pub max_turns: u32,
    /// Whether to auto-research before planning
    /// Default: true
    pub auto_research: bool,
    /// Model for the meta-agent (should be high-capability)
    /// Default: "claude-opus-4-6"
    pub meta_agent_model: String,
}

impl Default for ConversationalConfig {
    fn default() -> Self {
        Self {
            auto_advance: false,
            max_turns: 50,
            auto_research: true,
            meta_agent_model: "claude-opus-4-6".to_string(),
        }
    }
}
```

#### TUI Mockup

```
┌─ ROKO CONVERSATION ───────────────────── ◉ PLANNING ──┐
│                                                         │
│  ┌─ INTENT ───────────────────────────────────────────┐ │
│  │ "Add retry logic with exponential backoff to the   │ │
│  │  agent dispatcher when LLM API calls fail"         │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ┌─ TIMELINE ─────────────────────────────────────────┐ │
│  │ ✓ IDEATION    classified as: feature enhancement   │ │
│  │ ✓ DRAFTING    PRD: retry-logic-dispatch  [view]    │ │
│  │ ✓ RESEARCH    3 sources, 2 crate comparisons       │ │
│  │ ▸ PLANNING    generating tasks.toml...             │ │
│  │ ○ EXECUTING                                        │ │
│  │ ○ REVIEWING                                        │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ┌─ CONVERSATION ─────────────────────────────────────┐ │
│  │ META: I've drafted a PRD with 3 sections:          │ │
│  │  1. Retry policy (exponential backoff + jitter)    │ │
│  │  2. Circuit breaker integration                    │ │
│  │  3. Provider-specific error classification         │ │
│  │                                                    │ │
│  │ YOU: Looks good, but cap retries at 3 not 5        │ │
│  │                                                    │ │
│  │ META: Updated. Research found `backon` crate       │ │
│  │  (MIT, 2.1K stars) as alternative to hand-rolled.  │ │
│  │  Generating plan with 4 tasks...                   │ │
│  │                                                    │ │
│  │ ┌─────────────────────────────────────────────┐    │ │
│  │ │ Tasks Preview (4 tasks, ~25 min estimated)  │    │ │
│  │ │ 1. Add backon dependency + retry wrapper    │    │ │
│  │ │ 2. Wire into dispatcher dispatch_agent()    │    │ │
│  │ │ 3. Add provider error classification        │    │ │
│  │ │ 4. Integration tests for retry scenarios    │    │ │
│  │ └─────────────────────────────────────────────┘    │ │
│  │                                                    │ │
│  │ META: Approve this plan? [y]es [e]dit [r]esearch   │ │
│  └────────────────────────────────────────────────────┘ │
│                                                         │
│  ╭─ INPUT ────────────────────────────────────────────╮ │
│  │ > _                                                │ │
│  ╰────────────────────────────────────────────────────╯ │
│                                                         │
│  CONV  [Enter] send  [Tab] focus  [Esc] back  [?] help │
└─────────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `ConversationalSession.phase == Ideation` | `roko prd idea` | Meta-agent extracts idea text |
| `ConversationalSession.phase == Drafting` | `roko prd draft new` | Meta-agent refines PRD |
| `ConversationalSession.phase == Researching` | `roko research enhance-prd` | Auto-triggered if `auto_research` |
| `ConversationalSession.phase == Planning` | `roko prd plan <slug>` | Plan generation |
| `ConversationalSession.phase == Executing` | `roko plan run` | PlanRunner with live feedback |
| `OperatorDecision` | `ConversationalSession.decisions` | Persisted as Engrams |
| `ConversationTurn` | `.roko/conversations/<id>.jsonl` | Append-only session log |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_phase_transitions_are_sequential() {
        let valid_transitions = [
            (ConversationalPhase::Ideation, ConversationalPhase::Drafting),
            (ConversationalPhase::Drafting, ConversationalPhase::Researching),
            (ConversationalPhase::Drafting, ConversationalPhase::Planning),
            (ConversationalPhase::Researching, ConversationalPhase::Planning),
            (ConversationalPhase::Planning, ConversationalPhase::Executing),
            (ConversationalPhase::Executing, ConversationalPhase::Reviewing),
            (ConversationalPhase::Reviewing, ConversationalPhase::Done),
            // Failure replan loop:
            (ConversationalPhase::Executing, ConversationalPhase::Planning),
            // Pause/resume:
            (ConversationalPhase::Executing, ConversationalPhase::Paused),
        ];
        for (from, to) in &valid_transitions {
            assert!(is_valid_transition(*from, *to), "{from:?} → {to:?} should be valid");
        }
    }

    #[test]
    fn session_cannot_skip_drafting() {
        assert!(!is_valid_transition(
            ConversationalPhase::Ideation,
            ConversationalPhase::Executing
        ));
    }

    #[test]
    fn operator_decision_required_before_executing() {
        let session = ConversationalSession::new("add retry logic");
        assert!(session.can_advance_to(ConversationalPhase::Executing).is_err(),
            "Must have operator approval before execution");
    }

    #[test]
    fn conversation_turns_are_persisted() {
        let mut session = ConversationalSession::new("test intent");
        session.add_turn(Speaker::Operator, "add retry logic".into());
        session.add_turn(Speaker::MetaAgent, "I'll draft a PRD...".into());
        assert_eq!(session.turns.len(), 2);
        assert_eq!(session.turns[0].speaker, Speaker::Operator);
    }

    #[test]
    fn auto_advance_skips_confirmation_for_safe_phases() {
        let mut config = ConversationalConfig::default();
        config.auto_advance = true;
        let session = ConversationalSession::with_config("test", config);
        // Ideation→Drafting and Drafting→Researching are safe to auto-advance
        assert!(session.config.auto_advance);
        // Executing still requires confirmation even with auto_advance
        // (executing runs real agent work and costs money)
    }
}
```

---

## 2. Time-Travel Debugging

> Rewind agent decisions via the Witness DAG and episode log — inspect, replay, and understand why agents made specific choices at any point in execution history.

### Problem Statement

When an agent fails a gate or produces unexpected output, the operator currently sees only the final result: a pass/fail verdict, an output diff, or a cost number. The *reasoning chain* that led to the failure is opaque. The operator cannot answer questions like:

- "What context did the agent have when it made this decision?"
- "What would have happened if the agent had used a different model?"
- "At what point did the execution diverge from the plan?"

This is the **agent observability gap**: traditional debugging tools (breakpoints, stack traces, log grep) were designed for deterministic software. Agent decisions are stochastic, context-sensitive, and distributed across turns. Debugging them requires a fundamentally different interface.

### Research Foundations

**Beyond Black-Box Benchmarking** (Moshkovich et al., 2025) introduced observability taxonomies for agentic systems, extending OpenTelemetry to handle non-deterministic decision chains. Their user study found 79% agreement that non-deterministic flow is the primary debugging challenge. Roko's hash-chained EventLog (BLAKE3) in the orchestrator already provides the audit substrate; time-travel debugging adds the *interactive* layer.

**E-MARS+** (Enhanced Memory Architecture for Reasoning Systems, 2025) includes timeline replay as an evaluation mechanism — rewinding to any point in an agent's history and inspecting reasoning state. Their cross-session consistency evaluation directly maps to Roko's episode log, where episodes span multiple agent turns within a task.

**AgentOps** (2024–2025) commercialized "session replay" for autonomous agents — capturing hierarchical multi-agent traces with tool usage patterns, self-correction loops, and planning stages. LangGraph Studio added checkpoint-based state rewind. These validate the UX pattern; Roko's contribution is integrating replay with the Witness DAG's cryptographic provenance chain.

### Design

Time-travel debugging overlays a temporal navigation layer on the episode log and Witness DAG. The operator selects a point in execution history and the TUI reconstructs the agent's state at that moment: available context, model selection rationale, gate pipeline state, and Daimon affect vector.

#### Temporal Index

```rust
/// A navigable index over execution history, linking episodes to
/// their causal predecessors and successors in the Witness DAG.
#[derive(Debug, Clone)]
pub struct TemporalIndex {
    /// All episodes, sorted chronologically
    pub timeline: Vec<TimelineEntry>,
    /// DAG edges: episode_id → Vec<predecessor_episode_ids>
    pub causal_predecessors: HashMap<String, Vec<String>>,
    /// DAG edges: episode_id → Vec<successor_episode_ids>
    pub causal_successors: HashMap<String, Vec<String>>,
    /// Branching points where execution diverged from plan
    pub divergence_points: Vec<DivergencePoint>,
    /// Cursor position in the timeline (for TUI navigation)
    pub cursor: usize,
    /// Optional filter: show only episodes matching criteria
    pub filter: Option<TimelineFilter>,
}

/// A single entry in the execution timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// The episode this entry represents
    pub episode_id: String,
    /// Agent that produced this episode
    pub agent_id: String,
    /// Task being worked on
    pub task_id: String,
    /// Plan this task belongs to
    pub plan_id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Duration of the episode
    pub duration: Duration,
    /// Model used
    pub model: String,
    /// Whether the episode succeeded
    pub success: bool,
    /// Gate verdicts for this episode
    pub gate_verdicts: Vec<GateVerdict>,
    /// Daimon PAD vector at the time of this episode
    pub pad_at_episode: Option<PadSnapshot>,
    /// Token/cost usage
    pub usage: Usage,
    /// Reasoning summary (if available)
    pub reasoning_summary: Option<String>,
    /// Depth in the causal DAG (0 = root trigger)
    pub causal_depth: u32,
}

/// A point where actual execution diverged from the planned path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergencePoint {
    /// Episode where divergence was detected
    pub episode_id: String,
    /// What was expected (from plan)
    pub expected: String,
    /// What actually happened
    pub actual: String,
    /// Severity: info, warning, critical
    pub severity: DivergenceSeverity,
    /// Causal explanation (if available from agent reasoning)
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceSeverity {
    /// Minor deviation, execution continued normally
    Info,
    /// Notable deviation, may affect downstream tasks
    Warning,
    /// Major divergence, likely caused failure
    Critical,
}

/// Snapshot of Daimon state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadSnapshot {
    pub pleasure: f32,
    pub arousal: f32,
    pub dominance: f32,
    pub behavioral_state: String,
}

/// Filter criteria for the timeline view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineFilter {
    /// Show only episodes from this agent
    pub agent_id: Option<String>,
    /// Show only episodes for this task
    pub task_id: Option<String>,
    /// Show only failed episodes
    pub failures_only: bool,
    /// Show only divergence points
    pub divergences_only: bool,
    /// Time range
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

/// Reconstructed agent state at a specific point in history.
/// This is what the operator sees when they "rewind" to an episode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconstructedState {
    /// The episode being inspected
    pub episode: Episode,
    /// Context that was available to the agent at this point
    pub available_context: ContextSnapshot,
    /// Model selection rationale from CascadeRouter
    pub routing_rationale: Option<RoutingRationale>,
    /// Gate pipeline state at this moment
    pub gate_state: Vec<GateStatus>,
    /// Daimon affect state
    pub daimon_state: PadSnapshot,
    /// Neuro knowledge entries the agent had access to
    pub knowledge_context: Vec<KnowledgeEntrySummary>,
    /// Counterfactual: what would have happened with a different model?
    pub counterfactuals: Vec<Counterfactual>,
}

/// A what-if scenario computed from HDC perturbation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counterfactual {
    /// What was changed (e.g., "model: haiku → sonnet")
    pub perturbation: String,
    /// HDC similarity score to the actual outcome (0.0–1.0)
    pub similarity: f64,
    /// Predicted outcome based on historical data
    pub predicted_outcome: String,
    /// Confidence in the prediction
    pub confidence: f64,
}

/// Summary of a knowledge entry (for display in reconstructed state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntrySummary {
    pub id: String,
    pub kind: String,
    pub content_preview: String,
    pub confidence: f64,
}

/// Why the CascadeRouter selected a particular model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRationale {
    pub selected_model: String,
    pub stage: String,
    pub pass_rate_estimate: f64,
    pub cost_estimate: f64,
    pub alternatives_considered: Vec<(String, f64, f64)>,
}

/// Per-gate status at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateStatus {
    pub gate: String,
    pub state: String,
    pub last_result: Option<bool>,
}

/// Snapshot of the context assembled for an agent at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    /// System prompt layers that were active
    pub prompt_layers: Vec<String>,
    /// Total token budget allocated
    pub token_budget: u64,
    /// Knowledge entries injected
    pub knowledge_count: usize,
    /// Playbook rules that were active
    pub active_playbooks: Vec<String>,
}
```

#### TUI Mockup

```
┌─ TIME-TRAVEL DEBUGGER ───────── Plan: retry-logic ──┐
│                                                       │
│  TIMELINE (12 episodes, 3 agents, 42 min)            │
│  ─────────────────────────────────────────────────── │
│  t=0    ◉ rust-impl    task-1  haiku   ✓  $0.02     │
│  t=3m   ◉ rust-impl    task-1  haiku   ✓  $0.03     │
│  t=8m   ◉ rust-impl    task-2  sonnet  ✓  $0.12     │
│  t=14m  ◉ reviewer     task-2  haiku   ✓  $0.01     │
│  t=18m  ◉ rust-impl    task-3  sonnet  ✗  $0.15  ◄─┤
│  t=24m  ◉ rust-impl    task-3  opus    ✓  $0.34  △  │
│  t=31m  ◉ rust-impl    task-4  sonnet  ✓  $0.11     │
│  t=38m  ◉ reviewer     task-4  haiku   ✓  $0.01     │
│  ─────────────────────────────────────────────────── │
│          ▁▂▃▅▃▇▅▂  cost over time                    │
│                                                       │
│  ┌─ EPISODE DETAIL (t=18m) ──────────────────────┐   │
│  │ Agent: rust-impl  Task: task-3  Model: sonnet  │   │
│  │ Duration: 5m 42s  Tokens: 12,340  Cost: $0.15  │   │
│  │ Success: ✗  Reason: test gate failed            │   │
│  │                                                 │   │
│  │ DAIMON: P:-0.3 A:0.8 D:0.4  Struggling         │   │
│  │                                                 │   │
│  │ GATES:  ✓ compile  ✗ test  ○ clippy  ○ diff     │   │
│  │  └─ test: 2/47 failed (test_retry_overflow,     │   │
│  │         test_backoff_jitter)                     │   │
│  │                                                 │   │
│  │ CONTEXT: 6 prompt layers, 4 knowledge entries   │   │
│  │ ROUTING: sonnet selected (pass_rate: 0.72,      │   │
│  │          confidence stage, haiku rejected at     │   │
│  │          0.45 pass_rate)                         │   │
│  │                                                 │   │
│  │ ▲ DIVERGENCE: Expected task-3 to pass on first  │   │
│  │   attempt. Agent used retry count 5 instead of  │   │
│  │   configured max 3.                             │   │
│  │                                                 │   │
│  │ COUNTERFACTUALS:                                │   │
│  │  opus   → predicted ✓ (0.89 confidence)         │   │
│  │  haiku  → predicted ✗ (0.91 confidence)         │   │
│  └─────────────────────────────────────────────────┘   │
│                                                       │
│  [j/k] navigate  [Enter] expand  [c] counterfactual  │
│  [d] diff context  [r] replay  [f] filter  [q] back  │
└───────────────────────────────────────────────────────┘
```

#### Causal DAG Navigation

The `[r]` key enters DAG navigation mode, showing the causal chain:

```
┌─ CAUSAL CHAIN ─── task-3 failure ────────────────────┐
│                                                       │
│  [trigger]                                            │
│     │                                                 │
│     ▼                                                 │
│  ┌─────────────┐    ┌──────────────┐                 │
│  │ task-2 ✓    │───▶│ task-3 ✗     │                 │
│  │ retry wrap  │    │ wire into    │                 │
│  │ haiku $0.12 │    │ dispatcher   │                 │
│  └─────────────┘    │ sonnet $0.15 │                 │
│                     └──────┬───────┘                 │
│                            │ escalated               │
│                     ┌──────▼───────┐                 │
│                     │ task-3 ✓     │                 │
│                     │ (retry)      │                 │
│                     │ opus $0.34   │                 │
│                     └──────┬───────┘                 │
│                            │                         │
│                     ┌──────▼───────┐                 │
│                     │ task-4 ✓     │                 │
│                     │ integration  │                 │
│                     │ tests        │                 │
│                     └──────────────┘                 │
│                                                       │
│  [←/→] traverse  [Enter] inspect  [Esc] timeline     │
└───────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `EpisodeLogger` (roko-learn) | `TemporalIndex.timeline` | Read `.roko/episodes.jsonl` |
| `EventLog` (roko-orchestrator) | `TemporalIndex.causal_predecessors` | Hash-chain traversal |
| `ChainWitnessEngine` (roko-golem) | `DivergencePoint` detection | DAG node comparison |
| `CascadeRouter` (roko-learn) | `RoutingRationale` | Router decision log |
| `DaimonState` (roko-core) | `PadSnapshot` | PAD vector at episode time |
| `KnowledgeStore` (roko-neuro) | `KnowledgeEntrySummary` | Entries available at episode time |
| `DreamCycle` counterfactuals | `Counterfactual` | `.roko/dreams/counterfactuals.jsonl` |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_index_builds_from_episodes() {
        let episodes = vec![
            make_episode("ep-1", "task-1", true, Utc::now()),
            make_episode("ep-2", "task-2", true, Utc::now() + Duration::minutes(5)),
            make_episode("ep-3", "task-3", false, Utc::now() + Duration::minutes(10)),
        ];
        let index = TemporalIndex::from_episodes(&episodes);
        assert_eq!(index.timeline.len(), 3);
        assert!(index.timeline[0].timestamp < index.timeline[1].timestamp);
    }

    #[test]
    fn divergence_detection_finds_plan_mismatches() {
        let planned = vec!["task-1", "task-2", "task-3"];
        let actual = vec![
            make_episode("ep-1", "task-1", true, Utc::now()),
            make_episode("ep-2", "task-2", false, Utc::now() + Duration::minutes(5)),
            // task-2 failed and was retried — this is a divergence
            make_episode("ep-3", "task-2", true, Utc::now() + Duration::minutes(10)),
        ];
        let divergences = detect_divergences(&planned, &actual);
        assert_eq!(divergences.len(), 1);
        assert_eq!(divergences[0].severity, DivergenceSeverity::Warning);
    }

    #[test]
    fn causal_predecessors_trace_backward() {
        let index = make_test_index_with_dag();
        let predecessors = index.causal_predecessors.get("ep-3").unwrap();
        assert!(predecessors.contains(&"ep-2".to_string()));
    }

    #[test]
    fn filter_shows_only_failures() {
        let mut index = make_test_index();
        index.filter = Some(TimelineFilter {
            failures_only: true,
            ..Default::default()
        });
        let visible: Vec<_> = index.visible_entries().collect();
        assert!(visible.iter().all(|e| !e.success));
    }

    #[test]
    fn reconstructed_state_includes_context_at_time() {
        let state = reconstruct_state_at("ep-3", &episodes, &knowledge_store);
        assert!(state.knowledge_context.len() > 0,
            "Should include knowledge available at episode time");
        assert!(state.routing_rationale.is_some(),
            "Should include model selection rationale");
    }

    #[test]
    fn counterfactuals_have_bounded_confidence() {
        let counterfactuals = generate_counterfactuals("ep-3", &episodes);
        for cf in &counterfactuals {
            assert!(cf.confidence >= 0.0 && cf.confidence <= 1.0);
            assert!(cf.similarity >= 0.0 && cf.similarity <= 1.0);
        }
    }
}
```

---

## 3. Dream Journal Interface

> Review what agents learned during Dreams offline consolidation — a browsable log of cluster discoveries, knowledge distillation, counterfactual experiments, and regression alerts.

### Problem Statement

Roko's Dreams subsystem (`roko-dreams`) runs offline consolidation that transforms raw episodes into durable knowledge. It clusters episodes by (plan_id, task_type, outcome, model), distills insights and playbooks, detects regressions, and generates counterfactual hypotheses. But the output — `DreamCycleReport` written to `.roko/dreams/dream-<timestamp>.json` — is a dense JSON file that no human reads.

The dream process is the system's most important self-improvement mechanism, yet it's invisible. Operators cannot answer: "What did the agents learn overnight?" or "Why was this playbook generated?" or "What regressions were detected?"

### Research Foundations

**NeuroDream** (Tutuncuoglu, 2024) demonstrated a biologically inspired "dream phase" where models consolidate via latent replay, achieving 38% reduction in catastrophic forgetting. The dream phase's latent embeddings can be rendered as a browsable journal of abstracted patterns — validating the concept of a human-readable dream output.

**GW-Dreamer** (Maytie et al., 2025) uses Global Workspace Theory to dream in a shared latent space. Their per-modality decomposition of the dreaming process provides a visualization metaphor: each specialist module's contribution to the dream rendered as a separate channel. For Roko, each cluster's contribution to the dream cycle is one "channel."

**Interpretable World Model Imaginations** (Wenninghoff & Schwammberger, xAI 2025) proposed rendering agent "imaginations" — internal simulations generated during training — as contrastive explanations (predicted vs. actual). This is the closest existing work to a dream journal UI: making the agent's counterfactual reasoning visible.

### Design

The Dream Journal presents `DreamCycleReport` data as a structured, navigable log. Each dream cycle becomes a "journal entry" with sections for discoveries, regressions, playbooks, and counterfactuals.

#### Rust Structs

```rust
/// A rendered view of a DreamCycleReport for TUI display.
#[derive(Debug, Clone)]
pub struct DreamJournalView {
    /// All dream cycle entries, newest first
    pub entries: Vec<DreamJournalEntry>,
    /// Currently selected entry index
    pub selected_entry: usize,
    /// Currently selected section within the entry
    pub selected_section: DreamJournalSection,
    /// Summary statistics across all entries
    pub aggregate: DreamJournalAggregate,
}

/// A single dream cycle rendered as a journal entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamJournalEntry {
    /// Dream cycle timestamp
    pub timestamp: DateTime<Utc>,
    /// Duration of the dream cycle
    pub duration: Duration,
    /// Number of episodes processed
    pub episodes_processed: usize,
    /// Number of clusters discovered
    pub clusters_discovered: usize,
    /// Headline discoveries (most significant findings)
    pub headlines: Vec<DreamHeadline>,
    /// Knowledge entries created during this cycle
    pub knowledge_created: Vec<KnowledgeEntrySummary>,
    /// Playbooks synthesized from successful clusters
    pub playbooks_created: Vec<PlaybookSummary>,
    /// Regressions detected (performance drops, failure patterns)
    pub regressions: Vec<RegressionAlert>,
    /// Cross-domain strategy hypotheses
    pub strategy_hypotheses: Vec<StrategySummary>,
    /// Counterfactual experiments and their results
    pub counterfactuals: Vec<CounterfactualExperiment>,
    /// C-Factor regression analysis
    pub cfactor_trend: Option<CFactorTrend>,
    /// Performance notes from the dream cycle
    pub notes: Vec<String>,
}

/// A headline discovery — the most important finding from a dream cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamHeadline {
    /// Emoji-free icon for TUI (✦ discovery, ⚠ regression, ◈ playbook)
    pub icon: char,
    /// Short headline text (≤80 chars)
    pub headline: String,
    /// Detail text (expandable)
    pub detail: String,
    /// Severity: discovery, improvement, regression, critical
    pub severity: HeadlineSeverity,
    /// Source cluster key
    pub source_cluster: Option<DreamClusterKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadlineSeverity {
    /// New pattern or insight discovered
    Discovery,
    /// Performance improvement detected
    Improvement,
    /// Performance regression detected
    Regression,
    /// Critical regression requiring immediate attention
    Critical,
}

/// Summary of a playbook synthesized during dreaming.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookSummary {
    pub name: String,
    pub trigger_pattern: String,
    pub success_rate: f64,
    pub source_episodes: usize,
}

/// A regression alert from the dream cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionAlert {
    /// What regressed (e.g., "test pass rate for rust-impl tasks")
    pub metric: String,
    /// Previous value
    pub previous: f64,
    /// Current value
    pub current: f64,
    /// Percentage drop
    pub drop_percent: f64,
    /// Affected plan/task context
    pub context: String,
    /// Suggested action
    pub suggestion: Option<String>,
}

/// A cross-domain strategy hypothesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategySummary {
    /// The hypothesis text
    pub hypothesis: String,
    /// Supporting evidence (cluster summaries)
    pub evidence: Vec<String>,
    /// Confidence in the hypothesis
    pub confidence: f64,
}

/// A counterfactual experiment result from the dream cycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualExperiment {
    /// What was varied (model, plan, task_type, etc.)
    pub axis: String,
    /// Original value
    pub original: String,
    /// Counterfactual value
    pub alternative: String,
    /// Predicted outcome difference
    pub predicted_effect: String,
    /// HDC similarity score
    pub similarity: f64,
}

/// Aggregate stats across all dream journal entries.
#[derive(Debug, Clone, Default)]
pub struct DreamJournalAggregate {
    pub total_cycles: usize,
    pub total_episodes_processed: usize,
    pub total_knowledge_created: usize,
    pub total_playbooks_created: usize,
    pub total_regressions_detected: usize,
    pub avg_episodes_per_cycle: f64,
    pub knowledge_creation_trend: Vec<(DateTime<Utc>, usize)>,
}

/// Which section of a journal entry is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DreamJournalSection {
    #[default]
    Headlines,
    Knowledge,
    Playbooks,
    Regressions,
    Strategies,
    Counterfactuals,
}
```

#### TUI Mockup

```
┌─ DREAM JOURNAL ──────────────────── 7 cycles total ──┐
│                                                        │
│  ┌─ ENTRIES ──────────────────────────────────────────┐│
│  │ ▸ Apr 13, 02:30  38 eps  4 clusters  2 insights  ││
│  │   Apr 12, 03:15  52 eps  6 clusters  3 insights  ││
│  │   Apr 11, 02:45  29 eps  3 clusters  1 playbook   ││
│  │   Apr 10, 04:00  61 eps  7 clusters  ⚠ regression ││
│  └────────────────────────────────────────────────────┘│
│                                                        │
│  ┌─ Apr 13 DREAM CYCLE ── 38 eps in 12 min ─────────┐│
│  │                                                    ││
│  │  HEADLINES                                         ││
│  │  ✦ Discovered: sonnet outperforms opus on simple   ││
│  │    refactor tasks (87% vs 82%, 3x cheaper)         ││
│  │  ✦ New heuristic: tasks touching >5 files benefit  ││
│  │    from research phase (pass rate +23%)            ││
│  │                                                    ││
│  │  KNOWLEDGE CREATED (2)                             ││
│  │  ├─ Insight: "Provider rate limits cluster at      ││
│  │  │  14:00-16:00 UTC; schedule heavy work earlier"  ││
│  │  │  confidence: 0.78  sources: 12 episodes         ││
│  │  └─ Heuristic: "Retry with backoff before model    ││
│  │     escalation saves 40% cost on transient errors" ││
│  │     confidence: 0.83  sources: 8 episodes          ││
│  │                                                    ││
│  │  COUNTERFACTUALS                                   ││
│  │  ├─ If model=haiku instead of sonnet on task-3:    ││
│  │  │  predicted ✗ (similarity: 0.34)                 ││
│  │  └─ If research phase added to plan-7:             ││
│  │     predicted ✓ (similarity: 0.76)                 ││
│  │                                                    ││
│  │  PLAYBOOKS: none this cycle                        ││
│  │  REGRESSIONS: none this cycle                      ││
│  │                                                    ││
│  │  ▁▂▃▅▃▂▅▇  knowledge created per cycle             ││
│  └────────────────────────────────────────────────────┘│
│                                                        │
│  [j/k] entries  [Tab] sections  [Enter] expand  [q]   │
└────────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `DreamCycleReport` (roko-dreams) | `DreamJournalEntry` | Parse `.roko/dreams/dream-*.json` |
| `DreamClusterReport` | `DreamHeadline` | Cluster → headline generation |
| `KnowledgeEntry` (roko-neuro) | `KnowledgeEntrySummary` | Created entries from cycle |
| `Playbook` (roko-learn) | `PlaybookSummary` | Playbooks created this cycle |
| `DreamCycle.cfactor_regression` | `CFactorTrend` | C-Factor regression data |
| `.roko/dreams/counterfactuals.jsonl` | `CounterfactualExperiment` | HDC perturbation results |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dream_journal_loads_from_reports() {
        let reports = load_dream_reports(".roko/dreams/");
        let journal = DreamJournalView::from_reports(reports);
        assert!(journal.entries.len() > 0);
        assert!(journal.entries[0].timestamp >= journal.entries[1].timestamp,
            "Entries should be newest-first");
    }

    #[test]
    fn headlines_generated_from_clusters() {
        let cluster = make_test_cluster(success_count: 8, failure_count: 2);
        let headlines = generate_headlines(&[cluster]);
        assert!(headlines.len() >= 1, "Should generate at least one headline");
        assert!(headlines[0].headline.len() <= 80, "Headlines ≤80 chars");
    }

    #[test]
    fn regression_alert_fires_on_20_percent_drop() {
        let previous_rate = 0.85;
        let current_rate = 0.65;
        let alert = check_regression(previous_rate, current_rate);
        assert!(alert.is_some());
        assert_eq!(alert.unwrap().severity, HeadlineSeverity::Regression);
    }

    #[test]
    fn aggregate_stats_computed_correctly() {
        let entries = vec![
            make_entry(episodes: 38, knowledge: 2),
            make_entry(episodes: 52, knowledge: 3),
        ];
        let agg = DreamJournalAggregate::from_entries(&entries);
        assert_eq!(agg.total_cycles, 2);
        assert_eq!(agg.total_episodes_processed, 90);
        assert_eq!(agg.total_knowledge_created, 5);
        assert!((agg.avg_episodes_per_cycle - 45.0).abs() < 0.01);
    }
}
```

---

## 4. Agent Garden

> Visualize the fleet of cognitive agents as a living ecosystem — agents are organisms with health, growth rings, interaction tendrils, and environmental conditions reflecting system state.

### Problem Statement

The current agent list in the TUI sidebar is a flat roster: name, status icon, task count. This is adequate for 2–3 agents but breaks down when the fleet scales. With 10+ agents working across multiple plans, the sidebar cannot convey:

- **Relative health**: Which agents are thriving vs. struggling?
- **Growth trajectories**: Which agents are accumulating knowledge and improving?
- **Interaction patterns**: Which agents collaborate, and how intensely?
- **Ecosystem balance**: Is the fleet well-distributed across tasks, or clustering?

The Agent Garden reimagines the fleet as an ecosystem, drawing on the biological metaphor already established by the Spectre creature system but extending it to the *collective* level.

### Research Foundations

**Generative Agents** (Park et al., UIST 2023) populated a Sims-like sandbox with 25 LLM agents exhibiting emergent social behaviors. The 2D world view with observable agent behaviors, memory streams, and social graphs is the definitive archetype for agent garden visualization. Roko extends this by grounding the visualization in real performance data rather than simulated personality.

**AIDO** (Xing et al., NeurIPS 2024) constructed AI-Driven Digital Organisms with multiscale visualization — molecular to cellular to individual. This multiscale zoom metaphor maps directly to Roko's needs: individual agent → task-level → plan-level → system-level views.

**Self-Evolving AI Agents Survey** (EvoAgentX, 2025) catalogued how agents evolve capabilities based on interaction data. The "digital organism" metaphor becomes literal when agents learn from experience. The survey provides the taxonomy for what an Agent Garden must display: adaptation history, capability growth, and the distinction between learned and built-in behaviors.

### Design

The Agent Garden is a spatial visualization where each agent is a Spectre creature positioned in a 2D field. Spatial position encodes task assignment, proximity encodes collaboration intensity, and organism properties (size, coloring, tendril activity) encode performance metrics.

#### Rust Structs

```rust
/// The Agent Garden: a spatial ecosystem view of the agent fleet.
#[derive(Debug, Clone)]
pub struct AgentGarden {
    /// All organisms in the garden (one per active agent)
    pub organisms: Vec<GardenOrganism>,
    /// Environment conditions (system-level metrics)
    pub environment: GardenEnvironment,
    /// Interaction edges between organisms
    pub interactions: Vec<GardenInteraction>,
    /// Pheromone field (stigmergic signals)
    pub pheromones: Vec<PheromoneDeposit>,
    /// Viewport bounds for rendering
    pub viewport: GardenViewport,
    /// Selected organism (for detail panel)
    pub selected: Option<usize>,
    /// Animation time accumulator
    pub time: f64,
}

/// A single agent rendered as a garden organism.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenOrganism {
    /// Agent identity
    pub agent_id: String,
    /// Position in garden space (0.0–1.0 normalized)
    pub position: [f32; 2],
    /// Target position (organisms drift toward targets)
    pub target_position: [f32; 2],
    /// Size factor (1.0 = base size, scales with knowledge)
    /// Formula: 1.0 + 0.3 * log2(1 + total_knowledge_entries)
    pub size: f32,
    /// Growth rings (visual accumulation of completed tasks)
    pub growth_rings: u32,
    /// Current behavioral state (from Daimon)
    pub behavioral_state: String,
    /// Spectre rendering parameters (inherited from creature system)
    pub spectre_params: SpectreAnimationParams,
    /// Health score (0.0–1.0, composite of recent success rate + gate pass rate)
    pub health: f32,
    /// Age: number of completed episodes
    pub maturity: u32,
    /// Current task assignment (determines position cluster)
    pub task_cluster: Option<String>,
    /// C-Factor contribution
    pub cfactor_contribution: f64,
    /// Knowledge tier distribution
    pub knowledge_tiers: KnowledgeTierCounts,
}

/// Spectre animation parameters for garden rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpectreAnimationParams {
    pub breathing_rate: f32,
    pub glow_color: [u8; 3],
    pub glow_intensity: f32,
    pub eye_state: String,
    pub tendril_count: u32,
}

/// Knowledge tier counts for size/density calculation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeTierCounts {
    pub transient: u32,
    pub working: u32,
    pub consolidated: u32,
    pub persistent: u32,
}

/// System-level conditions rendered as environmental factors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenEnvironment {
    /// Overall system health (0.0–1.0)
    /// Rendered as background brightness: healthy=dim rose, struggling=amber
    pub system_health: f32,
    /// API provider status (affects "weather")
    pub provider_health: ProviderHealthStatus,
    /// Budget utilization (0.0–1.0)
    /// Rendered as "season": low=spring/lush, high=autumn/sparse
    pub budget_utilization: f32,
    /// Time pressure: active deadlines affect environmental urgency
    pub time_pressure: f32,
    /// C-Factor (collective health)
    pub cfactor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthStatus {
    /// All providers healthy — clear weather
    Healthy,
    /// Some providers degraded — overcast
    Degraded,
    /// Provider outage — storm
    Outage,
}

/// An interaction between two organisms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenInteraction {
    /// Source organism index
    pub from: usize,
    /// Target organism index
    pub to: usize,
    /// Interaction type
    pub kind: InteractionKind,
    /// Strength (0.0–1.0), controls tendril thickness
    pub strength: f32,
    /// Recency: time since last interaction (affects visibility)
    pub last_interaction: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
    /// Agents share a task dependency
    TaskDependency,
    /// Agent output feeds into another's input
    DataFlow,
    /// Agents share knowledge via mesh sync
    KnowledgeSync,
    /// Pheromone-mediated indirect collaboration
    Stigmergy,
}

/// A pheromone deposit in the garden field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PheromoneDeposit {
    /// Position in garden space
    pub position: [f32; 2],
    /// Pheromone type (Wisdom, Warning, Discovery)
    pub kind: String,
    /// Intensity (decays over time)
    pub intensity: f32,
    /// Depositing agent
    pub source_agent: String,
    /// Deposit time
    pub deposited_at: DateTime<Utc>,
}

/// Viewport for garden rendering.
#[derive(Debug, Clone)]
pub struct GardenViewport {
    /// Zoom level (1.0 = fit all organisms, 2.0+ = zoom in)
    pub zoom: f32,
    /// Center offset for panning
    pub center: [f32; 2],
    /// Terminal area allocated to garden
    pub area: (u16, u16, u16, u16),
}
```

#### TUI Mockup

```
┌─ AGENT GARDEN ─────────── C:1.23 ── 5 agents ── $4.12 ──┐
│                                                            │
│  ╭─────────────────────────── plan-1 tasks ──────────────╮ │
│  │                                                       │ │
│  │      ╭─╮                    ╭──╮                      │ │
│  │  ╭───╯ ╰───╮          ╭────╯  ╰────╮                 │ │
│  │  │  ◉    ◉  │ ≋≋≋≋≋≋≋ │  ◉      ◉  │                │ │
│  │  ╰─────────╯          ╰────────────╯                 │ │
│  │   rust-impl              reviewer                     │ │
│  │   ▃▅▇▆▅ 3/7             ▅▇███ 1/3                    │ │
│  │   Engaged                Focused                      │ │
│  │                                                       │ │
│  ╰───────────────────────────────────────────────────────╯ │
│                                                            │
│  ╭─── plan-2 tasks ──╮   ╭─── idle ──────────────────╮    │
│  │                    │   │                            │    │
│  │   ╭─╮     ╭─╮     │   │     ╭─╮                   │    │
│  │   │◉◉│    │◉◉│    │   │    ╭╯ ╰╮                  │    │
│  │   ╰──╯    ╰──╯    │   │    │─ ─│                  │    │
│  │  front-end  api    │   │    ╰───╯                  │    │
│  │  ▃▅▆ 2/5  ▅▇ 1/4  │   │   researcher              │    │
│  │  Exploring Engaged │   │   Resting   ✧ ✧           │    │
│  ╰────────────────────╯   ╰──────────────────────────╯    │
│                                                            │
│  ┌─ ENVIRONMENT ──────────────────────────────────────┐    │
│  │ Weather: clear ☀   Budget: ████░░░░ 41%            │    │
│  │ Providers: all healthy   Pressure: low             │    │
│  └────────────────────────────────────────────────────┘    │
│                                                            │
│  [←/→/↑/↓] navigate  [Enter] select  [z] zoom  [q] back  │
└────────────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `PlanRunner` agent state (roko-cli) | `GardenOrganism` | Active agent list + status |
| `DaimonState` (roko-core) | `SpectreAnimationParams` | PAD → behavioral state → animation |
| `KnowledgeStore` (roko-neuro) | `KnowledgeTierCounts` | Per-agent knowledge accumulation |
| `EpisodeLogger` (roko-learn) | `growth_rings`, `maturity` | Episode count per agent |
| Agent Mesh (roko-conductor) | `GardenInteraction` | Peer connections, pheromones |
| `CascadeRouter` (roko-learn) | `GardenEnvironment.provider_health` | Provider circuit breaker state |
| Cost tracking (roko-learn) | `GardenEnvironment.budget_utilization` | Budget fraction |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn organism_size_scales_with_knowledge() {
        let counts = KnowledgeTierCounts { transient: 0, working: 0, consolidated: 0, persistent: 0 };
        let base_size = organism_size(&counts);
        let counts_100 = KnowledgeTierCounts { transient: 30, working: 50, consolidated: 15, persistent: 5 };
        let grown_size = organism_size(&counts_100);
        assert!(grown_size > base_size, "More knowledge = larger organism");
        assert!(grown_size < 3.0, "Size should be bounded");
    }

    #[test]
    fn organisms_cluster_by_task() {
        let organisms = vec![
            make_organism("agent-1", Some("plan-1")),
            make_organism("agent-2", Some("plan-1")),
            make_organism("agent-3", Some("plan-2")),
        ];
        let layout = compute_garden_layout(&organisms);
        let dist_same_plan = distance(layout[0].position, layout[1].position);
        let dist_diff_plan = distance(layout[0].position, layout[2].position);
        assert!(dist_same_plan < dist_diff_plan,
            "Same-plan agents should cluster closer together");
    }

    #[test]
    fn pheromones_decay_over_time() {
        let deposit = PheromoneDeposit {
            intensity: 1.0,
            deposited_at: Utc::now() - Duration::minutes(30),
            ..Default::default()
        };
        let current_intensity = decayed_intensity(&deposit, Utc::now());
        assert!(current_intensity < 1.0, "Pheromones should decay");
        assert!(current_intensity > 0.0, "Should not fully decay in 30 min");
    }

    #[test]
    fn environment_reflects_provider_health() {
        let env = GardenEnvironment::from_system_state(&system_state);
        match system_state.provider_status {
            CircuitBreakerState::Closed => assert_eq!(env.provider_health, ProviderHealthStatus::Healthy),
            CircuitBreakerState::Open => assert_eq!(env.provider_health, ProviderHealthStatus::Outage),
            CircuitBreakerState::HalfOpen => assert_eq!(env.provider_health, ProviderHealthStatus::Degraded),
        }
    }
}
```

---

## 5. Pair Programming with Affect

> Agents express their cognitive state through the Daimon affect system — the operator sees not just *what* an agent is doing, but *how it's experiencing* the work. Affect modulates the interaction style.

### Problem Statement

Current agent output is purely informational: code, test results, gate verdicts. The agent's *cognitive experience* — its confidence level, frustration with repeated failures, excitement about a novel approach — is invisible. This creates an empathy gap: the operator cannot gauge whether an agent needs a different prompt strategy, a model escalation, or simply a different task assignment.

The Daimon subsystem already computes PAD vectors and behavioral states. This innovation exposes them as a first-class UX element during pair programming sessions (single-agent, interactive work).

### Research Foundations

**Empathetic Conversational Agents** (IJHCI, 2025) reviewed neural and physiological signals for emotion recognition in conversational agents, distinguishing cognitive empathy (understanding state) from affective empathy (appropriate response). For Roko, the operator doesn't need to empathize with the agent but does need to *read* its state to make better decisions.

**RLVER** (Wang et al., Tencent, 2025) demonstrated the first end-to-end RL framework using verifiable emotion rewards. Fine-tuning with emotional scores boosted empathy benchmarks from 13.3 to 79.2 without degrading task performance — proving that affective capability isn't cosmetic. For Roko, displaying affect is functionally useful: a Struggling agent might benefit from simpler task decomposition.

**Affective Computing Survey** (Intelligent Computing, 2024) formalized the PAD dimensional model as the standard three-axis framework. For Roko: Pleasure maps to task success trend, Arousal maps to resource utilization intensity, Dominance maps to autonomy level. This gives principled axes for the pair programming display.

### Design

The Pair Programming interface augments the single-agent work view with a persistent Daimon sidebar that shows the agent's affect state, trend, and how it's modulating behavior. The agent's output includes tone markers derived from the PAD vector.

#### Rust Structs

```rust
/// Pair programming session with affect-aware interaction.
#[derive(Debug, Clone)]
pub struct PairProgrammingSession {
    /// The agent being paired with
    pub agent_id: String,
    /// Current task being worked on
    pub task_id: String,
    /// Session turns (interleaved operator + agent)
    pub turns: Vec<PairTurn>,
    /// Live Daimon state (updated each agent turn)
    pub daimon: LiveDaimonState,
    /// Affect history (for trend visualization)
    pub affect_history: VecDeque<AffectSample>,
    /// Maximum history samples
    pub max_history: usize,
    /// Active prompt modulations (how affect changes the prompt)
    pub active_modulations: Vec<PromptModulation>,
    /// Configuration
    pub config: PairConfig,
}

/// The live Daimon state display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveDaimonState {
    /// Current PAD vector
    pub pleasure: f32,
    pub arousal: f32,
    pub dominance: f32,
    /// Current behavioral state label
    pub behavioral_state: String,
    /// Behavioral state color (ROSEDUST)
    pub state_color: [u8; 3],
    /// Somatic marker: the agent's gut-feel about current approach
    pub somatic_signal: SomaticSignal,
    /// Dispatch strategy being used (from behavioral state)
    pub dispatch_strategy: String,
    /// Tone descriptor for the agent's current output style
    pub tone: AffectTone,
}

/// Somatic marker signal — the agent's learned gut reaction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SomaticSignal {
    /// Approach valence: positive=familiar success, negative=familiar failure
    /// Range: -1.0 to +1.0
    pub valence: f32,
    /// Signal strength: how strongly the somatic marker fires
    /// Range: 0.0 (no signal) to 1.0 (strong signal)
    pub strength: f32,
}

/// How the affect state translates to output tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffectTone {
    /// High pleasure, moderate arousal: confident, clear
    Confident,
    /// Low pleasure, high arousal: cautious, hedging
    Cautious,
    /// High arousal, high dominance: assertive, direct
    Assertive,
    /// Low arousal, high pleasure: relaxed, exploratory
    Exploratory,
    /// Low pleasure, low dominance: uncertain, asking for guidance
    Uncertain,
    /// Moderate everything: balanced, neutral
    Neutral,
}

/// A recorded affect sample for trend visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectSample {
    pub timestamp: DateTime<Utc>,
    pub pleasure: f32,
    pub arousal: f32,
    pub dominance: f32,
    pub behavioral_state: String,
    pub turn_index: usize,
}

/// How affect modulates the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptModulation {
    /// What aspect is modulated
    pub aspect: String,
    /// Original value
    pub base_value: String,
    /// Modulated value (affected by Daimon state)
    pub modulated_value: String,
    /// Why (which PAD dimension drove the modulation)
    pub reason: String,
}

/// Pair programming configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairConfig {
    /// Whether to show the affect sidebar
    /// Default: true
    pub show_affect: bool,
    /// Whether to show prompt modulations
    /// Default: false (advanced mode)
    pub show_modulations: bool,
    /// Whether the agent adapts tone based on affect
    /// Default: true
    pub affect_modulated_tone: bool,
    /// Affect history window size
    /// Default: 50 samples
    pub history_window: usize,
}

impl Default for PairConfig {
    fn default() -> Self {
        Self {
            show_affect: true,
            show_modulations: false,
            affect_modulated_tone: true,
            history_window: 50,
        }
    }
}
```

#### TUI Mockup

```
┌─ PAIR PROGRAMMING ─── rust-impl ─── task-3 ── Engaged ──┐
│                                                           │
│  ┌─ AGENT OUTPUT ──────────────────┐ ┌─ DAIMON ────────┐ │
│  │                                 │ │                  │ │
│  │ I've analyzed the dispatcher    │ │  ENGAGED ▓▓▓▓░  │ │
│  │ and identified the retry point. │ │                  │ │
│  │ The `dispatch_agent()` method   │ │  P: ▓▓▓▓░ +0.6  │ │
│  │ in dispatcher/mod.rs L142 is    │ │  A: ▓▓▓░░ +0.4  │ │
│  │ where we need the wrapper.      │ │  D: ▓▓▓▓▓ +0.8  │ │
│  │                                 │ │                  │ │
│  │ [confident] I'm fairly sure     │ │  Somatic: +0.7   │ │
│  │ `backon` is the right choice    │ │  (familiar ✓)    │ │
│  │ here — I've seen similar        │ │                  │ │
│  │ patterns succeed in 8 previous  │ │  Strategy:       │ │
│  │ episodes.                       │ │  Balanced         │ │
│  │                                 │ │                  │ │
│  │ ```rust                         │ │  Tone:           │ │
│  │ use backon::ExponentialBuilder; │ │  Confident       │ │
│  │                                 │ │                  │ │
│  │ pub async fn dispatch_with_     │ │  ── History ──   │ │
│  │   retry(&self, ...) {           │ │  P ▃▅▆▇▇▆▅▆▇   │ │
│  │     backon::retry(|| {          │ │  A ▅▃▃▂▃▅▅▃▃   │ │
│  │       self.dispatch_agent(..)   │ │  D ▆▇▇▇▆▇▇▇▇   │ │
│  │     })                          │ │                  │ │
│  │     .with_max_times(3)          │ │  Modulations:    │ │
│  │     .await                      │ │  context: +20%   │ │
│  │ }                               │ │  (high P → more  │ │
│  │ ```                             │ │   context)       │ │
│  └─────────────────────────────────┘ └──────────────────┘ │
│                                                           │
│  ╭─ INPUT ──────────────────────────────────────────────╮ │
│  │ > Looks right. What about jitter for the backoff?    │ │
│  ╰──────────────────────────────────────────────────────╯ │
│                                                           │
│  PAIR  [Enter] send  [Tab] focus  [a] affect  [q] back   │
└───────────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `DaimonState` (roko-core) | `LiveDaimonState` | PAD vector + behavioral state |
| Somatic marker k-d tree | `SomaticSignal` | 8D strategy space lookup |
| `SystemPromptBuilder` (roko-compose) | `PromptModulation` | Tone/context adjustments |
| `CascadeRouter` (roko-learn) | `dispatch_strategy` | Model selection rationale |
| Episode output stream | `PairTurn` | Agent turn text + gate results |
| Mood-congruent retrieval (roko-neuro) | Knowledge selection bias | PAD → retrieval weighting |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affect_tone_maps_from_pad() {
        assert_eq!(pad_to_tone(0.6, 0.4, 0.8), AffectTone::Confident);
        assert_eq!(pad_to_tone(-0.3, 0.8, 0.3), AffectTone::Cautious);
        assert_eq!(pad_to_tone(-0.5, 0.2, -0.3), AffectTone::Uncertain);
    }

    #[test]
    fn somatic_signal_bounded() {
        let signal = compute_somatic_signal(&strategy_context);
        assert!(signal.valence >= -1.0 && signal.valence <= 1.0);
        assert!(signal.strength >= 0.0 && signal.strength <= 1.0);
    }

    #[test]
    fn affect_history_respects_window_size() {
        let mut session = PairProgrammingSession::new("agent-1", "task-1");
        session.config.history_window = 10;
        for i in 0..20 {
            session.record_affect_sample(AffectSample {
                pleasure: i as f32 * 0.05,
                ..Default::default()
            });
        }
        assert_eq!(session.affect_history.len(), 10);
    }

    #[test]
    fn prompt_modulation_tracks_reason() {
        let pad = (0.8, 0.3, 0.7); // high pleasure
        let modulations = compute_modulations(pad);
        assert!(modulations.iter().any(|m| m.aspect == "context_richness"),
            "High pleasure should increase context richness");
    }
}
```

---

## 6. Collaborative Planning

> Human and agents co-create plans through a mixed-initiative interface — both parties can propose, edit, reorder, and validate tasks in a shared plan workspace.

### Problem Statement

Roko's plan generation (`roko prd plan <slug>`) is agent-driven: the agent reads a PRD and produces a `tasks.toml`. The operator can review the plan but cannot collaboratively edit it. This is a one-directional handoff: PRD → agent → plan → operator review → execute.

Real planning is iterative: the operator knows domain constraints the agent doesn't; the agent knows code-level details the operator might miss. The gap is a co-editing interface where both parties contribute simultaneously, with clear initiative indicators showing who's driving.

### Research Foundations

**Cocoa** (Feng et al., 2024) introduced the mixed-initiative paradigm for human-AI co-planning and co-execution. Their lab study (n=16) found that interleaved co-planning and co-execution improved agent steerability without sacrificing ease-of-use. The computational notebook metaphor maps well to Roko's TOML-based task definitions.

**Human-AI Co-Design Review** (AAAI Symposium, 2025) identified calibrating AI initiative level, maintaining shared context, and supporting transparent rationale as key challenges. These directly inform the design: the TUI must show who proposed each task, why, and allow either party to modify.

**Initiative Levels in Human-AI Collaboration** (DIS 2024) found that dynamic initiative switching — where leadership shifts based on context — produces better outcomes than fixed roles. Using the Co-Creative Framework for Interaction Design (CoFI), the collaborative planning interface supports fluid handoffs when the agent encounters uncertainty (e.g., unfamiliar codebase area).

### Design

The Collaborative Planning interface is a split view: the plan structure (task DAG) on the left, and a conversation + rationale panel on the right. Both the operator and agent can propose, edit, reorder, and delete tasks. Each modification is attributed (human or agent) and optionally annotated with rationale.

#### Rust Structs

```rust
/// A collaborative plan editing session.
#[derive(Debug, Clone)]
pub struct CollaborativePlanSession {
    /// Session identifier
    pub id: String,
    /// The plan being co-edited
    pub plan: CollaborativePlan,
    /// Edit history (for undo/redo)
    pub history: Vec<PlanEdit>,
    /// Current undo cursor
    pub undo_cursor: usize,
    /// Who currently has initiative (drives the next proposal)
    pub initiative: Initiative,
    /// Conversation about the plan
    pub discussion: Vec<PlanDiscussionTurn>,
    /// Validation results from the last check
    pub validation: PlanValidation,
}

/// A plan with per-task attribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaborativePlan {
    /// Plan identifier
    pub plan_id: String,
    /// Source PRD slug
    pub prd_slug: String,
    /// Tasks with attribution
    pub tasks: Vec<CollaborativeTask>,
    /// DAG edges (task_id → Vec<dependency_task_id>)
    pub dependencies: HashMap<String, Vec<String>>,
    /// Plan-level metadata
    pub metadata: PlanMetadata,
}

/// A task with provenance tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaborativeTask {
    /// Task identifier
    pub id: String,
    /// Task description
    pub description: String,
    /// Who proposed this task
    pub proposed_by: Author,
    /// Who last edited this task
    pub last_edited_by: Author,
    /// Rationale for this task's existence
    pub rationale: Option<String>,
    /// Estimated complexity (agent's assessment)
    pub complexity: Option<TaskComplexity>,
    /// Files this task will likely modify (agent's prediction)
    pub predicted_files: Vec<String>,
    /// Whether this task has been approved by the operator
    pub approved: bool,
    /// Agent template to use for execution
    pub agent_template: String,
    /// Task status in the collaborative editor
    pub edit_status: TaskEditStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    /// The human operator
    Human,
    /// The planning agent
    Agent,
    /// System (auto-generated, e.g., dependency inference)
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskComplexity {
    /// Single file, few lines
    Trivial,
    /// Multiple functions, one file
    Simple,
    /// Cross-file changes
    Moderate,
    /// Architectural changes, multiple crates
    Complex,
    /// Research required, unknown scope
    Research,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEditStatus {
    /// Proposed but not yet reviewed
    Proposed,
    /// Under active editing
    Editing,
    /// Approved for execution
    Approved,
    /// Flagged for discussion
    Flagged,
    /// Removed from plan
    Removed,
}

/// Who is driving the current interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Initiative {
    /// Operator is proposing/editing
    Human,
    /// Agent is proposing/editing
    Agent,
    /// Both are reviewing simultaneously
    Shared,
}

/// A modification to the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEdit {
    /// Who made the edit
    pub author: Author,
    /// What changed
    pub operation: PlanOperation,
    /// Optional rationale
    pub rationale: Option<String>,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Operations that can be performed on a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanOperation {
    /// Add a new task
    AddTask { task: CollaborativeTask },
    /// Remove a task by ID
    RemoveTask { task_id: String },
    /// Edit a task's description
    EditTask { task_id: String, field: String, old_value: String, new_value: String },
    /// Add a dependency edge
    AddDependency { from: String, to: String },
    /// Remove a dependency edge
    RemoveDependency { from: String, to: String },
    /// Reorder tasks
    ReorderTasks { task_ids: Vec<String> },
    /// Approve a task
    ApproveTask { task_id: String },
    /// Flag a task for discussion
    FlagTask { task_id: String, reason: String },
}

/// A discussion turn about the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDiscussionTurn {
    pub author: Author,
    pub content: String,
    /// References a specific task (optional)
    pub references_task: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Validation results for the current plan state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanValidation {
    /// Whether the plan is valid (no cycles, all deps resolvable)
    pub is_valid: bool,
    /// Warnings (non-blocking issues)
    pub warnings: Vec<ValidationWarning>,
    /// Errors (must fix before execution)
    pub errors: Vec<ValidationError>,
    /// Estimated total execution time
    pub estimated_duration: Option<Duration>,
    /// Estimated total cost
    pub estimated_cost: Option<f64>,
    /// Critical path (longest dependency chain)
    pub critical_path: Vec<String>,
}

/// A validation warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationWarning {
    pub task_id: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

/// A validation error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub task_id: Option<String>,
    pub message: String,
}
```

#### TUI Mockup

```
┌─ COLLABORATIVE PLANNER ─── retry-logic ─── AGENT initiative ──┐
│                                                                 │
│  ┌─ PLAN DAG ─────────────────────┐ ┌─ DISCUSSION ──────────┐ │
│  │                                 │ │                        │ │
│  │  [1] Add backon dependency      │ │ AGENT: I propose 4    │ │
│  │      ├─ proposed: agent         │ │ tasks based on the     │ │
│  │      ├─ complexity: trivial     │ │ PRD. Task 1 and 2 can  │ │
│  │      ├─ files: Cargo.toml       │ │ run in parallel.       │ │
│  │      └─ status: ✓ approved      │ │                        │ │
│  │      │                          │ │ YOU: Split task 2 into │ │
│  │  [2] Wire retry into dispatch   │ │ "wire retry" and "add  │ │
│  │      ├─ proposed: agent ✎human  │ │ error classification"  │ │
│  │      ├─ complexity: moderate    │ │                        │ │
│  │      ├─ files: dispatcher/      │ │ AGENT: Good call.      │ │
│  │      │   mod.rs                 │ │ Updated. The error     │ │
│  │      └─ status: ✓ approved      │ │ classifier should run  │ │
│  │      │                          │ │ before the retry       │ │
│  │  [3] Error classification  NEW  │ │ wrapper (added dep).   │ │
│  │      ├─ proposed: human         │ │                        │ │
│  │      ├─ complexity: simple      │ │ SYS: Validation ✓     │ │
│  │      ├─ files: dispatcher/      │ │ No cycles. Est: 22min  │ │
│  │      │   errors.rs              │ │ Critical path: 1→3→2→4 │ │
│  │      └─ status: ○ proposed      │ │                        │ │
│  │      │                          │ │                        │ │
│  │  [4] Integration tests          │ │                        │ │
│  │      ├─ proposed: agent         │ │                        │ │
│  │      ├─ depends: [2, 3]         │ │                        │ │
│  │      └─ status: ○ proposed      │ │                        │ │
│  │                                 │ │                        │ │
│  │  ── DAG ──                      │ │                        │ │
│  │  [1] ──┬──▶ [2] ──▶ [4]        │ │                        │ │
│  │        └──▶ [3] ──┘             │ │                        │ │
│  └─────────────────────────────────┘ └────────────────────────┘ │
│                                                                 │
│  [a] add task  [e] edit  [d] delete  [Enter] approve           │
│  [↑↓] navigate  [Tab] switch pane  [x] execute  [q] back      │
└─────────────────────────────────────────────────────────────────┘
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `roko prd plan <slug>` | `CollaborativePlan` | Initial plan generation |
| Operator edits (TUI) | `PlanEdit` with `Author::Human` | Keyboard-driven task CRUD |
| Agent proposals (via dispatcher) | `PlanEdit` with `Author::Agent` | Agent suggests tasks |
| `UnifiedTaskDag` (roko-orchestrator) | `dependencies`, `critical_path` | DAG validation |
| `PlanStateMachine` (roko-orchestrator) | `PlanValidation` | Cycle detection, file conflicts |
| `CollaborativePlan.tasks` → `tasks.toml` | `plan run` | Export to standard plan format |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_edit_is_undoable() {
        let mut session = CollaborativePlanSession::new("test-plan");
        session.apply_edit(PlanEdit {
            author: Author::Human,
            operation: PlanOperation::AddTask { task: make_task("task-1") },
            rationale: None,
            timestamp: Utc::now(),
        });
        assert_eq!(session.plan.tasks.len(), 1);
        session.undo();
        assert_eq!(session.plan.tasks.len(), 0);
    }

    #[test]
    fn circular_dependency_detected() {
        let mut plan = CollaborativePlan::new("test");
        plan.add_task(make_task("a"));
        plan.add_task(make_task("b"));
        plan.dependencies.insert("a".into(), vec!["b".into()]);
        plan.dependencies.insert("b".into(), vec!["a".into()]);
        let validation = plan.validate();
        assert!(!validation.is_valid);
        assert!(validation.errors.iter().any(|e| e.message.contains("cycle")));
    }

    #[test]
    fn task_attribution_preserved() {
        let task = CollaborativeTask {
            proposed_by: Author::Agent,
            last_edited_by: Author::Human,
            ..make_task("task-1")
        };
        assert_eq!(task.proposed_by, Author::Agent);
        assert_eq!(task.last_edited_by, Author::Human);
    }

    #[test]
    fn critical_path_computed() {
        let plan = make_plan_with_deps();
        let validation = plan.validate();
        assert!(validation.critical_path.len() > 0,
            "Should compute critical path");
    }

    #[test]
    fn initiative_shifts_on_agent_uncertainty() {
        let mut session = CollaborativePlanSession::new("test");
        session.initiative = Initiative::Agent;
        // Agent flags a task as needing human input
        session.apply_edit(PlanEdit {
            author: Author::Agent,
            operation: PlanOperation::FlagTask {
                task_id: "task-2".into(),
                reason: "Unsure about error taxonomy".into(),
            },
            ..Default::default()
        });
        // Initiative should shift to human
        assert_eq!(session.initiative, Initiative::Human);
    }
}
```

---

## 7. Knowledge Map

> Interactive visualization of the entire Neuro knowledge base — a spatial map where knowledge entries cluster by topic, connections show causal links, and HDC similarity determines proximity.

### Problem Statement

Roko's Neuro subsystem accumulates knowledge across four tiers (Transient → Working → Consolidated → Persistent) and seven kinds (Fact, Insight, Procedure, Heuristic, Playbook, Constraint, AntiKnowledge). As the knowledge base grows past 100+ entries, the flat `knowledge.jsonl` file becomes impossible to navigate. Operators cannot answer:

- "What does the system know about error handling?"
- "Which insights have been promoted to persistent tier?"
- "Are there contradictions between knowledge entries?"
- "How is knowledge distributed across domains?"

The Knowledge Map renders the Neuro store as an interactive spatial visualization, using HDC similarity scores to position related knowledge near each other.

### Research Foundations

**Explaining HDC Classifiers** (Smets et al., Neurocomputing, 2025) demonstrated the first explanation and interpretation methods for hyperdimensional computing models. HDC's reversible operations (Bind=XOR, Bundle=majority, Permute=shift) enable decomposition that reveals which components drive classification. For Roko, this means HDC fingerprints on knowledge entries can be decomposed to reveal *why* two entries are similar.

**Interactive Knowledge Graph Visualization** (ASIS&T, 2025) developed a WebGL visualization for 28,000-entity knowledge graphs using hierarchical clustering with expand/collapse navigation. The yFiles NodeAggregation algorithm reduces visual complexity to manageable levels. For Roko, the tier system provides a natural hierarchy: start with tier clusters, drill into individual entries.

**HDC: A Fast, Robust, and Interpretable Paradigm** (PLOS Computational Biology, 2024) positioned HDC as intrinsically more interpretable than deep neural networks. Similarity matching between hypervectors reveals essential components — enabling a "semantic map" where related knowledge clusters visually. This is the theoretical basis for why Roko's 10,240-bit BSC fingerprints can power a spatial visualization.

### Design

The Knowledge Map is a Braille-canvas visualization (high-resolution terminal rendering) where each knowledge entry is a point positioned by HDC similarity. Clusters emerge naturally from the similarity space. The operator can zoom, filter by tier/kind/tag, and inspect individual entries.

#### Rust Structs

```rust
/// The Knowledge Map: a spatial visualization of the Neuro knowledge base.
#[derive(Debug, Clone)]
pub struct KnowledgeMap {
    /// All knowledge nodes with computed positions
    pub nodes: Vec<KnowledgeNode>,
    /// Edges between related nodes (HDC similarity above threshold)
    pub edges: Vec<KnowledgeEdge>,
    /// Cluster assignments (from HDC k-medoids)
    pub clusters: Vec<KnowledgeCluster>,
    /// Current viewport
    pub viewport: MapViewport,
    /// Active filter
    pub filter: KnowledgeMapFilter,
    /// Selected node (for detail panel)
    pub selected: Option<usize>,
    /// Search query (for highlight)
    pub search: Option<String>,
    /// Layout algorithm used
    pub layout: MapLayout,
}

/// A knowledge entry positioned in the map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNode {
    /// The underlying knowledge entry
    pub entry: KnowledgeEntry,
    /// Position in map space (computed from HDC similarity)
    pub position: [f32; 2],
    /// Node size (proportional to confidence × tier weight)
    pub size: f32,
    /// Color (determined by KnowledgeKind)
    pub color: [u8; 3],
    /// Whether this node is highlighted by current search
    pub highlighted: bool,
    /// Cluster this node belongs to
    pub cluster_id: Option<usize>,
    /// Decay-adjusted confidence (current effective value)
    pub current_confidence: f64,
}

/// An edge between two knowledge entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEdge {
    /// Source node index
    pub from: usize,
    /// Target node index
    pub to: usize,
    /// Relationship type
    pub kind: KnowledgeRelation,
    /// Strength (0.0–1.0, from HDC similarity or explicit link)
    pub strength: f32,
}

/// Types of relationships between knowledge entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeRelation {
    /// High HDC similarity (cosine/Hamming distance below threshold)
    HdcSimilar,
    /// Explicit causal link (CausalLink kind)
    CausalLink,
    /// Contradiction (AntiKnowledge refutes an Insight)
    Contradiction,
    /// Shared source episodes
    SharedProvenance,
    /// Same topic tags
    TopicOverlap,
}

/// A cluster of related knowledge entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCluster {
    /// Cluster identifier
    pub id: usize,
    /// Centroid position in map space
    pub centroid: [f32; 2],
    /// Inferred topic label (from most common tags)
    pub label: String,
    /// Node indices in this cluster
    pub members: Vec<usize>,
    /// Tier distribution within the cluster
    pub tier_distribution: TierDistribution,
    /// Kind distribution
    pub kind_distribution: HashMap<String, usize>,
}

/// Distribution of knowledge tiers within a cluster.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierDistribution {
    pub transient: usize,
    pub working: usize,
    pub consolidated: usize,
    pub persistent: usize,
}

/// Filter for the knowledge map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeMapFilter {
    /// Show only entries of these kinds
    pub kinds: Option<Vec<String>>,
    /// Show only entries at these tiers
    pub tiers: Option<Vec<String>>,
    /// Show only entries with these tags
    pub tags: Option<Vec<String>>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f64>,
    /// Text search query
    pub text_search: Option<String>,
}

/// Viewport for map navigation.
#[derive(Debug, Clone)]
pub struct MapViewport {
    /// Zoom level (1.0 = fit all, higher = closer)
    pub zoom: f32,
    /// Center offset for panning
    pub center: [f32; 2],
    /// Rendering area (terminal coordinates)
    pub area: (u16, u16, u16, u16),
    /// Whether to use Braille markers (high-res) or Block markers
    pub marker_type: MapMarker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMarker {
    /// 2×4 sub-pixel Braille characters (highest resolution)
    Braille,
    /// 2×2 quadrant characters
    Quadrant,
    /// 1×1 block characters (lowest resolution, widest support)
    Block,
}

/// Layout algorithm for positioning nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapLayout {
    /// Force-directed layout from HDC similarity (spring forces)
    ForceDirected,
    /// t-SNE dimensionality reduction on HDC vectors
    Tsne,
    /// UMAP dimensionality reduction on HDC vectors
    Umap,
    /// Grid layout by tier × kind
    Grid,
}

/// Parameters for the force-directed layout.
#[derive(Debug, Clone)]
pub struct ForceDirectedParams {
    /// Repulsion constant (Coulomb's law between all nodes)
    /// Default: 100.0
    pub repulsion: f32,
    /// Attraction constant (Hooke's law along edges)
    /// Default: 0.01
    pub attraction: f32,
    /// Damping factor (velocity reduction per step)
    /// Default: 0.9
    pub damping: f32,
    /// Maximum iterations before convergence
    /// Default: 500
    pub max_iterations: u32,
    /// Convergence threshold (max velocity)
    /// Default: 0.01
    pub convergence_threshold: f32,
}

impl Default for ForceDirectedParams {
    fn default() -> Self {
        Self {
            repulsion: 100.0,
            attraction: 0.01,
            damping: 0.9,
            max_iterations: 500,
            convergence_threshold: 0.01,
        }
    }
}
```

#### Color Encoding

```
Knowledge Kind → ROSEDUST Color:
  Fact           → fg (#E8DFD5)        — neutral, foundational
  Insight        → rose (#D4778C)      — primary discovery
  Procedure      → teal (#5DB8A3)      — actionable steps
  Heuristic      → gold (#D4A857)      — learned rules
  Playbook       → lavender (#A08CC4)  — compiled strategies
  Constraint     → coral (#C47A5C)     — boundaries
  AntiKnowledge  → danger (#C45C50)    — what to avoid

Tier → Node Size:
  Transient    → 1x (smallest)
  Working      → 1.5x
  Consolidated → 2x
  Persistent   → 3x (largest)

Confidence → Opacity:
  1.0 → full brightness
  0.5 → 50% dimmed
  0.1 → barely visible (about to be GC'd)
```

#### TUI Mockup

```
┌─ KNOWLEDGE MAP ──── 142 entries ── 6 clusters ──────────┐
│                                                          │
│  ┌─ MAP (Braille canvas) ─────────────────────────────┐ │
│  │                                                     │ │
│  │        ·  ·                    ◉                    │ │
│  │      · ·◉· ·              ◉  ◉ ◉                   │ │
│  │     · ·  · · ·          ·  ◉  · ·                  │ │
│  │      · ·· ·              · ·◉·                     │ │
│  │        ·                    ·                       │ │
│  │   [error handling]      [retry logic]              │ │
│  │                                                     │ │
│  │                  ·· ·                               │ │
│  │               · ◉·◉·· ·         ◈  ◈              │ │
│  │                ·  ◉· ·          ◈ ◈                │ │
│  │               · ··  ·                              │ │
│  │          [model routing]     [anti-patterns]        │ │
│  │                                     ◈ = danger     │ │
│  │     · ·                                            │ │
│  │    · ◉ ·          ○ ○                              │ │
│  │     · ·          ○  ○                              │ │
│  │  [gate tuning]  [procedures]                       │ │
│  │                                                     │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌─ SELECTED: Insight ──────────────────────────────┐    │
│  │ "Provider rate limits cluster at 14:00-16:00 UTC" │    │
│  │ Tier: Working  Confidence: 0.78  Sources: 12 eps  │    │
│  │ Tags: [providers, scheduling, rate-limits]         │    │
│  │ Created: Apr 13  Half-life: 30d  Decayed: 0.76    │    │
│  │ Similar: "Schedule heavy work before 14:00" (0.87) │    │
│  │ Similar: "Anthropic rate limits stricter" (0.72)   │    │
│  └────────────────────────────────────────────────────┘    │
│                                                          │
│  ┌─ LEGEND ────────────────────────────────────────────┐ │
│  │ ◉ Insight  ○ Procedure  ◈ AntiKnowledge  · Fact    │ │
│  │ Size = tier  Brightness = confidence                │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                          │
│  [←→↑↓] pan  [+/-] zoom  [/] search  [f] filter         │
│  [t] tier filter  [k] kind filter  [Enter] detail  [q]   │
└──────────────────────────────────────────────────────────┘
```

#### HDC Similarity Layout Algorithm

```rust
/// Compute 2D positions for knowledge nodes from their HDC fingerprints.
///
/// Uses a force-directed layout where:
/// - Repulsive force between all pairs (Coulomb)
/// - Attractive force between pairs with HDC similarity > threshold (Spring)
/// - The resulting equilibrium positions cluster similar knowledge together
pub fn layout_from_hdc(
    entries: &[KnowledgeEntry],
    params: &ForceDirectedParams,
    similarity_threshold: f64,
) -> Vec<[f32; 2]> {
    let n = entries.len();

    // Initialize positions randomly (seeded from first entry's HDC vector)
    let mut positions: Vec<[f32; 2]> = entries.iter().enumerate().map(|(i, _)| {
        let angle = (i as f32 / n as f32) * std::f32::consts::TAU;
        let radius = 0.3 + 0.2 * ((i * 7) % 13) as f32 / 13.0;
        [radius * angle.cos(), radius * angle.sin()]
    }).collect();

    let mut velocities = vec![[0.0f32; 2]; n];

    // Precompute HDC similarities for pairs above threshold
    let edges: Vec<(usize, usize, f64)> = entries.iter().enumerate()
        .flat_map(|(i, a)| {
            entries.iter().enumerate().skip(i + 1).filter_map(move |(j, b)| {
                let sim = hdc_similarity(&a.hdc_vector, &b.hdc_vector);
                if sim > similarity_threshold { Some((i, j, sim)) } else { None }
            })
        })
        .collect();

    for _iteration in 0..params.max_iterations {
        let mut forces = vec![[0.0f32; 2]; n];

        // Repulsive forces (all pairs)
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = positions[i][0] - positions[j][0];
                let dy = positions[i][1] - positions[j][1];
                let dist_sq = (dx * dx + dy * dy).max(0.0001);
                let force = params.repulsion / dist_sq;
                let fx = force * dx / dist_sq.sqrt();
                let fy = force * dy / dist_sq.sqrt();
                forces[i][0] += fx;
                forces[i][1] += fy;
                forces[j][0] -= fx;
                forces[j][1] -= fy;
            }
        }

        // Attractive forces (similar pairs)
        for &(i, j, sim) in &edges {
            let dx = positions[j][0] - positions[i][0];
            let dy = positions[j][1] - positions[i][1];
            let dist = (dx * dx + dy * dy).sqrt().max(0.0001);
            let force = params.attraction * dist * sim as f32;
            forces[i][0] += force * dx / dist;
            forces[i][1] += force * dy / dist;
            forces[j][0] -= force * dx / dist;
            forces[j][1] -= force * dy / dist;
        }

        // Apply forces with damping
        let mut max_velocity = 0.0f32;
        for i in 0..n {
            velocities[i][0] = (velocities[i][0] + forces[i][0]) * params.damping;
            velocities[i][1] = (velocities[i][1] + forces[i][1]) * params.damping;
            positions[i][0] += velocities[i][0];
            positions[i][1] += velocities[i][1];
            let v = (velocities[i][0].powi(2) + velocities[i][1].powi(2)).sqrt();
            max_velocity = max_velocity.max(v);
        }

        if max_velocity < params.convergence_threshold {
            break;
        }
    }

    // Normalize to [0, 1] range
    normalize_positions(&mut positions);
    positions
}

/// Compute HDC similarity between two fingerprints.
/// Uses normalized Hamming distance for BSC vectors (10,240 bits).
///
/// Returns: similarity in [0.0, 1.0] where 1.0 = identical.
fn hdc_similarity(a: &Option<Vec<u8>>, b: &Option<Vec<u8>>) -> f64 {
    match (a, b) {
        (Some(va), Some(vb)) if va.len() == vb.len() => {
            let total_bits = va.len() * 8;
            let hamming: u32 = va.iter().zip(vb.iter())
                .map(|(a, b)| (a ^ b).count_ones())
                .sum();
            1.0 - (hamming as f64 / total_bits as f64)
        }
        _ => 0.0,
    }
}
```

### Integration Wiring

| From | To | Mechanism |
|---|---|---|
| `KnowledgeStore` (roko-neuro) | `KnowledgeNode` | Read `.roko/neuro/knowledge.jsonl` |
| HDC fingerprints (bardo-primitives) | `layout_from_hdc` | Position computation |
| `KnowledgeKind` | Color encoding | Kind → ROSEDUST color |
| Tier (Neuro) | Node size | Tier weight → visual size |
| Ebbinghaus decay (roko-neuro) | `current_confidence` | Time-adjusted confidence |
| `KnowledgeConfirmationRecord` | `KnowledgeEdge::SharedProvenance` | Cross-entry confirmation |
| `DreamClusterReport` | Cluster labels | Dream clusters annotate topics |

### Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdc_similarity_is_symmetric() {
        let a = Some(vec![0b10110101; 1280]); // 10,240 bits
        let b = Some(vec![0b10100101; 1280]);
        assert_eq!(hdc_similarity(&a, &b), hdc_similarity(&b, &a));
    }

    #[test]
    fn identical_vectors_have_similarity_one() {
        let a = Some(vec![0b11001100; 1280]);
        assert!((hdc_similarity(&a, &a.clone()) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn force_directed_converges() {
        let entries = make_test_entries(20);
        let params = ForceDirectedParams::default();
        let positions = layout_from_hdc(&entries, &params, 0.526);
        // All positions should be in [0, 1] range after normalization
        for pos in &positions {
            assert!(pos[0] >= 0.0 && pos[0] <= 1.0);
            assert!(pos[1] >= 0.0 && pos[1] <= 1.0);
        }
    }

    #[test]
    fn similar_entries_cluster_together() {
        let mut entries = make_test_entries(10);
        // Make entries 0-4 similar to each other
        let shared_vec = vec![0b11110000; 1280];
        for i in 0..5 {
            entries[i].hdc_vector = Some(shared_vec.clone());
        }
        let positions = layout_from_hdc(&entries, &ForceDirectedParams::default(), 0.526);
        let cluster_diameter = max_distance(&positions[0..5]);
        let cross_distance = min_distance_between(&positions[0..5], &positions[5..10]);
        assert!(cluster_diameter < cross_distance,
            "Similar entries should be closer than dissimilar ones");
    }

    #[test]
    fn filter_by_kind_reduces_visible_nodes() {
        let map = make_test_map(50);
        let filtered = map.apply_filter(&KnowledgeMapFilter {
            kinds: Some(vec!["insight".into()]),
            ..Default::default()
        });
        assert!(filtered.visible_nodes().count() < 50);
        assert!(filtered.visible_nodes().all(|n| n.entry.kind == KnowledgeKind::Insight));
    }

    #[test]
    fn contradiction_edges_connect_anti_knowledge() {
        let entries = vec![
            make_entry(KnowledgeKind::Insight, "Always use haiku for simple tasks"),
            make_entry(KnowledgeKind::AntiKnowledge, "Haiku fails on refactoring tasks"),
        ];
        let edges = compute_edges(&entries);
        assert!(edges.iter().any(|e| e.kind == KnowledgeRelation::Contradiction));
    }

    #[test]
    fn node_size_proportional_to_tier() {
        let transient = node_size(KnowledgeTier::Transient, 0.8);
        let persistent = node_size(KnowledgeTier::Persistent, 0.8);
        assert!(persistent > transient, "Persistent nodes should be larger");
    }
}
```

---

## Implementation Priority

| Innovation | Complexity | Value | Dependencies | Priority |
|---|---|---|---|---|
| **Conversational Development** | High | Very High | PRD lifecycle, meta-agent | P1 — unlocks non-expert usage |
| **Dream Journal Interface** | Low | High | `DreamCycleReport` (exists) | P1 — data exists, just needs rendering |
| **Time-Travel Debugging** | Medium | High | Episode log (exists), EventLog | P2 — observability gap is painful |
| **Collaborative Planning** | Medium | High | Plan generation, UnifiedTaskDag | P2 — improves plan quality |
| **Knowledge Map** | Medium | Medium | KnowledgeStore, HDC vectors | P3 — value scales with knowledge size |
| **Agent Garden** | High | Medium | Spectre system, Mesh | P3 — visual polish, ecosystem view |
| **Pair Programming with Affect** | Low | Medium | DaimonState (exists) | P3 — requires Daimon to be fully wired |

---

## Shared Infrastructure

All seven innovations share these infrastructure components:

### Engram Persistence

Every innovation produces Engrams:
- `ConversationTurn` → Engram with `kind: ConversationTurn`
- `TimelineEntry` → Engram with `kind: DebugTrace`
- `DreamJournalEntry` → Engram with `kind: DreamJournal`
- `PlanEdit` → Engram with `kind: PlanEdit`
- `KnowledgeNode` (position) → Engram with `kind: MapLayout`

### A2UI Integration

Innovations 1 (Conversational Development) and 5 (Pair Programming) emit A2UI components within their output streams, using the existing [A2UI protocol](./15-generative-interfaces-a2ui.md):

```jsonl
{"a2ui": "status", "items": [{"label": "IDEATION", "state": "pass"}, {"label": "DRAFTING", "state": "pass"}, {"label": "PLANNING", "state": "pending"}]}
{"a2ui": "kv", "items": [{"key": "Pleasure", "value": "+0.6"}, {"key": "Arousal", "value": "+0.4"}, {"key": "Dominance", "value": "+0.8"}]}
```

### ROSEDUST Theme Extension

New color assignments for innovation-specific elements (within existing palette):

| Element | Color | Existing ROSEDUST Name |
|---|---|---|
| Operator turns | fg (#E8DFD5) | Primary foreground |
| Agent turns | rose (#D4778C) | Primary accent |
| System messages | muted (#8A7F8E) | Muted foreground |
| Divergence points | danger (#C45C50) | Danger red |
| Dream headlines | lavender (#A08CC4) | Lavender accent |
| Approved tasks | teal (#5DB8A3) | Teal/success |
| Flagged tasks | gold (#D4A857) | Gold/warning |

---

## Cross-References

- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the base TUI architecture these innovations extend
- See [09-tui-29-screens.md](./09-tui-29-screens.md) for the screen inventory (innovations add new screens)
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for the Spectre system (used by Agent Garden)
- See [12-spectre-as-collective-display.md](./12-spectre-as-collective-display.md) for collective visualization (extended by Agent Garden)
- See [15-generative-interfaces-a2ui.md](./15-generative-interfaces-a2ui.md) for the A2UI protocol (used by Conversational Dev and Pair Programming)
- See topic [09-daimon](../09-daimon/INDEX.md) for the PAD vector and behavioral states (used by Pair Programming and Agent Garden)
- See topic [10-dreams](../10-dreams/INDEX.md) for the Dreams consolidation system (used by Dream Journal)
- See topic [06-neuro](../06-neuro/INDEX.md) for the knowledge store and HDC encoding (used by Knowledge Map)
- See topic [05-learning](../05-learning/INDEX.md) for the episode logger and CascadeRouter (used by Time-Travel Debugging)
- See topic [01-orchestration](../01-orchestration/INDEX.md) for the plan DAG and parallel executor (used by Collaborative Planning)
- See topic [11-safety](../11-safety/INDEX.md) for the Witness DAG (used by Time-Travel Debugging)

---

## Academic References

### Conversational Development
- Qian, C. et al. (2024). "ChatDev: Communicative Agents for Software Development." *ACL 2024*. ACL Anthology.
- Yang, J. et al. (2024). "SWE-agent: Agent-Computer Interfaces Enable Automated Software Engineering." *NeurIPS 2024*. arXiv:2405.15793.
- Xu, S. et al. (2024). "CoRE: LLM as Interpreter for Natural Language Programming." arXiv:2405.06907.

### Time-Travel Debugging
- Moshkovich, D. et al. (2025). "Beyond Black-Box Benchmarking: Observability, Analytics, and Optimization of Agentic Systems." arXiv:2503.06745.
- E-MARS+ (2025). "Hindsight is 20/20: Building Agent Memory that Retains, Recalls, and Reflects." arXiv:2512.12818.
- AgentOps (2024–2025). Session replay for autonomous agents. Commercial platform.

### Dream Journal Interface
- Tutuncuoglu, B.T. (2024). "NeuroDream: A Sleep-Inspired Memory Consolidation Framework for Artificial Neural Networks." SSRN 5377250.
- Maytie, L. et al. (2025). "Multimodal Dreaming: A Global Workspace Approach to World Model-Based Reinforcement Learning." arXiv:2502.21142.
- Wenninghoff, N. & Schwammberger, M. (2025). "Interpretable World Model Imaginations as Deep Reinforcement Learning Explanation." *xAI 2025*, Springer CCIS vol. 2578.

### Agent Garden
- Park, J.S. et al. (2023). "Generative Agents: Interactive Simulacra of Human Behavior." *UIST '23*. arXiv:2304.03442.
- Xing, E. et al. (2024). "AIDO: Toward AI-Driven Digital Organism." *NeurIPS 2024*. arXiv:2412.06993.
- EvoAgentX (2025). "A Comprehensive Survey of Self-Evolving AI Agents." arXiv:2508.07407.

### Pair Programming with Affect
- Empathetic Conversational Agents (2025). "Utilizing Neural and Physiological Signals for Enhanced Empathetic Interactions." *IJHCI*. Taylor & Francis.
- Wang, P. et al. (2025). "RLVER: Reinforcement Learning with Verifiable Emotion Rewards for Empathetic Agents." arXiv:2507.03112. Tencent.
- Affective Computing Survey (2024). "Recent Advances, Challenges, and Future Trends." *Intelligent Computing*. Science Partner Journal.

### Collaborative Planning
- Feng, K.J.K. et al. (2024). "Cocoa: Co-Planning and Co-Execution with AI Agents." arXiv:2412.10999.
- AAAI Symposium (2025). "Human-AI Co-Design and Co-Creation: A Review of Emerging Approaches, Challenges, and Future Directions." *AAAI Symposium Series*.
- DIS 2024. "When Should I Lead or Follow: Understanding Initiative Levels in Human-AI Collaborative Gameplay." *ACM DIS 2024*.

### Knowledge Map
- Smets, L. et al. (2025). "Explaining and Interpreting Hyperdimensional Computing Classifiers on Tabular Data." *Neurocomputing*. Elsevier.
- ASIS&T (2025). "Interactive Graph Visualization and Teaming Recommendation in an Interdisciplinary Project's Talent Knowledge Graph." arXiv:2508.19489.
- HDC for Biology (2024). "Hyperdimensional Computing: A Fast, Robust, and Interpretable Paradigm for Biological Data." *PLOS Computational Biology*.
