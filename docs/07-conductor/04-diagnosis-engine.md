# Diagnosis Engine

> Thirty-four patterns across twenty error categories.
> Given raw error text, return a typed diagnosis with category,
> confidence, and suggested intervention.


> **Implementation**: Built

---

## Purpose

The Diagnosis Engine replaces ad-hoc error parsing with structured
classification. Instead of each component grepping for "error[E0308]"
in raw output, the engine accepts raw error text and returns a typed
`Diagnosis` with:

- Error category (one of 20 enumerated types)
- Confidence score (0.0 to 1.0)
- Suggested intervention (one of 9 actions)
- Description and context

This structured classification enables:

1. **Consistent handling**: The same error always gets the same
   classification, regardless of which component encounters it.
2. **Appropriate response**: A missing import (cheap to fix) gets
   `AutoFix`. A borrow checker error (requires understanding) gets
   `RestartAgent` with additional context.
3. **Learning**: Error categories feed into the efficiency tracking
   system, enabling per-category success rate analysis.
4. **Observability**: The dashboard can show "12 CompileErrors, 3
   TestFailures, 1 LifetimeError" instead of "16 errors."

---

## Error Categories

The `ErrorCategory` enum defines twenty categories covering the
full range of errors encountered in production batch runs:

```rust
pub enum ErrorCategory {
    CompileError,
    TestFailure,
    TypeMismatch,
    BorrowCheckerError,
    LifetimeError,
    ImportError,
    MissingFile,
    PermissionDenied,
    NetworkError,
    TimeoutError,
    OomError,
    DiskFull,
    LlmRateLimit,
    LlmContextOverflow,
    LlmRefusal,
    ProcessCrash,
    LoopDetected,
    ClippyWarning,
    GitConflict,
    DependencyError,
}
```

### Category Groupings

**Rust compiler errors** (6 categories):
- `CompileError` — general compilation failure
- `TypeMismatch` — E0308, expected vs. found types
- `BorrowCheckerError` — E0382, E0505, E0507, use-after-move and
  borrow violations
- `LifetimeError` — E0106, E0495, E0621, lifetime annotations
- `ImportError` — E0432, E0433, unresolved imports and modules
- `ClippyWarning` — clippy lint violations

**Test and verification** (1 category):
- `TestFailure` — test assertion failures, panic in tests

**File system** (3 categories):
- `MissingFile` — file not found errors
- `PermissionDenied` — file permission errors
- `DiskFull` — no space left on device

**Infrastructure** (3 categories):
- `NetworkError` — connection failures, DNS resolution
- `TimeoutError` — operation timeouts
- `OomError` — out of memory

**LLM provider** (3 categories):
- `LlmRateLimit` — 429 errors, quota exceeded
- `LlmContextOverflow` — prompt exceeds model context window
- `LlmRefusal` — content policy rejection

**Process** (2 categories):
- `ProcessCrash` — agent process exited unexpectedly
- `LoopDetected` — agent entering a detected loop

**Version control** (1 category):
- `GitConflict` — merge conflicts, rebase failures

**Dependencies** (1 category):
- `DependencyError` — cargo dependency resolution, feature flags

---

## Suggested Interventions

Each diagnosis maps to one of nine intervention actions:

```rust
pub enum SuggestedIntervention {
    RetryWithContext,
    AutoFix,
    RestartAgent,
    AbortPlan,
    BackoffRetry,
    MergeResolution,
    ReduceContext,
    SwitchModel,
    WarnAndContinue,
}
```

### Intervention Semantics

| Intervention | When Used | What Happens |
|-------------|-----------|-------------|
| `RetryWithContext` | Transient errors; adding context may help | Retry the operation with additional error context in the prompt |
| `AutoFix` | Simple, well-understood errors (imports, missing fields) | Route to cheap Haiku-tier auto-fix agent |
| `RestartAgent` | Agent is confused or stuck | Kill and respawn with fresh context + error analysis |
| `AbortPlan` | Unrecoverable errors | Mark plan as failed immediately |
| `BackoffRetry` | Rate limits, temporary outages | Wait with exponential backoff, then retry |
| `MergeResolution` | Git merge conflicts | Spawn merge resolver agent |
| `ReduceContext` | Context overflow | Compact context and retry with smaller prompt |
| `SwitchModel` | Model limitation (refusal, context overflow) | Route to a different model |
| `WarnAndContinue` | Clippy warnings, non-blocking issues | Log the warning but do not interrupt execution |

### Category-to-Intervention Mapping

