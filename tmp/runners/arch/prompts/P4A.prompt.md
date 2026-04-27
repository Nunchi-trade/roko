## Batch P4A: Wire CLI Entry Points

### Write Scope
- **MODIFY**: `crates/roko-cli/src/run.rs` (add WorkflowEngine path)

### Dependencies
- P2D (WorkflowEngine)

### DO NOT
- Remove existing code — ADD the WorkflowEngine path alongside existing code
- Modify any other files beyond the write scope
- Add Cargo.toml dependencies unless roko-runtime is not already a dependency
- Shell out to `claude` CLI for the new path

### Existing Code Context

`crates/roko-cli/src/run.rs` currently dispatches prompts through the existing orchestration
code. We want to add a parallel code path that uses `WorkflowEngine` from `roko-runtime`,
selectable via a config option or flag.

### Task

Add a function in `run.rs` that uses `WorkflowEngine` to execute a workflow. This does NOT
replace the existing path — it adds an alternative that can be selected.

#### Modifications to `crates/roko-cli/src/run.rs`

Add a new function (do NOT replace existing functions):

```rust
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig};
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::effect_driver::EffectServices;
use roko_core::foundation::{ModelCaller, PromptAssembler, FeedbackSink, GateRunner};

/// Execute a prompt via the new WorkflowEngine (event-driven architecture).
///
/// This is an alternative to the existing orchestrate.rs path. It uses:
/// - PipelineStateV2 for state machine decisions
/// - EffectDriver for side-effect execution
/// - RuntimeEvent bus for observability
///
/// Enable via config or `--engine v2` flag (to be wired).
pub async fn run_with_workflow_engine(
    prompt: &str,
    workdir: &std::path::Path,
    workflow_template: &str,
    enabled_gates: Vec<String>,
) -> anyhow::Result<()> {
    // Import the concrete service implementations
    use roko_agent::model_call_service::ModelCallService;
    use roko_compose::prompt_assembly_service::PromptAssemblyService;
    use roko_learn::feedback_service::FeedbackService;
    use roko_gate::gate_service::GateService;
    use std::sync::Arc;

    // Build services
    let model_caller: Arc<dyn ModelCaller> = Arc::new(
        ModelCallService::new("claude-sonnet-4-20250514".to_string()),
    );
    let prompt_assembler: Arc<dyn PromptAssembler> = Arc::new(
        PromptAssemblyService::new(),
    );
    let feedback_sink: Arc<dyn FeedbackSink> = Arc::new(
        FeedbackService::from_roko_dir(&workdir.join(".roko")),
    );
    let gate_runner: Arc<dyn GateRunner> = Arc::new(
        GateService::new(),
    );

    let services = EffectServices {
        model_caller,
        prompt_assembler,
        feedback_sink,
        gate_runner,
    };

    // Select workflow config
    let workflow = match workflow_template {
        "express" => WorkflowConfig::express(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::standard(),
    };

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow,
        enabled_gates,
        commit_prefix: Some("feat".to_string()),
    };

    // Run the workflow
    let engine = WorkflowEngine::new(services);
    let result = engine.run(config).await?;

    println!("Workflow complete: {} ({})", result.run_id, result.outcome);
    println!("Iterations: {}", result.iterations);

    Ok(())
}
```

**Important**: Read the actual `run.rs` file first to understand its current structure. The
function above should fit naturally alongside existing code. Do NOT restructure the file.

If `run.rs` has a main dispatch function, add a comment showing where the new path would be
called from, but do NOT modify the dispatch logic — that's a separate wiring task.

### Done Criteria
```bash
grep -q 'WorkflowEngine\|workflow_engine' crates/roko-cli/src/run.rs
grep -q 'run_with_workflow_engine' crates/roko-cli/src/run.rs
! grep -rn 'Command::new.*claude' crates/roko-cli/src/run.rs
cargo check -p roko-cli
```
