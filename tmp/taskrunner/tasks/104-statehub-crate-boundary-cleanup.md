# Task 104: StateHub Crate Boundary Cleanup

```toml
id = 104
title = "Move StateHub to a real crate boundary, remove path-included compatibility aliases"
track = "architecture-cleanup"
wave = "wave-6"
priority = "critical"
blocked_by = []
touches = [
    "crates/roko-core/src/state_hub.rs",
    "crates/roko-core/src/pulse_bus.rs",
    "crates/roko-core/src/lib.rs",
    "crates/roko-runtime/src/",
    "crates/roko-serve/src/lib.rs",
    "crates/roko-serve/src/event_bus.rs",
    "crates/roko-cli/src/lib.rs",
    "docs/v1/12-interfaces/22-statehub-projection-layer.md",
]
exclusive_files = []
estimated_minutes = 300
```

## Context

The architecture merge left `StateHub` working, but the crate boundary is not clean.
`crates/roko-core/src/state_hub.rs` and `pulse_bus.rs` look like core modules, but they use
`roko_runtime` and are not exported by `roko-core/src/lib.rs`. `roko-serve` currently works
around this by aliasing itself as `roko_core`, re-exporting real `roko-core`, and path-including
the source file.

This is an ad-hoc compatibility layer. It should not become the stable architecture.

Current audit findings:
- `crates/roko-serve/src/lib.rs` uses `extern crate self as roko_core` and path-includes
  `roko-core/src/state_hub.rs`.
- `crates/roko-cli/src/lib.rs` exposes compatibility re-exports.
- `docs/v1/12-interfaces/22-statehub-projection-layer.md` says `StateHub` exists in
  `roko-core`, but that file is not compiled as a `roko-core` module.
- `crates/roko-serve/src/event_bus.rs` duplicates much of `roko-runtime::event_bus`.
- `start_orchestrator_event_bridge` is public even though comments warn about duplicate
  REST-originated events.

## Decision

`StateHub` should live behind a real crate boundary that can legally depend on the runtime
event types it consumes. Preferred order:

1. Move it into `roko-runtime` if dependency direction stays clean.
2. If that creates cycles, create a small `roko-state` or `roko-projection` crate.
3. Do not put runtime-dependent code in `roko-core`.

## Implementation Detail

### Current source facts

- `crates/roko-core/src/state_hub.rs` imports `roko_runtime::event_bus::{self, EventBus}`
  and `roko_core::dashboard_snapshot::{DashboardEvent, DashboardSnapshot}`. This cannot be
  compiled as a normal `roko-core` module because `roko-runtime` already depends on
  `roko-core`.
- `crates/roko-core/src/pulse_bus.rs` has the same dependency-direction issue: it wraps
  `roko_runtime::event_bus::EventBus` while implementing `roko_core::Bus`.
- `crates/roko-core/src/lib.rs` exports `dashboard_snapshot` and `RuntimeEvent`, but does
  not export `state_hub` or `pulse_bus`.
- `crates/roko-serve/src/lib.rs` currently works around this with:
  - `#![allow(hidden_glob_reexports)]`
  - `extern crate roko_core as roko_core_crate`
  - `extern crate self as roko_core`
  - `#[path = "../../roko-core/src/state_hub.rs"] pub mod state_hub_compat`
  - `pub mod state_hub { pub use crate::state_hub_compat::*; }`
  - `pub use state_hub_compat::{SharedStateHub, StateHub}`
- `crates/roko-cli/src/lib.rs` re-exports `roko_serve::state_hub::*` as
  `roko_cli::state_hub`, so CLI and serve share the path-included concrete type.
- `crates/roko-serve/src/event_bus.rs` and `crates/roko-runtime/src/event_bus.rs` are
  duplicate bounded broadcast buses. The serve version uses `publish()` and the runtime
  version uses `emit()`, but both expose subscribe/replay/ring semantics.
- `crates/roko-serve/src/lib.rs::start_orchestrator_event_bridge()` is public and maps
  `StateHub` events back into `EventBus`; comments at the start path intentionally avoid
  starting it because it can duplicate REST-originated events.

### Recommended crate boundary

Use `roko-runtime` unless a fresh `cargo metadata`/build proves a cycle:

- `roko-runtime` already depends on `roko-core`, so `StateHub` can legally import
  `roko_core::dashboard_snapshot::{DashboardEvent, DashboardSnapshot}` there.
- `roko-serve` and `roko-cli` already depend on `roko-runtime`.
- Moving to `roko-runtime` avoids creating another crate for a thin runtime projection/bus
  boundary.

If `roko-runtime` creates a real cycle after inspecting current manifests, stop and create
a small `roko-state`/`roko-projection` crate that depends on `roko-core` and is depended on
by `roko-runtime`, `roko-serve`, and `roko-cli`. Do not put the moved code back into
`roko-core`.

### Mechanical move plan

1. Move `crates/roko-core/src/state_hub.rs` to `crates/roko-runtime/src/state_hub.rs`.
   Inside the moved file:
   - replace `use roko_runtime::event_bus::{self, EventBus};` with
     `use crate::event_bus::{self, EventBus};`;
   - keep `use roko_core::dashboard_snapshot::{DashboardEvent, DashboardSnapshot};`;
   - keep public names `StateHub`, `SharedStateHub`, `StateHubSender`, and
     `shared_state_hub()`.