| Category | Primary Intervention | Rationale |
|----------|---------------------|-----------|
| CompileError | RetryWithContext | Agent may fix with error details |
| TestFailure | RetryWithContext | Agent may fix with test output |
| TypeMismatch | RetryWithContext | Agent needs the expected/found types |
| BorrowCheckerError | RestartAgent | Borrow errors require fresh approach |
| LifetimeError | RestartAgent | Lifetime errors are structurally difficult |
| ImportError | AutoFix | Missing imports are cheap to fix |
| MissingFile | RetryWithContext | Agent may need to create the file |
| PermissionDenied | AbortPlan | Cannot fix permissions from agent context |
| NetworkError | BackoffRetry | Likely transient |
| TimeoutError | BackoffRetry | Likely transient |
| OomError | AbortPlan | Resource exhaustion requires operator action |
| DiskFull | AbortPlan | Resource exhaustion requires cleanup |
| LlmRateLimit | BackoffRetry | Wait for rate limit window to expire |
| LlmContextOverflow | ReduceContext | Compact and retry |
| LlmRefusal | SwitchModel | Try a different model |
| ProcessCrash | RestartAgent | Agent died; respawn |
| LoopDetected | RestartAgent | Agent is stuck; fresh context needed |
| ClippyWarning | WarnAndContinue | Non-blocking |
| GitConflict | MergeResolution | Spawn merge resolver |
| DependencyError | RetryWithContext | Agent may fix with dependency info |

---

## Pattern Matching

The engine contains 34 built-in patterns. Each pattern is a substring
match with an associated category, confidence, and suggested
intervention:

```rust
struct ErrorPattern {
    substring: &'static str,
    category: ErrorCategory,
    confidence: f64,
    intervention: SuggestedIntervention,
}
```

### Pattern Examples

| Pattern Substring | Category | Confidence | Intervention |
|------------------|----------|-----------|-------------|
| `"error[E0308]"` | TypeMismatch | 0.95 | RetryWithContext |
| `"error[E0382]"` | BorrowCheckerError | 0.95 | RestartAgent |
| `"error[E0106]"` | LifetimeError | 0.95 | RestartAgent |
| `"error[E0432]"` | ImportError | 0.95 | AutoFix |
| `"error[E0433]"` | ImportError | 0.95 | AutoFix |
| `"error[E0063]"` | CompileError | 0.90 | AutoFix |
| `"cannot find"` | ImportError | 0.70 | RetryWithContext |
| `"test result: FAILED"` | TestFailure | 0.90 | RetryWithContext |
| `"panicked at"` | TestFailure | 0.85 | RetryWithContext |
| `"Connection refused"` | NetworkError | 0.80 | BackoffRetry |
| `"rate limit"` | LlmRateLimit | 0.90 | BackoffRetry |
| `"context_length_exceeded"` | LlmContextOverflow | 0.95 | ReduceContext |
| `"No space left"` | DiskFull | 0.95 | AbortPlan |
| `"CONFLICT"` | GitConflict | 0.80 | MergeResolution |
| `"clippy::"`  | ClippyWarning | 0.90 | WarnAndContinue |

The confidence score reflects how specific the substring match is.
Rust error codes (E0308, E0382) are highly specific — confidence 0.95.
Generic substrings ("cannot find") are less specific — confidence 0.70.

### Matching Algorithm

```rust
impl DiagnosisEngine {
    pub fn diagnose(&self, error_text: &str) -> Vec<Diagnosis> {
        let lower = error_text.to_lowercase();
        self.patterns.iter()
            .filter(|p| lower.contains(&p.substring.to_lowercase()))
            .map(|p| Diagnosis {
                category: p.category,
                confidence: p.confidence,
                intervention: p.intervention,
                description: format!("Matched pattern: {}", p.substring),
            })
            .collect()
    }
}
```

Multiple patterns can match the same error text. The caller receives
all matching diagnoses and can select the highest-confidence one or
use the full set for richer context.

---

## Integration Points

### With the Conductor

When a watcher fires, the Conductor can pass the error context through
the Diagnosis Engine before making its decision. This enriches the
intervention signal with structured error classification:

```
Watcher fires: "compile-fail-repeat: 3 identical errors"
    │
    ▼
Diagnosis Engine: error text → [Diagnosis { category: ImportError, confidence: 0.95, intervention: AutoFix }]
    │
    ▼
Enriched decision: Restart with { intervention: AutoFix, context: "E0432: unresolved import" }
```

### With the Auto-Fix Pipeline

