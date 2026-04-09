//! Complexity-based step selection for the enrichment pipeline.
//!
//! Not every enrichment step is worth running for every task. Simple bug fixes
//! do not need a research memo or fixture manifest. This module selects which
//! steps to run based on the [`TaskComplexityBand`] of the plan/task.
//!
//! # Default skip rules
//!
//! - **Fast** (trivial single-file change): skip Research, Dependencies,
//!   Fixtures, Integration, Reviews, Invariants, Scribe.
//! - **Standard** (multi-file within one crate): run all steps.
//! - **Complex** (cross-crate / architectural): run all steps + extra
//!   verification (caller may add additional verify passes).

use roko_core::TaskComplexityBand;

use super::step::EnrichStep;

/// Default steps to skip for [`TaskComplexityBand::Fast`].
const FAST_SKIP: &[EnrichStep] = &[
    EnrichStep::Research,
    EnrichStep::Dependencies,
    EnrichStep::Fixtures,
    EnrichStep::Integration,
    EnrichStep::Reviews,
    EnrichStep::Invariants,
    EnrichStep::Scribe,
];

/// Selects which enrichment steps to run based on task complexity.
///
/// Callers can override the default skip lists per complexity band.
pub struct StepSelector {
    /// Steps to skip for Fast tasks.
    fast_skip: Vec<EnrichStep>,
    /// Steps to skip for Standard tasks.
    standard_skip: Vec<EnrichStep>,
    /// Steps to skip for Complex tasks.
    complex_skip: Vec<EnrichStep>,
    /// Additional steps to add for Complex tasks (beyond the normal set).
    complex_extras: Vec<EnrichStep>,
}

impl Default for StepSelector {
    fn default() -> Self {
        Self {
            fast_skip: FAST_SKIP.to_vec(),
            standard_skip: Vec::new(),
            complex_skip: Vec::new(),
            complex_extras: Vec::new(),
        }
    }
}

impl StepSelector {
    /// Create a selector with default skip rules.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the skip list for Fast tasks.
    #[must_use]
    pub fn with_fast_skip(mut self, skip: Vec<EnrichStep>) -> Self {
        self.fast_skip = skip;
        self
    }

    /// Override the skip list for Standard tasks.
    #[must_use]
    pub fn with_standard_skip(mut self, skip: Vec<EnrichStep>) -> Self {
        self.standard_skip = skip;
        self
    }

    /// Override the skip list for Complex tasks.
    #[must_use]
    pub fn with_complex_skip(mut self, skip: Vec<EnrichStep>) -> Self {
        self.complex_skip = skip;
        self
    }

    /// Set additional steps to include for Complex tasks.
    ///
    /// These are appended to the normal step list after filtering.
    /// Useful for adding extra verification passes.
    #[must_use]
    pub fn with_complex_extras(mut self, extras: Vec<EnrichStep>) -> Self {
        self.complex_extras = extras;
        self
    }

    /// Select which steps to run from the given list, based on complexity.
    ///
    /// Returns the filtered list of steps in the same order as the input,
    /// with any complexity-specific extras appended at the end.
    #[must_use]
    pub fn select_steps(
        &self,
        complexity: TaskComplexityBand,
        steps: &[EnrichStep],
    ) -> Vec<EnrichStep> {
        let skip_list = match complexity {
            TaskComplexityBand::Fast => &self.fast_skip,
            TaskComplexityBand::Standard => &self.standard_skip,
            // Complex + future-proof unknown bands.
            _ => &self.complex_skip,
        };

        let mut selected: Vec<EnrichStep> = steps
            .iter()
            .filter(|s| !skip_list.contains(s))
            .copied()
            .collect();

        // For complex tasks, append extras that aren't already present.
        if complexity == TaskComplexityBand::Complex {
            for extra in &self.complex_extras {
                if !selected.contains(extra) {
                    selected.push(*extra);
                }
            }
        }

        selected
    }

    /// How many steps would be skipped for this complexity band.
    #[must_use]
    pub fn skip_count(&self, complexity: TaskComplexityBand, total_steps: usize) -> usize {
        let skip_list = match complexity {
            TaskComplexityBand::Fast => &self.fast_skip,
            TaskComplexityBand::Standard => &self.standard_skip,
            // Complex + future-proof unknown bands.
            _ => &self.complex_skip,
        };
        skip_list.len().min(total_steps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrichment::step::ALL_ORDERED;

    #[test]
    fn fast_skips_heavy_steps() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Fast, ALL_ORDERED);

        // Should keep Prd, Briefs, Tasks, Decompose, Verify, Tests.
        assert!(!selected.contains(&EnrichStep::Research));
        assert!(!selected.contains(&EnrichStep::Dependencies));
        assert!(!selected.contains(&EnrichStep::Fixtures));
        assert!(!selected.contains(&EnrichStep::Integration));
        assert!(!selected.contains(&EnrichStep::Reviews));
        assert!(!selected.contains(&EnrichStep::Invariants));
        assert!(!selected.contains(&EnrichStep::Scribe));
    }

