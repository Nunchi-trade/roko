## Batch P2D: WorkflowEngine Facade

### Write Scope
- **CREATE**: `crates/roko-runtime/src/workflow_engine.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod workflow_engine;` and re-export)

### Dependencies
- P2C (EffectDriver, EffectServices)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Shell out to any CLI
- Put decision logic here — delegate to PipelineStateV2

### Task

Create `WorkflowEngine` — the top-level facade that ties together PipelineStateV2 (state machine)
and EffectDriver (side-effects) into a simple async run loop.

This is the single entry point that CLI, ACP, and HTTP all use to execute a workflow.

#### File: `crates/roko-runtime/src/workflow_engine.rs`

```rust
//! WorkflowEngine — top-level workflow execution facade.
//!
//! Ties together PipelineStateV2 (decisions) and EffectDriver (effects)
//! into a run loop. This is the shared entry point for CLI, ACP, and HTTP.

use anyhow::Result;
use roko_core::foundation::{EventConsumer, FeedbackEvent};
use roko_core::runtime_event::{RuntimeEvent, WorkflowOutcome};
use std::path::PathBuf;
use std::sync::Arc;

use crate::effect_driver::{EffectDriver, EffectServices};
use crate::event_bus::emit_runtime_event;
use crate::pipeline_state::{PipelineStateV2, PipelineInput, PipelineOutput, WorkflowConfig};

/// Configuration for a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowRunConfig {
    /// User prompt
    pub prompt: String,
    /// Working directory
    pub workdir: PathBuf,
    /// Workflow configuration (express/standard/full)
    pub workflow: WorkflowConfig,
    /// Which gates to run
    pub enabled_gates: Vec<String>,
    /// Commit message prefix
    pub commit_prefix: Option<String>,
}

/// Result of a workflow run.
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    pub run_id: String,
    pub outcome: WorkflowOutcome,
    pub iterations: u32,
}

/// The top-level workflow execution engine.
///
/// Usage:
/// ```ignore
/// let engine = WorkflowEngine::new(services);
/// let result = engine.run(config).await?;
/// ```
pub struct WorkflowEngine {
    services: EffectServices,
    /// Optional event consumers to notify
    consumers: Vec<Arc<dyn EventConsumer>>,
}

impl WorkflowEngine {
    /// Create a new WorkflowEngine with the given services.
    pub fn new(services: EffectServices) -> Self {
        Self {
            services,
            consumers: Vec::new(),
        }
    }

    /// Add an event consumer that will be notified of all RuntimeEvents.
    pub fn add_consumer(&mut self, consumer: Arc<dyn EventConsumer>) {
        self.consumers.push(consumer);
    }

    /// Execute a workflow run.
    ///
    /// This is the main entry point. It:
    /// 1. Creates a PipelineStateV2 from the config
    /// 2. Creates an EffectDriver from the services
    /// 3. Runs the state machine loop: step → execute → feed back → repeat
    /// 4. Returns the outcome
    pub async fn run(&self, config: WorkflowRunConfig) -> Result<WorkflowResult> {
        let run_id = generate_run_id();

        // Initialize state machine
        let mut pipeline = PipelineStateV2::new(
            config.workflow.clone(),
            config.prompt.clone(),
        );

        // Initialize effect driver
        let driver = EffectDriver::new(
            EffectServices {
                model_caller: Arc::clone(&self.services.model_caller),
                prompt_assembler: Arc::clone(&self.services.prompt_assembler),
                feedback_sink: Arc::clone(&self.services.feedback_sink),
                gate_runner: Arc::clone(&self.services.gate_runner),
            },
            run_id.clone(),
            config.workdir.clone(),
        );

        // Emit workflow started
        let template_name = if config.workflow.has_strategy {
            "full"
        } else if config.workflow.has_review {
            "standard"
        } else {
            "express"
        };

        self.emit(RuntimeEvent::WorkflowStarted {
            run_id: run_id.clone(),
            template: template_name.to_string(),
            prompt: config.prompt.clone(),
        });

        // Start the state machine
        let mut output = pipeline.step(PipelineInput::Start);

        // Run loop: execute action → get result → feed back into state machine
        loop {
            let old_phase = pipeline.phase.label();

            let input = match &output {
                PipelineOutput::SpawnStrategist { prompt } => {
                    driver.spawn_agent("strategist", prompt, None).await
                }
                PipelineOutput::SpawnImplementer { prompt, context } => {
                    driver.spawn_agent("implementer", prompt, context.as_deref()).await
                }
                PipelineOutput::SpawnAutoFixer { error_output } => {
                    driver.spawn_agent("autofix", "Fix the following errors", Some(error_output)).await
                }
                PipelineOutput::SpawnReviewer { diff_context } => {
                    driver.spawn_agent("reviewer", "Review the changes", diff_context.as_deref()).await
                }
                PipelineOutput::RunGates => {
                    driver.run_gates(&config.enabled_gates).await
                }
                PipelineOutput::Commit => {
                    let message = config.commit_prefix
                        .as_deref()
                        .map(|p| format!("{}: {}", p, truncate(&config.prompt, 60)))
                        .unwrap_or_else(|| format!("feat: {}", truncate(&config.prompt, 60)));
                    driver.commit(&message).await
                }
                PipelineOutput::Done { outcome } => {
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: outcome.clone(),
                    });

                    // Record final feedback
                    let _ = self.services.feedback_sink.record(FeedbackEvent::WorkflowComplete {
                        run_id: run_id.clone(),
                        outcome: outcome.to_string(),
                        total_cost_usd: 0.0, // TODO(arch): accumulate from agent calls
                        total_tokens: 0,
                        duration_ms: 0,
                    }).await;

                    return Ok(WorkflowResult {
                        run_id,
                        outcome: outcome.clone(),
                        iterations: pipeline.iteration,
                    });
                }
                PipelineOutput::Halt { reason } => {
                    let outcome = WorkflowOutcome::Halted { reason: reason.clone() };
                    self.emit(RuntimeEvent::WorkflowCompleted {
                        run_id: run_id.clone(),
                        outcome: outcome.clone(),
                    });

                    return Ok(WorkflowResult {
                        run_id,
                        outcome,
                        iterations: pipeline.iteration,
                    });
                }
            };

            // Feed result back into the state machine
            output = pipeline.step(input);
            let new_phase = pipeline.phase.label();

            // Emit phase transition if phase changed
            if old_phase != new_phase {
                self.emit(RuntimeEvent::PhaseTransition {
                    run_id: run_id.clone(),
                    from: old_phase.to_string(),
                    to: new_phase.to_string(),
                });
            }
        }
    }

    fn emit(&self, event: RuntimeEvent) {
        // Notify local consumers
        for consumer in &self.consumers {
            consumer.consume(&event);
        }
        // Emit to global bus
        emit_runtime_event(event);
    }
}

fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("run_{:x}", ts)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod workflow_engine;
pub use workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowResult};
```

### Done Criteria
```bash
grep -q 'pub struct WorkflowEngine' crates/roko-runtime/src/workflow_engine.rs
grep -q 'pub async fn run' crates/roko-runtime/src/workflow_engine.rs
grep -q 'pub mod workflow_engine' crates/roko-runtime/src/lib.rs
! grep -rn 'Command::new' crates/roko-runtime/src/workflow_engine.rs
cargo check -p roko-runtime
```
