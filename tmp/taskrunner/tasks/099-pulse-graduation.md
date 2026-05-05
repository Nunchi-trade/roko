# Task 099: Pulse Graduation + GraduationCell

```toml
id = 99
title = "Add Pulse::graduate(), graduation policy config, and GraduationCell React Cell"
track = "v2-core-abstractions"
wave = "wave-4"
priority = "high"
blocked_by = [67, 97]
touches = [
    "Cargo.toml",
    "crates/roko-core/src/pulse.rs",
    "crates/roko-core/src/config/graduation.rs",
    "crates/roko-core/src/config/schema.rs",
    "crates/roko-core/src/config/mod.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-graph/src/cells/mod.rs",
    "crates/roko-graph/src/cells/graduation.rs",
    "crates/roko-graph/src/lib.rs",
    "crates/roko-graph/Cargo.toml",
]
exclusive_files = ["crates/roko-graph/src/cells/graduation.rs"]
estimated_minutes = 240
```

## Context

Graduation is the controlled promotion of ephemeral `Pulse`s to durable
`Signal`s in the Store. Without graduation policies, the system
either logs everything (wasteful) or loses important events (silently).
Graduation policies formalize the decision: each policy declares which Bus
topics should always/never/sometimes graduate.

This task covers three checklist items:

- **P3-5**: Add `Pulse::graduate()` — the conversion method
- **P3-6**: Add `[graduation]` config section to `roko.toml`
- **P3-7**: Implement `GraduationCell` — a React Cell that watches the Bus
  and promotes matching Pulses to the Store

Checklist items: P3-5, P3-6, P3-7.

## Background

Read these files before starting:

1. `crates/roko-core/src/pulse.rs` — the full `Pulse` struct and
   `TopicFilter`. The `graduate()` method goes on `Pulse` directly.
2. `crates/roko-core/src/engram.rs` — the `Signal` struct. Pay attention
   to how `Signal::builder()` works — `graduate()` should use the builder
   pattern or construct the Signal directly. Note the `from_pulse_synthetic`
   method already on `Signal` — `graduate()` is the deliberate promotion
   path (with provenance/score), while `from_pulse_synthetic` is a
   read-only scoring helper.
3. `crates/roko-core/src/config/schema.rs` — the top-level `RokoConfig`
   struct and its field layout. The new `graduation` section goes here.
4. `crates/roko-core/src/config/mod.rs` — the re-export list. New config
   types must be added here.
5. `crates/roko-core/src/traits.rs` — the `React` trait with `decide()`
   and `decide_with_pulses()`. `GraduationCell` implements `React`.
6. `crates/roko-core/src/cell.rs` — `Cell` trait (GraduationCell must
   implement this too).
7. `tmp/v2-refactoring/09-GRADUATION.md` — the full graduation design spec
   with default policy tables.

Note: `roko-graph` does **not yet exist** as a crate. Check:
```bash
ls crates/roko-graph/
```
If the crate is missing, you must create it from scratch (see step 5 below).
If it already exists (another task created it), add to it.

## Current Checkout Corrections

These notes are authoritative for this checkout and override stale examples below:

- `crates/roko-graph` does not exist in this checkout. Creating it requires adding the
  workspace member to root `Cargo.toml`, so that file is part of this task's touch list.
- `Signal` is currently an alias for `Engram` (`crates/roko-core/src/signal.rs`). Core
  structs and `PolicyOutputs` still use `Engram` terminology in many APIs.
- `PolicyOutputs` has fields `engrams: Vec<Engram>` and `pulses: Vec<Pulse>`, not
  `signals`. `GraduationCell::decide_with_pulses()` must return
  `PolicyOutputs { engrams: graduated, pulses: Vec::new() }`.
- `TopicFilter` currently has only `Exact`, `Prefix`, and `All`. Do not implement wildcard,
  suffix, boolean, or glob filters in this task. The default `agent.*.turn.completed` and
  `agent.*.output` policies from the design spec are not exactly representable; document
  them as deferred unless a prior task has expanded `TopicFilter`.
- `EngramBuilder` already supports `.body()`, `.provenance()`, `.score()`,
  `.created_at_ms()`, `.lineage()`, `.tag()`, and `.build()`. `Pulse::graduate()` should
  use the builder, copy all pulse tags, and add audit tags such as `pulse_topic` and
  `pulse_seq`.
