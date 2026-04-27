## Batch P2A: PipelineState v2 (Config-Driven)

### Write Scope
- **CREATE**: `crates/roko-runtime/src/pipeline_state.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod pipeline_state;` and re-export)

### Dependencies
- P0A (RuntimeEvent types — for WorkflowOutcome)

### DO NOT
- Modify any other files
- Modify the existing PipelineState in `roko-acp/src/pipeline.rs` (that's ACP-specific)
- Add Cargo.toml dependencies
- Put side-effects in the state machine (this is a PURE state machine)

### Context

There is already a `PipelineState` in `roko-acp/src/pipeline.rs`. That one is ACP-specific.
This new `PipelineStateV2` is the shared, entry-point-agnostic state machine that all
consumers (CLI, ACP, HTTP) will use. It lives in `roko-runtime` because it's infrastructure.

The key difference: the ACP pipeline has 9 phases with ACP-specific concerns baked in.
PipelineStateV2 is config-driven — which phases to include (strategy, review, etc.) comes
from a `WorkflowConfig` struct, not hardcoded logic.

### Task

Create a pure state machine `PipelineStateV2` with config-driven phase selection.

#### File: `crates/roko-runtime/src/pipeline_state.rs`

```rust
//! PipelineStateV2 — config-driven workflow state machine.
//!
//! This is a PURE state machine with no side effects. It takes events and
//! returns actions. The EffectDriver (P2C) executes the actions.
//!
//! Config determines which phases are active:
//! - Express:  implement → gate → commit
//! - Standard: implement → gate → review → commit
//! - Full:     strategy → implement → gate → review → commit

use roko_core::runtime_event::WorkflowOutcome;

/// Configuration for the pipeline. Determines which phases are active.
#[derive(Debug, Clone)]
pub struct WorkflowConfig {
    /// Include a strategist phase before implementation
    pub has_strategy: bool,
    /// Include a review phase after gates pass
    pub has_review: bool,
    /// Maximum implement → gate → review iterations
    pub max_iterations: u32,
    /// Maximum autofix attempts per gate failure
    pub max_autofix_attempts: u32,
}

impl WorkflowConfig {
    /// Express: implement → gate → commit
    pub fn express() -> Self {
        Self {
            has_strategy: false,
            has_review: false,
            max_iterations: 1,
            max_autofix_attempts: 1,
        }
    }

    /// Standard: implement → gate → review → commit
    pub fn standard() -> Self {
        Self {
            has_strategy: false,
            has_review: true,
            max_iterations: 2,
            max_autofix_attempts: 2,
        }
    }

    /// Full: strategy → implement → gate → review → commit
    pub fn full() -> Self {
        Self {
            has_strategy: true,
            has_review: true,
            max_iterations: 3,
            max_autofix_attempts: 2,
        }
    }
}

/// Current phase of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Pending,
    Strategizing,
    Implementing,
    Gating,
    AutoFixing,
    Reviewing,
    Committing,
    Complete,
    Halted { reason: String },
    Cancelled,
}

impl Phase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Phase::Complete | Phase::Halted { .. } | Phase::Cancelled)
    }

    pub fn label(&self) -> &'static str {
        match self {
            Phase::Pending => "pending",
            Phase::Strategizing => "strategizing",
            Phase::Implementing => "implementing",
            Phase::Gating => "gating",
            Phase::AutoFixing => "auto_fixing",
            Phase::Reviewing => "reviewing",
            Phase::Committing => "committing",
            Phase::Complete => "complete",
            Phase::Halted { .. } => "halted",
            Phase::Cancelled => "cancelled",
        }
    }
}

/// Events fed into the state machine.
#[derive(Debug, Clone)]
pub enum PipelineInput {
    /// Start the pipeline
    Start,
    /// Strategist completed with a brief
    StrategyComplete { brief: String },
    /// Strategy phase was skipped
    StrategySkipped,
    /// Agent completed with output
    AgentCompleted { output: String, files_changed: u32 },
    /// Agent failed
    AgentFailed { error: String },
    /// All gates passed
    GatesPassed,
    /// A gate failed
    GateFailed { gate: String, output: String },
    /// Review approved
    ReviewApproved { summary: String },
    /// Review requests revisions
    ReviewRevise { findings: Vec<String> },
    /// Commit done
    CommitDone { hash: String },
    /// User cancelled
    UserCancel,
    /// Timeout or budget exceeded
    ResourceExhausted { reason: String },
}

/// Actions the state machine asks the effect driver to execute.
#[derive(Debug, Clone)]
pub enum PipelineOutput {
    /// Spawn a strategist agent
    SpawnStrategist { prompt: String },
    /// Spawn an implementer agent
    SpawnImplementer { prompt: String, context: Option<String> },
    /// Spawn an autofix agent
    SpawnAutoFixer { error_output: String },
    /// Run verification gates
    RunGates,
    /// Spawn a reviewer agent
    SpawnReviewer { diff_context: Option<String> },
    /// Create a commit
    Commit,
    /// Pipeline is done
    Done { outcome: WorkflowOutcome },
    /// Pipeline is halted
    Halt { reason: String },
}

/// Pure state machine for workflow pipelines.
#[derive(Debug, Clone)]
pub struct PipelineStateV2 {
    pub phase: Phase,
    pub config: WorkflowConfig,
    pub iteration: u32,
    pub autofix_attempts: u32,
    pub original_prompt: String,
    pub strategist_brief: Option<String>,
    pub review_findings: Vec<String>,
    pub last_gate_failure: Option<String>,
    pub files_changed: u32,
    pub commit_hash: Option<String>,
}

impl PipelineStateV2 {
    pub fn new(config: WorkflowConfig, prompt: String) -> Self {
        Self {
            phase: Phase::Pending,
            config,
            iteration: 0,
            autofix_attempts: 0,
            original_prompt: prompt,
            strategist_brief: None,
            review_findings: Vec::new(),
            last_gate_failure: None,
            files_changed: 0,
            commit_hash: None,
        }
    }

    /// Feed an event into the state machine, get an action back.
    /// This is the ONLY way to drive the state machine. No side-effects here.
    pub fn step(&mut self, input: PipelineInput) -> PipelineOutput {
        match (&self.phase, input) {
            // ── Start ──
            (Phase::Pending, PipelineInput::Start) => {
                if self.config.has_strategy {
                    self.phase = Phase::Strategizing;
                    PipelineOutput::SpawnStrategist {
                        prompt: self.original_prompt.clone(),
                    }
                } else {
                    self.phase = Phase::Implementing;
                    self.iteration = 1;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: None,
                    }
                }
            }

            // ── Strategy ──
            (Phase::Strategizing, PipelineInput::StrategyComplete { brief }) => {
                self.strategist_brief = Some(brief.clone());
                self.phase = Phase::Implementing;
                self.iteration = 1;
                PipelineOutput::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: Some(brief),
                }
            }
            (Phase::Strategizing, PipelineInput::StrategySkipped) => {
                self.phase = Phase::Implementing;
                self.iteration = 1;
                PipelineOutput::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: None,
                }
            }

            // ── Implementation ──
            (Phase::Implementing, PipelineInput::AgentCompleted { output: _, files_changed }) => {
                self.files_changed = files_changed;
                self.phase = Phase::Gating;
                self.autofix_attempts = 0;
                PipelineOutput::RunGates
            }
            (Phase::Implementing, PipelineInput::AgentFailed { error }) => {
                self.phase = Phase::Halted { reason: error.clone() };
                PipelineOutput::Halt { reason: error }
            }

            // ── Gating ──
            (Phase::Gating, PipelineInput::GatesPassed) => {
                if self.config.has_review {
                    self.phase = Phase::Reviewing;
                    PipelineOutput::SpawnReviewer { diff_context: None }
                } else {
                    self.phase = Phase::Committing;
                    PipelineOutput::Commit
                }
            }
            (Phase::Gating, PipelineInput::GateFailed { gate, output }) => {
                self.last_gate_failure = Some(output.clone());
                if self.autofix_attempts < self.config.max_autofix_attempts {
                    self.autofix_attempts += 1;
                    self.phase = Phase::AutoFixing;
                    PipelineOutput::SpawnAutoFixer { error_output: output }
                } else if self.iteration < self.config.max_iterations {
                    // Retry full implementation with gate feedback
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: Some(format!(
                            "Previous attempt failed gate '{}'. Error:\n{}",
                            gate, output
                        )),
                    }
                } else {
                    let reason = format!("Gate '{}' failed after {} iterations", gate, self.iteration);
                    self.phase = Phase::Halted { reason: reason.clone() };
                    PipelineOutput::Halt { reason }
                }
            }

            // ── AutoFix ──
            (Phase::AutoFixing, PipelineInput::AgentCompleted { .. }) => {
                self.phase = Phase::Gating;
                PipelineOutput::RunGates
            }
            (Phase::AutoFixing, PipelineInput::AgentFailed { error }) => {
                // Autofix failed — try full re-implementation if iterations remain
                if self.iteration < self.config.max_iterations {
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: self.last_gate_failure.clone(),
                    }
                } else {
                    let reason = format!("Autofix failed: {}", error);
                    self.phase = Phase::Halted { reason: reason.clone() };
                    PipelineOutput::Halt { reason }
                }
            }

            // ── Review ──
            (Phase::Reviewing, PipelineInput::ReviewApproved { .. }) => {
                self.phase = Phase::Committing;
                PipelineOutput::Commit
            }
            (Phase::Reviewing, PipelineInput::ReviewRevise { findings }) => {
                self.review_findings.extend(findings);
                if self.iteration < self.config.max_iterations {
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    let feedback = self.review_findings.join("\n- ");
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: Some(format!("Review findings:\n- {}", feedback)),
                    }
                } else {
                    // Max iterations reached, commit anyway
                    self.phase = Phase::Committing;
                    PipelineOutput::Commit
                }
            }

            // ── Commit ──
            (Phase::Committing, PipelineInput::CommitDone { hash }) => {
                self.commit_hash = Some(hash.clone());
                self.phase = Phase::Complete;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Success {
                        commit_hash: Some(hash),
                    },
                }
            }

            // ── Universal transitions ──
            (_, PipelineInput::UserCancel) => {
                self.phase = Phase::Cancelled;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Cancelled,
                }
            }
            (_, PipelineInput::ResourceExhausted { reason }) => {
                self.phase = Phase::Halted { reason: reason.clone() };
                PipelineOutput::Halt { reason }
            }

            // ── Invalid transition ──
            (phase, input) => {
                let reason = format!(
                    "Invalid transition: {:?} in phase {:?}",
                    std::mem::discriminant(&input),
                    phase
                );
                self.phase = Phase::Halted { reason: reason.clone() };
                PipelineOutput::Halt { reason }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn express_happy_path() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::express(),
            "fix bug".into(),
        );

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.phase, Phase::Implementing);

        let out = sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 2 });
        assert!(matches!(out, PipelineOutput::RunGates));

        let out = sm.step(PipelineInput::GatesPassed);
        assert!(matches!(out, PipelineOutput::Commit));

        let out = sm.step(PipelineInput::CommitDone { hash: "abc".into() });
        assert!(matches!(out, PipelineOutput::Done { .. }));
        assert_eq!(sm.phase, Phase::Complete);
    }

    #[test]
    fn standard_with_review() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "add feature".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 3 });
        sm.step(PipelineInput::GatesPassed);

        assert_eq!(sm.phase, Phase::Reviewing);

        let out = sm.step(PipelineInput::ReviewApproved { summary: "lgtm".into() });
        assert!(matches!(out, PipelineOutput::Commit));
    }

    #[test]
    fn full_with_strategy() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::full(),
            "complex task".into(),
        );

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnStrategist { .. }));
        assert_eq!(sm.phase, Phase::Strategizing);

        let out = sm.step(PipelineInput::StrategyComplete { brief: "plan".into() });
        assert!(matches!(out, PipelineOutput::SpawnImplementer { context: Some(_), .. }));
    }

    #[test]
    fn gate_failure_triggers_autofix() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "fix".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "done".into(), files_changed: 1 });

        let out = sm.step(PipelineInput::GateFailed {
            gate: "compile".into(),
            output: "error[E0308]".into(),
        });
        assert!(matches!(out, PipelineOutput::SpawnAutoFixer { .. }));
        assert_eq!(sm.phase, Phase::AutoFixing);
    }

    #[test]
    fn cancel_from_any_phase() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::express(),
            "task".into(),
        );
        sm.step(PipelineInput::Start);

        let out = sm.step(PipelineInput::UserCancel);
        assert!(matches!(out, PipelineOutput::Done { outcome: WorkflowOutcome::Cancelled }));
        assert_eq!(sm.phase, Phase::Cancelled);
    }

    #[test]
    fn review_revise_triggers_reimplementation() {
        let mut sm = PipelineStateV2::new(
            WorkflowConfig::standard(),
            "feature".into(),
        );

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted { output: "v1".into(), files_changed: 2 });
        sm.step(PipelineInput::GatesPassed);

        let out = sm.step(PipelineInput::ReviewRevise {
            findings: vec!["needs error handling".into()],
        });
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.iteration, 2);
    }
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod pipeline_state;
pub use pipeline_state::{PipelineStateV2, WorkflowConfig, Phase, PipelineInput, PipelineOutput};
```

### Done Criteria
```bash
grep -q 'pub struct PipelineStateV2' crates/roko-runtime/src/pipeline_state.rs
grep -q 'pub fn step' crates/roko-runtime/src/pipeline_state.rs
grep -q 'pub mod pipeline_state' crates/roko-runtime/src/lib.rs
cargo check -p roko-runtime
cargo test -p roko-runtime --lib -- pipeline_state
```
