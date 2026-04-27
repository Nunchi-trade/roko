## Batch P0C: EventBus RuntimeEvent Support

### Write Scope
- **MODIFY**: `crates/roko-runtime/src/event_bus.rs` (add runtime_event_bus singleton)
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (if needed for re-export)

### Dependencies
- P0A must be complete (RuntimeEvent exists in `roko-core`)

### DO NOT
- Modify any other files
- Replace or reorganize existing EventBus code
- Remove the existing `global_event_bus()` singleton
- Add Cargo.toml dependencies (roko-core is already a dependency of roko-runtime)

### Existing Code (inlined for reference)

The current `event_bus.rs` already has a generic `EventBus<E>` and a singleton for
`RokoEvent`. You are adding a parallel singleton for `RuntimeEvent`.

```rust
// Existing (DO NOT MODIFY):
pub fn global_event_bus() -> &'static EventBus<RokoEvent> {
    static BUS: OnceLock<EventBus<RokoEvent>> = OnceLock::new();
    BUS.get_or_init(|| EventBus::new(1024))
}
```

### Task

Add a new function `runtime_event_bus()` that returns a singleton `EventBus<RuntimeEvent>`.
This is the bus that the workflow engine publishes to and all observers subscribe to.

#### Add to `crates/roko-runtime/src/event_bus.rs`:

```rust
use roko_core::RuntimeEvent;

/// Global event bus for workflow runtime events.
///
/// The workflow engine emits `RuntimeEvent`s here; adapters (ACP, SSE, JSONL, TUI)
/// subscribe to receive them.
pub fn runtime_event_bus() -> &'static EventBus<RuntimeEvent> {
    static BUS: OnceLock<EventBus<RuntimeEvent>> = OnceLock::new();
    BUS.get_or_init(|| EventBus::new(2048))
}
```

Also add a convenience helper for emitting events:

```rust
/// Emit a RuntimeEvent to the global runtime bus.
/// Convenience wrapper around `runtime_event_bus().emit(event)`.
pub fn emit_runtime_event(event: RuntimeEvent) -> u64 {
    runtime_event_bus().emit(event)
}
```

#### Ensure re-export in `crates/roko-runtime/src/lib.rs`:

If `event_bus` is already `pub mod`, the new functions are automatically accessible
as `roko_runtime::event_bus::runtime_event_bus()`. If it's not re-exported, add:

```rust
pub use event_bus::{runtime_event_bus, emit_runtime_event};
```

### Done Criteria
```bash
grep -q 'runtime_event_bus' crates/roko-runtime/src/event_bus.rs
grep -q 'emit_runtime_event' crates/roko-runtime/src/event_bus.rs
grep -q 'RuntimeEvent' crates/roko-runtime/src/event_bus.rs
cargo check -p roko-runtime
```