2. Move `crates/roko-core/src/pulse_bus.rs` to `crates/roko-runtime/src/pulse_bus.rs` if
   it is still not exported from `roko-core`. Inside the moved file:
   - replace `use crate::{Bus, Pulse, TopicFilter, error::Result};` with imports from
     `roko_core`;
   - replace `use roko_runtime::event_bus::{Envelope, EventBus};` with
     `use crate::event_bus::{Envelope, EventBus};`.
3. Export the moved modules from `crates/roko-runtime/src/lib.rs`:
   - `pub mod state_hub;`
   - `pub use state_hub::{SharedStateHub, StateHub, StateHubSender, shared_state_hub};`
   - add equivalent `pulse_bus` exports if moved.
4. In `crates/roko-serve/src/lib.rs`, delete the fake crate alias/path include block and
   replace any needed public surface with normal imports/re-exports, for example
   `pub use roko_runtime::{SharedStateHub, StateHub};` only if external callers still need
   them from `roko_serve`.
5. In `crates/roko-serve/src/state.rs`, replace every `roko_core::SharedStateHub` and
   `roko_core::StateHub` use with `roko_runtime::SharedStateHub` and
   `roko_runtime::StateHub`.
6. In `crates/roko-cli/src/lib.rs`, remove the `roko_serve::state_hub::*` compatibility
   re-export. If existing CLI modules still import `crate::state_hub`, re-export from
   `roko_runtime::state_hub::*` as a temporary internal compatibility layer and mark it
   `pub(crate)` if possible. Prefer updating call sites to `roko_runtime::...` directly.
7. Update CLI/serve call sites found by:

```bash
rg -n 'crate::state_hub|roko_cli::state_hub|roko_core::SharedStateHub|roko_core::StateHub|roko_serve::state_hub' crates/roko-cli/src crates/roko-serve/src --glob '*.rs'
```

8. Consolidate `crates/roko-serve/src/event_bus.rs`:
   - preferred: delete the duplicate implementation and import/type-alias
     `roko_runtime::event_bus::{Envelope, EventBus, BusSender}` in serve;
   - if API names would cause too much churn, keep a thin wrapper with `publish()` delegating
     to runtime `emit()`, but do not keep a second ring/broadcast implementation.
9. Fix bridge-loop risk by narrowing visibility and/or provenance:
   - make `start_orchestrator_event_bridge` `pub(crate)` if no external crate uses it; and
   - add origin/provenance filtering before publishing `DashboardEvent -> ServerEvent`, or
     leave it unstarted with a test/documented invariant that REST-originated events are not
     bridged back.
10. Update `docs/v1/12-interfaces/22-statehub-projection-layer.md` to name the final owning
    crate and remove statements that it is a fake `roko-core` module.

### Tests to add or update

- Move the existing `state_hub.rs` tests with the module and keep them passing under
  `cargo test -p roko-runtime state_hub`.
- Add a compile-only/use test in either `roko-runtime` or `roko-serve` that constructs
  `SharedStateHub::new_in_process()`, publishes a `DashboardEvent`, and reads the snapshot.
- Add/update a serve test that proves `AppState::state_hub_for_workdir()` writes through the
  moved `StateHub` and still creates `.roko/events.jsonl`.
- If a wrapper remains for serve `EventBus`, add a test asserting `publish()`, `subscribe()`,
  and `replay_from()` delegate to the runtime bus.

## What to Change

1. Move `StateHub`, `SharedStateHub`, and related pulse/projection helpers out of the fake
   `roko-core` source location into the selected real crate.
2. Replace `#[path = ".../state_hub.rs"]` and `extern crate self as roko_core` compatibility
   tricks with normal imports.
3. Re-export only the final public surface needed by `roko-cli` and `roko-serve`.
4. Consolidate or wrap `crates/roko-serve/src/event_bus.rs` around
   `roko_runtime::event_bus::EventBus` instead of keeping a second bounded event bus.
5. Fix bridge-loop risk: either add event provenance/origin filtering or reduce/remove public
   access to bridge functions that can duplicate REST-originated events.
6. Update docs to reflect the real location and status.

## What NOT to Do

- Do not add another path include.
- Do not make `roko-core` depend on `roko-runtime`.
- Do not keep both event bus implementations without a documented reason.
- Do not expose transitional compatibility modules as public API.

## Wire Target

```bash
cargo test -p roko-runtime state_hub
cargo test -p roko-serve state_hub event_bus
rg 'extern crate self as roko_core|state_hub_compat|hidden_glob_reexports|#\[path.*state_hub' crates/roko-serve crates/roko-cli crates/roko-core
```

Expected observable behavior: `StateHub` is constructed and exercised through a normal crate
boundary, serve/CLI compile without path-included compatibility aliases, and the grep command
returns no compatibility workaround hits.

## Verification

Compilation and tests can be deferred until merge coalescing if the batch policy says so, but
the final task is not done until these checks are clean:

- [ ] `rg 'extern crate self as roko_core|state_hub_compat|hidden_glob_reexports' crates/roko-serve crates/roko-cli` has no compatibility workaround hits.
- [ ] `rg '#\\[path.*state_hub|#\\[path.*pulse_bus' crates/` returns no hits.
- [ ] `rg 'pub mod state_hub|pub use .*StateHub' crates/` shows a normal crate boundary.
- [ ] `rg 'struct EventBus' crates/roko-serve/src crates/roko-runtime/src` shows one implementation or one explicit wrapper.
- [ ] `docs/v1/12-interfaces/22-statehub-projection-layer.md` names the real owning crate.

## Status Log

| Time | Agent | Action |
|------|-------|--------|
| 2026-05-05 | wp-arch2 audit | Created cleanup task after audit found path-included StateHub, fake crate aliasing, duplicate EventBus implementations, and stale docs. |
