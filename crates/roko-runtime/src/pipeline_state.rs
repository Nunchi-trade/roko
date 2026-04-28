//! PipelineStateV2: config-driven workflow state machine.
//!
//! This is a PURE state machine with no side effects. It takes events and
//! returns actions. The effect driver executes the actions.
//!
//! Config determines which phases are active:
//! - Express: implement -> gate -> commit
//! - Standard: implement -> gate -> review -> commit
//! - Full: strategy -> implement -> gate -> review -> commit

// TODO(arch): Use roko_core::runtime_event::WorkflowOutcome once the crate
// dependency graph matches the architecture reference. In this checkout,
// roko-core currently depends on roko-runtime, so importing roko-core here
// would create a circular dependency and Cargo.toml edits are out of scope.
/// Outcome of a completed workflow run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowOutcome {
    /// Workflow completed successfully, optionally with a commit hash.
    Success {
        /// Commit hash created by the workflow, when commit creation was requested.
        commit_hash: Option<String>,
    },
    /// Workflow halted due to an error or resource limit.
    Halted {
        /// Human-readable halt reason.
        reason: String,
    },
    /// Workflow was cancelled by the user.
    Cancelled,
}

/// Configuration for the pipeline. Determines which phases are active.
#[derive(Debug, Clone)]
pub struct WorkflowConfig {
    /// Include a strategist phase before implementation.
    pub has_strategy: bool,
    /// Include a review phase after gates pass.
    pub has_review: bool,
    /// Maximum implement -> gate -> review iterations.
    pub max_iterations: u32,
    /// Maximum autofix attempts per gate failure.
    pub max_autofix_attempts: u32,
}

impl WorkflowConfig {
    /// Express: implement -> gate -> commit.
    pub fn express() -> Self {
        Self {
            has_strategy: false,
            has_review: false,
            max_iterations: 1,
            max_autofix_attempts: 1,
        }
    }

    /// Standard: implement -> gate -> review -> commit.
    pub fn standard() -> Self {
        Self {
            has_strategy: false,
            has_review: true,
            max_iterations: 2,
            max_autofix_attempts: 2,
        }
    }

    /// Full: strategy -> implement -> gate -> review -> commit.
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
    /// Pipeline has been created but not started.
    Pending,
    /// Strategy phase is running.
    Strategizing,
    /// Implementation phase is running.
    Implementing,
    /// Verification gates are running.
    Gating,
    /// Autofix phase is running after a gate failure.
    AutoFixing,
    /// Review phase is running after gates pass.
    Reviewing,
    /// Commit creation is running.
    Committing,
    /// Workflow completed successfully.
    Complete,
    /// Workflow halted before completion.
    Halted {
        /// Human-readable halt reason.
        reason: String,
    },
    /// Workflow was cancelled by the user.
    Cancelled,
}

impl Phase {
    /// Returns true when no further state transitions should be accepted.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Phase::Complete | Phase::Halted { .. } | Phase::Cancelled
        )
    }

    /// Stable lowercase label for logs, events, and UI adapters.
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
    /// Start the pipeline.
    Start,
    /// Strategist completed with a brief.
    StrategyComplete {
        /// Strategy brief to pass into implementation context.
        brief: String,
    },
    /// Strategy phase was skipped.
    StrategySkipped,
    /// Agent completed with output.
    AgentCompleted {
        /// Final textual output from the agent.
        output: String,
        /// Number of files changed by the agent.
        files_changed: u32,
    },
    /// Agent failed.
    AgentFailed {
        /// Human-readable agent error.
        error: String,
    },
    /// All gates passed.
    GatesPassed,
    /// A gate failed.
    GateFailed {
        /// Gate name that failed.
        gate: String,
        /// Gate output or diagnostic text.
        output: String,
    },
    /// Review approved.
    ReviewApproved {
        /// Review summary supplied by the reviewer.
        summary: String,
    },
    /// Review requests revisions.
    ReviewRevise {
        /// Findings that should guide the next implementation pass.
        findings: Vec<String>,
    },
    /// Commit done.
    CommitDone {
        /// Commit hash created by the effect driver.
        hash: String,
    },
    /// User cancelled.
    UserCancel,
    /// Timeout or budget exceeded.
    ResourceExhausted {
        /// Human-readable resource exhaustion reason.
        reason: String,
    },
}

