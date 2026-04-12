# 01 — Gate Implementations

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/`)
> **Status**: 11 gate implementations, 7 wired into the rung selector

---

## 1. Overview

Roko ships 11 concrete gate implementations. Each implements the `Gate` trait (see
[00-gate-trait.md](./00-gate-trait.md)) and returns `Verdict` directly — not
`Result<Verdict>`. Every gate handles its own errors internally, converting
infrastructure failures (spawn errors, timeouts, malformed input) into
`Verdict::fail()`.

The gates are organized by verification layer, from cheapest/fastest to most
expensive/thorough:

| Gate | Module | Rung | Cost | What it verifies |
|---|---|---|---|---|
| `CompileGate` | `compile.rs` | 0 (Compile) | Low | Code compiles |
| `ClippyGate` | `clippy_gate.rs` | 1 (Lint) | Low | No lint violations |
| `TestGate` | `test_gate.rs` | 2 (Test) | Medium | Tests pass |
| `SymbolGate` | `symbol_gate.rs` | 3 (Symbol) | Zero (no subprocess) | Required symbols exist |
| `DiffGate` | `diff_gate.rs` | N/A (pre-pipeline) | Zero | Non-vacuous changes |
| `ShellGate` | `shell.rs` | Any | Varies | Arbitrary shell command |
| `GeneratedTestGate` | `generated_test_gate.rs` | 4 (GeneratedTest) | High | Auto-generated tests pass |
| `PropertyTestGate` | `property_test_gate.rs` | 5 (PropertyTest) | High | Property-based tests pass |
| `IntegrationGate` | `integration_gate.rs` | 6 (Integration) | Highest | Integration tests pass |
| `LlmJudgeGate` | `llm_judge_gate.rs` | N/A (auxiliary) | Medium | LLM-based quality judgment |
| `VerifyChainGate` | `verify_chain_gate.rs` | N/A (auxiliary) | Low | Chain verification |

> **Citation**: crates/roko-gate/src/lib.rs — Module declarations for all 11 gates.

---

## 2. ShellGate — The Foundation

**File**: `crates/roko-gate/src/shell.rs` (186 lines)

`ShellGate` is the simplest gate and the building block for others. It runs an arbitrary
shell command and passes if the exit code is 0.

```rust
pub struct ShellGate {
    program: String,
    args: Vec<String>,
    timeout_ms: u64,     // default: 5 minutes (300,000 ms)
    name: String,
}
```

### Construction

```rust
ShellGate::new("cargo", vec!["fmt".into(), "--check".into()])
    .with_timeout_ms(60_000)
    .with_name("format_check")
