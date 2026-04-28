//! Pure pipeline state machine for workflow execution.
//!
//! This module is side-effect-free: it receives events and emits actions.
//! All actual I/O (spawning agents, running gates, committing) is performed
//! by the runner module.

use serde::{Deserialize, Serialize};

/// Pipeline execution phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    /// Workflow created but not yet started.
    Pending,
    /// Strategist is analyzing the prompt and producing a brief.
    Strategizing,
    /// Implementer is writing code.
    Implementing,
    /// Auto-fixer is patching gate failures.
    AutoFixing,
    /// Gates (compile, test, clippy) are running.
    Gating,
    /// Reviewer agent(s) are analyzing changes.
    Reviewing,
    /// Creating a commit.
    Committing,
    /// Pipeline completed successfully.
    Complete,
    /// Pipeline halted (timeout, budget, or user cancel).
    Halted { reason: String },
    /// Pipeline cancelled by user.
    Cancelled,
}

impl PipelinePhase {
    /// Returns true if the pipeline is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Halted { .. } | Self::Cancelled)
    }
}

/// Events fed into the state machine from the executor.
#[derive(Debug, Clone)]
pub enum PipelineEvent {
    /// Start the pipeline.
    Start,
    /// Strategist produced a brief.
    StrategyComplete { brief: String },
    /// Strategist was skipped (express/standard workflows).
    StrategySkipped,
    /// Agent completed its work.
    AgentCompleted { output: String, files_changed: u32 },
    /// Agent failed.
    AgentFailed { error: String },
    /// All gates passed.
    GatesPassed,
    /// A gate failed.
    GateFailed { gate: String, output: String },
    /// Review verdict: approved.
    ReviewApproved { summary: String },
    /// Review verdict: revise needed.
    ReviewRevise { findings: Vec<String> },
    /// Commit succeeded.
    CommitDone { message: String },
    /// Timeout reached.
    Timeout,
    /// Budget exceeded.
    BudgetExceeded,
    /// User cancelled.
    UserCancel,
}

/// Actions emitted by the state machine for the executor to perform.
#[derive(Debug, Clone)]
pub enum PipelineAction {
    /// Spawn a strategist agent.
    SpawnStrategist { prompt: String },
    /// Spawn an implementer agent.
    SpawnImplementer { prompt: String, context: String },
    /// Spawn an auto-fixer agent.
    SpawnAutoFixer { error_output: String },
    /// Run the gate pipeline (compile, test, clippy as configured).
    RunGates,
    /// Spawn reviewer agent(s).
    SpawnReviewer { diff_context: String },
    /// Create a commit.
    Commit,
    /// Pipeline is done.
    Done,
    /// Pipeline halted — persist state for resume.
    Halt { reason: String },
}

/// Workflow template that determines which phases to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTemplate {
    /// Implement → gate → commit (fastest).
    Express,
    /// Implement → gate → review → commit.
    Standard,
    /// Strategy → implement → gate → multi-review → commit.
    Full,
}

