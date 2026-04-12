# 08 — Harness Engineering

> Sub-doc 08 of **02-agents** · Roko Documentation
>
> This document covers the Meta-Harness research (Lee et al., 2026), the
> evidence for harness quality as the dominant factor in agent performance,
> the six harness principles, and how they map to Roko's implementation.
> Nuance: the "6× gap" cited below refers to ref [46] from the Meta-Harness
> paper (SWE-bench mobile), not to a general claim about all agent tasks.

---

## The Meta-Harness Thesis

The central finding of harness engineering research is that the **harness** —
the scaffolding around an LLM (prompts, tools, context management, retry
logic) — contributes more to agent performance than the model itself. This
is counter-intuitive: most effort goes into model improvement, but the
evidence shows that a better harness on a weaker model often outperforms a
worse harness on a stronger model.

The key paper is:

> Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM
> Agents." arXiv:2603.28052.

Their findings across multiple benchmarks:

| Benchmark | Harness improvement | Notes |
|---|---|---|
| Text classification | **+7.7 accuracy points** | Same model, better harness |
| IMO math problems | **+4.7 points** | Structured tool access + validation |
| Token efficiency | **4× fewer tokens** | Context pruning + right-sized prompts |
| SWE-bench mobile | **6× performance gap** | ref [46]; harness vs. no harness |

### The nuance on "6×"

The "6× gap" number comes from reference [46] in the Meta-Harness paper,
which is a SWE-bench mobile benchmark. It measures the performance difference
between a bare model (no harness) and the same model with a full harness
(tools, file access, test execution, context management). This is a
**specific benchmark result**, not a general claim about all agent tasks.
The +7.7 and +4.7 numbers from text classification and math are more
representative of typical harness impact.

The practical takeaway: harness quality is consistently the largest lever
for agent performance, but the exact magnitude varies by task type.

---

## Six Harness Principles

The Meta-Harness paper identifies six principles for effective agent harnesses.
Here is how each maps to Roko's implementation:

### 1. Design Tools for the Model, Not for Humans

**Principle:** LLMs use tools differently than humans. Tool interfaces should
be optimized for how models reason — structured JSON schemas, unambiguous
parameter names, clear error messages that help the model self-correct.

**Roko implementation:** The `ToolDef` struct in `roko-core::tool` carries
a JSON schema that the `ToolDispatcher` validates against (step 1). The
`Translator` layer ensures each model gets tools in its preferred wire format
(see sub-doc 10, Format Translation). The `RenderedTools` enum allows
different representations:
- `JsonArray` for OpenAI-compatible models
- `CliFlag` for Claude CLI
- `SystemPromptBlock` for ReAct models without native tool support

### 2. Provide the Right Context, Not More Context

**Principle:** More context does not always help. Models perform better with
focused, relevant context than with entire files dumped into the prompt.

**Roko implementation:** The `SystemPromptBuilder` in `roko-compose`
constructs 6-layer prompts where each layer provides targeted context:
- Layer 0: Global (project name, structure)
- Layer 1: Task (specific task description, dependencies)
- Layer 2: Role (role-specific constraints)
- Layer 3: Tools (only the tools available to this role)
- Layer 4: History (relevant previous outputs, not all history)
- Layer 5: Meta (budget, time constraints)

The `prune` submodule in the ToolLoop enforces context-growth guards that
drop oldest tool results when the conversation approaches the context limit.

### 3. Validate Before Executing

**Principle:** Check tool call arguments before running them. Invalid
arguments waste tokens and risk side effects.

