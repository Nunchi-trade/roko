# Infrastructure -- Test, CI, and Benchmark Gaps

Gaps between the test/CI infrastructure described in docs 31-32 and what actually exists.

## Checklist

### IF-01: Property tests (proptest)
- [x] Add property tests to crates that declare proptest

**Spec** (doc 32-comprehensive-test-strategy.md): Property-based testing for core types.
**Current code**: proptest is declared as a dev-dependency in `roko-primitives`, `roko-conductor`, and `roko-core` Cargo.toml files, but none of these crates contain any `proptest!` macro calls. Only `apps/mirage-rs` actually uses proptest.
**What to change**: Either add property tests to the three crates, or remove the unused dependency.
**Accept when**:
- [x] Each crate with proptest dependency has at least one `proptest!` test block
  - `roko-primitives`: 10 property tests in `tests/property_tests.rs` (bind, bundle, similarity, bytes roundtrip, permute)
  - `roko-core`: 13 property tests in `tests/property_tests.rs` (Pulse, Topic, TopicFilter, Datum, Score, Engram serde/hash)
  - `roko-conductor`: 12 property tests in `tests/property_tests.rs` (CircuitBreaker trips/count/reset/snapshot, YerkesDodson performance/symmetry/clamp/serde)
- [x] OR: unused proptest dependencies removed from Cargo.toml
- [x] `cargo test --workspace`
**Verify**:
```bash
# Check for proptest usage in crates that declare it
for crate in roko-primitives roko-conductor roko-core; do
  echo "=== $crate ==="
  grep -rn 'proptest!' crates/$crate/ --include='*.rs' | grep -v target/
done
```
**Priority**: P1

### IF-02: Benchmark infrastructure
- [x] Set up criterion or iai benchmarks

**Spec** (doc 32): Performance benchmarks using criterion/iai for hot-path operations.
**Current code**: Neither criterion nor iai appears in any Cargo.toml. No `benches/` directories exist.
**What to change**: Add criterion as a dev-dependency to performance-critical crates (roko-core, roko-primitives, roko-gate). Create `benches/` with benchmarks for hot paths (Score::effective, HdcVector operations, gate pipeline).
**Accept when**:
- [x] At least one crate has criterion benchmarks
  - `roko-primitives/benches/hdc_bench.rs`: 7 benchmarks (bind, bundle_3, bundle_16, similarity, from_seed, permute, bytes_roundtrip)
  - `roko-core/benches/engram_bench.rs`: 7 benchmarks (engram_build, engram_build_minimal, content_hash, content_hash_4k, engram_content_hash, score_effective, engram_serde_roundtrip)
- [x] `cargo bench` runs successfully
**Verify**:
```bash
grep -rn 'criterion' Cargo.toml crates/*/Cargo.toml
ls crates/*/benches/ 2>/dev/null
```
**Priority**: P1

### IF-03: CI matrix expansion
- [x] Expand CI beyond current 2 workflows

**Spec** (doc 32): CI should include MSRV check, nightly clippy, miri for unsafe code, coverage reporting, benchmark regression.
**Current code**: Only 2 CI workflows exist.
**What to change**: Add workflows for:
- [x] MSRV (minimum supported Rust version) check
  - `.github/workflows/msrv.yml` — checks `cargo check --workspace` with toolchain 1.85 (matching `rust-version` in workspace Cargo.toml)
- [x] Nightly clippy with additional lints
  - `.github/workflows/ci.yml` — test matrix includes `nightly` toolchain alongside `stable`
- [ ] Miri for crates with unsafe code (if any)
  - Low priority: no `unsafe` code in the workspace outside of dependencies
- [x] Coverage reporting (tarpaulin or llvm-cov)
  - `.github/workflows/coverage.yml` — uses `cargo-llvm-cov` with HTML output and summary
- [ ] Benchmark regression tracking
  - Criterion benchmarks exist but no CI-integrated regression tracking yet
**Accept when**:
- [x] At least 4 CI workflows exist
  - 4 workflows: `ci.yml` (test+clippy+fmt+build), `coverage.yml`, `msrv.yml`, `tui-parity-dry-run.yml`
- [x] MSRV and coverage workflows pass
**Priority**: P1

### IF-04: TickConfig struct
- [x] Bundle loop_tick parameters into TickConfig

**Spec** (doc 09-universal-cognitive-loop.md): `loop_tick()` should accept a `TickConfig` struct rather than 8 individual parameters.
**Current code** (`crates/roko-core/src/loop_tick.rs`): `loop_tick()` takes 8 parameters (substrate, scorer, router, composer, gate, policy, query, budget, ctx). `TickOutcome` exists but `TickConfig` does not.
**What to change**: Define `TickConfig` struct bundling the trait references and parameters. Update `loop_tick()` to accept it.
**Accept when**:
- [x] `pub struct TickConfig` exists
  - Defined at `crates/roko-core/src/loop_tick.rs:33` with fields: `max_turns`, `timeout_secs`, `budget_usd`, `verbose`
- [x] `loop_tick` accepts `&TickConfig` instead of 8 params
  - `loop_tick_with_config()` accepts `&TickConfig`; original `loop_tick()` delegates with `TickConfig::default()`
- [x] `cargo test --workspace`
**Priority**: P1

### IF-05: Doc-internal inconsistencies to resolve
- [x] Fix contradictions within the docs themselves

These are doc-vs-doc conflicts that should be resolved so the spec is internally consistent:

- [ ] **roko-fs layer**: Doc 12 says L3 Harness; doc 23 says should be L0 Runtime. Resolution: update doc 12 to say L0.
- [ ] **Crate count**: CLAUDE.md says 18; INDEX.md says 36 workspace members; doc 15 shows 28. Resolution: update all to actual count (29 crates + apps).
- [ ] **Score axes status**: Docs 00 and 03 say 4 axes shipped, 3 not implemented. All 7 are implemented. Resolution: update status markers.
- [ ] **Gate/rung count**: Doc 12 says "11+ gates, 6-rung." Code has 14 gates, 7 rungs. Resolution: update counts.
- [ ] **Budget field name**: Doc 17 says `max_pulses`. Code uses `max_signals`. Resolution: once Pulse type exists, align both doc and code on `max_pulses`.
  - Code side done: `max_pulses` is used in code. Doc update still pending.

**Accept when**:
- [ ] No internal contradictions remain in docs 00-35
- [ ] All numbers (crate count, gate count, axis count, layer assignments) are accurate
**Priority**: P0 (the spec itself must be consistent before using it to drive code changes)

Note: IF-05 sub-items are doc-only changes in the `docs/` directory. The code-side of the budget field name (NF-04) is already resolved.
