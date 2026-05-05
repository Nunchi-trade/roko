# CLI and Command Graph

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the CLI as a Graph interpreter where each subcommand triggers a Graph, the 9-verb model, layered config resolution as a Signal merge pipeline, progressive help as a React Cell, and scaffolders as template Graphs.

---

## 1. The CLI as a Graph Interpreter

The `roko` binary is a single Rust executable that serves as the canonical entry point for all operations. Every meaningful CLI operation triggers a **Graph** -- either a cognitive loop Graph (for `roko run`), an executor Graph (for `roko plan run`), or a Lens Graph (for `roko status`). The CLI is not a bag of ad-hoc commands; it is a Graph interpreter with a command-line frontend.

This matters architecturally because it means the CLI, TUI, HTTP API, and web dashboard all fire the same Graphs. The CLI is one rendering of a universal verb set. See [03-GRAPH.md](../../unified/03-GRAPH.md) for the Graph primitive and [04-EXECUTION.md](../../unified/04-EXECUTION.md) for how Graphs run.

The binary is built with `clap` for argument parsing and links against every layer of the crate stack: `roko-core` (kernel), `roko-agent` (LLM dispatch), `roko-compose` (prompt assembly), `roko-gate` (verification), `roko-orchestrator` (plan DAG), `roko-learn` (feedback loops), `roko-neuro` (knowledge), `roko-daimon` (affect), and `roko-serve` (HTTP).

**Source**: `crates/roko-cli/src/main.rs` (entry point), `crates/roko-cli/src/lib.rs` (library surface).

---

## 2. Six Invocation Modes

The same binary supports six interaction modes, selected by invocation pattern:

| Mode | Invocation | What Happens |
|---|---|---|
| **Default Interactive** | `roko` (no subcommand) | Workspace detection, session resume, intent classification, interactive prompt |
| **One-Shot** | `roko run "prompt"` | Single cognitive loop iteration, exit with semantic code |
| **REPL** | `roko repl` | Persistent session, Daimon affect tracks across turns |
| **Pipe** | `echo "prompt" \| roko` | Stdin-driven, suppress TUI chrome, JSON to stdout, diagnostics to stderr |
| **Daemon** | `roko daemon --start` | Background service with `launchd`/`systemd`, monitors event sources |
| **Serve** | `roko serve` | HTTP API server (axum), exposes the same Graphs over REST/WebSocket/SSE |

All six modes consume the same event stream. The orchestrator emits `AgentEvent` Pulses through async channels -- `WaveStart`, `AgentSpawn`, `GatePass`, `GateFail`, `PlanPhaseChange`. The TUI renders these visually. Headless mode serializes them as JSON lines. Serve mode streams them over SSE/WebSocket. Same Pulses, different Lens Cells.

---

## 3. The 9-Verb Model on the CLI

Every high-frequency workflow maps to one of nine canonical verbs. The CLI is the naming authority for these verbs:

| Verb | CLI Rendering | Graph Triggered | Cell Protocols Invoked |
|---|---|---|---|
| `ask` | `roko ask <prompt>` | Cognitive Loop (single iteration) | Compose -> Execute -> Verify |
| `plan` | `roko plan ...` | Plan generation Graph | Route (select strategy) |
| `do` | `roko do ...` or `roko plan run ...` | Executor Graph with gates | Execute (full DAG) |
| `watch` | `roko watch <session>` | Lens Graph over `active_tasks` | Observe (subscribe) |
| `inspect` | `roko inspect <id>` | Lens Graph over Store | Observe + Store query |
| `replay` | `roko replay <episode>` | Replay Graph | Store query -> Execute |
| `learn` | `roko learn ...` | Lens Graph over `heuristic_library` | Score (read calibration) |
| `tune` | `roko tune ...` or `roko config ...` | Config mutation Graph | React (emit config Pulses) |
| `connect` | `roko connect ...` or `roko plugin ...` | Integration Graph | Connect protocol |

The exact command tree is broader than the verb set. `roko prd`, `roko research`, `roko knowledge`, `roko agent` are all specialized sub-trees. But every help page teaches the adjacent verb, not an isolated silo. A user who runs `roko plan run` should learn that `roko watch` exists for live progress.

### Slash Commands

The interactive shell, TUI chat pane, and web surface all accept slash commands with identical semantics:

| Slash Command | Verb | CLI Equivalent |
|---|---|---|
| `/edit <file>` | ask | `roko ask --context <file> ...` |
| `/run <cmd>` | do | `roko do --cmd "<cmd>"` |
| `/plan` | plan | `roko plan create` |
| `/watch` | watch | `roko watch <session>` |
| `/inspect <id>` | inspect | `roko inspect <kind> <id>` |
| `/explain` | inspect | `roko explain <topic>` |
| `/heuristics` | learn | `roko learn heuristics` |
| `/replay <episode>` | replay | `roko replay <episode>` |
| `/tune <key>` | tune | `roko config set <key> <value>` |

---

## 4. Layered Config Resolution as a Signal Merge Pipeline

Configuration in Roko follows a four-layer priority system. Expressed in unified vocabulary, this is a **Signal merge pipeline** where higher-priority Signals shadow lower ones:

```
Priority 1 (highest): CLI flags       →  Signal(kind=ConfigOverride, source=cli)
Priority 2:           Environment vars →  Signal(kind=ConfigOverride, source=env)
Priority 3:           roko.toml        →  Signal(kind=Config, source=file)
Priority 4 (lowest):  Compiled defaults → Signal(kind=Config, source=builtin)
```

The merge is a Pipeline Graph of Score Cells. Each layer's Signal is scored by priority. The highest-priority Signal for each config key wins. The result is a single resolved `Config` Signal that drives the runtime.

```toml
# Minimal roko.toml -- everything else uses defaults
[agent]
model = "claude-sonnet-4-6"
```

The merge is also deployment-shape-aware. A `profile` key selects one of five deployment shapes (laptop, single-server, container, clustered, edge), which supplies shape-specific defaults that sit between the compiled defaults and the user's `roko.toml` overrides.

**Source**: `crates/roko-cli/src/config.rs` (`load_layered` function).

---

## 5. Progressive Help as a React Cell

The CLI's error-as-teacher system is a React Cell. It watches error Pulses on the Bus and emits teaching Signals:

```rust
/// Every error message follows the error-as-teacher format.
pub struct TeachingError {
    pub what: String,   // What happened
    pub why: String,    // Why it matters
    pub fix: String,    // Exactly what to do next
    pub context: Option<String>,  // Link to deeper explanation
}
```

Examples of the format in practice:

```
ERROR: Gate 'compile' failed -- `cargo build` exited with code 1
WHY: The agent's code changes introduced a compilation error.
FIX: The agent will retry with the compiler error in context.
     If retries are exhausted, check: roko episode list --failed
CTX: Gate pipeline docs: roko explain gates
```

```
ERROR: tool 'cargo.test' not allowed for role 'reader'
WHY: This role excludes test execution.
FIX: Try one of:
       roko ask --role implementer "run cargo test"
       roko config set roles.reader.tools +=cargo.test
CTX: Permission recovery: roko explain tools
```

The `roko explain <topic>` command provides three-level progressive disclosure:
- **Level 1**: One paragraph (always shown first)
- **Level 2**: Detailed explanation with configuration examples (on Enter)
- **Level 3**: Full configuration reference and custom implementation guide (on Enter again)

This maps to the Observe protocol: each level is a progressively deeper Lens over the same underlying knowledge.

---

## 6. Scaffolders as Template Graphs

The `roko new` command generates complete, compilable implementations for every extension point. Each scaffold is a **template Graph** -- a pre-built Graph with typed inputs and outputs that produces working code.

| Scaffold Type | What It Generates | Graph Pattern |
|---|---|---|
| `roko new domain <name>` | Complete domain profile bundle | Rack with domain-specific Macros |
| `roko new gate <name>` | Custom Verify Cell implementation | Single Cell with test harness |
| `roko new scorer <name>` | Custom Score Cell | Single Cell with calibration setup |
| `roko new router <name>` | Custom Route Cell | Single Cell with feedback method |
| `roko new probe <name>` | T0 deterministic check | Zero-LLM Cell, pure function |
| `roko new template <name>` | Agent template with prompt config | Rack with role/model/tool Macros |

The Rack pattern (see [03-GRAPH.md](../../unified/03-GRAPH.md)) is the underlying abstraction. A scaffold is a Rack where Macros are the customization points (knobs) and Slots are the extension points (jacks). The user fills in the Macros; the Rack produces a working Cell.

