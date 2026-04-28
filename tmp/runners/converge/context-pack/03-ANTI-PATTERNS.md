## Anti-Patterns — DO NOT Do These

These are concrete examples of code that has been written in this codebase before and caused
problems. Each one has a "BAD" example and a "GOOD" replacement.

### AP-1: Never shell out to claude CLI

**BAD** (actual code found in codebase):
```rust
let output = Command::new("claude")
    .arg("--print")
    .arg("--dangerously-skip-permissions")
    .arg(&prompt)
    .current_dir(&workdir)
    .output()
    .await?;
```

**GOOD** — use the provider abstraction:
```rust
let response = model_caller.call(ModelCallRequest {
    model: model_spec,
    messages: vec![...],
    ..Default::default()
}).await?;
```

**Why**: Shelling out bypasses cost tracking, feedback recording, provider health monitoring,
rate limiting, and model routing. It also makes the code untestable without a real claude binary.

---

### AP-2: Never inline prompt strings

**BAD**:
```rust
let prompt = format!(
    "You are the {} agent. Your task is to {}. \
     Follow these conventions: {}",
    role, task, conventions
);
```

**GOOD** — use PromptAssemblyService:
```rust
let prompt = prompt_assembler.assemble(PromptSpec {
    role: Some(role.to_string()),
    task: Some(task.clone()),
    ..Default::default()
}).await?;
```

**Why**: Inline prompts skip the 9-layer system prompt builder, miss anti-patterns, miss
conventions, miss gate feedback from prior iterations, and can't be A/B tested.

---

### AP-3: Never add execution logic to a specific entry point

**BAD** — putting gate execution in the CLI:
```rust
// in roko-cli/src/run.rs
async fn run_gates(workdir: &Path) -> Result<()> {
    let compile = CompileGate;
    let test = TestGate;
    compile.verify(&signal, &ctx).await?;
    test.verify(&signal, &ctx).await?;
}
```

**GOOD** — use the shared GateService:
```rust
let report = gate_runner.run_gates(GateConfig {
    workdir: workdir.to_path_buf(),
    enabled_gates: vec!["compile".into(), "test".into()],
    ..Default::default()
}).await?;
```

---

### AP-4: Never put decisions in the effect driver

**BAD**:
```rust
impl EffectDriver {
    async fn handle_gate_result(&self, result: &GateReport) {
        if result.all_passed() {
            self.run_reviewer().await;  // Decision!
        } else if self.iteration < self.max_iterations {
            self.run_autofix().await;   // Decision!
        }
    }
}
```

**GOOD** — decisions in the state machine, effects in the driver:
```rust
let action = pipeline_state.step(PipelineInput::GatesPassed);
match action {
    PipelineOutput::SpawnReviewer { .. } => driver.spawn_agent(...).await,
    PipelineOutput::Halt { reason } => driver.halt(reason).await,
}
```

---

### AP-5: Never hardcode roles

**BAD**:
```rust
if role == "implementer" {
    model = "claude-sonnet-4-20250514";
} else if role == "reviewer" {
    model = "claude-opus-4-20250514";
}
```

**GOOD** — roles come from config / CascadeRouter:
```rust
let model = cascade_router.select(&TaskRequirements {
    role: spec.role.clone(),
    ..Default::default()
});
```

---

### AP-6: Never skip feedback recording

**BAD**:
```rust
let response = provider.call(request).await?;
process_response(response);  // No feedback recorded!
```

**GOOD**:
```rust
let response = model_caller.call(request).await?;
// ModelCallService internally records to FeedbackSink
```

---

### AP-7: Never copy code between entry points

**BAD** — duplicating gate logic across CLI and ACP:
```rust
// roko-cli/src/run.rs
async fn cli_run_gates() { /* 50 lines */ }
// roko-acp/src/runner.rs
async fn acp_run_gates() { /* same 50 lines, slightly different */ }
```

**GOOD** — shared service:
```rust
let report = gate_service.run_gates(config).await?;
```

---

### AP-8: Never create local copies of foundation traits

**BAD** — the arch runner created local copies in effect_driver.rs:
```rust
// In roko-runtime/src/effect_driver.rs
pub trait ModelCaller: Send + Sync {           // LOCAL COPY — wrong!
    fn call(&self, ...) -> BoxFuture<...>;
}
```

**GOOD** — import from roko-core:
```rust
use roko_core::foundation::ModelCaller;        // Canonical trait
```

This is the #1 problem the convergence runner fixes. The arch runner created local trait
copies because of a crate cycle (roko-core depended on roko-runtime). Track F fixes the
crate cycle so all crates use the canonical roko-core traits.

---

### AP-9: Never create new crates

All 18 crates already exist. Your code goes into existing crate directories.

---

### AP-10: Never use `todo!()` in public APIs

```rust
// BAD
pub async fn stream(&self, req: ModelCallRequest) -> Result<ModelCallStream> {
    todo!()
}

// GOOD
pub async fn stream(&self, req: ModelCallRequest) -> Result<ModelCallStream> {
    let response = self.call(req).await?;
    Ok(ModelCallStream::from_complete(response))
}
```
