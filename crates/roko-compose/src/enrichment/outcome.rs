//! Step outcome types for the enrichment pipeline.
//!
//! Every step execution returns a [`StepOutcome`] that describes what happened.
//! The pipeline collects these for reporting without stopping on failure.

use super::step::EnrichStep;

/// Reason a step was skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The run is in dry-run mode.
    DryRun,
    /// Output exists and all inputs are older than the output (fresh).
    Fresh,
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
}