impl WorkflowTemplate {
    /// Parse from config string. Returns None for "none".
    /// For "auto", estimates from prompt complexity.
    pub fn from_config(s: &str) -> Option<Self> {
        match s {
            "express" => Some(Self::Express),
            "standard" => Some(Self::Standard),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// Select a template automatically based on prompt characteristics.
    pub fn auto_select(prompt: &str) -> Self {
        let word_count = prompt.split_whitespace().count();
        let has_multi_file_hints = prompt.contains("files")
            || prompt.contains("modules")
            || prompt.contains("system")
            || prompt.contains("architecture")
            || prompt.contains("refactor");
        let has_simple_hints = prompt.contains("fix")
            || prompt.contains("typo")
            || prompt.contains("rename")
            || prompt.contains("update")
            || prompt.contains("bump");

        if has_simple_hints && word_count < 15 {
            Self::Express
        } else if has_multi_file_hints || word_count > 50 {
            Self::Full
        } else {
            Self::Standard
        }
    }

    /// Whether this template includes a strategist phase.
    pub fn has_strategy(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether this template includes a review phase.
    pub fn has_review(&self) -> bool {
        matches!(self, Self::Standard | Self::Full)
    }
}

/// Mutable pipeline state. Fed events, produces actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineState {
    /// Current phase.
    pub phase: PipelinePhase,
    /// Workflow template in use.
    pub template: WorkflowTemplate,
    /// How many implement→gate→review loops we've done.
    pub iteration: u32,
    /// Maximum iterations before halting.
    pub max_iterations: u32,
    /// Original user prompt.
    pub original_prompt: String,
    /// Strategist brief (if produced).
    pub strategist_brief: Option<String>,
    /// Accumulated review findings from previous iterations.
    pub review_findings: Vec<String>,
    /// Last gate failure output (for autofix context).
    pub last_gate_failure: Option<String>,
    /// Number of files changed by the last implementation.
    pub files_changed: u32,
    /// Commit message (after commit).
    pub commit_message: Option<String>,
}

impl PipelineState {
    /// Create a new pipeline state for the given template and prompt.
    pub fn new(template: WorkflowTemplate, prompt: String, max_iterations: u32) -> Self {
        Self {
            phase: PipelinePhase::Pending,
            template,
            iteration: 0,
            max_iterations,
            original_prompt: prompt,
            strategist_brief: None,
            review_findings: Vec::new(),
            last_gate_failure: None,
            files_changed: 0,
            commit_message: None,
        }
    }

    /// Feed an event into the state machine and get the next action.
    pub fn step(&mut self, event: PipelineEvent) -> PipelineAction {
        match (&self.phase, event) {
            // ── Start ──
            (PipelinePhase::Pending, PipelineEvent::Start) => {
                if self.template.has_strategy() {
                    self.phase = PipelinePhase::Strategizing;
                    PipelineAction::SpawnStrategist {
                        prompt: self.original_prompt.clone(),
                    }
                } else {
                    self.phase = PipelinePhase::Implementing;
                    self.iteration = 1;
                    PipelineAction::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: String::new(),
                    }
                }
            }

            // ── Strategy phase ──
            (PipelinePhase::Strategizing, PipelineEvent::StrategyComplete { brief }) => {
                self.strategist_brief = Some(brief.clone());
                self.phase = PipelinePhase::Implementing;
                self.iteration = 1;
                PipelineAction::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: brief,
                }
            }
            (PipelinePhase::Strategizing, PipelineEvent::StrategySkipped) => {
                self.phase = PipelinePhase::Implementing;
                self.iteration = 1;
                PipelineAction::SpawnImplementer {
                    prompt: self.original_prompt.clone(),
                    context: String::new(),
                }
            }

            // ── Implementation phase ──
            (PipelinePhase::Implementing, PipelineEvent::AgentCompleted { files_changed, .. }) => {
                self.files_changed = files_changed;
                self.phase = PipelinePhase::Gating;
                PipelineAction::RunGates
            }
            (PipelinePhase::Implementing, PipelineEvent::AgentFailed { error }) => {
                if self.iteration < self.max_iterations {
                    // Retry implementation with error context.
                    self.iteration += 1;
                    PipelineAction::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: format!("Previous attempt failed:\n{error}"),
                    }
                } else {
                    self.phase = PipelinePhase::Halted {
                        reason: format!("Implementation failed after {} attempts", self.iteration),
                    };
                    PipelineAction::Halt {
                        reason: format!("Implementation failed: {error}"),
                    }
                }
            }

            // ── AutoFix phase ──
            (PipelinePhase::AutoFixing, PipelineEvent::AgentCompleted { files_changed, .. }) => {
                self.files_changed += files_changed;
                self.phase = PipelinePhase::Gating;
                PipelineAction::RunGates
            }
            (PipelinePhase::AutoFixing, PipelineEvent::AgentFailed { error }) => {
                // AutoFix failed — fall back to full reimplementation if we have iterations left.
                if self.iteration < self.max_iterations {
                    self.iteration += 1;
                    self.phase = PipelinePhase::Implementing;
                    let context = if let Some(ref gate_err) = self.last_gate_failure {
                        format!("Gate failure:\n{gate_err}\n\nAutofix also failed:\n{error}")
                    } else {
                        format!("Autofix failed:\n{error}")
                    };
                    PipelineAction::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context,
                    }
                } else {
                    self.phase = PipelinePhase::Halted {
                        reason: "AutoFix failed, no iterations remaining".into(),
                    };
                    PipelineAction::Halt {
                        reason: format!("AutoFix failed: {error}"),
                    }
                }
            }

