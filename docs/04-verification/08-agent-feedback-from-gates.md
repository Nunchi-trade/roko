# 08 — Agent Feedback from Gates

> **Layer**: L3 Harness — Verification → L2 Engine (feedback channel)
> **Crate**: `roko-gate` (`crates/roko-gate/src/feedback.rs`)
> **Status**: Implemented (375 lines)

---

## 1. Overview

Raw gate output — compiler stderr, test logs, linter JSON — is verbose and full of
noise. Progress bars, download messages, repeated blank lines, and Cargo metadata all
waste agent context tokens without contributing actionable information. The feedback
module solves this: it parses raw output into structured `GateFeedback` containing only
actionable items classified by severity.

This is the bridge between the Harness layer (L3, where gates run) and the agent's
context window. It ensures that when an agent retries after a gate failure, it sees
exactly the errors and warnings it needs to fix — and nothing else.

> **Citation**: crates/roko-gate/src/feedback.rs — Full implementation.

---

## 2. The GateFeedback Type

```rust
pub struct GateFeedback {
    pub rung: u8,                  // Which rung produced this feedback
    pub passed: bool,              // Whether the gate passed
    pub errors: Vec<String>,       // Error-level items (must fix)
    pub warnings: Vec<String>,     // Warning-level items (should fix)
    pub suggestions: Vec<String>,  // Informational/help items
}
```

Three severity buckets, ordered from most to least critical:
- **errors**: Compilation errors, test failures, panics. The agent *must* fix these.
- **warnings**: Unused variables, deprecated usage, style issues. The agent *should*
  fix these.
- **suggestions**: Compiler help messages, notes, file location pointers. These *inform*
  the agent about what to do.

### Helper Methods

```rust
impl GateFeedback {
    pub fn item_count(&self) -> usize;   // Total items across all categories
    pub fn is_empty(&self) -> bool;       // True if no actionable items
    pub fn items(&self) -> Vec<FeedbackItem>;  // All items, errors first
}
```

The `items()` method returns all feedback as `FeedbackItem` structs (severity + message),
ordered: errors first, then warnings, then suggestions. This ordering ensures the most
critical information appears first in the agent's context window.

---

## 3. The Classification Pipeline

### 3.1 Per-Line Classification

```rust
fn classify_line(line: &str) -> Option<(Severity, &str)>
```

Each line of raw output is classified into a severity or `None` (noise). The classifier
is a priority chain:

1. **Empty/whitespace**: → `None`
2. **Noise patterns**: → `None`
3. **Error patterns**: → `Some(Severity::Error, line)`
4. **Warning patterns**: → `Some(Severity::Warning, line)`
5. **Suggestion patterns**: → `Some(Severity::Info, line)`
6. **Anything else**: → `None` (dropped as context)

### 3.2 Noise Detection

```rust
fn is_noise(line: &str) -> bool
```

Lines that are pure noise — no actionable information:

| Pattern | Example |
|---|---|
| Cargo progress | `Downloading`, `Downloaded`, `Compiling`, `Checking`, `Finished`, `Running`, `Documenting`, `Fresh`, `Packaging` |
| npm deprecation | `npm WARN deprecated stable@0.1.0: deprecated package` |
| Progress bars | Lines containing `━`, `▓`, `░` |

These lines are common in build output and contribute nothing to error diagnosis.

### 3.3 Error Detection

```rust
fn is_error_line(line: &str) -> bool
```

Lines that indicate errors:

| Pattern | What It Catches |
|---|---|
| `error` (starts with) | Rust `error:` messages |
| `Error:` (starts with) | Generic error format |
| `ERROR:` (starts with) | Uppercase error format |
| `FAILED` (starts with) | Test failure markers |
| `FAIL ` (starts with) | Go test failure |
| Contains `error[E` | Rustc error codes like `error[E0425]` |
| Contains `panicked at` | Panic messages |
| `thread '...' panicked` | Thread panic with test name |