The `AutoFix` intervention routes errors to a lightweight Haiku-tier
agent. The Diagnosis Engine's classification determines which errors
qualify for auto-fix:

- `ImportError` → auto-fixable (add the correct `use` statement)
- `CompileError` with E0063 (missing struct field) → auto-fixable
- `TypeMismatch` → sometimes auto-fixable if the conversion is simple
- `BorrowCheckerError` → not auto-fixable (requires architectural understanding)

The cost difference is significant: an auto-fix costs ~$0.01 (Haiku,
small context). A full re-implementation cycle costs ~$2.00+ (Opus,
full context). When 6 out of 8 errors are missing imports, the engine
saves $11.94 by routing them to auto-fix instead of full re-implementation.

### With the Learning System

Error categories feed into the efficiency tracking system:

```
AgentEfficiencyEvent {
    outcome: "gate_failed",
    gate_errors: [
        { category: "ImportError", count: 3 },
        { category: "TypeMismatch", count: 1 },
    ],
    // ...
}
```

Over time, this data reveals patterns:
- "Plans touching `src/auth/` have 40% LifetimeError rate"
- "Haiku agents produce 3x more ImportError than Sonnet"
- "Auto-fix resolves ImportError 95% of the time"

These patterns inform:
- Prompt engineering (add lifetime notes for auth-related tasks)
- Model routing (use Sonnet for auth tasks, Haiku for others)
- Auto-fix thresholds (route ImportError to auto-fix with high confidence)

---

## Design Decisions

### Why Substring Matching Instead of Regex

The engine uses simple substring matching (`contains()`), not regular
expressions. Rationale:

1. **Performance**: Substring matching is O(n) per pattern, O(n*m) for
   all patterns. Regex compilation and matching adds overhead.
2. **Readability**: `"error[E0308]"` is immediately clear. A regex
   for the same match would be less readable.
3. **Maintainability**: Adding a new pattern is adding a string literal
   and its metadata. No regex debugging.
4. **Coverage**: The 34 patterns cover the most common errors
   encountered in production. Regex would be needed for complex
   extraction (e.g., parsing the expected/found types from a type
   mismatch), but the diagnosis engine's job is classification, not
   extraction.

### Why 34 Patterns

The pattern count (34) was derived from production data. During batch
runs in March-April 2026, every distinct error type was cataloged.
The 34 patterns cover approximately 95% of observed errors by
frequency. The remaining 5% are rare edge cases that fall through to
the default `CompileError` category.

The test `has_at_least_20_patterns()` ensures the pattern set is not
accidentally reduced. New patterns are added as new error types are
encountered in production.

### Why Twenty Categories

The category count (20) balances granularity against complexity:

- Too few categories (e.g., "CompileError" for everything) loses
  the information needed for appropriate intervention routing.
- Too many categories (e.g., one per rustc error code) creates
  maintenance burden without proportional benefit.

Twenty categories cover the natural groupings of errors at the level
of actionable difference — each category maps to a different
intervention strategy.

---

## Production Error Distribution

From production batch runs, the approximate error frequency distribution:

| Category | Frequency | Auto-Fix Rate |
|----------|----------|--------------|
| ImportError | 35% | 95% |
| CompileError (general) | 20% | 30% |
| TypeMismatch | 15% | 50% |
| TestFailure | 12% | 0% (requires understanding) |
| BorrowCheckerError | 5% | 10% |
| LifetimeError | 4% | 10% |
| LlmRateLimit | 3% | N/A (retry) |
| All others | 6% | varies |

The key insight: over a third of all errors are import errors, and
95% of those can be auto-fixed for $0.01 each. Without the diagnosis
engine, all errors would go through full re-implementation at $2+ each.
The engine's cost savings are dominated by this single category.

---

## Future: Confidence-Weighted Routing

Currently, the diagnosis engine classifies errors and suggests
interventions. A future enhancement is confidence-weighted routing:

1. **High confidence (>0.9)**: Route directly to suggested intervention.
   No human review needed.
2. **Medium confidence (0.6-0.9)**: Route to suggested intervention but
   flag for review.
3. **Low confidence (<0.6)**: Fall back to RestartAgent (the safest
   generic intervention).

This tiered routing would reduce false-positive auto-fixes (where the
engine misclassifies an error and the auto-fix agent wastes a turn on
something it cannot fix).

---

## File Reference

| File | What |
|------|------|
| `crates/roko-conductor/src/diagnosis.rs` | DiagnosisEngine, ErrorCategory, SuggestedIntervention, 34 patterns |
