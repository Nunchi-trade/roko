# Compositional kinds

> Layer 0 Kernel -- Signal Type System
> Status: **Specification** -- migration path from flat enum to compound kinds
> Canonical source: `crates/roko-core/src/kind.rs`
> Cross-references: [02-engram-data-type.md](02-engram-data-type.md), [08-scorer-gate-router-composer-policy.md](08-scorer-gate-router-composer-policy.md)

> **Implementation**: Specified

---

## Purpose

The current `Kind` enum is flat: a Signal is a `GateVerdict` or a `PromptSection` or a `Task`, but it cannot be both. Some signals naturally span multiple domains -- a gate verdict that contains a test result, a prompt section derived from an episode, a task that carries routing metadata. Today these cross-domain signals are forced into a single Kind with extra tags for the secondary domain.

This document specifies `Kind::Compound(Vec<Kind>)` -- a compositional extension to the Kind system that lets a Signal carry multiple kinds simultaneously. It defines the semantics, dispatch rules, migration path, and backwards-compatibility constraints.

The same composition model also gives learning-layer artifacts a clean type-level home when a record needs to carry both heuristic identity and worldview membership. That stays in the kind system rather than being represented as tags or ad hoc fields. See also
[tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md) and [Naming and Glossary](./01-naming-and-glossary.md).

---

## 1. The problem with flat kinds

Three concrete problems:

**Problem 1: Verdict + Test.** A test gate produces a verdict that contains detailed test results. The Signal must be `Kind::GateVerdict` for the gate pipeline and `Kind::TestResult` for the test analysis system. Today, the orchestrator emits two separate Signals -- one GateVerdict and one TestResult -- with a shared lineage link. This works but doubles the storage and requires consumers to join on lineage.

**Problem 2: Episode + Skill.** When the skill library extracts a skill from an episode, the resulting Signal is both a Skill (for the skill library) and an Episode derivative (for the learning system). Today it is `Kind::Skill` with a tag `source=episode`, which the episode logger cannot query by Kind alone.

**Problem 3: Routing feedback + Cost.** A routing feedback Signal carries cost information. The cost normalization system needs `Kind::Metric`-like access, but the Signal is `Kind::RouterFeedback`. Tags bridge the gap, but tag-based dispatch is stringly typed and fragile.

---

## 2. The Compound variant

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    // ... all existing variants unchanged ...

    /// A signal that participates in multiple domains simultaneously.
    /// The inner Vec is sorted and deduplicated on construction.
    /// Dispatch matches if ANY inner kind matches the filter.
    Compound(Vec<Kind>),
}
```

### 2.1 Construction

Compound kinds are built via a helper that enforces invariants:

```rust
impl Kind {
    /// Create a compound kind from multiple kinds.
    /// Flattens nested Compounds, deduplicates, and sorts.
    /// Returns the single inner kind if only one remains after dedup.
    pub fn compound(kinds: impl IntoIterator<Item = Kind>) -> Kind {
        let mut flat: Vec<Kind> = Vec::new();
        for k in kinds {
            match k {
                Kind::Compound(inner) => flat.extend(inner),
                other => flat.push(other),
            }
        }
        flat.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        flat.dedup();
        match flat.len() {
            0 => Kind::Custom("empty_compound".into()),
            1 => flat.into_iter().next().unwrap(),
            _ => Kind::Compound(flat),
        }
    }

