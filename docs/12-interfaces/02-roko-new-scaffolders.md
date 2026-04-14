# `roko new` Scaffolders

> Generate working boilerplate for every Synapse trait, domain plugin, and extension point — every scaffold compiles immediately with passing tests.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [00-cli-overview.md](./00-cli-overview.md), [00-architecture](../00-architecture/INDEX.md) for Synapse traits
**Key sources**: `refactoring-prd/06-interfaces.md` §1, `refactoring-prd/10-developer-guide.md` §2-3

---

## Abstract

The `roko new` command family generates complete, compilable implementations for every extension point in the Roko framework. Unlike scaffolding systems that produce empty stubs, every `roko new` output is a working implementation with passing tests that can be immediately compiled and run. This follows the design principle "generators, not blank files" from the developer guide.

The scaffolders target the six Synapse traits (`Substrate`, `Scorer`, `Gate`, `Router`, `Composer`, `Policy`) plus domain plugins, T0 Probes, EventSource plugins, and agent templates. Each generated file includes inline documentation explaining the trait contract, the generated implementation, and how to customize it.

This command group is planned for Tier 4 (Interfaces) in `refactoring-prd/07-implementation-priorities.md` and is not yet implemented. The specifications below define the target behavior.

---

## Scaffold Types

### `roko new domain <name>`

Generates a complete domain plugin — the largest scaffold. A domain plugin encapsulates everything needed to extend Roko into a new problem space.

```bash
roko new domain medical
```

Generated structure:

```
roko-domain-medical/
├── Cargo.toml              # Crate with roko-core dependency
├── src/
│   ├── lib.rs              # Domain registration
│   ├── gates/
│   │   └── compliance.rs   # Example domain gate
│   ├── probes/
│   │   └── vitals_check.rs # Example T0 probe
│   ├── tools/
│   │   └── lookup.rs       # Example domain tool
│   └── templates/
│       └── default.toml    # Default agent template
└── tests/
    └── integration.rs      # Tests that compile and pass
```

### `roko new gate <name>`

Generates a custom Gate implementation. Gates are the L3 Harness verification layer — they check Engrams against ground truth and return a `Verdict`.

```bash
roko new gate schema-validator
```

Generated file (`gates/schema_validator.rs`):

```rust
use async_trait::async_trait;
use roko_core::{Context, Gate, Signal, Verdict};

/// SchemaValidator gate — validates output against a JSON schema.
///
/// # How to customize
/// 1. Add your schema to the `schema` field
/// 2. Implement your validation logic in `verify()`
/// 3. Return Verdict::pass() or Verdict::fail() with a reason
pub struct SchemaValidatorGate {
    /// The JSON schema to validate against.
    pub schema: serde_json::Value,
}

impl SchemaValidatorGate {
    pub fn new(schema: serde_json::Value) -> Self {
        Self { schema }
    }
}

#[async_trait]
impl Gate for SchemaValidatorGate {
    async fn verify(
        &self,
        output: &Signal, // will be renamed to Engram in Tier 0D
        _context: &Context,
    ) -> Verdict {
        // TODO: Replace with your validation logic
        let body = output.body.as_text().unwrap_or_default();
        if body.is_empty() {
            Verdict::fail("Output body is empty")
        } else {
            Verdict::pass("Schema validation passed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind, Signal};

    #[tokio::test]
    async fn passes_non_empty_output() {
        let gate = SchemaValidatorGate::new(serde_json::json!({}));
        let signal = Signal::builder(Kind::AgentOutput)
            .body(Body::text("valid output"))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(verdict.passed);
    }

    #[tokio::test]
    async fn fails_empty_output() {
        let gate = SchemaValidatorGate::new(serde_json::json!({}));
        let signal = Signal::builder(Kind::AgentOutput)
            .body(Body::text(""))
            .build();
        let verdict = gate.verify(&signal, &Context::now()).await;
        assert!(!verdict.passed);
    }
}
```

The generated code uses `Signal` (the current Rust type name) with a comment noting the planned rename to `Engram` in Tier 0D.

### `roko new scorer <name>`

Generates a custom Scorer that rates Engrams along the 7-axis Score.

```bash
roko new scorer recency-weighted
```

### `roko new router <name>`

Generates a custom Router with the `select()` and `feedback()` methods.

```bash
roko new router domain-specific
```

### `roko new policy <name>`

Generates a custom Policy that observes Engram streams and emits new Engrams.

```bash
roko new policy anomaly-detector
```

### `roko new substrate <name>`

Generates a custom Substrate for persistence — the L0 Runtime storage layer.

```bash
roko new substrate postgres
```

### `roko new probe <name>`

Generates a T0 Probe — a zero-LLM deterministic check that runs at every gamma tick.

```bash
roko new probe memory-usage
```

Generated probe:

```rust
use roko_core::EngineState;

/// MemoryUsage probe — checks system memory pressure.
///
/// T0 probes run at every gamma tick (~5-15s) with zero LLM cost.
/// They return a prediction error scalar [0.0, 1.0] that drives
/// T0/T1/T2 tier gating.
///
/// error < 0.2  → T0 (suppress, no LLM)     ~80% of ticks
/// error < 0.6  → T1 (fast model, shallow)   ~15% of ticks
/// error ≥ 0.6  → T2 (full model, deep)      ~5% of ticks
pub fn probe(state: &EngineState) -> f32 {
    // TODO: Replace with your probe logic
    // Return a value in [0.0, 1.0] representing prediction error
    0.1 // Low error = no LLM needed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_returns_valid_range() {
        let state = EngineState::default();
        let result = probe(&state);
        assert!((0.0..=1.0).contains(&result));
    }
}
```

### `roko new event-source <name>`

Generates an EventSource plugin for the `roko-plugin` SDK.

```bash
roko new event-source github-webhook
```

### `roko new template <name>`

Generates an agent template with system prompt, model configuration, and gate pipeline.

```bash
roko new template code-reviewer
```

---

## Design Principles

1. **Every scaffold compiles immediately** — no placeholder `todo!()` macros, no unresolved imports, no missing dependencies. The generated code is a valid, testable implementation.

2. **Tests pass out of the box** — every scaffold includes at least two tests that pass. The user can run `cargo test` immediately after generation and see green.

3. **Inline documentation** — every generated file includes doc comments explaining the trait contract, the generated implementation logic, and clear `TODO` markers indicating where the user should add their custom logic.

4. **Minimal dependencies** — scaffolds depend only on `roko-core` and standard library types. Domain-specific dependencies are commented out with instructions for adding them.

5. **Consistent structure** — all scaffolds follow the same file layout: type definition, constructor, trait implementation, test module. Users who have seen one scaffold can navigate any other.

---

## Current Status and Gaps

The `roko new` command family is **not yet implemented**. It is planned for Tier 4 (Interfaces) in the implementation roadmap. The specifications above define the target behavior.

**Implementation path**: The scaffolders will be added to `roko-cli/src/` as a new `scaffold.rs` module, using string templates with variable interpolation. Each scaffold type corresponds to a template file embedded in the binary via `include_str!`.

---

## Cross-References

- See [00-cli-overview.md](./00-cli-overview.md) for CLI architecture
- See topic [00-architecture](../00-architecture/INDEX.md) for the six Synapse traits
- See topic [04-verification](../04-verification/INDEX.md) for Gate trait details
- See topic [18-tools](../18-tools/INDEX.md) for tool definition format