            // ── Gating phase ──
            (PipelinePhase::Gating, PipelineEvent::GatesPassed) => {
                if self.template.has_review() {
                    self.phase = PipelinePhase::Reviewing;
                    PipelineAction::SpawnReviewer {
                        diff_context: String::new(),
                    }
                } else {
                    // Express: skip review, go straight to commit.
                    self.phase = PipelinePhase::Committing;
                    PipelineAction::Commit
                }
            }
            (PipelinePhase::Gating, PipelineEvent::GateFailed { output, .. }) => {
                self.last_gate_failure = Some(output.clone());
                if self.iteration < self.max_iterations {
                    // Try autofix first.
                    self.phase = PipelinePhase::AutoFixing;
                    PipelineAction::SpawnAutoFixer {
                        error_output: output,
                    }
                } else {
                    // No iterations left — halt.
                    self.phase = PipelinePhase::Halted {
                        reason: "Gate failed, no iterations remaining".into(),
                    };
                    PipelineAction::Halt {
                        reason: format!("Gate failed:\n{output}"),
                    }
                }
            }

            // ── Review phase ──
            (PipelinePhase::Reviewing, PipelineEvent::ReviewApproved { .. }) => {
                self.phase = PipelinePhase::Committing;
                PipelineAction::Commit
            }
            (PipelinePhase::Reviewing, PipelineEvent::ReviewRevise { findings }) => {
                self.review_findings.extend(findings.clone());
                if self.iteration < self.max_iterations {
                    self.iteration += 1;
                    self.phase = PipelinePhase::Implementing;
                    let feedback = findings.join("\n- ");
                    PipelineAction::SpawnImplementer {
                        prompt: self.original_prompt.clone(),
                        context: format!("Review feedback:\n- {feedback}"),
                    }
                } else {
                    // Accept with caveats — commit anyway.
                    self.phase = PipelinePhase::Committing;
                    PipelineAction::Commit
                }
            }

            // ── Committing phase ──
            (PipelinePhase::Committing, PipelineEvent::CommitDone { message }) => {
                self.commit_message = Some(message);
                self.phase = PipelinePhase::Complete;
                PipelineAction::Done
            }

            // ── Global interrupts ──
            (_, PipelineEvent::UserCancel) => {
                self.phase = PipelinePhase::Cancelled;
                PipelineAction::Halt {
                    reason: "Cancelled by user".into(),
                }
            }
            (_, PipelineEvent::Timeout) => {
                let reason = format!("Timeout in phase {:?}", self.phase);
                self.phase = PipelinePhase::Halted {
                    reason: reason.clone(),
                };
                PipelineAction::Halt { reason }
            }
            (_, PipelineEvent::BudgetExceeded) => {
                let reason = format!("Budget exceeded in phase {:?}", self.phase);
                self.phase = PipelinePhase::Halted {
                    reason: reason.clone(),
                };
                PipelineAction::Halt { reason }
            }

