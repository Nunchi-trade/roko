//! PipelineStateV2: config-driven workflow state machine.
//!
//! This is a PURE state machine with no side effects. It takes events and
//! returns actions. The effect driver executes the actions.
//!
//! Config determines which phases are active:
//! - Express: implement -> gate -> commit
//! - Standard: implement -> gate -> review -> commit
//! - Full: strategy -> implement -> gate -> review -> commit

use serde::{Deserialize, Serialize};

// TODO(arch): Use roko_core::runtime_event::WorkflowOutcome once the crate
// dependency graph matches the architecture reference. In this checkout,
// roko-core currently depends on roko-runtime, so importing roko-core here
// would create a circular dependency and Cargo.toml edits are out of scope.
/// Outcome of a completed workflow run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Typed result of a commit effect.
///
/// This is the active state-machine contract for commit effects. The legacy
/// `PipelineInput::CommitDone` and `PipelineInput::CommitFailed` variants remain
/// as compatibility adapters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitOutcome {
    /// A git commit was created.
    Created {
        /// Created commit hash.
        hash: String,
    },
    /// The commit effect found no changes to commit.
    NoChanges,
    /// Commit creation was intentionally rejected before running git commit.
    Rejected {
        /// Human-readable rejection reason.
        reason: String,
    },
    /// Commit creation failed.
    Failed {
        /// Human-readable failure details.
        error: String,
    },
}

impl CommitOutcome {
    /// Convert a legacy successful commit input into a typed outcome.
    pub fn from_commit_done(hash: impl Into<String>) -> Self {
        Self::Created { hash: hash.into() }
    }

    /// Convert a legacy failed commit input into a typed outcome.
    pub fn from_commit_failed(error: impl Into<String>) -> Self {
        Self::Failed {
            error: error.into(),
        }
    }

    /// Convert the current legacy pipeline commit inputs into a typed outcome.
    pub fn from_pipeline_input(input: &PipelineInput) -> Option<Self> {
        match input {
            PipelineInput::CommitFinished { outcome } => Some(outcome.clone()),
            PipelineInput::CommitDone { hash: legacy_hash } => {
                Some(Self::from_commit_done(legacy_hash.clone()))
            }
            PipelineInput::CommitFailed { error } => Some(Self::from_commit_failed(error.clone())),
            _ => None,
        }
    }

    /// Return the created commit hash, if this outcome actually created a commit.
    pub fn created_hash(&self) -> Option<&str> {
        match self {
            Self::Created { hash } => Some(hash),
            Self::NoChanges | Self::Rejected { .. } | Self::Failed { .. } => None,
        }
    }
}