- `Score` is the value struct at `roko_core::Score`; the protocol trait named `Score` is
  not re-exported at the root. Use `roko_core::Score::NEUTRAL` or `Score::default()`.
- `Provenance::trusted("graduation-policy")` and `Provenance::agent(...)` exist. Do not
  construct `Provenance` by guessing private/default fields.
- `RokoConfig` has an explicit `Default` impl. Adding `graduation` to the struct also
  requires adding it to that impl.
- TOML shaped as `[[graduation.policies]]` parses through `RokoConfig`, not directly into
  `GraduationConfig`. A direct `GraduationConfig` test must use `[[policies]]`.

## Recovery Worker 19 Checkout Notes

Use these corrections when implementing the snippets below:

- Root `Cargo.toml` and existing crate manifests use workspace package fields. If
  `crates/roko-graph` is still missing, create its manifest in the local style:
  `edition.workspace = true`, `rust-version.workspace = true`, `license.workspace = true`,
  `authors.workspace = true`, and `[lints] workspace = true`. Do not hardcode only
  `edition = "2021"` unless the surrounding workspace has changed.
- `React` in `crates/roko-core/src/traits.rs` is synchronous:
  `decide(&[Engram], &Context) -> Vec<Engram>` and
  `decide_with_pulses(&[Engram], &[Pulse], &Context) -> PolicyOutputs`. Use `Engram`/`Signal`
  aliases consistently, but return through `PolicyOutputs.engrams`.
- Policy evaluation must scan all matching policies before deciding. A later `never` policy
  must block an earlier `always` policy, so do not return on the first matching `always`.
  Mechanical rule: collect whether any matching policy has `never`, whether any has `always`,
  and the first/lowest `sample_every`; then apply `never`, then `always`, then sampling.
- Prefer sampling from `pulse.seq` for deterministic behavior across restarts and tests.
  The `AtomicU64` counter in the example is optional telemetry; if it is kept, do not use
  `load()` before `fetch_add()` as the sampling sequence because that creates an off-by-one
  first pulse.
- `Pulse::graduate()` should use `Engram::builder(self.kind.clone())`, copy `body`,
  `created_at_ms`, `provenance`, `score`, every existing pulse tag, and audit tags
  `pulse_topic`/`pulse_seq`. Adding these tags changes the content hash, which is intended;
  do not add fake lineage because there is no source engram hash.
- `Provenance::default()` is not the recommended path here. Tests and cell code should use
  `Provenance::trusted("graduation-policy")` or `Provenance::agent("...")`, both of which
  exist in this checkout.
- Config parsing for enum filters uses serde's externally tagged shape, e.g.
  `watch = { Prefix = "gate.verdict." }`, `watch = { Exact = "cost.charged" }` if serde for
  `Topic` supports the string shape, or the exact shape produced by `toml::to_string` for
  `TopicFilter::Exact(Topic::new(...))`. Add a focused test so the final TOML shape is proven.
- Add `pub use config::graduation::{GraduationConfig, GraduationPolicy};` in
  `crates/roko-core/src/lib.rs` in addition to the `config/mod.rs` re-export. Existing root
  exports already include `FeedRegistry`/`FeedKind`; extend rather than rearranging unrelated
  exports.

## Mechanical Implementation Plan

1. Add `crates/roko-core/src/config/graduation.rs` with
   `GraduationConfig { policies: Vec<GraduationPolicy> }` and
   `GraduationPolicy { watch: TopicFilter, always: bool, never: bool, sample_every: usize }`.
   Implement `Default` for `GraduationConfig` as `default_policies()`.
2. Policy precedence is: no topic match -> false, `never` -> false, `always` -> true,
   otherwise sample with `sample_every.max(1)`. Never must win over always to prevent noisy
   streams from being persisted accidentally.
3. Add only representable defaults: always `Prefix("gate.verdict.")`,
   `Prefix("safety.approval.")`, `Prefix("conductor.circuit.")`, exact `cost.charged`, and
   never `Prefix("heartbeat.")`. Defer the agent wildcard policies until `TopicFilter`
   supports them.
4. In `config/mod.rs`, add `pub mod graduation;` and re-export
   `GraduationConfig`/`GraduationPolicy`. In `schema.rs`, add
   `pub graduation: GraduationConfig` to `RokoConfig` and its default impl.
5. Add `Pulse::graduate(&self, provenance: Provenance, score: Score) -> Signal` in
   `pulse.rs`. Preserve kind, body, created_at_ms, provenance, score, pulse tags, and add
   pulse audit tags. There is no source engram content hash for lineage, so do not fabricate
   one.
