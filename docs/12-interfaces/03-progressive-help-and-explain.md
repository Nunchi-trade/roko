# Progressive Help and Explain System

> Layered disclosure: beginners see three commands; experts see the full trait system. Errors are instructions, not dead ends.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md)
**Key sources**: `refactoring-prd/10-developer-guide.md` §0-1, §11, `refactoring-prd/06-interfaces.md`

---

## Abstract

Roko's CLI help system is designed around **progressive disclosure** — the principle that information should be presented in layers, with the simplest and most commonly needed information first, and deeper detail available on demand. This applies to three distinct aspects of the CLI experience: the help output (`roko --help`), the explain system (`roko explain <topic>`), the error message format, and the interactive config wizard (`roko config wizard`).

The help system is inspired by Karpathy's observation (2025) that context engineering is fundamentally about presenting the right information at the right level of detail at the right time. The same principle applies to developer experience — a user who just installed Roko needs different information than one who is debugging a failing gate pipeline.

---

## `roko status` — Glanceable Project Health

The `roko status` command is designed to be the single most frequently run command. It answers: "What is going on in my project right now?"

```
$ roko status

ROKO STATUS — my-project
─────────────────────────
Engrams:  1,247 (↑32 today)
Episodes: 89 (last: 2m ago, pass)
Gates:    94% pass rate (↑2% this week)
Cost:     $4.23 today / $18.91 this week
Model:    claude-sonnet-4-6 (cascade: 82% T0, 14% T1, 4% T2)
Neuro:    142 entries (23 persistent, 89 working, 30 transient)
C-Factor: 1.23 (↑0.04)
```

With `--cfactor`, it also computes and persists the latest C-Factor snapshot. With `--json`, it outputs structured JSON for CI pipelines and dashboards.

---

## `roko explain` — Layered Topic Documentation

The `roko explain` command provides in-terminal documentation about Roko concepts. It is designed for the developer who wants to understand *why* something works the way it does, not just *how* to use it.

### Usage

```bash
roko explain gates           # What are gates? How does the pipeline work?
roko explain routing         # How does model routing work?
roko explain cognitive       # What is the universal cognitive loop?
roko explain neuro           # How does the knowledge system work?
roko explain daimon          # What is the affect engine?
roko explain dreams          # How does offline consolidation work?
roko explain engram          # What is the Engram data type?
roko explain cfactor         # What is collective intelligence measurement?
```

### Output Format

Each explain topic follows a three-level structure:

**Level 1 — One paragraph** (always shown first):
```
$ roko explain gates

GATES — Verification Pipeline
──────────────────────────────
Gates verify that agent output meets quality standards. They are the
Layer 3 (Harness) verification mechanism. Each gate receives an Engram
and returns a Verdict: pass, fail, or skip. Gates run in sequence as a
pipeline. If any gate fails, the agent can retry with the failure
context injected.

Common gates: compile, test, clippy, diff, symbol, LLM-judge.

Press Enter for more, or q to exit.
```

**Level 2 — Detailed explanation** (on Enter):
```
HOW THE GATE PIPELINE WORKS
────────────────────────────
1. Agent produces output Engram
2. Pipeline runs gates in order: compile → test → clippy → ...
3. Each gate returns Verdict { passed, reason, evidence }
4. If all pass → output is persisted, episode logged
5. If any fail → failure context injected, agent retries
6. After max retries → task marked failed, conductor notified

ADAPTIVE THRESHOLDS
───────────────────
Gate thresholds are not static. The adaptive threshold system uses an
Exponential Moving Average (EMA) per rung, persisted in
`.roko/learn/gate-thresholds.json`. Thresholds tighten as the agent
improves and relax when it struggles, preventing both complacency and
frustration.

6-RUNG PIPELINE
───────────────
Rung 0: Syntax    (compile, parse)
Rung 1: Semantics (tests, type checks)
Rung 2: Style     (clippy, lint, format)
Rung 3: Safety    (security audit, SAST)
Rung 4: Quality   (diff review, complexity)
Rung 5: Judge     (LLM-based review)

Press Enter for more, or q to exit.
```

**Level 3 — Configuration and advanced topics** (on Enter again):
```
CONFIGURING GATES
─────────────────
In roko.toml:

[gates]
pipeline = ["compile", "test", "clippy"]

[gates.compile]
command = "cargo build"
timeout_ms = 60000

[gates.test]
command = "cargo test"
timeout_ms = 120000

CREATING CUSTOM GATES
─────────────────────
roko new gate <name>    # generates a working Gate implementation

See the developer guide for the full Gate trait:
  trait Gate {
      async fn verify(&self, output: &Signal, context: &Context) -> Verdict;
  }
```

