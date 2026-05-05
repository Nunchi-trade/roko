# Gate Feedback and Retry

> Depth for [02-CELL.md](../../unified/02-CELL.md). How Verify Cell verdicts flow back to agents as structured feedback Signals, closing the retry Loop.

---

## Overview

When a Verify Cell produces a failing Verdict, the raw output (compiler stderr, test logs, linter JSON) can be thousands of lines long. Most of it is noise: progress bars, download messages, metadata. The Gate Feedback system is a Cell that sits between the Verify pipeline and the Compose pipeline. It transforms raw Verify output into a structured `GateFeedback` Signal containing only actionable items classified by severity, then feeds this back into the Compose Cell for prompt assembly on retry.

This doc describes the feedback classification Pipeline, the retry Loop pattern, the token economy that makes it viable, and the section-effectiveness learning Loop that closes the cycle.

---

## 1. The Feedback Classification Pipeline

The feedback path is a three-Cell Pipeline:

```
Raw Verify output (thousands of lines)
    |
    v
[ClassifyCell]       -- per-line severity classification
    |
    v
[FilterCell]         -- drop noise, keep actionable items
    |
    v
[StructureCell]      -- produce GateFeedback Signal
    |
    v
GateFeedback Signal  (~45 lines, errors first)
```

### 1.1 GateFeedback as a Signal

```rust
/// Structured feedback from a Verify Cell, expressed as a Signal.
/// This is the bridge between the Verify protocol (L3 Harness) and
/// the Compose protocol (prompt assembly for retry).
pub struct GateFeedback {
    pub rung: u8,                  // Which rung produced this feedback
    pub passed: bool,              // Whether the Verify Cell passed
    pub errors: Vec<String>,       // Must-fix items (Error severity)
    pub warnings: Vec<String>,     // Should-fix items (Warning severity)
    pub suggestions: Vec<String>,  // Informational items (help, notes, source pointers)
}

impl GateFeedback {
    /// Total actionable items across all categories.
    pub fn item_count(&self) -> usize {
        self.errors.len() + self.warnings.len() + self.suggestions.len()
    }

    /// All items ordered by severity: errors first, then warnings, then suggestions.
    /// This ordering ensures the most critical information appears first
    /// in the agent's context window.
    pub fn items(&self) -> Vec<FeedbackItem> {
        let mut out = Vec::with_capacity(self.item_count());
        out.extend(self.errors.iter().map(|m| FeedbackItem { severity: Severity::Error, message: m.clone() }));
        out.extend(self.warnings.iter().map(|m| FeedbackItem { severity: Severity::Warning, message: m.clone() }));
        out.extend(self.suggestions.iter().map(|m| FeedbackItem { severity: Severity::Info, message: m.clone() }));
        out
    }
}

/// Severity ordering: Info < Warning < Error.
/// Derives PartialOrd/Ord so items can be sorted, filtered by threshold,
/// or used in policies ("fail if any Error, warn if any Warning").
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,       // 0
    Warning,    // 1
    Error,      // 2
}
```

### 1.2 Per-Line Classification

Each line of raw output passes through a priority chain:

```rust
fn classify_line(line: &str) -> Option<(Severity, &str)> {
    // 1. Empty/whitespace -> None
    if line.trim().is_empty() { return None; }

    // 2. Noise patterns -> None
    if is_noise(line) { return None; }

    // 3. Error patterns -> Error
    if is_error_line(line) { return Some((Severity::Error, line)); }

    // 4. Warning patterns -> Warning
    if is_warning_line(line) { return Some((Severity::Warning, line)); }

    // 5. Suggestion patterns -> Info
    if is_suggestion_line(line) { return Some((Severity::Info, line)); }

    // 6. Anything else -> None (dropped)
    None
}
```

**Noise patterns** (dropped entirely):

| Pattern | Example |
|---|---|
| Cargo progress | `Downloading`, `Compiling`, `Checking`, `Finished`, `Running`, `Fresh` |
| npm deprecation | `npm WARN deprecated stable@0.1.0` |
| Progress bars | Lines containing `━`, `▓`, `░` |

**Error patterns** (must fix):

| Pattern | Catches |
|---|---|
| Starts with `error` | Rust `error:` messages |
| Contains `error[E` | Rustc error codes like `error[E0425]` |
| Contains `panicked at` | Panic messages |
| Starts with `FAILED` | Test failure markers |

**Warning patterns** (should fix):

| Pattern | Catches |
|---|---|
| Starts with `warning` | Rust warnings |
| Starts with `warn[` | Clippy warning codes |