6. Create `roko-graph` only if still missing: Cargo.toml, `src/lib.rs`,
   `src/cells/mod.rs`, `src/cells/graduation.rs`, and root workspace member.
7. Implement `GraduationCell` with `Cell` and sync `React`. It should inspect pulses in
   `decide_with_pulses()`, graduate matching pulses, and leave incoming signals unchanged.
8. Add tests for policy precedence, default policies, config parsing via `RokoConfig`,
   `Pulse::graduate()` preservation, and `GraduationCell` output field names.

## What to Change

### 1. Add `Pulse::graduate()` to `crates/roko-core/src/pulse.rs`

```rust
use crate::{Provenance, Score, Signal};

impl Pulse {
    /// Promote this ephemeral Pulse to a durable Signal.
    ///
    /// This is the deliberate graduation path — the only way to get a Pulse
    /// into the Store. Unlike `Signal::from_pulse_synthetic()` (which is a
    /// read-only scoring helper), `graduate()` carries explicit provenance
    /// and score metadata for the audit trail.
    ///
    /// # Arguments
    ///
    /// * `provenance` — who or what decided to graduate this Pulse
    /// * `score` — the initial relevance/importance score for the Signal
    pub fn graduate(&self, provenance: Provenance, score: Score) -> Signal {
        let mut builder = Signal::builder(self.kind.clone())
            .body(self.body.clone())
            .provenance(provenance)
            .score(score)
            .created_at_ms(self.created_at_ms)
            .tag("pulse_topic", self.topic.to_string())
            .tag("pulse_seq", self.seq.to_string());
        for (key, value) in &self.tags {
            builder = builder.tag(key.clone(), value.clone());
        }
        builder.build()
    }
}
```

**Check the `Signal::builder()` API** in `engram.rs` before writing this.
The builder may not have a `provenance()` or `score()` method yet — if not,
add the fields directly in the `Signal` construction. Add a unit test to
`pulse.rs` (see step 6).

### 2. Add `GraduationConfig` to `crates/roko-core/src/config/schema.rs`

Add a new config module `crates/roko-core/src/config/graduation.rs`:

```rust
//! Graduation policy configuration — which Pulses get promoted to Signals.

use serde::{Deserialize, Serialize};
use crate::pulse::TopicFilter;

/// A single graduation policy: watch these topics, apply these criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationPolicy {
    /// Bus topic(s) to watch.
    pub watch: TopicFilter,

    /// Always graduate Pulses matching this policy. `never` still wins when
    /// multiple matching policies apply.
    #[serde(default)]
    pub always: bool,

    /// Never graduate Pulses matching this policy.
    #[serde(default)]
    pub never: bool,

    /// Graduated every Nth matching Pulse (1 = every pulse, 10 = 10%).
    /// Ignored when `always` or `never` is set.
    #[serde(default = "default_sample_every")]
    pub sample_every: usize,
}

fn default_sample_every() -> usize { 1 }

impl GraduationPolicy {
    /// Should this Pulse be graduated according to this policy?
    pub fn should_graduate(&self, topic: &crate::pulse::Topic, seq: u64) -> bool {
        if !self.watch.matches(topic) {
            return false;
        }
        if self.never {
            return false;
        }
        if self.always {
            return true;
        }
        // Sample: graduate every Nth pulse.
        let n = self.sample_every.max(1) as u64;
        seq % n == 0
    }
}

/// Top-level graduation config section in `roko.toml`.
///
/// # Example TOML
///
/// ```toml
/// [[graduation.policies]]
/// watch = { Prefix = "gate.verdict." }
/// always = true
///
/// [[graduation.policies]]
/// watch = { Prefix = "heartbeat." }
/// never = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraduationConfig {
    #[serde(default)]
    pub policies: Vec<GraduationPolicy>,
}