    /// Check whether this kind matches a filter kind.
    /// For Compound kinds, returns true if any inner kind matches.
    pub fn matches(&self, filter: &Kind) -> bool {
        match (self, filter) {
            (Kind::Compound(kinds), _) => kinds.iter().any(|k| k.matches(filter)),
            (_, Kind::Compound(filters)) => filters.iter().any(|f| self.matches(f)),
            (a, b) => a == b,
        }
    }
}
```

### 2.2 Invariants

1. **No nesting.** `Kind::compound()` flattens nested Compounds. A Compound never contains another Compound.
2. **Sorted and deduplicated.** The inner Vec is sorted by `as_str()` and deduplicated. This makes equality comparison and hashing deterministic.
3. **Minimum size 2.** A Compound with one element is unwrapped to that element. A Compound with zero elements becomes `Kind::Custom("empty_compound")` (an error state).
4. **No Custom inside Compound.** Compound kinds should use only the predefined variants. Custom kinds inside Compounds make dispatch unpredictable.

---

## 3. Dispatch semantics

### 3.1 Filter matching

When a consumer filters by Kind, Compound kinds match if any inner kind matches the filter:

```
Signal { kind: Compound([GateVerdict, TestResult]) }

  matches(Kind::GateVerdict)  => true
  matches(Kind::TestResult)   => true
  matches(Kind::Task)         => false
  matches(Compound([GateVerdict, TestResult]))  => true
```

This means a Compound signal appears in every query that any of its component kinds would appear in. A GateVerdict+TestResult signal shows up in gate verdict queries AND test result queries.

### 3.2 Trait dispatch

When a Gate receives a Compound signal, it should verify only the kinds it understands:

```
Gate "compile":
  Input: Compound([GateVerdict, CompileDiagnostic])
  Behavior: verify the CompileDiagnostic aspect, ignore the GateVerdict aspect
  (the GateVerdict is the output of verification, not the input)
```

The dispatch rule: each consumer processes the kind(s) it recognizes and ignores the rest. Consumers must not fail on unrecognized kinds within a Compound.

### 3.3 Scoring

The Scorer evaluates each component kind independently and takes the maximum score across dimensions:

```
fn score_compound(signal: &Signal, kinds: &[Kind]) -> Score {
    let scores: Vec<Score> = kinds.iter()
        .filter_map(|k| score_for_kind(signal, k))
        .collect();

    Score {
        relevance:  scores.iter().map(|s| s.relevance).max_by(f32_cmp),
        confidence: scores.iter().map(|s| s.confidence).max_by(f32_cmp),
        urgency:    scores.iter().map(|s| s.urgency).max_by(f32_cmp),
        // ... remaining axes
    }
}
```

---

## 4. Serialization

Compound kinds serialize as a JSON array:

```json
{ "kind": { "compound": ["gate_verdict", "test_result"] } }
```

Single kinds serialize as before:

```json
{ "kind": "gate_verdict" }
```

Backwards compatibility: existing Signals with flat kinds deserialize without change. The `#[serde(rename_all = "snake_case")]` attribute handles both forms.

---

## 5. Migration path

### Phase 1: Add the variant (non-breaking)

Add `Kind::Compound(Vec<Kind>)` to the enum. Because the enum is `#[non_exhaustive]`, adding a variant does not break downstream matches. All existing code that matches on specific kinds continues to work -- it simply does not match Compound signals.

```rust
// Existing code (still compiles, still works):
match signal.kind {
    Kind::GateVerdict => handle_verdict(signal),
    Kind::TestResult => handle_test(signal),
    _ => {} // Compound falls through here
}
```

### Phase 2: Add matches() and compound() helpers

Add the helper methods. Update Substrate query filtering to use `Kind::matches()` instead of `==`. This is the only breaking change -- any code that does `signal.kind == Kind::GateVerdict` must switch to `signal.kind.matches(&Kind::GateVerdict)`.

Search for affected call sites:

```bash
grep -rn 'signal\.kind ==' crates/ --include='*.rs' | grep -v target/
grep -rn '\.kind ==' crates/ --include='*.rs' | grep -v target/
```

### Phase 3: Emit Compound signals

Update the orchestrator to emit Compound signals where appropriate:

| Current | Compound replacement |
|---|---|
| Two Signals: GateVerdict + TestResult | One Signal: Compound([GateVerdict, TestResult]) |
| Skill with tag `source=episode` | Compound([Skill, Episode]) |
| RouterFeedback with cost tags | Compound([RouterFeedback, Metric]) |