    #[test]
    fn fast_keeps_core_steps() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Fast, ALL_ORDERED);

        assert!(selected.contains(&EnrichStep::Prd));
        assert!(selected.contains(&EnrichStep::Briefs));
        assert!(selected.contains(&EnrichStep::Tasks));
        assert!(selected.contains(&EnrichStep::Decompose));
        assert!(selected.contains(&EnrichStep::Verify));
        assert!(selected.contains(&EnrichStep::Tests));
    }

    #[test]
    fn fast_reduces_step_count() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Fast, ALL_ORDERED);

        assert_eq!(selected.len(), ALL_ORDERED.len() - FAST_SKIP.len());
        assert_eq!(selected.len(), 6);
    }

    #[test]
    fn standard_runs_all_steps() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Standard, ALL_ORDERED);

        assert_eq!(selected.len(), ALL_ORDERED.len());
        for step in ALL_ORDERED {
            assert!(selected.contains(step), "standard should include {step}");
        }
    }

    #[test]
    fn complex_runs_all_steps() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Complex, ALL_ORDERED);

        assert_eq!(selected.len(), ALL_ORDERED.len());
        for step in ALL_ORDERED {
            assert!(selected.contains(step), "complex should include {step}");
        }
    }

    #[test]
    fn complex_with_extras_appends_extra_steps() {
        let selector = StepSelector::new().with_complex_extras(vec![EnrichStep::Verify]);

        let selected = selector.select_steps(TaskComplexityBand::Complex, ALL_ORDERED);

        // Verify is already in ALL_ORDERED, so it should not be duplicated.
        assert_eq!(selected.len(), ALL_ORDERED.len());
    }

    #[test]
    fn custom_fast_skip_list() {
        // Only skip Research for fast tasks.
        let selector = StepSelector::new().with_fast_skip(vec![EnrichStep::Research]);

        let selected = selector.select_steps(TaskComplexityBand::Fast, ALL_ORDERED);

        assert_eq!(selected.len(), ALL_ORDERED.len() - 1);
        assert!(!selected.contains(&EnrichStep::Research));
        assert!(selected.contains(&EnrichStep::Dependencies));
    }

    #[test]
    fn custom_standard_skip_list() {
        let selector = StepSelector::new()
            .with_standard_skip(vec![EnrichStep::Scribe, EnrichStep::Invariants]);

        let selected = selector.select_steps(TaskComplexityBand::Standard, ALL_ORDERED);

        assert_eq!(selected.len(), ALL_ORDERED.len() - 2);
        assert!(!selected.contains(&EnrichStep::Scribe));
        assert!(!selected.contains(&EnrichStep::Invariants));
    }

    #[test]
    fn preserves_input_order() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Standard, ALL_ORDERED);

        // Verify the order matches ALL_ORDERED exactly.
        for (i, step) in selected.iter().enumerate() {
            assert_eq!(*step, ALL_ORDERED[i], "order mismatch at index {i}");
        }
    }

    #[test]
    fn empty_steps_returns_empty() {
        let selector = StepSelector::new();
        let selected = selector.select_steps(TaskComplexityBand::Fast, &[]);
        assert!(selected.is_empty());
    }

    #[test]
    fn skip_count_matches_actual_skips() {
        let selector = StepSelector::new();
        let count = selector.skip_count(TaskComplexityBand::Fast, ALL_ORDERED.len());
        assert_eq!(count, FAST_SKIP.len());

        let count = selector.skip_count(TaskComplexityBand::Standard, ALL_ORDERED.len());
        assert_eq!(count, 0);
    }

    #[test]
    fn complex_extras_novel_step_appended() {
        // Use a subset of steps and add an extra that isn't in the subset.
        let subset = &[EnrichStep::Prd, EnrichStep::Briefs, EnrichStep::Tasks];
        let selector = StepSelector::new().with_complex_extras(vec![EnrichStep::Verify]);

        let selected = selector.select_steps(TaskComplexityBand::Complex, subset);

        assert_eq!(selected.len(), 4);
        assert_eq!(selected[3], EnrichStep::Verify);
    }

    #[test]
    fn select_steps_with_single_step_input() {
        let selector = StepSelector::new();

        // Research is in the fast skip list.
        let selected = selector.select_steps(TaskComplexityBand::Fast, &[EnrichStep::Research]);
        assert!(selected.is_empty());

        // Prd is not in the fast skip list.
        let selected = selector.select_steps(TaskComplexityBand::Fast, &[EnrichStep::Prd]);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0], EnrichStep::Prd);
    }
}