```

### Verdict Construction

The `ShellGate::verify()` method demonstrates the canonical pattern used by all
shell-spawning gates:

1. **Read payload**: Extract `GatePayload` from signal body for `working_dir` and
   `extra_env`. If absent, use process defaults.
2. **Build command**: Set program, args, cwd, env vars, `kill_on_drop(true)`.
3. **Run with timeout**: `tokio::time::timeout(duration, cmd.output()).await`
4. **Handle three outcomes**:
   - `Err(_)` → timeout → `Verdict::fail("timed out after N ms")`
   - `Ok(Err(io_err))` → spawn failure → `Verdict::fail("spawn failed: ...")`
   - `Ok(Ok(output))` → check exit code → `Verdict::pass()` or `Verdict::fail()`
5. **Always attach**: `.with_detail(combined_output).with_duration(elapsed_ms)`

> **Citation**: crates/roko-gate/src/shell.rs:57–118 — Full `ShellGate::verify()`
> implementation.

---

## 3. CompileGate — Rung 0

**File**: `crates/roko-gate/src/compile.rs` (195 lines)

`CompileGate` wraps `ShellGate`'s pattern with build-system awareness. It reads
`BuildSystem` from the `GatePayload` and runs the appropriate check command.

```rust
pub struct CompileGate {
    build_system: BuildSystem,
    extra_args: Vec<String>,
    timeout_ms: u64,     // default: 10 minutes (600,000 ms)
    name: String,        // e.g., "compile:cargo"
}
```

### Build System Dispatch

The `BuildSystem` enum (defined in `payload.rs`) supports:
- **Cargo**: `cargo check --workspace`
- **Npm**: `npm run build`
- **Go**: `go build ./...`
- **Make**: `make`

Each variant implements `program()`, `check_args()`, `test_args()`, and `lint_args()`.

### Error Summarization

On failure, `CompileGate` extracts up to 3 error-level diagnostics from stderr via
`summarize_errors()`. This keeps the verdict's `reason` field concise while the full
output lives in `detail`.

```rust
fn summarize_errors(stderr: &str, max: usize) -> String {
    let errors: Vec<&str> = stderr.lines()
        .filter(|l| l.trim_start().starts_with("error:") || l.trim_start().starts_with("error["))
        .take(max)
        .collect();
    if !errors.is_empty() { errors.join("; ") }
    else { stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("compilation failed").to_string() }
}
```

> **Citation**: crates/roko-gate/src/compile.rs:134–151 — `summarize_errors()`
> function.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> Rung 0 is compilation: "cheapest check that proves the code at least compiles."

---

## 4. ClippyGate — Rung 1

**File**: `crates/roko-gate/src/clippy_gate.rs` (212 lines)

`ClippyGate` runs the language-appropriate linter. For Rust, this is `cargo clippy --
-D warnings`; for Go, `go vet`; for Node, `npm run lint`.

```rust
pub struct ClippyGate {
    build_system: BuildSystem,
    extra_args: Vec<String>,
    timeout_ms: u64,     // default: 5 minutes (300,000 ms)
    name: String,        // e.g., "clippy:cargo"
}
```

### Argument Splicing

ClippyGate handles the Cargo-specific `--` sentinel carefully: extra args are inserted
*before* the `--` separator so they apply to the Cargo invocation (e.g., `--features ci`)
rather than to Clippy's own flags.

```rust
let dash_idx = base.iter().position(|a| *a == "--");
if let Some(idx) = dash_idx {
    // Insert extra_args before the "--"
    for arg in &base[..idx] { cmd.arg(arg); }
    for arg in &self.extra_args { cmd.arg(arg); }
    for arg in &base[idx..] { cmd.arg(arg); }
}
```

### Design: Lint Before Test

ClippyGate is designed to run before `TestGate` in a short-circuit pipeline. Lint checks
are fast (seconds) and catch many issues that would cause expensive test failures
(minutes). Running lint first saves time by failing fast.

> **Citation**: crates/roko-gate/src/clippy_gate.rs:70–93 — Argument splicing logic.

---

## 5. TestGate — Rung 2

**File**: `crates/roko-gate/src/test_gate.rs` (384 lines)

`TestGate` runs the project's test suite and parses passed/failed/ignored counts from
the output.

```rust
pub struct TestGate {
    build_system: BuildSystem,
    selector: TestSelector,
    extra_args: Vec<String>,
    timeout_ms: u64,     // default: 15 minutes (900,000 ms)
    name: String,        // e.g., "test:cargo"
}
```

### Test Selectors

The `TestSelector` enum controls which tests run:

```rust
pub enum TestSelector {
    All,                    // Run everything
    Quick,                  // --lib (unit tests only)
    Patterns(Vec<String>),  // Specific test patterns
}
```

For Cargo, `Quick` adds `--lib`. For Go, `Patterns(["TestFoo", "TestBar"])` becomes
`-run TestFoo|TestBar`.

### Test Count Parsing

`parse_test_counts()` dispatches by build system:
- **Cargo**: Parses `test result: ok. N passed; M failed; K ignored` lines, aggregating
  across multiple test targets.
- **Go**: Counts `--- PASS:`, `--- FAIL:`, `--- SKIP:` markers.

Parsed counts are attached to the verdict via `.with_test_count(tc)`. This enables
downstream policies to classify "mostly passing" runs (e.g., ≥90% pass rate with ≥20
tests and ≥1 failure) differently from total failures.

> **Citation**: crates/roko-gate/src/test_gate.rs:166–241 — `parse_test_counts()` and
> per-build-system parsers.

> **Citation**: bardo-backup/tmp/mori-agents/20-verification-first-architecture.md —
> Rung 1 (test execution), verification-first architecture.

---

## 6. SymbolGate — Rung 3

**File**: `crates/roko-gate/src/symbol_gate.rs` (1001 lines)

`SymbolGate` is unique among the gates: it spawns no subprocesses and makes no LLM
calls. It parses Rust source files directly and verifies that every symbol in a
`SymbolManifest` exists with the correct kind, visibility, and module path.

```rust
pub struct SymbolGate {
    source_roots: Vec<PathBuf>,
    name: String,
}
```

### What It Catches

The most common agent failure is: "I was told to create `pub struct RateLimiter` and
did not." SymbolGate catches this at effectively zero cost, before any compilation or
test execution.

### Mismatch Taxonomy

When expectations are not met, SymbolGate produces a structured error digest:

```
4 symbol expectation(s) unmet:
  MISSING: struct RateLimiter at core::rate_limit
  WRONG_VIS: fn check_rate at core::rate_limit (found: private, expected: pub)
  WRONG_KIND: Limiter at core::rate_limit (found: struct, expected: trait)
  WRONG_PATH: struct Clock at core::time (found at: core::clock)
  AMBIGUOUS: fn foo at core::util (2 matches)
