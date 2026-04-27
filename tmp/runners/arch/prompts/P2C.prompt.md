## Batch P2C: EffectDriver

### Write Scope
- **CREATE**: `crates/roko-runtime/src/effect_driver.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod effect_driver;` and re-export)

### Dependencies
- P1A (ModelCaller trait impl)
- P1B (PromptAssembler trait impl)
- P1C (FeedbackSink trait impl)
- P1D (GateRunner trait impl)
- P2A (PipelineStateV2, PipelineOutput)
- P2B (TaskScheduler)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Put DECISION LOGIC here — decisions live in PipelineStateV2 (the state machine)
- Shell out to `claude` CLI
- Use `if phase == ...` pattern checks — the state machine handles transitions

### Key Rule: The EffectDriver Does NOT Decide

The state machine (`PipelineStateV2`) decides what happens next by returning a `PipelineOutput`.
The EffectDriver just executes it. For example:

```
BAD:  if gates_passed { spawn_reviewer() }     ← decision in driver
GOOD: match output { SpawnReviewer => spawn() } ← driver just executes
```

### Task

Create `EffectDriver` — the component that executes `PipelineOutput` actions by delegating
to the foundation service traits (ModelCaller, PromptAssembler, GateRunner, FeedbackSink).

#### File: `crates/roko-runtime/src/effect_driver.rs`

