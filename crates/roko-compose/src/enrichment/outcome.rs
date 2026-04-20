//! Step outcome types for the enrichment pipeline.
//!
//! Every step execution returns a [`StepOutcome`] that describes what happened.
//! The pipeline collects these for reporting without stopping on failure.
//!
//! COMP-09: Added per-step cost tracking via [`StepCost`].

use super::step::EnrichStep;

/// Reason a step was skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The run is in dry-run mode.
    DryRun,
    /// Output exists and all inputs are older than the output (fresh).
    Fresh,
    /// COMP-09: Adaptive selection decided this step is not worth running
    /// based on prior outcome history.
    AdaptiveSkip,
}

/// COMP-09: Per-step cost tracking for efficiency learning.
///
/// Records actual resource consumption for a single enrichment step.
/// These are emitted alongside [`StepOutcome`] and can be fed into the
/// learning system to improve future step selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepCost {
    /// Wall-clock time spent on this step, in milliseconds.
    pub elapsed_ms: u64,
    /// Estimated input tokens consumed.
    pub input_tokens: u32,
    /// Estimated output tokens produced.
    pub output_tokens: u32,
    /// Number of LLM calls made (including retries/repairs).
    pub llm_calls: u32,
    /// Size of the output artifact in bytes.
    pub output_bytes: usize,
}

/// Outcome of executing a single enrichment step.
#[derive(Clone, Debug)]
pub enum StepOutcome {
    /// Step produced new output.
    Generated {
        /// Which step ran.
        step: EnrichStep,
        /// Number of LLM calls made (0 for non-LLM steps, 1 for normal, 2 if
        /// a TOML repair was needed).
        llm_calls: u32,
        /// COMP-09: Per-step cost tracking.
        cost: StepCost,
    },
    /// Step was skipped.
    Skipped {
        /// Which step was skipped.
        step: EnrichStep,
        /// Why it was skipped.
        reason: SkipReason,
    },
    /// Step failed.
    Failed {
        /// Which step failed.
        step: EnrichStep,
        /// Human-readable error message.
        message: String,
        /// COMP-09: Per-step cost tracking (partial cost before failure).
        cost: StepCost,
    },
}

impl StepOutcome {
    /// The step this outcome is for.
    #[must_use]
    pub const fn step(&self) -> EnrichStep {
        match self {
            Self::Generated { step, .. }
            | Self::Skipped { step, .. }
            | Self::Failed { step, .. } => *step,
        }
    }

    /// Whether this outcome represents a failure.
    #[must_use]
    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// Whether this outcome represents a skip.
    #[must_use]
    pub const fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped { .. })
    }

    /// COMP-09: Extract the cost of this outcome, if any.
    #[must_use]
    pub fn cost(&self) -> Option<&StepCost> {
        match self {
            Self::Generated { cost, .. } | Self::Failed { cost, .. } => Some(cost),
            Self::Skipped { .. } => None,
        }
    }

    /// Whether this outcome was a success (generated new output).
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Generated { .. })
    }
}