```

Five mismatch categories: `MISSING`, `WRONG_VIS`, `WRONG_KIND`, `WRONG_PATH`,
`AMBIGUOUS`. Each tells the agent (or the human) exactly what went wrong and where.

### Symbol Extraction

The scanner is a lightweight single-pass line-based extractor. It handles:
- Visibility modifiers: `pub`, `pub(crate)`, `pub(super)`, `pub(in path)`
- Modifier keywords: `async`, `unsafe`, `extern "C"`, `const fn`
- Item kinds: `struct`, `enum`, `trait`, `fn`, `type`, `const`, `static`, `mod`

It deliberately does not descend into `mod foo { ... }` blocks (rare in idiomatic Rust).

> **Citation**: crates/roko-gate/src/symbol_gate.rs:449–478 — `extract_symbols()`
> lightweight parser.

---

## 7. DiffGate — Vacuous-Implementation Rejection

**File**: `crates/roko-gate/src/diff_gate.rs` (358 lines)

`DiffGate` solves a specific failure mode: agents that "pass" gates by producing
vacuous implementations. Without this gate, an agent can replace function bodies with
`todo!()` or `Ok(())` to make compile and lint gates happy while doing no actual work.

### Rejection Criteria

A diff is rejected when:
1. **Empty diff**: Zero added lines.
2. **Below threshold**: Non-whitespace added lines < `min_added_lines` (default: 1).
3. **All forbidden tokens**: Every substantive added line matches a forbidden token.

Default forbidden tokens:
- `todo!()`, `todo!`, `unimplemented!()`, `unimplemented!`
- `panic!("not implemented")`, `Ok(())`, `return Ok(())`

### Analysis Output

```rust
pub struct DiffAnalysis {
    pub added_lines: u32,
    pub non_whitespace_added: u32,
    pub all_added_are_forbidden: bool,
}
```

The `analyze_diff()` function is pure: no I/O, no subprocess, just string scanning.
It skips diff headers (`+++`, `---`, `@@`), counts `+` lines, filters whitespace and
comments, and checks against the forbidden token list.

> **Citation**: crates/roko-gate/src/diff_gate.rs:1–33 — Module doc: "Rejects changes
> that 'pass' gates only because they introduced no substantive work."

---

## 8. Remaining Gates (Scaffold Status)

### 8.1 GeneratedTestGate (Rung 4)

Runs tests that were automatically generated by the agent or a dedicated test-generation
agent. These are distinct from the project's existing test suite (Rung 2). Generated
tests are more targeted — they specifically exercise the code the agent just wrote.

### 8.2 PropertyTestGate (Rung 5)

Runs property-based tests (QuickCheck / proptest style). Property tests provide stronger
guarantees than example-based tests: they assert invariants over randomized inputs.

### 8.3 IntegrationGate (Rung 6)

Runs the full integration test suite. This is the most expensive gate — it may involve
standing up services, databases, or network connections.

### 8.4 LlmJudgeGate (Auxiliary)

Uses an LLM to evaluate code quality, correctness, or adherence to specifications. This
is the only gate that consults a model rather than a deterministic tool. It is used
when properties are too nuanced for automated checking (e.g., "does this implementation
match the PRD's intent?").

### 8.5 VerifyChainGate (Auxiliary)

Verifies chain-related artifacts. Part of the Phase 2+ architecture.

> **Citation**: crates/roko-gate/src/lib.rs — All 11 gate modules declared.

---

## 9. Shared Patterns Across All Gates

### 9.1 Builder Pattern

Every gate uses a builder-style API with `with_*` methods:

```rust
CompileGate::cargo()
    .with_extra_args(vec!["--features".into(), "ci".into()])
    .with_timeout_ms(120_000)
```

### 9.2 GatePayload

Gates that shell out to external tools read their configuration from a `GatePayload`
in the signal body:

```rust
pub struct GatePayload {
    pub working_dir: PathBuf,
    pub target_dir: Option<PathBuf>,
    pub extra_env: HashMap<String, String>,
    // ... build system, test selector, etc.
}
```

This decouples the gate from the orchestrator: the orchestrator constructs the signal
with the right payload, and the gate reads it.

### 9.3 Timeout + kill_on_drop

Every subprocess-spawning gate sets `cmd.kill_on_drop(true)` and wraps execution in
`tokio::time::timeout()`. This ensures:
- No zombie processes from abandoned gates
- Bounded execution time for every gate

### 9.4 Error Summarization

Each gate has a gate-specific `summarize_*` function that extracts the most relevant
error lines from stderr. This keeps verdict reasons concise (3 lines) while preserving
full output in `detail`.

> **Citation**: crates/roko-gate/src/compile.rs:130–151 — `summarize_errors()`.
> crates/roko-gate/src/test_gate.rs:244–267 — `summarize_test_failures()`.
> crates/roko-gate/src/clippy_gate.rs:146–163 — `summarize_lint_issues()`.

---

## 10. Gate Composition

Gates are designed to be composed, not used in isolation. The primary composition
mechanism is the `GatePipeline` (see [03-gate-pipeline.md](./03-gate-pipeline.md)),
which chains gates sequentially with optional short-circuit behavior. The `RungSelector`
(see [02-6-rung-selector.md](./02-6-rung-selector.md)) determines which gates to
include based on plan complexity and prior failures.

The composition chain is:

```
RungSelector picks rungs for this plan's complexity
    ↓
GatePipeline composes the selected gates
    ↓
Each gate runs in sequence (or short-circuits on failure)
    ↓
Aggregated Verdict flows to ratchet, thresholds, feedback
```

> **Citation**: crates/roko-gate/src/gate_pipeline.rs — GatePipeline composes
> `Vec<Box<dyn Gate>>` sequentially.