Every scaffold compiles immediately and passes tests out of the box. This is the "generators, not blank files" principle -- the generated code is a working implementation, not a stub.

---

## 7. Diff-First Review as Structured Signal

The interactive shell renders proposed edits as diffs with per-hunk control:

```
Proposed 3 hunks:
  [1/3] src/core.rs: add lowercase normalization
  [2/3] src/core.rs: add empty-check
  [3/3] tests/core.rs: add regression test

Apply: [a]ll, [1,2] subset, [n]one, [e]dit, [x] explain >
```

Each hunk decision becomes a structured Signal:
- **Accepted hunks** reinforce the path that led to them (positive evidence for the heuristic)
- **Rejected hunks** become negative evidence for replay and future heuristic tuning
- **Edited hunks** preserve the operator's correction in the transcript

This review state persists in the session transcript, enabling later replay and learning. A resumed session restores the last approved or pending diff-first review state, recent Bus progress, tool approval memory, and active budget totals.

---

## 8. Exit Codes as Semantic Signals

The CLI uses semantic exit codes that encode the outcome of Graph execution:

| Code | Meaning | Signal Kind |
|---|---|---|
| 0 | Flow succeeded | `Verdict::Pass` |
| 1 | Generic failure | `Error` |
| 2 | Bad CLI usage | `ConfigError` |
| 10 | Graph validation failed | `Verdict::Fail(validation)` |
| 11 | Capability denied | `Verdict::Fail(auth)` |
| 12 | Budget exceeded | `Verdict::Fail(budget)` |
| 13 | Deadline exceeded | `Verdict::Fail(timeout)` |
| 14 | Human input timeout | `Verdict::Fail(human_timeout)` |
| 15 | Cancelled | `Cancelled` |

These codes enable shell automation: `roko ask "fix the bug" && git commit` works because success is 0 and all failures are non-zero. The `--json` flag adds structured error output for CI pipelines.

---

## What This Enables

- **One binary, all modes**: The same `roko` executable handles interactive chat, one-shot execution, background daemon, HTTP server, and pipe-driven CI.
- **Verb portability**: Learn `ask/plan/do/watch` once, use them in CLI, TUI, chat, and web.
- **Config as Signal**: The layered resolution system means zero-config works out of the box, while every layer can be independently overridden.
- **Teachable errors**: Every error tells you what to do next, in the vocabulary you are already using.
- **Working scaffolds**: `roko new gate my-gate` produces code that compiles and passes tests in under 5 seconds.

---

## Feedback Loops

- **Heuristic learning from hunk review**: Accepted and rejected hunks become training Signals for the heuristic library, improving future proposals.
- **Session continuity**: Transcripts, approval history, and budget state persist across sessions. A resumed session picks up where the previous one left off.
- **Progressive help refinement**: The explain system can be extended by agents themselves -- an agent that discovers a new pattern can propose a heuristic that feeds back into future explain output.

---

## Open Questions

1. **Verb convergence timeline**: `roko run` and `roko ask` currently coexist. When does `ask` become the canonical single-turn verb and `run` becomes a compatibility alias?
2. **Interactive shell scope**: How much of the interactive shell (intent classification, diff-first review, slash commands) should be built before the TUI chat pane, given that both serve the same purpose?
3. **Plugin CLI surface**: The `roko plugin` command family assumes a five-tier plugin SPI that is not yet implemented. Should the CLI surface ship before the SPI, or should they be gated together?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Wire `ask` as canonical single-turn verb | `crates/roko-cli/src/main.rs` | Add `roko ask` subcommand that maps to `run_once` with streaming |
| Build interactive default shell | `crates/roko-cli/src/` | `roko` with no subcommand: workspace detection, session resume, intent classification |
| Implement slash commands | `crates/roko-cli/src/` | Parse `/edit`, `/plan`, `/watch` etc. in interactive and TUI modes |
| Implement diff-first review | `crates/roko-cli/src/` | Per-hunk approval flow with transcript persistence |
| Wire error-as-teacher format | `crates/roko-cli/src/`, `crates/roko-core/src/error.rs` | Systematic `TeachingError` usage across all error paths |
| Implement `roko explain` | `crates/roko-cli/src/` | Three-level progressive disclosure for all topic areas |
| Implement `roko new` scaffolders | `crates/roko-cli/src/scaffold.rs` | Template-based code generation for all Cell types |
