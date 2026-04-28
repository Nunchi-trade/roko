# Rules

You are modifying a Rust workspace (`roko`) with 18 crates. Follow these rules strictly.

## Mandatory

1. **One implementation, one code path.** Never duplicate traits, types, or logic that exists elsewhere.
   Before defining anything, check `crates/roko-core/src/foundation.rs` — if it's there, import it.

2. **Wire, don't build.** If the type/trait/function exists but isn't called, wire it in. Don't create
   a new version. The codebase already has what you need — connect it.

3. **No stubs that silently pass.** Never return `Ok(())`, `true`, `Verdict::pass`, or empty success
   without doing real work. If functionality isn't ready, return an explicit error.

4. **No empty string placeholders.** Never emit events or create requests with `String::new()` for
   fields like `agent_id`, `model`, or `checkpoint_path`. Use `Option<String>` if the value may be
   absent, or fill it with the real value.

5. **No debug strings as data contracts.** Never use `format!("{:?}", event)` as a serialization
   format. Use `serde_json::to_string(&event)` with proper `Serialize`/`Deserialize` derives.

6. **Typed outcomes, not success-carrying-errors.** Don't encode errors inside success variants
   (e.g., `CommitDone { hash: "error: ..." }`). Use `Result<T, E>` or typed enums.

7. **Foundation types are canonical.** `crates/roko-core/src/foundation.rs` owns:
   `ModelCallRequest`, `ModelCallResult`, `ModelCaller`, `PromptSpec`, `PromptAssembler`,
   `FeedbackEvent`, `FeedbackSink`, `GateConfig`, `GateService`, `EventConsumer`, `Effect`,
   `EffectExecutor`, `AffectPolicy`, `DispatchModulation`.
   Do NOT define local copies of these in any other crate.

8. **Preserve public API.** Don't change function signatures that are called from other crates
   unless you also update all callers. `cargo check --workspace` must pass.

## Style

- Follow existing code style. Use `anyhow::Result` for errors in CLI/runtime, typed errors in core.
- Derive `Serialize, Deserialize` for anything that crosses a persistence boundary.
- Use `tokio::sync::Mutex` (never `std::sync::Mutex`) when the guard is held across `.await`.
- Don't add unnecessary dependencies. Prefer what's already in `Cargo.toml`.
- Don't add doc comments, tests, or clippy annotations unless the batch specifically asks for them.
- Keep changes minimal. Only modify what the batch prompt asks for.
