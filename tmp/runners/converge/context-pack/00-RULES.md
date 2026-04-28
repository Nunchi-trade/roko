## Mandatory Rules for All Batches

You are an unattended Codex batch agent. There is no prior chat history. This prompt is
entirely self-contained — everything you need is inlined below.

### Execution discipline

1. **Work ONLY within the listed write scope.** Do not modify files outside your scope.
2. **Run verify commands** before declaring success. If `cargo check` fails, fix the errors.
3. **If blocked**, implement the maximum possible and leave a `// TODO(converge): <reason>` comment.
4. **Do NOT create new crates.** All work goes into existing crate directories.
5. **Do NOT add Cargo.toml dependencies** unless the prompt explicitly lists them.
6. **Do NOT modify Cargo.toml** unless the prompt explicitly instructs you to.
7. **Do NOT spawn subagents or delegate.** Each batch is small enough for one agent.
8. **Do NOT create test files** in separate directories. Put `#[cfg(test)] mod tests` at the
   bottom of the implementation file.

### Code quality

9. **No `todo!()` or `unimplemented!()` in public API methods.** Use `Err(...)` or sensible
    defaults instead. Internal helper stubs are acceptable with `// TODO(converge)` markers.
10. **Use `async_trait`** for async trait methods. The crate is already available workspace-wide.
11. **Follow existing naming conventions.** Study the crate's `lib.rs` for style guidance.
12. **All public types need `pub` visibility** and should be re-exported from `lib.rs`.
13. **Prefer `anyhow::Result`** for fallible functions unless the crate uses a custom error type.

### Convergence-specific rules

14. **Use roko-core foundation traits.** The canonical traits live in `roko_core::foundation`.
    Do NOT create local copies of `ModelCaller`, `PromptAssembler`, `FeedbackSink`, `GateRunner`,
    `EventConsumer`, or `EffectExecutor`. Import them from `roko_core`.
15. **Use roko-core RuntimeEvent.** The canonical enum lives in `roko_core::runtime_event`.
    Do NOT create local copies. Import from `roko_core`.
16. **Feature-gate legacy code with `#[cfg(feature = "legacy-orchestrate")]`** when the prompt
    instructs you to wrap code in a feature gate. Do NOT delete legacy code — gate it.
17. **Wire, don't rebuild.** Existing services have real implementations. Your job is to connect
    them to the WorkflowEngine path, not rewrite them.

### Anti-patterns (condensed — see 03-ANTI-PATTERNS.md for details)

18. **NEVER `Command::new("claude")`** — use `ModelCallService` / `ModelCaller` trait.
19. **NEVER `format!("You are the...")`** — use `PromptAssemblyService` / `SystemPromptBuilder`.
20. **NEVER put decision logic in the effect driver** — decisions live in the state machine.
21. **NEVER hardcode roles** — roles come from config or `AgentRole` enum.
22. **NEVER skip feedback recording** — `FeedbackService` must see every model call.
23. **NEVER copy code between entry points** — extract shared services.
24. **NEVER add execution logic to a specific entry point** — use shared services under
    `roko-runtime`, `roko-agent`, `roko-compose`, etc.