### 3.4 Warning Detection

```rust
fn is_warning_line(line: &str) -> bool
```

| Pattern | What It Catches |
|---|---|
| `warning` (starts with) | Rust warnings |
| `Warning:` (starts with) | Generic warning format |
| `WARNING:` (starts with) | Uppercase warning format |
| `warn[` (starts with) | Clippy warning codes |

### 3.5 Suggestion Detection

```rust
fn is_suggestion_line(line: &str) -> bool
```

| Pattern | What It Catches |
|---|---|
| `help:` (starts with) | Compiler help messages |
| Contains `= help:` | Inline help annotations |
| `note:` (starts with) | Compiler notes |
| Contains `= note:` | Inline note annotations |
| `suggestion:` (starts with) | Explicit suggestions |
| `hint:` (starts with) | Hint messages |
| `-->` (starts with or contains) | Source location pointers |

> **Citation**: crates/roko-gate/src/feedback.rs:99–192 — Classification functions.

---

## 4. The Public API

```rust
pub fn feedback_for_agent(gate_output: &str, rung: u8) -> GateFeedback
```

The main entry point. Takes raw gate output (typically stdout + stderr concatenated) and
a rung number, returns structured feedback.

### Algorithm

```
for each line in gate_output:
    classify_line(line)
    match severity:
        Error   → push to errors, set has_errors = true
        Warning → push to warnings
        Info    → push to suggestions
        None    → skip (noise)

passed = !has_errors
return GateFeedback { rung, passed, errors, warnings, suggestions }
```

### Pass Detection

The feedback's `passed` field is based purely on whether any error-level items were
found. If there are warnings but no errors, the feedback says `passed = true`. This
aligns with the convention that warnings don't block compilation or test execution.

---

## 5. How the Orchestrator Uses Feedback

After a gate failure, the orchestrator generates feedback and injects it into the
agent's retry prompt:

```rust
let verdict = pipeline.verify(signal, ctx).await;

if !verdict.passed {
    let feedback = feedback_for_agent(
        verdict.detail.as_deref().unwrap_or(""),
        current_rung,
    );

    // Inject into retry prompt
    let retry_context = format!(
        "Your previous attempt failed at rung {}.\n\
         Errors ({}):\n{}\n\
         Warnings ({}):\n{}\n\
         Suggestions ({}):\n{}",
        feedback.rung,
        feedback.errors.len(),
        feedback.errors.join("\n"),
        feedback.warnings.len(),
        feedback.warnings.join("\n"),
        feedback.suggestions.len(),
        feedback.suggestions.join("\n"),
    );
    // ... feed retry_context to the agent
}
```

This pattern:
1. Extracts only actionable information from potentially thousands of lines of output
2. Categorizes by severity so the agent knows what to prioritize
3. Preserves source location pointers (the `-->` lines) so the agent knows *where*
   to fix

---

## 6. Token Economy

The feedback module is a critical part of Roko's token economy. Consider a typical
`cargo check` failure:

| Raw output | ~2,000 lines |
|---|---|
| Noise (Downloading, Compiling, etc.) | ~1,500 lines |
| Errors | ~10 lines |
| Warnings | ~20 lines |
| Suggestions | ~15 lines |
| **Filtered feedback** | **~45 lines** |

That's a 97.75% reduction. At ~4 tokens per line, the feedback saves ~7,800 tokens per
gate failure. Over a 5-attempt retry loop with 3 gate failures, that's ~23,400 tokens
saved — a significant fraction of the agent's context window.

> **Citation**: bardo-backup/tmp/mori-refactor/06-harness.md — "Raw gate output
> (compiler stderr, test logs, linter JSON) is verbose and full of noise that wastes
> agent context tokens."

---

## 7. Gate-to-Scaffold Feedback Loop

The feedback module is one half of a closed loop. The other half is the section
effectiveness tracker (see the Scaffold layer documentation and implementation plan
task 2J.05–2J.06):