/// Configuration for the pipeline. Determines which phases are active.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, serde::Deserialize)]
struct WorkflowConfigToml {
    template: Option<String>,
    has_strategy: Option<bool>,
    has_review: Option<bool>,
    max_iterations: Option<u32>,
    max_autofix_attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowTomlScope {
    Root,
    Workflow,
    WorkflowStep,
    Other,
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

    /// Parse a `WorkflowConfig` from a TOML string.
    ///
    /// The string may contain a `[workflow]` table or just the bare keys. If a
    /// `template` key is present (`"express"`, `"standard"`, or `"full"`), that
    /// preset is used as the base; any additional keys override the preset values.
    ///
    /// Returns an error if the TOML is malformed or `template` is an unknown value.
    pub fn from_toml_str(s: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let raw = parse_workflow_config_toml(s)?;

        let mut config = match raw.template.as_deref() {
            Some("express") => Self::express(),
            Some("standard") | None => Self::standard(),
            Some("full") => Self::full(),
            Some(template) => {
                return Err(config_parse_error(format!(
                    "unknown workflow template: {template}"
                )));
            }
        };

        if let Some(has_strategy) = raw.has_strategy {
            config.has_strategy = has_strategy;
        }
        if let Some(has_review) = raw.has_review {
            config.has_review = has_review;
        }
        if let Some(max_iterations) = raw.max_iterations {
            config.max_iterations = max_iterations;
        }
        if let Some(max_autofix_attempts) = raw.max_autofix_attempts {
            config.max_autofix_attempts = max_autofix_attempts;
        }

        Ok(config)
    }

    /// Load a `WorkflowConfig` from a TOML file on disk.
    ///
    /// The file is read synchronously (this is configuration loading, not hot path).
    /// Returns an error if the file cannot be read or the TOML is invalid.
    pub fn from_toml(
        path: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_toml_str(&contents)
    }
}

fn parse_workflow_config_toml(
    s: &str,
) -> Result<WorkflowConfigToml, Box<dyn std::error::Error + Send + Sync>> {
    let mut workflow = WorkflowConfigToml {
        template: None,
        has_strategy: None,
        has_review: None,
        max_iterations: None,
        max_autofix_attempts: None,
    };
    let mut scope = WorkflowTomlScope::Root;
    let mut saw_workflow_table = false;
    let mut saw_workflow_steps = false;
    let mut steps_have_strategy = false;
    let mut steps_have_review = false;

    for (idx, raw_line) in s.lines().enumerate() {
        let line_number = idx + 1;
        let line = strip_toml_comment(raw_line)
            .map_err(|err| config_parse_error(format!("line {line_number}: {err}")))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("[[") || line.starts_with('[') {
            scope = parse_workflow_toml_scope(line)
                .map_err(|err| config_parse_error(format!("line {line_number}: {err}")))?;
            if scope == WorkflowTomlScope::Workflow {
                saw_workflow_table = true;
            } else if scope == WorkflowTomlScope::WorkflowStep {
                saw_workflow_steps = true;
            }
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(config_parse_error(format!(
                "line {line_number}: expected key = value"
            )));
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() {
            return Err(config_parse_error(format!(
                "line {line_number}: expected key = value"
            )));
        }

        let should_read = if saw_workflow_table {
            scope == WorkflowTomlScope::Workflow
        } else {
            scope == WorkflowTomlScope::Root
        };
        if !should_read {
            if scope == WorkflowTomlScope::WorkflowStep && key == "name" {
                match parse_toml_string(value, line_number)?.as_str() {
                    "strategy" => steps_have_strategy = true,
                    "review" => steps_have_review = true,
                    _ => {}
                }
            }
            continue;
        }

        match key {
            "template" => workflow.template = Some(parse_toml_string(value, line_number)?),
            "has_strategy" => workflow.has_strategy = Some(parse_toml_bool(value, line_number)?),
            "has_review" => workflow.has_review = Some(parse_toml_bool(value, line_number)?),
            "max_iterations" => {
                workflow.max_iterations = Some(parse_toml_u32(value, line_number)?);
            }
            "max_autofix_attempts" => {
                workflow.max_autofix_attempts = Some(parse_toml_u32(value, line_number)?);
            }
            _ => {}
        }
    }

    if saw_workflow_steps {
        workflow.has_strategy = workflow.has_strategy.or(Some(steps_have_strategy));
        workflow.has_review = workflow.has_review.or(Some(steps_have_review));
    }

    Ok(workflow)
}

fn parse_workflow_toml_scope(
    line: &str,
) -> Result<WorkflowTomlScope, Box<dyn std::error::Error + Send + Sync>> {
    if line.starts_with("[[") {
        if !line.ends_with("]]") {
            return Err(config_parse_error("unterminated array table header"));
        }
        let name = line.trim_start_matches("[[").trim_end_matches("]]").trim();
        return Ok(match name {
            "workflow.steps" => WorkflowTomlScope::WorkflowStep,
            _ => WorkflowTomlScope::Other,
        });
    }

    if !line.ends_with(']') {
        return Err(config_parse_error("unterminated table header"));
    }
    let name = line.trim_start_matches('[').trim_end_matches(']').trim();
    Ok(match name {
        "workflow" => WorkflowTomlScope::Workflow,
        _ => WorkflowTomlScope::Other,
    })
}

fn strip_toml_comment(line: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut in_string = false;
    let mut escaped = false;
    let mut out = String::new();

    for ch in line.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '#' => break,
            _ => out.push(ch),
        }
    }

    if in_string {
        return Err(config_parse_error("unterminated string"));
    }

    Ok(out)
}

fn parse_toml_string(
    value: &str,
    line_number: usize,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err(config_parse_error(format!(
            "line {line_number}: expected string value"
        )));
    }

    let inner = &value[1..value.len() - 1];
    let mut parsed = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let Some(escaped) = chars.next() else {
                return Err(config_parse_error(format!(
                    "line {line_number}: dangling escape in string"
                )));
            };
            let value = match escaped {
                '"' => '"',
                '\\' => '\\',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            };
            parsed.push(value);
        } else {
            parsed.push(ch);
        }
    }
    Ok(parsed)
}