**Roko implementation:** The `ToolDispatcher`'s 7-step pipeline validates
args against JSON schema (step 1), checks permissions (step 4), and runs
SafetyLayer policies (step 5) — all before handler execution (step 6). This
"validate first, execute later" pattern catches:
- Schema violations (missing required fields)
- Permission denials (role doesn't have write access)
- Safety violations (destructive bash commands, worktree escapes)
- Rate limit breaches

### 4. Compress History Intelligently

**Principle:** Long conversations degrade model performance. Compress old
context while preserving recent and important messages.

**Roko implementation:** The `prune` submodule estimates token usage from
message byte length and drops the oldest tool results when the total exceeds
the configured limit. The strategy preserves:
- System prompt (always)
- First user message (always)
- Most recent N messages (tail window)
- Tool results with errors (diagnostic value)

The `Checkpoint` struct (§36.57) saves the full conversation state for
resumption, so even aggressive pruning doesn't lose data permanently.

### 5. Graduate Autonomy Based on Confidence

**Principle:** Don't give agents full autonomy from the start. Start with
constrained permissions, escalate as confidence grows.

**Roko implementation:** The role-based permission system (sub-doc 04)
implements graduated autonomy:
- `Validator` roles: read-only (can check but not modify)
- `Reviewer` roles: read-only (can comment but not change)
- `Implementer` roles: read + write + exec (full autonomy)
- `Conductor` roles: read-only (can orchestrate but not implement)

The `SafetyLayer` provides a floor that even high-autonomy roles cannot
breach: destructive bash commands are blocked regardless of permissions.

The CascadeRouter (sub-doc 12) implements model-level autonomy graduation:
tasks start at the cheapest model tier and escalate only when confidence
is low, using Thompson sampling over weighted signals.

### 6. Close the Feedback Loop

**Principle:** Agent performance improves when results feed back into future
decisions. Record what worked, what failed, and why.

**Roko implementation:** Four feedback mechanisms are wired:
1. **EpisodeLogger** — Records every agent turn + gate result to
   `.roko/episodes.jsonl`.
2. **Efficiency events** — Per-turn token/cost/time metrics to
   `.roko/learn/efficiency.jsonl`.
3. **CascadeRouter persistence** — Model routing decisions and outcomes to
   `.roko/learn/cascade-router.json`.
4. **Adaptive gate thresholds** — EMA per rung, adjusting pass criteria
   based on recent outcomes, to `.roko/learn/gate-thresholds.json`.

---

## Applying Meta-Harness to Roko

### Where Roko implements Meta-Harness principles well

1. **Tool validation pipeline** — The 7-step ToolDispatcher is exactly the
   "validate before executing" principle, implemented with audit signals for
   observability.

2. **Format-aware translation** — The Translator layer ensures each model
   gets tools in its preferred wire format, following the "tools for the
   model" principle.

3. **6-layer prompt construction** — The SystemPromptBuilder provides
   targeted, role-appropriate context rather than dumping everything.

4. **Feedback loop wiring** — Episode logging, efficiency tracking, and
   adaptive thresholds form a complete feedback loop.

### Where gaps remain

1. **ToolDispatcher not called from orchestrate.rs** — The #1 integration
   gap means the safety pipeline is bypassed for the primary execution path
   (Claude CLI). Claude CLI has its own safety, but Roko's safety policies
   are not applied.

2. **Role prompts are minimal** — The current role prompt templates are
   approximately 1 sentence each, versus Mori's ~2K-token role prompts that
   carried detailed behavioral instructions. This gap means agents don't get
   the nuanced persona guidance that Meta-Harness principle #1 calls for.

3. **Context pruning is basic** — The current prune strategy is byte-based
   rather than semantic. A smarter approach would preserve messages referenced
   by recent tool calls and drop messages about completed sub-tasks.

4. **No iterative refinement** — When a gate rejects an agent's output, the
   orchestrator currently marks the task as failed. Meta-Harness principle #6
   calls for feeding the gate feedback back into the agent for a retry with
   the specific failure reason.

---

## SWE-bench Context

The Meta-Harness paper draws heavily on SWE-bench (Jimenez et al., 2024),
where harness quality accounts for most of the performance variance between
agent systems. The finding that the same underlying model can score 25% or
85% on SWE-bench depending on the harness was a wake-up call for the field.

Roko's architecture is designed with this finding in mind: the six crate
layers (core, agent, orchestrator, gate, compose, learn) provide the
harness infrastructure, while the model is a pluggable component selected
at runtime. This separation means harness improvements benefit all models
simultaneously.

---

## Citations

1. Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM
   Agents." arXiv:2603.28052. — +7.7 accuracy, +4.7 math, 4× tokens.
2. Jimenez, C. E. et al. (2024). "SWE-bench: Can Language Models Resolve
   Real-World GitHub Issues?" — Benchmark context for harness variance.
3. ref [46] in Meta-Harness — SWE-bench mobile, source of the "6× gap"
   number between harness and no-harness configurations.
4. `crates/roko-agent/src/dispatcher/mod.rs` — 7-step pipeline.
5. `crates/roko-agent/src/safety/mod.rs` — SafetyLayer.
6. `crates/roko-compose/src/system_prompt_builder.rs` — 6-layer prompts.
7. `crates/roko-agent/src/tool_loop/prune.rs` — Context pruning.
8. Implementation plan `11-inconsistencies.md` — Gap #1 analysis.