---

## Error-as-Teacher Format

Every error message in Roko follows the **error-as-teacher** format. Instead of cryptic error codes or stack traces, errors explain what happened, why it matters, and how to fix it.

### Format

Every error message has four sections:

```
ERROR: <what happened>
───────────────────────
WHY: <why this matters>
FIX: <exactly what to do>
CTX: <additional context or documentation link>
```

### Examples

```
ERROR: Gate 'compile' failed — `cargo build` exited with code 1
───────────────────────────────────────────────────────────────
WHY: The agent's code changes introduced a compilation error.
     The output was not persisted and the task was not marked complete.
FIX: The agent will retry with the compiler error in context.
     If retries are exhausted, check the agent output with:
       roko episode list --failed
CTX: Gate pipeline docs: roko explain gates
```

```
ERROR: No provider configured — cannot dispatch agent
──────────────────────────────────────────────────────
WHY: Roko needs at least one LLM provider to run agents.
     No providers were found in roko.toml or environment.
FIX: Run the config wizard to set up a provider:
       roko config wizard
     Or add a provider manually to roko.toml:
       [providers.anthropic]
       api_key_env = "ANTHROPIC_API_KEY"
CTX: Provider setup: roko explain routing
```

```
ERROR: Model 'claude-opus-4-6' rate-limited — circuit breaker OPEN
──────────────────────────────────────────────────────────────
WHY: Provider returned HTTP 429. The circuit breaker has opened
     to prevent further failed requests. It will half-open in 5s.
FIX: No action needed — the cascade router will try the next
     tier model automatically. To check provider health:
       roko provider health
CTX: Rate limiting and circuit breakers: roko explain routing
```

### Implementation

The error format is implemented as a Rust struct:

```rust
pub struct TeachingError {
    pub what: String,
    pub why: String,
    pub fix: String,
    pub context: Option<String>,
}

impl std::fmt::Display for TeachingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ERROR: {}", self.what)?;
        writeln!(f, "{}", "─".repeat(self.what.len() + 7))?;
        writeln!(f, "WHY: {}", self.why)?;
        writeln!(f, "FIX: {}", self.fix)?;
        if let Some(ctx) = &self.context {
            writeln!(f, "CTX: {}", ctx)?;
        }
        Ok(())
    }
}
```

---

## Interactive Config Wizard

`roko config wizard` walks the user through initial setup with an interactive terminal questionnaire. It detects the project environment and pre-fills sensible defaults.

### Flow

```
$ roko config wizard

ROKO CONFIGURATION WIZARD
─────────────────────────

[1/5] PROJECT DETECTION
  Language:    Rust (detected Cargo.toml)
  Build:       cargo build
  Test:        cargo test
  Lint:        cargo clippy
  ✓ Auto-configured gates: compile, test, clippy

[2/5] MODEL PROVIDER
  ? Which LLM provider? [anthropic / openai / ollama / openrouter]
  > anthropic
  ? API key (or env var name): ANTHROPIC_API_KEY
  ✓ Provider configured

[3/5] MODEL ROUTING
  ? Default model: [claude-sonnet-4-6]
  ? Enable cascade routing? [Y/n]
  ✓ Cascade: haiku → sonnet → opus

[4/5] COGNITIVE FEATURES
  ? Enable Neuro (knowledge persistence)? [Y/n]
  ? Enable Daimon (affect tracking)? [Y/n]
  ? Enable Dreams (offline consolidation)? [y/N]
  ✓ Neuro and Daimon enabled

[5/5] SUMMARY
  Config written to: ./roko.toml
  Data directory:    ./.roko/
  Ready to run:      roko run "your prompt here"
```

---

## Current Status and Gaps

- `roko status` — **Implemented** and functional
- `roko explain` — **Not yet implemented** (Tier 4)
- Error-as-teacher format — **Partially implemented** (some commands use structured errors; not systematic)
- `roko config wizard` — **Not yet implemented** (Tier 4)

---

## Cross-references

- See [00-cli-overview.md](./00-cli-overview.md) for CLI design principles
- See [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md) for config resolution
- See topic [04-verification](../04-verification/INDEX.md) for gate pipeline details
- See topic [02-agents](../02-agents/INDEX.md) for model routing