fn parse_toml_bool(
    value: &str,
    line_number: usize,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(config_parse_error(format!(
            "line {line_number}: expected boolean value"
        ))),
    }
}

fn parse_toml_u32(
    value: &str,
    line_number: usize,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    value.parse::<u32>().map_err(|err| {
        config_parse_error(format!(
            "line {line_number}: expected unsigned integer: {err}"
        ))
    })
}

fn config_parse_error(message: impl Into<String>) -> Box<dyn std::error::Error + Send + Sync> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        message.into(),
    ))
}

/// Current phase of the pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        matches!(self, Self::Complete | Self::Halted { .. } | Self::Cancelled)
    }

    /// Stable lowercase label for logs, events, and UI adapters.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Strategizing => "strategizing",
            Self::Implementing => "implementing",
            Self::Gating => "gating",
            Self::AutoFixing => "auto_fixing",
            Self::Reviewing => "reviewing",
            Self::Committing => "committing",
            Self::Complete => "complete",
            Self::Halted { .. } => "halted",
            Self::Cancelled => "cancelled",
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
    /// Review rejected the implementation.
    ReviewRejected {
        /// Rejection reason supplied by the reviewer.
        reason: String,
    },
    /// Review outcome was ambiguous.
    ReviewUnclear {
        /// Ambiguous review summary supplied by the reviewer.
        summary: String,
    },
    /// Review requests revisions.
    ReviewRevise {
        /// Findings that should guide the next implementation pass.
        findings: Vec<String>,
    },
    /// Commit finished with a typed outcome.
    CommitFinished {
        /// Typed commit result from the effect driver.
        outcome: CommitOutcome,
    },
    /// Legacy commit-done input.
    CommitDone {
        /// Commit hash created by the effect driver.
        hash: String,
    },
    /// Legacy commit-failed input.
    CommitFailed {
        /// Commit failure details from the effect driver.
        error: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Serialize the current pipeline state to a JSON string.
    ///
    /// The resulting JSON can be passed to `from_checkpoint` to restore the exact
    /// state -- including phase, iteration count, and accumulated findings -- so a
    /// workflow can resume after a process restart.
    ///
    /// Returns an error only if serde_json serialization fails (in practice, never
    /// for this struct).
    pub fn checkpoint(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(serde_json::to_string(self)?)
    }

    /// Restore a `PipelineStateV2` from a JSON checkpoint string produced by
    /// [`checkpoint`].
    ///
    /// After restoring, call [`step`] with the next input as if the workflow is
    /// continuing from the saved phase. Terminal phases (`Complete`, `Halted`,
    /// `Cancelled`) are valid checkpoint states -- callers should check
    /// `phase.is_terminal()` before resuming.
    ///
    /// Returns an error if the JSON is malformed or missing required fields.
    pub fn from_checkpoint(json: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(serde_json::from_str(json)?)
    }

    /// Feed an event into the state machine, get an action back.
    /// This is the ONLY way to drive the state machine. No side effects here.
    #[allow(clippy::too_many_lines)]
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
                            "Previous attempt failed gate '{gate}'. Error:\n{output}"
                        )),
                    }
                } else {
                    let reason =
                        format!("Gate '{gate}' failed after {} iterations", self.iteration);
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
                self.request_review_revision(findings)
            }
            (Phase::Reviewing, PipelineInput::ReviewRejected { reason }) => {
                self.request_review_revision(vec![reason])
            }
            (Phase::Reviewing, PipelineInput::ReviewUnclear { summary }) => {
                self.request_review_revision(vec![format!("Unclear review outcome: {summary}")])
            }

            (Phase::Committing, PipelineInput::CommitFinished { outcome }) => {
                self.finish_commit(outcome)
            }
            (Phase::Committing, PipelineInput::CommitDone { hash: legacy_hash }) => {
                self.finish_commit(CommitOutcome::from_commit_done(legacy_hash))
            }
            (Phase::Committing, PipelineInput::CommitFailed { error }) => {
                self.finish_commit(CommitOutcome::from_commit_failed(error))
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

    fn finish_commit(&mut self, outcome: CommitOutcome) -> PipelineOutput {
        match outcome {
            CommitOutcome::Created { hash } => {
                self.commit_hash = Some(hash.clone());
                self.phase = Phase::Complete;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Success {
                        commit_hash: Some(hash),
                    },
                }
            }
            CommitOutcome::NoChanges => {
                self.commit_hash = None;
                self.phase = Phase::Complete;
                PipelineOutput::Done {
                    outcome: WorkflowOutcome::Success { commit_hash: None },
                }
            }
            CommitOutcome::Rejected { reason } | CommitOutcome::Failed { error: reason } => {
                self.phase = Phase::Halted {
                    reason: reason.clone(),
                };
                PipelineOutput::Halt { reason }
            }
        }
    }

    fn request_review_revision(&mut self, findings: Vec<String>) -> PipelineOutput {
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

        let out = sm.step(PipelineInput::CommitFinished {
            outcome: CommitOutcome::Created { hash: "abc".into() },
        });
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

    #[test]
    fn toml_express_template() {
        let cfg = WorkflowConfig::from_toml_str("template = \"express\"").unwrap();
        assert!(!cfg.has_strategy);
        assert!(!cfg.has_review);
        assert_eq!(cfg.max_iterations, 1);
    }

    #[test]
    fn toml_full_template_with_override() {
        let cfg = WorkflowConfig::from_toml_str("template = \"full\"\nmax_iterations = 5").unwrap();
        assert!(cfg.has_strategy);
        assert_eq!(cfg.max_iterations, 5);
    }

    #[test]
    fn toml_table_form() {
        let src = "[workflow]\ntemplate = \"standard\"\nmax_autofix_attempts = 3";
        let cfg = WorkflowConfig::from_toml_str(src).unwrap();
        assert_eq!(cfg.max_autofix_attempts, 3);
        assert!(cfg.has_review);
    }

    #[test]
    fn toml_bare_keys_no_template() {
        let cfg = WorkflowConfig::from_toml_str(
            "has_strategy = true\nhas_review = false\nmax_iterations = 2\nmax_autofix_attempts = 1",
        )
        .unwrap();
        assert!(cfg.has_strategy);
        assert!(!cfg.has_review);
    }

    #[test]
    fn toml_unknown_template_is_error() {
        assert!(WorkflowConfig::from_toml_str("template = \"bogus\"").is_err());
    }

    #[test]
    fn toml_steps_infer_strategy_and_review_flags() {
        let src = r#"
[workflow]
max_iterations = 4

[[workflow.steps]]
name = "strategy"
role = "strategist"

[[workflow.steps]]
name = "implement"
role = "implementer"
"#;
        let cfg = WorkflowConfig::from_toml_str(src).unwrap();
        assert!(cfg.has_strategy);
        assert!(!cfg.has_review);
        assert_eq!(cfg.max_iterations, 4);
    }

    #[test]
    fn checkpoint_round_trip_implementing() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::standard(), "add feature".into());
        sm.step(PipelineInput::Start);

        let json = sm.checkpoint().unwrap();
        let restored = PipelineStateV2::from_checkpoint(&json).unwrap();

        assert_eq!(restored.phase, Phase::Implementing);
        assert_eq!(restored.iteration, 1);
        assert_eq!(restored.original_prompt, "add feature");
    }

    #[test]
    fn checkpoint_preserves_review_findings() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::standard(), "feat".into());
        sm.review_findings = vec!["needs error handling".into(), "add docs".into()];
        sm.iteration = 2;
        sm.phase = Phase::Implementing;

        let json = sm.checkpoint().unwrap();
        let restored = PipelineStateV2::from_checkpoint(&json).unwrap();

        assert_eq!(
            restored.review_findings,
            vec!["needs error handling", "add docs"]
        );
        assert_eq!(restored.iteration, 2);
    }

    #[test]
    fn checkpoint_halted_phase() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "task".into());
        sm.phase = Phase::Halted {
            reason: "compile failed".into(),
        };

        let json = sm.checkpoint().unwrap();
        let restored = PipelineStateV2::from_checkpoint(&json).unwrap();

        assert!(restored.phase.is_terminal());
        assert!(
            matches!(restored.phase, Phase::Halted { ref reason } if reason == "compile failed")
        );
    }

    #[test]
    fn checkpoint_full_config() {
        let config = WorkflowConfig::full();
        let sm = PipelineStateV2::new(config, "complex".into());

        let json = sm.checkpoint().unwrap();
        let restored = PipelineStateV2::from_checkpoint(&json).unwrap();

        assert!(restored.config.has_strategy);
        assert!(restored.config.has_review);
        assert_eq!(restored.config.max_iterations, 3);
    }

    #[test]
    fn from_checkpoint_rejects_invalid_json() {
        assert!(PipelineStateV2::from_checkpoint("not json at all").is_err());
        assert!(PipelineStateV2::from_checkpoint("{}").is_err());
    }

    #[test]
    fn checkpoint_preserves_phase_and_iteration() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "fix bug".into());

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 2,
        });

        let checkpoint = sm.clone();

        assert_eq!(checkpoint.phase, Phase::Gating);
        assert_eq!(checkpoint.iteration, 1);
        assert_eq!(checkpoint.original_prompt, "fix bug");
    }

    #[test]
    fn resume_from_gating_checkpoint_skips_implement() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "fix bug".into());

        sm.step(PipelineInput::Start);
        sm.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 2,
        });

        let mut resumed = sm.clone();

        let out = resumed.step(PipelineInput::GatesPassed);
        assert!(matches!(out, PipelineOutput::Commit));

        let out = resumed.step(PipelineInput::CommitDone {
            hash: "abc123".into(),
        });
        assert!(matches!(
            out,
            PipelineOutput::Done {
                outcome: WorkflowOutcome::Success {
                    commit_hash: Some(hash)
                }
            } if hash == "abc123"
        ));
        assert_eq!(resumed.phase, Phase::Complete);
    }

    #[test]
    fn resume_does_not_affect_original() {
        let mut original = PipelineStateV2::new(WorkflowConfig::express(), "fix bug".into());

        original.step(PipelineInput::Start);
        original.step(PipelineInput::AgentCompleted {
            output: "done".into(),
            files_changed: 2,
        });

        let mut checkpoint = original.clone();

        original.step(PipelineInput::GatesPassed);
        original.step(PipelineInput::CommitDone {
            hash: "original".into(),
        });

        let out = checkpoint.step(PipelineInput::GateFailed {
            gate: "compile".into(),
            output: "error[E0308]".into(),
        });
        assert!(matches!(out, PipelineOutput::SpawnAutoFixer { .. }));

        checkpoint.step(PipelineInput::AgentCompleted {
            output: "fixed".into(),
            files_changed: 1,
        });
        checkpoint.step(PipelineInput::GatesPassed);
        checkpoint.step(PipelineInput::CommitDone {
            hash: "checkpoint".into(),
        });

        assert_eq!(original.phase, Phase::Complete);
        assert_eq!(checkpoint.phase, Phase::Complete);
        assert_eq!(original.autofix_attempts, 0);
        assert!(checkpoint.autofix_attempts > 0);
    }

    #[test]
    fn commit_outcome_created_carries_hash() {
        let outcome = CommitOutcome::from_pipeline_input(&PipelineInput::CommitDone {
            hash: "abc123".into(),
        });

        assert_eq!(
            outcome,
            Some(CommitOutcome::Created {
                hash: "abc123".into()
            })
        );
        assert_eq!(
            outcome.as_ref().and_then(CommitOutcome::created_hash),
            Some("abc123")
        );
    }

    #[test]
    fn commit_outcome_no_changes_has_no_hash() {
        let outcome = CommitOutcome::NoChanges;

        assert_eq!(outcome.created_hash(), None);
    }

    #[test]
    fn commit_no_changes_is_not_created_commit() {
        let mut sm = PipelineStateV2::new(WorkflowConfig::express(), "fix bug".into());
        sm.phase = Phase::Committing;

        let out = sm.step(PipelineInput::CommitFinished {
            outcome: CommitOutcome::NoChanges,
        });

        assert!(matches!(
            out,
            PipelineOutput::Done {
                outcome: WorkflowOutcome::Success { commit_hash: None }
            }
        ));
        assert_eq!(sm.phase, Phase::Complete);
        assert_eq!(sm.commit_hash, None);
    }

    #[test]
    fn commit_outcome_failed_converts_from_legacy_commit_failed() {
        let outcome = CommitOutcome::from_pipeline_input(&PipelineInput::CommitFailed {
            error: "git commit failed".into(),
        });

        assert_eq!(
            outcome,
            Some(CommitOutcome::Failed {
                error: "git commit failed".into()
            })
        );
        assert_eq!(outcome.as_ref().and_then(CommitOutcome::created_hash), None);
    }
}