            // ── Invalid transitions ──
            (phase, event) => {
                tracing::warn!(
                    phase = ?phase,
                    event = ?event,
                    "unexpected pipeline event for current phase"
                );
                // Stay in current phase, no action.
                PipelineAction::Done
            }
        }
    }

    /// Returns a human-readable summary of current phase.
    pub fn phase_label(&self) -> &'static str {
        match self.phase {
            PipelinePhase::Pending => "Pending",
            PipelinePhase::Strategizing => "Strategizing",
            PipelinePhase::Implementing => "Implementing",
            PipelinePhase::AutoFixing => "Auto-fixing",
            PipelinePhase::Gating => "Running gates",
            PipelinePhase::Reviewing => "Reviewing",
            PipelinePhase::Committing => "Committing",
            PipelinePhase::Complete => "Complete",
            PipelinePhase::Halted { .. } => "Halted",
            PipelinePhase::Cancelled => "Cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn express_pipeline_skips_strategy_and_review() {
        let mut state = PipelineState::new(WorkflowTemplate::Express, "fix the bug".into(), 2);
        let action = state.step(PipelineEvent::Start);
        assert!(matches!(action, PipelineAction::SpawnImplementer { .. }));
        assert_eq!(state.phase, PipelinePhase::Implementing);

        let action = state.step(PipelineEvent::AgentCompleted {
            output: "done".into(),
            files_changed: 1,
        });
        assert!(matches!(action, PipelineAction::RunGates));

        let action = state.step(PipelineEvent::GatesPassed);
        // Express skips review.
        assert!(matches!(action, PipelineAction::Commit));
        assert_eq!(state.phase, PipelinePhase::Committing);
    }

    #[test]
    fn standard_pipeline_includes_review() {
        let mut state = PipelineState::new(WorkflowTemplate::Standard, "add auth".into(), 2);
        state.step(PipelineEvent::Start);
        state.step(PipelineEvent::AgentCompleted {
            output: "done".into(),
            files_changed: 2,
        });
        let action = state.step(PipelineEvent::GatesPassed);
        assert!(matches!(action, PipelineAction::SpawnReviewer { .. }));
        assert_eq!(state.phase, PipelinePhase::Reviewing);
    }

    #[test]
    fn full_pipeline_starts_with_strategist() {
        let mut state = PipelineState::new(WorkflowTemplate::Full, "build feature".into(), 2);
        let action = state.step(PipelineEvent::Start);
        assert!(matches!(action, PipelineAction::SpawnStrategist { .. }));
        assert_eq!(state.phase, PipelinePhase::Strategizing);

        let action = state.step(PipelineEvent::StrategyComplete {
            brief: "Use trait pattern".into(),
        });
        assert!(matches!(action, PipelineAction::SpawnImplementer { .. }));
    }

    #[test]
    fn gate_failure_triggers_autofix() {
        let mut state = PipelineState::new(WorkflowTemplate::Standard, "fix".into(), 2);
        state.step(PipelineEvent::Start);
        state.step(PipelineEvent::AgentCompleted {
            output: "done".into(),
            files_changed: 1,
        });
        let action = state.step(PipelineEvent::GateFailed {
            gate: "test".into(),
            output: "test failed".into(),
        });
        assert!(matches!(action, PipelineAction::SpawnAutoFixer { .. }));
        assert_eq!(state.phase, PipelinePhase::AutoFixing);
    }

    #[test]
    fn review_revise_retries_implementation() {
        let mut state = PipelineState::new(WorkflowTemplate::Standard, "add feature".into(), 2);
        state.step(PipelineEvent::Start);
        state.step(PipelineEvent::AgentCompleted {
            output: "done".into(),
            files_changed: 1,
        });
        state.step(PipelineEvent::GatesPassed);
        let action = state.step(PipelineEvent::ReviewRevise {
            findings: vec!["Missing error handling".into()],
        });
        assert!(matches!(action, PipelineAction::SpawnImplementer { .. }));
        assert_eq!(state.iteration, 2);
    }

    #[test]
    fn user_cancel_from_any_phase() {
        let mut state = PipelineState::new(WorkflowTemplate::Standard, "work".into(), 2);
        state.step(PipelineEvent::Start);
        let action = state.step(PipelineEvent::UserCancel);
        assert!(matches!(action, PipelineAction::Halt { .. }));
        assert_eq!(state.phase, PipelinePhase::Cancelled);
    }

    #[test]
    fn max_iterations_halts_on_gate_failure() {
        let mut state = PipelineState::new(WorkflowTemplate::Express, "fix".into(), 1);
        state.step(PipelineEvent::Start);
        state.step(PipelineEvent::AgentCompleted {
            output: "done".into(),
            files_changed: 1,
        });
        let action = state.step(PipelineEvent::GateFailed {
            gate: "compile".into(),
            output: "error".into(),
        });
        assert!(matches!(action, PipelineAction::Halt { .. }));
    }

    #[test]
    fn auto_select_express_for_simple() {
        assert_eq!(
            WorkflowTemplate::auto_select("fix the typo in main.rs"),
            WorkflowTemplate::Express
        );
    }

    #[test]
    fn auto_select_full_for_complex() {
        let prompt = "Refactor the authentication system to use JWT tokens. \
             This involves changes to the user model, the auth middleware, \
             the login endpoint, the signup endpoint, and the session management \
             modules. We also need to update all integration tests.";
        assert_eq!(
            WorkflowTemplate::auto_select(prompt),
            WorkflowTemplate::Full
        );
    }

    #[test]
    fn auto_select_standard_for_medium() {
        assert_eq!(
            WorkflowTemplate::auto_select("add a health check endpoint"),
            WorkflowTemplate::Standard
        );
    }
}
