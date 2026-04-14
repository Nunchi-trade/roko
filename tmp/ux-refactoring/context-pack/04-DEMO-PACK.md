# UX Refactoring Context Pack: Demo Pack

This pack condenses the demo planning docs for B batches.

## Non-negotiable sources

- `tmp/demo/DEMO-IMPLEMENTATION-PLAN.md`
- `tmp/demo/tasks/ERRATA.md`
- `tmp/ux-refactoring/B-demo-features.md`

## Hard invariants

- Respect `ERRATA.md` if it disagrees with an older task note.
- Solidity/Rust binding IDs are `U256` unless inspection proves otherwise.
- Token amounts should use integer exponent forms such as `10u128.pow(18)`.
- `StubLlm` must remain available for deterministic tests.
- `roko-demo` changes should keep the existing CLI modes working while adding
  the new scenario/event/provider surfaces.

## Expected demo outcomes

- Yield-routing scenario works first with `StubLlm`
- Real providers and events wire in next
- Benchmark, TUI mode, autonomy, and persistence layer on top of that spine