impl GraduationConfig {
    /// Return the default graduation policies from the v2 spec.
    ///
    /// These match the "always" and "never" tables in 09-GRADUATION.md.
    pub fn default_policies() -> Self {
        Self {
            policies: vec![
                // Always graduate these:
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.verdict.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("safety.approval.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Prefix("conductor.circuit.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                GraduationPolicy {
                    watch: TopicFilter::Exact(crate::pulse::Topic::new("cost.charged")),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
                // Never graduate these:
                GraduationPolicy {
                    watch: TopicFilter::Prefix("heartbeat.".into()),
                    always: false,
                    never: true,
                    sample_every: 1,
                },
            ],
        }
    }
}

impl Default for GraduationConfig {
    fn default() -> Self {
        Self::default_policies()
    }
}
```

Register the new module in `crates/roko-core/src/config/mod.rs`:

```rust
pub mod graduation;
pub use graduation::{GraduationConfig, GraduationPolicy};
```

Add the field to `RokoConfig` in `schema.rs`:

```rust
/// Graduation policies: which Bus topics get promoted to the Store.
#[serde(default)]
pub graduation: crate::config::graduation::GraduationConfig,
```

Find the existing `RokoConfig` struct definition and insert this field
in alphabetical order alongside the other optional sections.

### 3. Re-export from `crates/roko-core/src/lib.rs`

```rust
pub use config::graduation::{GraduationConfig, GraduationPolicy};
```

Add alongside the other `pub use config::` lines.

### 4. Create `crates/roko-graph/` crate (if it does not exist)

Check first:
```bash
ls crates/roko-graph/
```

If missing, create the minimal crate structure:

**`crates/roko-graph/Cargo.toml`**:
```toml
[package]
name = "roko-graph"
version = "0.1.0"
edition = "2021"

[dependencies]
roko-core = { path = "../roko-core" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
parking_lot = "0.12"
tokio = { version = "1", features = ["sync", "time"] }

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt"] }
```

Add to workspace `Cargo.toml`:
```toml
"crates/roko-graph",
```

**`crates/roko-graph/src/lib.rs`**:
```rust
//! roko-graph: Graph engine, Node types, and registered Cells.
//!
//! Phase 2+ (see v2-refactoring/CHECKLIST.md P2-*) builds the full Engine
//! here.  Phase 3 (this task) adds the GraduationCell as the first
//! resident Cell.

pub mod cells;
```

**`crates/roko-graph/src/cells/mod.rs`**:
```rust
pub mod graduation;
```

### 5. Implement `GraduationCell` in `crates/roko-graph/src/cells/graduation.rs`

```rust
//! GraduationCell — a React Cell that promotes qualifying Pulses to Signals.
//!
//! GraduationCell runs as a background React Cell in the Engine.  On each
//! tick it:
//! 1. Receives Pulses from the Bus (via `decide_with_pulses`)
//! 2. Evaluates each Pulse against the configured GraduationPolicies
//! 3. Calls `Pulse::graduate()` for matching Pulses
//! 4. Outputs the graduated Signals for the Engine to persist via the Store

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use roko_core::cell::Cell;
use roko_core::config::graduation::{GraduationConfig, GraduationPolicy};
use roko_core::error::Result;
use roko_core::pulse::{Topic, TopicFilter};
use roko_core::traits::React;
use roko_core::{
    Context, Signal, PolicyOutputs, Provenance, Pulse,
};

/// Watches the Bus and promotes qualifying Pulses to Signals.
///
/// Register this cell with the Engine at startup to enable automatic
/// graduation.  The policies are loaded from `roko.toml` `[graduation]`
/// section via `GraduationConfig`.
pub struct GraduationCell {
    /// Configured graduation policies.
    policies: Vec<GraduationPolicy>,
    /// Rolling counter used for sample_every calculations.
    pulse_counter: AtomicU64,
    /// Provenance tag attached to all graduated Signals.
    provenance_tag: String,
}

impl GraduationCell {
    /// Create a new GraduationCell with the given policies.
    pub fn new(config: GraduationConfig) -> Self {
        Self {
            policies: config.policies,
            pulse_counter: AtomicU64::new(0),
            provenance_tag: "graduation-policy".into(),
        }
    }

    /// Create a GraduationCell with the default v2 spec policies.
    pub fn with_default_policies() -> Self {
        Self::new(GraduationConfig::default_policies())
    }

    /// Evaluate a single Pulse against all policies and return whether it
    /// should graduate.
    pub fn should_graduate(&self, pulse: &Pulse) -> bool {
        let seq = self.pulse_counter.load(Ordering::Relaxed);
        for policy in &self.policies {
            if policy.watch.matches(&pulse.topic) {
                return policy.should_graduate(&pulse.topic, seq);
            }
        }
        // Default: do not graduate if no policy matches.
        false
    }

    /// Graduate a Pulse to a Signal with a default provenance and score.
    fn graduate_pulse(&self, pulse: &Pulse) -> Signal {
        // Use a neutral score (0.5) for graduated events — downstream
        // scorers will re-score based on content.
        let score = roko_core::Score::NEUTRAL;
        let provenance = Provenance::trusted(self.provenance_tag.clone());
        pulse.graduate(provenance, score)
    }
}

impl Cell for GraduationCell {
    fn cell_id(&self) -> &str { "graduation-policy" }
    fn cell_name(&self) -> &str { "Graduation Policy" }
    fn protocols(&self) -> &[&str] { &["React"] }
}

impl React for GraduationCell {
    fn decide(&self, _stream: &[Signal], _ctx: &Context) -> Vec<Signal> {
        // React::decide only receives Signals; graduation is pulse-driven.
        // Return empty — the real work happens in decide_with_pulses.
        Vec::new()
    }

    fn decide_with_pulses(
        &self,
        signals: &[Signal],
        pulses: &[Pulse],
        ctx: &Context,
    ) -> PolicyOutputs {
        let mut graduated = Vec::new();

        for pulse in pulses {
            self.pulse_counter.fetch_add(1, Ordering::Relaxed);
            if self.should_graduate(pulse) {
                graduated.push(self.graduate_pulse(pulse));
            }
        }

        PolicyOutputs {
            engrams: graduated,
            pulses: Vec::new(),
        }
    }

    fn name(&self) -> &str { "graduation-policy" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::config::graduation::{GraduationConfig, GraduationPolicy};
    use roko_core::pulse::{Topic, TopicFilter};
    use roko_core::{Body, Kind};

    fn make_pulse(topic: &str, seq: u64) -> Pulse {
        Pulse::builder(seq, Topic::new(topic), Kind::GateVerdict)
            .body(Body::text("test"))
            .build()
    }

    #[test]
    fn always_policy_graduates_matching_pulses() {
        let config = GraduationConfig {
            policies: vec![
                GraduationPolicy {
                    watch: TopicFilter::Prefix("gate.verdict.".into()),
                    always: true,
                    never: false,
                    sample_every: 1,
                },
            ],
        };
        let cell = GraduationCell::new(config);
        let pulse = make_pulse("gate.verdict.emitted", 1);
        assert!(cell.should_graduate(&pulse));
    }

    #[test]
    fn never_policy_blocks_matching_pulses() {
        let config = GraduationConfig {
            policies: vec![
                GraduationPolicy {
                    watch: TopicFilter::Prefix("heartbeat.".into()),
                    always: false,
                    never: true,
                    sample_every: 1,
                },
            ],
        };
        let cell = GraduationCell::new(config);
        let pulse = make_pulse("heartbeat.tick", 1);
        assert!(!cell.should_graduate(&pulse));
    }

    #[test]
    fn no_matching_policy_does_not_graduate() {
        let cell = GraduationCell::with_default_policies();
        let pulse = make_pulse("agent.output.token", 1);
        // "agent.output.*" is not in the default policy list.
        // Default = do not graduate.
        assert!(!cell.should_graduate(&pulse));
    }

    #[test]
    fn decide_with_pulses_promotes_to_signals() {
        let cell = GraduationCell::with_default_policies();
        let ctx = Context::default();

        let graduating = make_pulse("gate.verdict.emitted", 1);
        let blocked = make_pulse("heartbeat.tick", 2);

        let outputs = cell.decide_with_pulses(&[], &[graduating, blocked], &ctx);

        assert_eq!(outputs.engrams.len(), 1);
        assert!(outputs.pulses.is_empty());
    }

    #[test]
    fn graduate_method_preserves_kind_and_body() {
        let cell = GraduationCell::with_default_policies();
        let pulse = make_pulse("gate.verdict.emitted", 1);
        let signal = cell.graduate_pulse(&pulse);

        assert_eq!(signal.kind, Kind::GateVerdict);
        assert_eq!(signal.body, Body::text("test"));
    }
}
```

### 6. Add unit test for `Pulse::graduate()` in `crates/roko-core/src/pulse.rs`

```rust
#[cfg(test)]
mod graduation_tests {
    use super::*;
    use crate::{Body, Kind, Provenance};
    use crate::Score;

    #[test]
    fn graduate_preserves_kind_body_and_timestamp() {
        let pulse = Pulse::builder(1, Topic::new("gate.verdict.emitted"), Kind::GateVerdict)
            .body(Body::text("passed"))
            .created_at_ms(99999)
            .build();

        let signal = pulse.graduate(Provenance::trusted("graduation-policy"), Score::default());

        assert_eq!(signal.kind, Kind::GateVerdict);
        assert_eq!(signal.body, Body::text("passed"));
        assert_eq!(signal.created_at_ms, 99999);
    }

    #[test]
    fn graduated_signals_have_distinct_content_hashes() {
        let p1 = Pulse::builder(1, Topic::new("gate.verdict"), Kind::GateVerdict)
            .body(Body::text("passed"))
            .build();
        let p2 = Pulse::builder(2, Topic::new("gate.verdict"), Kind::GateVerdict)
            .body(Body::text("failed"))
            .build();

        let e1 = p1.graduate(Provenance::trusted("graduation-policy"), Score::default());
        let e2 = p2.graduate(Provenance::trusted("graduation-policy"), Score::default());

        // Different bodies → different content hashes.
        assert_ne!(e1.content_hash(), e2.content_hash());
    }
}
```

## What NOT to Do

- Do NOT implement the Engine itself (the scheduler, graph loader, TOML
  graph parser). That is Phase 2 (P2-1 through P2-8). This task adds the
  `GraduationCell` implementation that will be registered with the Engine
  when Phase 2 lands.
- Do NOT add a `roko learn graduation` subcommand. The design spec mentions
  it as a nice-to-have (Step 5 in 09-GRADUATION.md) but it is not part of
  P3-5, P3-6, or P3-7. Add a TODO comment instead.
- Do NOT change how existing agents or gates produce signals. Graduation is
  an additive path alongside existing `Store::put()` calls.
- Do NOT remove the `Signal::from_pulse_synthetic()` helper. It is a
  different operation (read-only scoring adapter) used by `Score` trait
  impls. `Pulse::graduate()` is the write path.
- Do NOT add `parking_lot` to `roko-core` if it is not already there.
  The `GraduationCell` uses an `AtomicU64` which requires no new deps in
  roko-core.
- If `roko-graph` does not yet exist, create only the minimal structure
  needed. Do not implement the Graph/Node/Edge types — those are P2-1.

## Wire Target

```bash
# Test graduation logic in roko-graph:
cargo test -p roko-graph -- graduation
# Expected: 5 new tests pass

# Test Pulse::graduate() in roko-core:
cargo test -p roko-core -- graduation_tests
# Expected: 2 new tests pass

# Verify config parses graduation section:
cargo test -p roko-core -- graduation_config
# (Add this test to config/graduation.rs, see below)
```

Add config parsing test to `graduation.rs`:

```rust
#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn default_policies_loaded() {
        let cfg = GraduationConfig::default_policies();
        assert!(!cfg.policies.is_empty());
        // At least one always-graduate policy.
        assert!(cfg.policies.iter().any(|p| p.always));
        // At least one never-graduate policy.
        assert!(cfg.policies.iter().any(|p| p.never));
    }

    #[test]
    fn roko_config_graduation_section_parses() {
        let toml_str = r#"
            [[graduation.policies]]
            watch = { Prefix = "gate.verdict." }
            always = true

            [[graduation.policies]]
            watch = { Prefix = "heartbeat." }
            never = true
        "#;

        let cfg: crate::config::schema::RokoConfig =
            toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.graduation.policies.len(), 2);
    }
}
```

## Verification

- [ ] `cargo build --workspace` — clean
- [ ] `cargo test --workspace` — no regressions
- [ ] `cargo clippy --workspace --no-deps -- -D warnings` — clean
- [ ] `cargo test -p roko-core -- graduation_tests` — 2 new tests pass
- [ ] `cargo test -p roko-core -- graduation_config` — 2 config tests pass
- [ ] `cargo test -p roko-graph -- graduation` — 5 GraduationCell tests pass
- [ ] `grep -n 'pub fn graduate' crates/roko-core/src/pulse.rs` — method exists
- [ ] `grep -n 'GraduationConfig' crates/roko-core/src/config/schema.rs` — field on RokoConfig
- [ ] `grep -n 'GraduationCell' crates/roko-graph/src/cells/graduation.rs` — struct exists
- [ ] `grep -n 'roko-graph' Cargo.toml` — workspace member (if crate was created)
- [ ] Existing `TopicFilter` tests in `pulse.rs` still pass (no regression)

## Status Log

| Time | Agent | Action |
|------|-------|--------|