**Suggestion patterns** (informational):

| Pattern | Catches |
|---|---|
| Starts with `help:` or contains `= help:` | Compiler help messages |
| Starts with `note:` or contains `= note:` | Compiler notes |
| Contains `-->` | Source location pointers |

### 1.3 The Main Entry Point

```rust
/// Transform raw Verify output into structured agent feedback.
/// This is the only public function. Everything else is internal.
pub fn feedback_for_agent(gate_output: &str, rung: u8) -> GateFeedback {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut suggestions = Vec::new();
    let mut has_errors = false;

    for line in gate_output.lines() {
        match classify_line(line) {
            Some((Severity::Error, msg)) => { errors.push(msg.to_string()); has_errors = true; }
            Some((Severity::Warning, msg)) => { warnings.push(msg.to_string()); }
            Some((Severity::Info, msg)) => { suggestions.push(msg.to_string()); }
            None => {} // noise, dropped
        }
    }

    GateFeedback { rung, passed: !has_errors, errors, warnings, suggestions }
}
```

Pass detection is purely error-based: warnings alone do not fail the gate. This matches the convention that warnings don't block compilation or test execution.

---

## 2. The Retry Loop

The retry mechanism is a Loop pattern (see [03-GRAPH.md](../../unified/03-GRAPH.md) for the Loop definition). The Verify Cell is the feedback source, and the Compose Cell is the feedback consumer.

```
                     ┌──────────────────────────────────────────┐
                     │                                          │
                     v                                          │
[Compose Cell]  ->  [Agent Cell]  ->  [Verify Cell]  ->  [Classify Cell]
  (prompt              (LLM             (gate               (feedback
   assembly)            call)            pipeline)           extraction)
      ^                                                        |
      |                                                        |
      └───────── GateFeedback Signal ──────────────────────────┘
```

### 2.1 Feedback Injection into Retry Prompt

After a gate failure, the orchestrator constructs a retry context from the GateFeedback Signal and passes it to the Compose Cell:

```rust
// After Verify Cell returns a failing Verdict:
let feedback = feedback_for_agent(
    verdict.detail.as_deref().unwrap_or(""),
    current_rung,
);

// The Compose Cell includes this as a high-priority section:
let retry_section = format!(
    "Your previous attempt failed at rung {}.\n\
     Errors ({}):\n{}\n\
     Warnings ({}):\n{}\n\
     Suggestions ({}):\n{}",
    feedback.rung,
    feedback.errors.len(), feedback.errors.join("\n"),
    feedback.warnings.len(), feedback.warnings.join("\n"),
    feedback.suggestions.len(), feedback.suggestions.join("\n"),
);
```

This Loop pattern ensures three properties:

1. **Only actionable information** reaches the agent (noise stripped)
2. **Severity ordering** tells the agent what to prioritize (errors first)
3. **Source location pointers** (the `-->` lines) tell the agent *where* to fix

### 2.2 Failure Classification for Retry Strategy

Not all failures warrant the same retry approach. The system classifies failures to choose the right strategy:

```
Failure Type          | Strategy
----------------------|------------------------------------------
Compile error         | Re-dispatch with error context, same model
Test failure          | Re-dispatch with test output, same model
Repeated same error   | Escalate model tier (e.g., Haiku -> Sonnet)
3+ same signature     | Replan task (structural change needed)
Negative progress     | Decompose into sub-tasks
Low promise score     | Replace task entirely
```

The retry budget is bounded: after N failures (configurable, default 3-5), the system escalates rather than continuing the Loop.

---

## 3. Token Economy

The feedback system is a critical part of the token economy. Consider a typical `cargo check` failure:

| Category | Lines |
|---|---|
| Raw output | ~2,000 |
| Noise (Downloading, Compiling, etc.) | ~1,500 |
| Errors | ~10 |
| Warnings | ~20 |
| Suggestions | ~15 |
| **Filtered feedback** | **~45** |

That is a **97.75% reduction**. At ~4 tokens per line, feedback saves ~7,800 tokens per gate failure. Over a 5-attempt retry Loop with 3 gate failures, that is ~23,400 tokens saved -- a significant fraction of the agent's context window.

This reduction has a compounding effect: smaller feedback Signals leave more room in the Compose Cell's token budget for other context sections (skills, task spec, code context), which improves the quality of the retry attempt.

---

## 4. The Gate-to-Scaffold Feedback Loop

The feedback Pipeline has two consumers, operating at different timescales:

```
Agent receives prompt (with sections assembled by Compose Cell)
    |
    v
Agent Cell produces code
    |
    v
Verify Cell checks code -> Verdict Signal
    |
    v
feedback_for_agent() -> GateFeedback Signal
    |
    +--> Consumer 1: Retry prompt (immediate, per-turn)
    |      The Compose Cell injects errors into the next attempt.
    |
    +--> Consumer 2: SectionEffectivenessRegistry (learning, consolidation speed)
           Tracks which prompt sections correlate with Verify success.
           Needs 50+ observations to make statistical claims.
           Adjusts section priorities for future prompts (lift > 0.05).
```

This is a nested Loop pattern:

- **Inner Loop** (cognitive speed): gate fail -> classify -> enrich prompt -> re-dispatch -> gate
- **Outer Loop** (consolidation speed): accumulate section-outcome correlations -> adjust section priorities -> observe new outcomes

The inner Loop operates at machine speed (sub-second feedback extraction). The outer Loop operates at consolidation speed (50+ observations before statistical claims about section effectiveness).

---

## 5. Verdict-Informed Model Escalation

The retry Loop interacts with the Route protocol. The cascade router queries Verdict history when selecting a model:

```
For task T, query Store for Verdict Signals where task_id == T:
  0 prior failures  ->  standard routing (no adjustment)
  1 prior failure   ->  escalate model tier by 1 (e.g., Haiku -> Sonnet)
  2+ prior failures ->  escalate to maximum tier (Opus)
  3+ same signature ->  flag for replanning (structural issue)
```

This means the retry Loop does not simply repeat with the same configuration. Each failure adjusts the routing, giving the agent progressively more capable models until the task succeeds or is replanned.

---

## 6. Serialization and Persistence

Both `GateFeedback` and `FeedbackItem` derive `Serialize`/`Deserialize`. This enables:

- **Episode logging**: Feedback persisted to `.roko/episodes.jsonl` for later analysis
- **Agent JSON input**: Structured feedback for agents that expect JSON
- **Cross-execution aggregation**: Learning from feedback patterns over time

---

## 7. Limitations

### 7.1 Language Bias

The classifier is Rust/Cargo-centric. Patterns for npm, Go, Python, and other toolchains are minimal. A future improvement: per-language classifier selection.

### 7.2 Single-Line Classification

Rustc errors span multiple lines (error, source snippet, help). The classifier treats each line independently, losing visual structure. A future improvement: group consecutive lines belonging to the same diagnostic into a single `FeedbackItem`.

### 7.3 No Structured Format Detection

Some tools emit JSON or SARIF. The classifier operates line-by-line on text. A future improvement: detect and parse structured formats before falling back to line classification.

---

## What This Enables

1. **Token-efficient retry Loops**: 97.75% noise reduction means agents can retry more times within the same context budget.
2. **Severity-driven prioritization**: Agents fix errors before warnings, improving first-attempt success rates on retry.
3. **Learning from feedback patterns**: The section-effectiveness registry discovers which prompt sections actually help agents pass gates, creating a meta-Loop that improves prompt quality over time.
4. **Model escalation**: Each retry is informed by prior failures, progressively routing to more capable models.

## Feedback Loops

| Loop | Speed | Input | Output |
|---|---|---|---|
| Gate fail -> classify -> retry | Cognitive (seconds) | Raw Verify output | Structured GateFeedback -> Compose Cell |
| Section effectiveness | Consolidation (hours) | Section-outcome correlations | Adjusted section priorities |
| Model escalation | Cognitive (per-attempt) | Verdict history | Route Cell model selection |
| Replan on repeated failure | Cognitive (per-task) | Same-signature failure streak | New task specification |

## Open Questions

1. **Multi-line grouping**: How to group consecutive diagnostic lines without a language-specific parser? Could use blank-line delimiters or indentation heuristics.
2. **Structured format priority**: When a tool emits both JSON and text output, which should the classifier prefer? JSON is more structured but may miss human-readable context.
3. **Cross-language classifier**: Should the classifier be a pluggable Cell, or a static function with a language parameter? The Cell pattern would allow language-specific classifiers to be discovered via MCP.
4. **Feedback Signal decay**: Currently feedback is ephemeral (used for immediate retry). Should GateFeedback be promoted to a durable Signal with demurrage, enabling Dreams to extract patterns from feedback history?

---

## References

- [02-CELL.md](../../unified/02-CELL.md) -- Verify protocol definition, Verdict type
- [verify-as-universal-oracle.md](verify-as-universal-oracle.md) -- Verify's four simultaneous roles
- `crates/roko-gate/src/feedback.rs` -- Implementation (375 lines, 14 tests)
