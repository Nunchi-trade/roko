# Task 026: Add And/Or/Not Combinators to TopicFilter

```toml
id = 26
title = "Add And/Or/Not combinator variants to TopicFilter for richer Bus subscriptions"
track = "wiring"
wave = "wave-1"
priority = "medium"
blocked_by = []
touches = [
    "crates/roko-core/src/pulse.rs",
    "crates/roko-core/src/pulse_bus.rs",
    "crates/roko-core/src/bus_backends.rs",
    "crates/roko-core/tests/property_tests.rs",
]
exclusive_files = ["crates/roko-core/src/pulse.rs"]
estimated_minutes = 60
```

## Context

`TopicFilter` currently has `Exact`, `Prefix`, and `All` variants. The v2 Bus architecture
needs richer subscription filtering: `And(Vec<TopicFilter>)`, `Or(Vec<TopicFilter>)`, and
`Not(Box<TopicFilter>)`. These are tree combinators that compose existing filters.

Without them, any subscriber that wants "gate events but not heartbeat" must filter in
userland, which defeats the purpose of filtered subscriptions.

Sources:
- `tmp/v2-refactoring/CHECKLIST.md` -- QW-2
- `tmp/v2-refactoring/03-QUICK-WINS.md` -- QW-2
- `tmp/v2-refactoring/01-CURRENT-STATE.md` -- "missing And/Or/Not filters"

## Background

Read these files first:
1. `crates/roko-core/src/pulse.rs` -- `TopicFilter` enum and `matches()` method (lines 180-216)
2. `crates/roko-core/src/pulse_bus.rs` -- `PulseBusReceiver` uses `filter.matches()` to decide delivery
3. `crates/roko-core/src/bus_backends.rs` -- `BroadcastBus` and `MemoryBus` also use `TopicFilter`
4. `crates/roko-core/tests/property_tests.rs` -- existing TopicFilter property tests

Important current-state details:

- Current `TopicFilter` variants are `Exact(Topic)`, `Prefix(String)`, and `All`.
- `tmp/v2-refactoring/03-QUICK-WINS.md` mentions `AnyOf`, but the current code does not have it.
  Treat that as stale source-doc language; `Or(Vec<TopicFilter>)` is the new disjunction.
- Runtime wiring already exists through these callsites:
  - `PulseBus::replay_from` filters with `f.matches(&env.payload.topic)`
  - `PulseBusReceiver::recv` filters with `self.filter.matches(...)`
  - `BroadcastBus::publish` filters each subscriber with `sub.filter.matches(...)`
  - `MemoryBus::replay_from` and `MemoryBus::publish` use the same `matches()` path
- CLI/runtime chain for consumers is `Bus::subscribe(TopicFilter)` -> concrete bus receiver ->
  `TopicFilter::matches(&pulse.topic)` on publish/replay/recv.

## What to Change

1. **Add three new variants to `TopicFilter`** in `crates/roko-core/src/pulse.rs`:
   ```rust
   And(Vec<TopicFilter>),
   Or(Vec<TopicFilter>),
   Not(Box<TopicFilter>),
   ```
   Place them after `All` or near the existing matching variants. Keep the enum derives as-is:
   `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `Serialize`, `Deserialize`.

2. **Extend the `matches()` method** to handle the new variants:
   - `And` -- all sub-filters must match
   - `Or` -- at least one sub-filter must match
   - `Not` -- the inner filter must NOT match
   Mechanical implementation:

   ```rust
   Self::And(filters) => filters.iter().all(|filter| filter.matches(topic)),
   Self::Or(filters) => filters.iter().any(|filter| filter.matches(topic)),
   Self::Not(filter) => !filter.matches(topic),
   ```

   This intentionally gives empty `And` vacuous truth (`true`) and empty `Or` no-match
   semantics (`false`) through standard iterator behavior.

3. **Add unit tests** for the new variants in the same file or in `property_tests.rs`:
   - `And` with mixed filters
   - `Or` with disjoint filters
   - `Not` inverting an exact match
   - Nested combinators: `Or([And([...]), Not(...)])`
   - Empty `And` returns true (vacuous truth)
   - Empty `Or` returns false (no match)

4. **Verify serde round-trip** -- the new variants must serialize/deserialize correctly since
   `TopicFilter` derives `Serialize, Deserialize`.

5. Add at least one bus-level test that uses a combinator through a real bus path:
   - in `pulse_bus.rs`, test `replay_from` or `subscribe` with
     `And([Prefix("gate."), Not(Exact("gate.heartbeat"))])`
   - in `bus_backends.rs`, test `MemoryBus::replay_from` or `BroadcastBus::publish` with
     `Or([Exact("gate.compile"), Exact("gate.test")])`

6. In `property_tests.rs`, add simple algebraic properties:
   - `Not(Exact(topic)).matches(topic)` is false
   - `And([All, Exact(topic)]).matches(topic)` is true
   - `Or([Exact(topic), Prefix("impossible.")]).matches(topic)` is true

## What NOT to Do

- Don't change how `PulseBus` or `BroadcastBus` dispatch pulses -- they already call
  `filter.matches()`, so the new variants will work automatically.
- Don't add `AnyOf` -- it already exists as a concept via `Or`.
- Don't add recursive depth limits -- TopicFilter is constructed in-process, not from
  user input. Keep it simple.
- Don't special-case empty vectors outside the iterator semantics above.
- Don't change `Topic::starts_with` prefix semantics.
- Don't alter `Bus::subscribe` signatures or receiver types.

## Wire Target

```bash
# The PulseBus already uses filter.matches() -- any new TopicFilter variant is
# automatically wired. Verify with:
cargo test -p roko-core -- topic_filter
cargo test -p roko-core -- property_tests
cargo test -p roko-core -- pulse_bus
cargo test -p roko-core -- bus_backends
```

Expected observable behavior:

- `TopicFilter::And([Prefix("gate."), Not(Exact("gate.heartbeat"))])` matches
  `gate.compile` and does not match `gate.heartbeat`.
- `TopicFilter::Or([...])` delivers any topic matching at least one child filter.
- Existing subscribers do not need code changes; they keep calling `filter.matches()`.

## Verification

- [ ] `cargo build --workspace`
- [ ] `cargo test -p roko-core`
- [ ] `grep -rn 'TopicFilter::And\|TopicFilter::Or\|TopicFilter::Not' crates/roko-core/ --include='*.rs' | grep -v target/` -- shows definitions AND test callsites
- [ ] `cargo clippy --workspace --no-deps -- -D warnings`

## Status Log

| Time | Agent | Action |
|------|-------|--------|
