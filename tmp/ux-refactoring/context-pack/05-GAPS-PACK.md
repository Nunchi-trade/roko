# UX Refactoring Context Pack: Runtime Gaps Pack

This pack condenses the deep-gap and feedback-loop documents for D/E batches.

## Source docs

- `tmp/integrate-prds/08-DEEP-ARCHITECTURAL-GAPS.md`
- `tmp/integrate-prds/09-REFACTORING-PRD-ADDITIONS.md`
- `tmp/integrate-prds/06-BUILD-SEQUENCE.md`
- `tmp/ux-refactoring/D-architectural-gaps.md`
- `tmp/ux-refactoring/E-feedback-loops.md`

## Reality constraints

- Some items in `D-architectural-gaps.md` are already done. Do not re-implement
  them; only touch the partial or missing ones in the current batch.
- `orchestrate.rs` remains a major source of truth for live wiring.
- Routing changes usually cross `roko-learn`, `roko-conductor`,
  `roko-compose`, and `roko-cli`.
- Dreams/heartbeat work should prefer incremental completion over speculative
  new abstractions when the existing scaffold is obvious.