/// Actions the state machine asks the effect driver to execute.
#[derive(Debug, Clone)]
pub enum PipelineOutput {
    /// Spawn a strategist agent.
    SpawnStrategist {
        /// Prompt to send to the strategist.
        prompt: String,
    },
    /// Spawn an implementer agent.
    SpawnImplementer {
        /// Original user prompt for the implementer.
        prompt: String,
        /// Optional strategy, gate, or review context for this iteration.
        context: Option<String>,
    },
    /// Spawn an autofix agent.
    SpawnAutoFixer {
        /// Gate output that the autofix agent should address.
        error_output: String,
    },
    /// Run verification gates.
    RunGates,
    /// Spawn a reviewer agent.
    SpawnReviewer {
        /// Optional diff context for the reviewer.
        diff_context: Option<String>,
    },
    /// Create a commit.
    Commit,
    /// Pipeline is done.
    Done {
        /// Final workflow outcome.
        outcome: WorkflowOutcome,
    },
    /// Pipeline is halted.
    Halt {
        /// Human-readable halt reason.
        reason: String,
    },
}

/// Pure state machine for workflow pipelines.
#[derive(Debug, Clone)]
pub struct PipelineStateV2 {
    /// Current pipeline phase.
    pub phase: Phase,
    /// Workflow phase and retry configuration.
    pub config: WorkflowConfig,
    /// Current implementation iteration, starting at 1 once implementation begins.
    pub iteration: u32,
    /// Autofix attempts used for the current gate failure.
    pub autofix_attempts: u32,
    /// Original user prompt for the workflow.
    pub original_prompt: String,
    /// Optional strategist brief from the strategy phase.
    pub strategist_brief: Option<String>,
    /// Accumulated review findings across iterations.
    pub review_findings: Vec<String>,
    /// Most recent gate failure output.
    pub last_gate_failure: Option<String>,
    /// Number of files changed by the most recent implementation pass.
    pub files_changed: u32,
    /// Commit hash produced when the workflow completes successfully.
    pub commit_hash: Option<String>,
}

impl PipelineStateV2 {
    /// Create a new pipeline state machine in the pending phase.
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
    /// This is the ONLY way to drive the state machine. No side effects here.
    pub fn step(&mut self, input: PipelineInput) -> PipelineOutput {
        match (&self.phase, input) {
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

            (
                Phase::Implementing,
                PipelineInput::AgentCompleted {
                    output: _,
                    files_changed,
                },
            ) => {
                self.files_changed = files_changed;
                self.phase = Phase::Gating;
                self.autofix_attempts = 0;
                PipelineOutput::RunGates
            }
            (Phase::Implementing, PipelineInput::AgentFailed { error }) => {
                self.phase = Phase::Halted {
                    reason: error.clone(),
                };
                PipelineOutput::Halt { reason: error }
            }

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
                    PipelineOutput::SpawnAutoFixer {
                        error_output: output,
                    }
                } else if self.iteration < self.config.max_iterations {
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
                    let reason =
                        format!("Gate '{}' failed after {} iterations", gate, self.iteration);
                    self.phase = Phase::Halted {
                        reason: reason.clone(),
                    };
                    PipelineOutput::Halt { reason }
                }
            }