```rust
//! EffectDriver — executes PipelineOutput actions via foundation services.
//!
//! The state machine (PipelineStateV2) decides WHAT to do by returning PipelineOutput.
//! The EffectDriver decides HOW by calling the foundation service traits.
//!
//! IMPORTANT: No decision logic here. If you find yourself writing
//! `if condition { ... } else { ... }` that determines the next workflow step,
//! that logic belongs in PipelineStateV2, not here.

use anyhow::Result;
use roko_core::foundation::{
    FeedbackEvent, FeedbackSink, GateRunner, GateConfig, ModelCallRequest,
    ModelCaller, PromptAssembler, PromptSpec, ChatMessage, MessageRole,
};
use roko_core::runtime_event::RuntimeEvent;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::PipelineInput;

/// Services required by the EffectDriver.
pub struct EffectServices {
    pub model_caller: Arc<dyn ModelCaller>,
    pub prompt_assembler: Arc<dyn PromptAssembler>,
    pub feedback_sink: Arc<dyn FeedbackSink>,
    pub gate_runner: Arc<dyn GateRunner>,
}

/// Drives workflow execution by translating PipelineOutput actions into
/// real side-effects via the foundation services.
pub struct EffectDriver {
    services: EffectServices,
    run_id: String,
    workdir: PathBuf,
}

impl EffectDriver {
    /// Create a new EffectDriver with the given services and context.
    pub fn new(services: EffectServices, run_id: String, workdir: PathBuf) -> Self {
        Self {
            services,
            run_id,
            workdir,
        }
    }

    /// Spawn an agent with the given role and prompt.
    ///
    /// Returns a PipelineInput::AgentCompleted or PipelineInput::AgentFailed
    /// that should be fed back into the state machine.
    pub async fn spawn_agent(
        &self,
        role: &str,
        user_prompt: &str,
        context: Option<&str>,
    ) -> PipelineInput {
        let agent_id = format!("{}_{}", role, uuid_short());

        // Build system prompt via PromptAssembler
        let system_prompt = match self.services.prompt_assembler.assemble(PromptSpec {
            role: Some(role.to_string()),
            task: Some(user_prompt.to_string()),
            gate_feedback: Vec::new(),
            ..Default::default()
        }).await {
            Ok(p) => p,
            Err(e) => {
                return PipelineInput::AgentFailed {
                    error: format!("Failed to assemble prompt: {}", e),
                };
            }
        };

        // Build the user message
        let mut user_content = user_prompt.to_string();
        if let Some(ctx) = context {
            user_content = format!("{}\n\n## Additional Context\n\n{}", user_content, ctx);
        }

        // Emit spawn event
        emit_runtime_event(RuntimeEvent::AgentSpawned {
            run_id: self.run_id.clone(),
            agent_id: agent_id.clone(),
            role: role.to_string(),
            model: String::new(), // resolved by ModelCaller
        });

        let start = Instant::now();

        // Call the model
        let result = self.services.model_caller.call(ModelCallRequest {
            model: String::new(), // let ModelCallService resolve
            system: Some(system_prompt),
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: user_content,
            }],
            max_tokens: None,
            temperature: None,
            role: Some(role.to_string()),
        }).await;

        let elapsed = start.elapsed();

        match result {
            Ok(response) => {
                // Record feedback
                let _ = self.services.feedback_sink.record(FeedbackEvent::ModelCall {
                    run_id: self.run_id.clone(),
                    model: response.model.clone(),
                    role: role.to_string(),
                    input_tokens: response.usage.input_tokens,
                    output_tokens: response.usage.output_tokens,
                    cost_usd: response.usage.cost_usd,
                    latency_ms: elapsed.as_millis() as u64,
                    success: true,
                }).await;

                // Emit completion event
                emit_runtime_event(RuntimeEvent::AgentCompleted {
                    run_id: self.run_id.clone(),
                    agent_id,
                    output: response.content.clone(),
                    tokens_used: response.usage.total_tokens,
                    cost_usd: response.usage.cost_usd,
                });

                PipelineInput::AgentCompleted {
                    output: response.content,
                    files_changed: 0, // TODO(arch): detect from git diff
                }
            }
            Err(e) => {
                // Emit failure event
                emit_runtime_event(RuntimeEvent::AgentFailed {
                    run_id: self.run_id.clone(),
                    agent_id,
                    error: e.to_string(),
                });

                PipelineInput::AgentFailed {
                    error: e.to_string(),
                }
            }
        }
    }

    /// Run verification gates.
    ///
    /// Returns PipelineInput::GatesPassed or PipelineInput::GateFailed.
    pub async fn run_gates(&self, enabled_gates: &[String]) -> PipelineInput {
        let config = GateConfig {
            workdir: self.workdir.clone(),
            enabled_gates: enabled_gates.to_vec(),
            max_rung: None,
        };

        let start = Instant::now();
        let result = self.services.gate_runner.run_gates(config).await;

        match result {
            Ok(report) => {
                let elapsed = start.elapsed();

                // Record feedback for each gate
                for verdict in &report.verdicts {
                    emit_runtime_event(if verdict.passed {
                        RuntimeEvent::GatePassed {
                            run_id: self.run_id.clone(),
                            gate_name: verdict.gate_name.clone(),
                            duration_ms: verdict.duration_ms,
                        }
                    } else {
                        RuntimeEvent::GateFailed {
                            run_id: self.run_id.clone(),
                            gate_name: verdict.gate_name.clone(),
                            output: verdict.output.clone(),
                            duration_ms: verdict.duration_ms,
                        }
                    });

                    let _ = self.services.feedback_sink.record(FeedbackEvent::GateResult {
                        run_id: self.run_id.clone(),
                        gate_name: verdict.gate_name.clone(),
                        passed: verdict.passed,
                        duration_ms: verdict.duration_ms,
                    }).await;
                }

                if report.all_passed() {
                    PipelineInput::GatesPassed
                } else {
                    let failure = report.first_failure().unwrap();
                    PipelineInput::GateFailed {
                        gate: failure.gate_name.clone(),
                        output: report.failure_summary(),
                    }
                }
            }
            Err(e) => {
                PipelineInput::GateFailed {
                    gate: "gate_runner".to_string(),
                    output: e.to_string(),
                }
            }
        }
    }

    /// Create a git commit.
    ///
    /// Returns PipelineInput::CommitDone.
    pub async fn commit(&self, message: &str) -> PipelineInput {
        // Use git commands to create a commit
        let result = tokio::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.workdir)
            .output()
            .await;

        if let Err(e) = result {
            return PipelineInput::AgentFailed {
                error: format!("git add failed: {}", e),
            };
        }

        let result = tokio::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.workdir)
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                // Get the commit hash
                let hash_output = tokio::process::Command::new("git")
                    .args(["rev-parse", "--short", "HEAD"])
                    .current_dir(&self.workdir)
                    .output()
                    .await;

                let hash = hash_output
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                PipelineInput::CommitDone { hash }
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("nothing to commit") {
                    PipelineInput::CommitDone { hash: "noop".to_string() }
                } else {
                    PipelineInput::AgentFailed {
                        error: format!("git commit failed: {}", stderr),
                    }
                }
            }
            Err(e) => {
                PipelineInput::AgentFailed {
                    error: format!("git commit failed: {}", e),
                }
            }
        }
    }

    /// Emit a runtime event directly.
    pub fn emit(&self, event: RuntimeEvent) {
        emit_runtime_event(event);
    }
}

/// Generate a short unique ID for agent instances.
fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{:x}", ts & 0xFFFF_FFFF)
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod effect_driver;
pub use effect_driver::{EffectDriver, EffectServices};
```

### Done Criteria
```bash
grep -q 'pub struct EffectDriver' crates/roko-runtime/src/effect_driver.rs
grep -q 'pub async fn spawn_agent' crates/roko-runtime/src/effect_driver.rs
grep -q 'pub async fn run_gates' crates/roko-runtime/src/effect_driver.rs
grep -q 'pub mod effect_driver' crates/roko-runtime/src/lib.rs
! grep -rn 'Command::new.*claude' crates/roko-runtime/src/effect_driver.rs
! grep -rn 'if.*phase.*==' crates/roko-runtime/src/effect_driver.rs
cargo check -p roko-runtime
```