### Phase 4: Update consumers

Update consumers to handle Compound kinds:

| Consumer | Change |
|---|---|
| Gate pipeline | Use `matches()` for verdict filtering |
| Cascade router | Use `matches()` for feedback filtering |
| Episode logger | Use `matches()` for episode filtering |
| Skill library | Use `matches()` for skill filtering |
| Dashboard | Display primary kind (first in sorted order) |

---

## 6. Limits and anti-patterns

### Maximum compound size

Cap at 4 kinds per Compound. A Signal that participates in 5+ domains is a design smell -- it should be split into separate Signals with shared lineage.

```rust
impl Kind {
    const MAX_COMPOUND_SIZE: usize = 4;

    pub fn compound(kinds: impl IntoIterator<Item = Kind>) -> Result<Kind, KindError> {
        // ... flatten, dedup, sort ...
        if flat.len() > Self::MAX_COMPOUND_SIZE {
            return Err(KindError::CompoundTooLarge {
                size: flat.len(),
                max: Self::MAX_COMPOUND_SIZE,
            });
        }
        // ...
    }
}
```

### Anti-patterns

1. **Compound as a grab bag.** Do not create Compound([Task, Plan, PlanPhase, Episode]) -- if everything is everything, the type system provides no value.
2. **Compound for versioning.** Do not use Compound([GateVerdict, Custom("gate_verdict_v2")]) -- use the existing variant with different body formats.
3. **Compound for metadata.** Do not use Compound([Task, Metric]) just because the task carries a metric. Tags and body fields handle metadata.

---

## 7. Configuration parameters

| Parameter | Default | Range | Description |
|---|---|---|---|
| `max_compound_size` | 4 | 2 - 8 | Maximum kinds in a Compound |
| `compound_scoring_strategy` | "max" | "max", "mean", "first" | How to combine scores across kinds |
| `compound_display_strategy` | "first" | "first", "all", "primary" | How to display Compound kinds in UI |

---

## 8. Error handling

| Condition | Response |
|---|---|
| Empty kinds iterator | Return `Kind::Custom("empty_compound")` |
| Single kind after dedup | Unwrap to that single kind (not a Compound) |
| Nested Compound | Flatten automatically |
| Exceeds max size | Return `Err(KindError::CompoundTooLarge)` |
| Custom kind inside Compound | Allow but log warning |
| Serialization of Compound with 50+ kinds | Reject at construction (max size enforced) |

---

## 9. Test criteria

1. `Kind::compound([GateVerdict, TestResult])` produces a sorted, deduplicated Compound.
2. `Kind::compound([GateVerdict])` returns `Kind::GateVerdict` (unwrapped).
3. `Kind::compound([])` returns `Kind::Custom("empty_compound")`.
4. Nested Compounds flatten: `compound([compound([A, B]), C])` equals `compound([A, B, C])`.
5. `Compound([GateVerdict, TestResult]).matches(&Kind::GateVerdict)` returns true.
6. `Compound([GateVerdict, TestResult]).matches(&Kind::Task)` returns false.
7. Serde round-trip preserves Compound kinds.
8. `as_str()` for Compound returns a joined form: "gate_verdict+test_result".
9. Hash equality: two Compounds with same kinds in different insertion order produce the same hash.
10. Exceeding max_compound_size returns an error.

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) -- Signal struct with Kind field
- [08-scorer-gate-router-composer-policy.md](08-scorer-gate-router-composer-policy.md) -- Trait dispatch by Kind
- [09-universal-cognitive-loop.md](09-universal-cognitive-loop.md) -- Loop stages that filter by Kind
- [01-naming-and-glossary.md](./01-naming-and-glossary.md) -- Canonical kind vocabulary, including heuristic and worldview terms
- [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md) -- Heuristic and worldview learning refinement
- `crates/roko-core/src/kind.rs` -- Current Kind enum
- `crates/roko-core/src/signal.rs` -- Signal struct
