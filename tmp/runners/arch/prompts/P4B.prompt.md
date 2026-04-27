## Batch P4B: Wire ACP Entry Points

### Write Scope
- **MODIFY**: `crates/roko-acp/src/runner.rs` (add WorkflowEngine path)
- **MODIFY**: `crates/roko-acp/src/bridge_events.rs` (add RuntimeEvent consumer registration)

### Dependencies
- P2D (WorkflowEngine)
- P3A (AcpAdapter)

### DO NOT
- Remove existing code — ADD alongside existing code
- Modify any other files
- Add Cargo.toml dependencies unless roko-runtime is not already a dependency
- Shell out to `claude` CLI for the new path

### Existing Code Context

`runner.rs` has `run_workflow_pipeline()` which drives the existing ACP-specific pipeline.
We want to add an alternative path that uses `WorkflowEngine` from `roko-runtime`.

`bridge_events.rs` has the dispatch logic that routes user prompts to execution. It
already handles `CognitiveEvent`s.

### Task

#### Modifications to `crates/roko-acp/src/runner.rs`

Add a new function that creates a WorkflowEngine with an AcpAdapter consumer:

```rust
use roko_runtime::workflow_engine::{WorkflowEngine, WorkflowRunConfig, WorkflowResult};
use roko_runtime::pipeline_state::WorkflowConfig;
use roko_runtime::effect_driver::EffectServices;
use roko_core::foundation::{ModelCaller, PromptAssembler, FeedbackSink, GateRunner};
use crate::acp_adapter::AcpAdapter;
use crate::bridge_events::CognitiveEvent;
use std::sync::Arc;

/// Execute a prompt via WorkflowEngine, bridging events to ACP protocol.
///
/// This is an alternative to run_workflow_pipeline() that uses the shared
/// WorkflowEngine architecture. Events are bridged to the ACP session via
/// AcpAdapter (EventConsumer → CognitiveEvent → session updates).
pub async fn run_with_workflow_engine(
    session_id: &str,
    prompt: &str,
    workdir: &std::path::Path,
    template: &str,
    event_sender: tokio::sync::mpsc::Sender<CognitiveEvent>,
) -> anyhow::Result<WorkflowResult> {
    use roko_agent::model_call_service::ModelCallService;
    use roko_compose::prompt_assembly_service::PromptAssemblyService;
    use roko_learn::feedback_service::FeedbackService;
    use roko_gate::gate_service::GateService;

    // Build foundation services
    let services = EffectServices {
        model_caller: Arc::new(ModelCallService::new("claude-sonnet-4-20250514".to_string())),
        prompt_assembler: Arc::new(PromptAssemblyService::new()),
        feedback_sink: Arc::new(FeedbackService::from_roko_dir(&workdir.join(".roko"))),
        gate_runner: Arc::new(GateService::new()),
    };

    // Create workflow config
    let workflow = match template {
        "express" => WorkflowConfig::express(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::standard(),
    };

    let config = WorkflowRunConfig {
        prompt: prompt.to_string(),
        workdir: workdir.to_path_buf(),
        workflow,
        enabled_gates: vec!["compile".into(), "test".into()],
        commit_prefix: Some("feat".to_string()),
    };

    // Create engine with ACP adapter
    let mut engine = WorkflowEngine::new(services);

    // Generate a run_id for the adapter to filter on
    // The engine generates its own run_id internally, but we need one
    // for the adapter. Use a deterministic one based on session.
    let run_id = format!("acp_{}", session_id);
    let adapter = AcpAdapter::new(
        session_id.to_string(),
        run_id,
        event_sender,
    );
    engine.add_consumer(Arc::new(adapter));

    // Run the workflow
    engine.run(config).await
}
```

#### Modifications to `crates/roko-acp/src/bridge_events.rs`

Add a comment and an import showing where the new path connects. Do NOT modify the existing
dispatch logic — just ensure the new function is importable and add a comment:

```rust
// Near the top of the file, add:
// TODO(arch): Wire run_with_workflow_engine as alternative to run_workflow_pipeline
// When workflow config selects v2 engine, call:
//   runner::run_with_workflow_engine(session_id, prompt, workdir, template, event_sender).await
```

Read the actual `bridge_events.rs` to find the right place for this comment. It should be
near the dispatch point where prompts are sent to execution.

### Done Criteria
```bash
grep -q 'run_with_workflow_engine' crates/roko-acp/src/runner.rs
grep -q 'WorkflowEngine\|workflow_engine' crates/roko-acp/src/runner.rs
grep -q 'AcpAdapter\|acp_adapter' crates/roko-acp/src/runner.rs
! grep -rn 'Command::new.*claude' crates/roko-acp/src/runner.rs
cargo check -p roko-acp
```
