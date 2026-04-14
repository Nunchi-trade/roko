# 12 — Forensic AI: Causal Replay

> **Layer**: L3 Harness — Verification × Compliance
> **Crates**: `roko-gate` (artifact_store), `roko-fs` (signal persistence),
>   `roko-learn` (episodes)
> **Status**: Foundations implemented (content-addressed artifacts, episode logging),
>   full replay pipeline designed


> **Implementation**: Shipping

---

## 1. Overview

Forensic AI causal replay is the ability to reconstruct, step by step, exactly what an
agent did, why it did it, and what verification outcomes resulted — with cryptographic
proof that the reconstruction is accurate. This is not debugging. This is audit-grade
reconstruction that can withstand regulatory scrutiny.

The system makes every agent action replayable: given a task ID, produce the complete
chain from initial prompt through every tool call, every gate verdict, every retry, to
the final outcome. Every artifact in this chain is content-addressed (BLAKE3 hashed),
so any tampering is detectable.

> **Citation**: refactoring-prd/09-innovations.md — Innovation IX: "Forensic AI Causal
> Replay, content-addressed replay of any agent action."

---

## 2. Why Forensic Replay Matters

### 2.1 Regulatory Compliance

Autonomous AI agents that make decisions affecting production systems, financial
instruments, or healthcare data must satisfy regulatory requirements for
explainability and auditability.

| Regulation | Requirement | How Forensic Replay Satisfies It |
|---|---|---|
| EU AI Act Art. 14 | Human oversight of high-risk AI | Complete action trace, gate verdicts as checkpoints |
| SEC/CFTC | Algorithmic trading audit trail | Content-addressed chain from decision to execution |
| HIPAA | Access audit for health data | Every file read/write by every agent, timestamped |
| SOX | Financial system change controls | Immutable verification artifacts for every code change |

### 2.2 Debugging Complex Failures

When an agent produces a subtle bug that passes all gates, forensic replay enables:
- Tracing back through the agent's reasoning to find where it went wrong
- Identifying which gate should have caught the issue (gap analysis)
- Determining whether the agent's tool calls were productive or wasteful

### 2.3 Learning System Validation

The learning system (skills, routing, experiments) makes decisions based on historical
data. Forensic replay can verify that those decisions were based on accurate data:
- Did the gate verdicts that trained the router actually correspond to correct
  verification outcomes?
- Did the skill extraction process correctly identify the tool call patterns that led
  to success?

> **Citation**: refactoring-prd/09-innovations.md — Regulatory compliance table with
> specific regulatory provisions.

---

## 3. The Content-Addressed Chain

Every element in the replay chain is identified by its BLAKE3 hash:

```
TaskSpec (hash: 0xa3f...)
    ↓
SystemPrompt (hash: 0xb7c...)
    ↓
AgentTurn 1 (hash: 0xc1d...)
    ├── ToolCall: Read "src/lib.rs" (hash: 0xd2e...)
    │   └── Result: file contents (hash: 0xe3f...)
    ├── ToolCall: Edit "src/lib.rs" (hash: 0xf4a...)
    │   └── Result: success (hash: 0xa5b...)
    └── Response: "I've added the new struct" (hash: 0xb6c...)
    ↓
GateVerdict Rung 0 (hash: 0xc7d...)
    └── Detail: compile output (hash: 0xd8e...)
    ↓
AgentTurn 2 (hash: 0xe9f...) [retry after gate failure]
    ├── ToolCall: Read "src/lib.rs" (hash: 0xfa0...)
    ...
    ↓
GateVerdict Rung 0 (hash: 0xab1...)  [pass]
GateVerdict Rung 1 (hash: 0xbc2...)  [pass]
GateVerdict Rung 2 (hash: 0xcd3...)  [pass]
    ↓
FinalOutcome (hash: 0xde4...)
```

Each node's hash incorporates its content. If any element is modified, its hash changes,
and the chain becomes inconsistent. This is the same principle that Git uses for commits
and that blockchains use for blocks.

---

## 4. Data Sources for Replay

### 4.1 Episode Log

**Path**: `.roko/episodes.jsonl`

Each line is a JSON object recording an agent turn:
```json
{
  "task_id": "plan-42-task-3",
  "turn": 1,
  "model": "claude-opus-4-6",
  "tool_calls": [
    {"tool": "Read", "args": {"path": "src/lib.rs"}, "duration_ms": 45},
    {"tool": "Edit", "args": {"path": "src/lib.rs", "..."}, "duration_ms": 12}
  ],
  "input_tokens": 12500,
  "output_tokens": 3200,
  "timestamp": "2026-04-10T14:30:00Z"
}
```

### 4.2 Signal Log

**Path**: `.roko/signals.jsonl`

Every signal (engram) written to the substrate, including gate verdicts:
```json
{
  "hash": "0xab1c2d3e...",
  "kind": "verdict",
  "body": {"gate": "compile:cargo", "passed": true, "duration_ms": 4200},
  "parent": "0x9f8e7d6c...",
  "timestamp": "2026-04-10T14:30:05Z"
}
```

### 4.3 Artifact Store

Content-addressed gate artifacts: compile output, test output, diff analysis results.
Each artifact is retrievable by its BLAKE3 hash.

### 4.4 Efficiency Events

**Path**: `.roko/learn/efficiency.jsonl`

Per-turn efficiency data: token counts, tool call metadata, gate timing, cost estimates.

---

## 5. The Replay Algorithm

Given a task ID, reconstruct the complete execution:

```
1. Query episode log for all turns with this task_id
   → Ordered list of agent turns

2. For each turn:
   a. Retrieve the system prompt (from prompt assembly logs)
   b. Retrieve tool call inputs and outputs (from episode log)
   c. Retrieve the agent's response

3. Query signal log for all verdicts associated with this task_id
   → Ordered list of gate verdicts

4. For each verdict:
   a. Retrieve the gate artifact from ArtifactStore by hash
   b. Reconstruct the verdict's inputs (the signal that was verified)

5. Build the causal chain:
   TaskSpec → Prompt → Turn 1 → ... → Turn N → Verdict 1 → ... → Verdict M → Outcome

6. Verify chain integrity:
   For each element, recompute its BLAKE3 hash and compare to the stored hash
   Any mismatch indicates tampering or corruption
```

---

## 6. Causal Analysis

Beyond reconstruction, forensic replay enables causal analysis:

### 6.1 What-If Analysis

"What if the agent had used a different model?" Replay the task with the same inputs
but a different model. Compare verdicts and outcomes. This powers the shadow testing
loop (Loop 12 in the evaluation lifecycle).

### 6.2 Root Cause Analysis

When a task fails, trace backward through the causal chain:
1. Which gate failed? (e.g., Rung 2: Test)
2. What was the test failure? (e.g., "assertion failed: expected 200, got 404")
3. Which agent edit introduced the failure? (e.g., Turn 3, Edit to routes.rs)
4. What was the agent's reasoning? (e.g., "I moved the route handler to a different module")
5. Was the reasoning correct? (e.g., "Yes, but the agent forgot to update the route registration")

This chain from verdict → edit → reasoning → root cause is what "forensic" means. It's
not just "what happened" but "why it happened."

### 6.3 Gap Analysis

When a bug escapes all gates (passes verification but is still wrong), forensic replay
identifies which gate *should* have caught it:

```
Bug: off-by-one error in pagination
Escaped gates: Compile (expected), Lint (expected), Test (gap!)

Analysis:
  - Existing tests don't cover pagination edge cases
  - Generated tests (Rung 4) would have caught this if the test generator
    had been prompted with "test boundary conditions for pagination"

Recommendation: Add pagination boundary test to the GeneratedTestGate's
  standard test generation templates
```

---

## 7. Immutability Guarantees

### 7.1 Content-Addressed Everything

Every artifact, signal, and episode entry is identified by its content hash. Changing
any byte changes the hash, making tampering detectable.

### 7.2 Append-Only Logs

The episode log (`.roko/episodes.jsonl`) and signal log (`.roko/signals.jsonl`) are
append-only JSONL files. New entries are appended; existing entries are never modified
or deleted during normal operation.

### 7.3 Artifact Store Immutability

The `ArtifactStore` has no delete or update operations (see
[04-artifact-store.md](./04-artifact-store.md)). Once stored, artifacts are permanent.

### 7.4 Hash Chain (Future)

The signals in the signal log form a hash chain: each signal's hash incorporates its
parent signal's hash. This creates a tamper-evident sequence — inserting, removing, or
reordering signals breaks the chain.

---

## 8. Pre-Certified Agent Templates

A practical application of forensic replay: **pre-certified agent templates** for
regulated industries.

A pre-certified template is a set of:
1. System prompt sections (versioned, hashed)
2. Gate pipeline configuration (which rungs, which gates)
3. Verification criteria (generated test templates)
4. Audit trail requirements (which data must be logged)

Organizations in regulated industries can deploy pre-certified templates knowing that:
- Every agent action will be logged and replayable
- Every verification outcome is content-addressed and immutable
- The complete chain from input to output is reconstructable
- Regulatory auditors can independently verify the chain

> **Citation**: refactoring-prd/09-innovations.md — "Pre-certified agent templates"
> for regulated industries.

---

## 9. Performance Considerations

Forensic replay adds overhead to the execution path:

| Operation | Overhead | When |
|---|---|---|
| BLAKE3 hashing | < 1ms per artifact | Every gate run |
| Episode logging | < 1ms per turn | Every agent turn |
| Signal logging | < 1ms per signal | Every signal write |
| Artifact storage | O(artifact_size) | Every gate run |
| Chain verification | O(chain_length) | On-demand (replay) |

The per-execution overhead is negligible (< 5ms total). The on-demand replay cost is
proportional to the chain length but is only incurred when forensic analysis is actually
needed.

### Storage Cost

For a typical plan execution (10 tasks, 3 attempts each, 5 gate runs per attempt):
- Episodes: ~150 JSONL entries, ~500 KB
- Signals: ~150 entries, ~300 KB
- Artifacts: ~150 artifacts, ~5 MB (mostly compile/test output)
- Total: ~6 MB per plan execution

At this rate, a year of continuous operation produces ~2 GB of forensic data — easily
manageable with periodic GC of old artifacts.

---

## 10. Relationship to Other Components

| Component | Relationship to Forensic Replay |
|---|---|
| ArtifactStore | Stores immutable gate artifacts |
| Episode Logger | Records agent turns |
| Signal Log | Records engrams and verdicts |
| GateRatchet | Ratchet state at each point in time |
| AdaptiveThresholds | Threshold state at each point in time |
| Efficiency Events | Per-turn cost and timing data |

---

## 11. Summary

Forensic AI causal replay transforms Roko's verification layer from "did the code pass
the gates?" to "how did the code come to pass (or fail) the gates, and can we prove
it?" The content-addressed, append-only architecture makes this reconstruction
cryptographically verifiable.

This is not a feature that most users need day-to-day. But for regulated industries,
for debugging complex multi-agent interactions, and for validating the learning system's
decisions, it is essential. The infrastructure cost is negligible (< 5ms per execution,
~6 MB per plan). The value is unbounded: any question about any past execution can be
answered definitively.
