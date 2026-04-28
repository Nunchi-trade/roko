# Anti-Patterns To Avoid

These are the specific anti-patterns the converge runner introduced or preserved.
Your changes must not introduce any of these. The runner checks for them automatically.

## AP-1: Stubs That Silently Pass

**BAD:**
```rust
fn run_gate(&self, config: &GateConfig) -> Result<GateResult> {
    Ok(GateResult { passed: true, ..Default::default() }) // stub
}
```

**GOOD:**
```rust
fn run_gate(&self, config: &GateConfig) -> Result<GateResult> {
    anyhow::bail!("custom gate not yet configured: {:?}", config.name)
}
```

## AP-2: `block_on` Inside Async

**BAD:** `futures::executor::block_on(async_fn())` inside a tokio runtime — panics.
**GOOD:** Use `.await` directly or `tokio::task::spawn_blocking`.

## AP-3: Duplicate Trait Definitions

**BAD:** Defining `pub trait AffectPolicy` in `effect_driver.rs` when it exists in `foundation.rs`.
**GOOD:** `use roko_core::foundation::AffectPolicy;`

## AP-4: Computed But Unused Values

**BAD:**
```rust
let modulation = policy.modulate(&context);
// modulation never used
```

**GOOD:** Apply the result or don't compute it.

## AP-5: Shell Out To Claude/Codex CLI

**BAD:** `Command::new("claude")` for model inference at runtime.
**GOOD:** Use `ModelCallService` with the appropriate provider.

## AP-6: Inline Prompt Strings

**BAD:** `format!("You are a helpful assistant...")` in dispatch code.
**GOOD:** Use `PromptAssemblyService` with templates.

## AP-7: `std::sync::Mutex` Held Across `.await`

**BAD:**
```rust
let guard = std_mutex.lock().unwrap();
some_async_fn().await; // guard still held — deadlock risk
```

**GOOD:** Use `tokio::sync::Mutex` or drop the guard before `.await`.

## AP-8: Debug Strings As Event Contracts

**BAD:** `writeln!(file, "{event:?}")` then parsing `Debug` output back into state.
**GOOD:** `serde_json::to_string(&event)` then `serde_json::from_str`.

## AP-9: Empty String Placeholders In Events

**BAD:** `RuntimeEvent::AgentSpawned { agent_id: String::new(), model: String::new() }`
**GOOD:** Use `Option<String>` or fill with the actual value from the effect/request.

## AP-10: Success Variants Carrying Error State

**BAD:** `CommitDone { hash: "error: git commit failed".into() }`
**GOOD:** Return `Err(...)` or use a typed enum with `CommitFailed` variant.