```
Agent receives prompt (with sections)
    ↓
Agent produces code
    ↓
Gate verifies code → Verdict
    ↓
feedback_for_agent() → GateFeedback
    ↓
Two consumers:
    1. Agent retry prompt (immediate)
    2. SectionEffectivenessRegistry (learning)
       → Which prompt sections correlated with gate success?
       → Adjust section priorities for future prompts
```

The immediate feedback (inject errors into retry prompt) operates at machine speed.
The section effectiveness learning operates at consolidation speed — it needs 50+
observations to make statistical claims about which prompt sections help.

> **Citation**: tmp/implementation-plans/modelrouting/12-advanced-patterns.md §C
> (Tasks 2J.05–2J.06) — Gate-to-scaffold feedback loop, section effectiveness tracking
> with lift > 0.05.

---

## 8. Severity Ordering

```rust
pub enum Severity {
    Info,       // 0
    Warning,    // 1
    Error,      // 2
}
```

`Severity` derives `PartialOrd` and `Ord`, with `Info < Warning < Error`. This enables:
- Sorting feedback items by severity (`items()` returns errors first)
- Filtering by minimum severity (e.g., "only show errors and warnings")
- Threshold-based policies (e.g., "fail if any Error, warn if any Warning")

---

## 9. Serde Support

Both `GateFeedback` and `FeedbackItem` derive `Serialize` and `Deserialize`. This
enables:
- Persisting feedback to the episode log (`.roko/episodes.jsonl`)
- Transmitting feedback as JSON to agents that expect structured input
- Aggregating feedback across executions for learning

The serde roundtrip test in the module verifies that serialization and deserialization
preserve all fields exactly.

---

## 10. Limitations and Future Work

### 10.1 Language-Specific Heuristics

The current classifier is biased toward Rust/Cargo output. The patterns for npm, Go,
and other build systems are minimal. Future work should add per-language classifiers:

```rust
fn classify_line_for_build_system(line: &str, build: BuildSystem) -> Option<(Severity, &str)>
```

### 10.2 Multi-Line Error Messages

Rustc error messages span multiple lines (the error, the source snippet, the help
message). The current classifier treats each line independently, which means:
- The error line is classified as Error
- The source snippet line is classified as noise (dropped)
- The help line is classified as Info

This loses the visual structure of the error. A future improvement would group
consecutive lines belonging to the same diagnostic into a single `FeedbackItem`.

### 10.3 Structured Error Formats

Some tools emit structured output (JSON, SARIF). The current classifier works on
line-by-line text. Future work should detect and parse structured formats:

```
if gate_output starts with "[" or "{":
    parse as JSON diagnostic array
else:
    use line-by-line classifier
```

---

## 11. Testing

The feedback module has 14 tests covering:

| Test | What It Verifies |
|---|---|
| `feedback_empty_output_passes` | Empty input → passed, no items |
| `feedback_extracts_errors` | Error lines extracted correctly |
| `feedback_extracts_warnings` | Warning lines extracted correctly |
| `feedback_extracts_suggestions` | Help/note lines extracted correctly |
| `feedback_filters_noise` | Cargo progress lines dropped |
| `feedback_mixed_output` | Mixed output classified correctly |
| `feedback_item_count` | Total count across categories |
| `feedback_items_ordering` | Errors first, then warnings, then suggestions |
| `feedback_rung_preserved` | Rung number roundtrips correctly |
| `feedback_test_failure_detected` | FAILED/panicked lines are errors |
| `feedback_severity_ordering` | Info < Warning < Error |
| `feedback_npm_deprecation_is_noise` | npm-specific noise detected |
| `feedback_progress_bars_are_noise` | Unicode progress bars detected |
| `feedback_serde_roundtrip` | JSON serialization preserves all fields |

> **Citation**: crates/roko-gate/src/feedback.rs:241–374 — Tests section.