            (Phase::AutoFixing, PipelineInput::AgentCompleted { .. }) => {
                self.phase = Phase::Gating;
                PipelineOutput::RunGates
            }
            (Phase::AutoFixing, PipelineInput::AgentFailed { error }) => {
                if self.iteration < self.config.max_iterations {
                    self.iteration += 1;
                    self.autofix_attempts = 0;
                    self.phase = Phase::Implementing;
                    PipelineOutput::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: self.last_gate_failure.clone(),
                    }
                } else {
                    let reason = format!("Autofix failed: {error}");
                    self.phase = Phase::Halted {
                        reason: reason.clone(),
                    };
                    PipelineOutput::Halt { reason }
                }
            }

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
                        context: Some(format!("Review findings:\n- {feedback}")),
                    }
                } else {
                    self.phase = Phase::Committing;
                    PipelineOutput::Commit
                }
            }

            (Phase::Committing, PipelineInput::CommitDone { hash }) => {
                self.commit_hash = Some(hash.clone());
                self.phase = Phase::Complete;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Success {
                        commit_hash: Some(hash),
                    },
                }
            }

            (_, PipelineInput::UserCancel) => {
                self.phase = Phase::Cancelled;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Cancelled,
                }
            }
            (_, PipelineInput::ResourceExhausted { reason }) => {
                self.phase = Phase::Halted {
                    reason: reason.clone(),
                };
                PipelineOutput::Halt { reason }
            }

            (phase, input) => {
                let reason = format!(
                    "Invalid transition: {:?} in phase {:?}",
                    std::mem::discriminant(&input),
                    phase
                );
                self.phase = Phase::Halted {
                    reason: reason.clone(),
                };
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
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "fix bug".into());

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.phase, Phase::Implementing);

        let out = sm.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 2,
        });
        assert!(matches!(out, PipelineOutput::RunGates));

        let out = sm.step(PipelineInput::GatesPassed);
        assert!(matches!(out, PipelineOutput::Commit));

        let out = sm.step(PipelineInput::CommitDone { hash: "abc".into() });
        assert!(matches!(out, PipelineOutput::Done { .. }));
        assert_eq!(sm.phase, Phase::Complete);
    }

    #[test]
    fn standard_with_review() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::standard(), "add feature".into());

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 3,
        });
        sm.step(PipelineInput::GatesPassed);

        assert_eq!(sm.phase, Phase::Reviewing);

        let out = sm.step(PipelineInput::ReviewApproved {
            summary: "lgtm".into(),
        });
        assert!(matches!(out, PipelineOutput::Commit));
    }

    #[test]
    fn full_with_strategy() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::full(), "complex task".into());

        let out = sm.step(PipelineInput::Start);
        assert!(matches!(out, PipelineOutput::SpawnStrategist { .. }));
        assert_eq!(sm.phase, Phase::Strategizing);

        let out = sm.step(PipelineInput::StrategyComplete {
            brief: "plan".into(),
        });
        assert!(matches!(
            out,
            PipelineOutput::SpawnImplementer {
                context: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn gate_failure_triggers_autofix() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::standard(), "fix".into());

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 1,
        });

        let out = sm.step(PipelineInput::GateFailed {
            gate: "compile".into(),
            output: "error[E0308]".into(),
        });
        assert!(matches!(out, PipelineOutput::SpawnAutoFixer { .. }));
        assert_eq!(sm.phase, Phase::AutoFixing);
    }

    #[test]
    fn cancel_from_any_phase() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "task".into());
        sm.step(PipelineInput::Start);

        let out = sm.step(PipelineInput::UserCancel);
        assert!(matches!(
            out,
            PipelineOutput::Done {
                outcome: WorkflowOutcome::Cancelled
            }
        ));
        assert_eq!(sm.phase, Phase::Cancelled);
    }

    #[test]
    fn review_revise_triggers_reimplementation() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::standard(), "feature".into());

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted {
            output: "v1".into(),
            files_changed: 2,
        });
        sm.step(PipelineInput::GatesPassed);

        let out = sm.step(PipelineInput::ReviewRevise {
            findings: vec!["needs error handling".into()],
        });
        assert!(matches!(out, PipelineOutput::SpawnImplementer { .. }));
        assert_eq!(sm.iteration, 2);
    }
}
