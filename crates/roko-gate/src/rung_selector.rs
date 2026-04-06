//! Complexity-based rung selection for the 6-rung gate pipeline (§10.14).
//!
//! Decides **which rungs to run** based on a plan's complexity and prior
//! failure count. Trivial plans don't pay for property tests; complex plans
//! run every rung. On repeated failure the selector escalates: a Trivial
//! plan that has failed twice is promoted to Standard, running additional
//! verification rungs.
//!
//! This is a **pure function** — no I/O, no global state, no randomness.
//! The [`GatePipeline`](crate::gate_pipeline::GatePipeline) consults
//! [`select_rungs`] before building its gate chain.

use serde::{Deserialize, Serialize};

// ─── Plan complexity ─────────────────────────────────────────────────────

/// Complexity classification for a plan, controls which gate rungs execute.
///
/// Defined here (not in `roko-core`) because `roko-core` ships the
/// task-level [`roko_core::TaskComplexityBand`] (Fast/Standard/Complex)
/// while this is the **plan-level** complexity with a finer 4-tier
/// granularity matching the Mori `PlanComplexity` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlanComplexity {
    /// Single-line / derive-only change. Compile + test only.
    Trivial,
    /// Small feature, few files. Adds lint + symbol checking.
    Simple,
    /// Normal plan. Adds generated behavioural tests.
    Standard,
    /// Large cross-crate work. Full rung suite.
    Complex,
}

impl PlanComplexity {
    /// Escalate one step toward `Complex`. Saturates at `Complex`.
    #[must_use]
    pub const fn escalate(self) -> Self {
        match self {
            Self::Trivial => Self::Simple,
            Self::Simple => Self::Standard,
            Self::Standard | Self::Complex => Self::Complex,
        }
    }

    /// Escalate by `n` steps (each step moves one tier toward `Complex`).
    #[must_use]
    pub fn escalate_by(self, n: u32) -> Self {
        let mut c = self;
        for _ in 0..n {
            c = c.escalate();
        }
        c
    }
}

// ─── Rung identifiers ───────────────────────────────────────────────────

/// Canonical rung identifiers. Numeric repr matches [`CANONICAL_ORDER`]
/// position, so derived `Ord` gives the correct execution sequence.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[non_exhaustive]
#[repr(u8)]
pub enum Rung {
    /// Rung 0: compile / type-check.
    Compile = 0,
    /// Rung 1: lint (clippy, eslint, …).
    Lint = 1,
    /// Rung 2: existing test suite.
    Test = 2,
    /// Rung 3: symbol-manifest check.
    Symbol = 3,
    /// Rung 4: generated behavioural tests.
    GeneratedTest = 4,
    /// Rung 5: property-based tests.
    PropertyTest = 5,
    /// Rung 6: integration scenario.
    Integration = 6,
}

/// Canonical execution order. Compile first, Integration last.
pub const CANONICAL_ORDER: [Rung; 7] = [
    Rung::Compile,
    Rung::Lint,
    Rung::Test,
    Rung::Symbol,
    Rung::GeneratedTest,
    Rung::PropertyTest,
    Rung::Integration,
];

impl Rung {
    /// Short display label for TUI / logging.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Compile => "compile",
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Symbol => "symbol",
            Self::GeneratedTest => "gen-test",
            Self::PropertyTest => "prop-test",
            Self::Integration => "integration",
        }
    }
}

// ─── Rung capabilities ──────────────────────────────────────────────────

/// Availability caps — a rung is included only if the project supports it.
///
/// A cap can only **remove** a rung the complexity band selected; it can
/// never *add* one the band did not select.
#[derive(Clone, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct RungCaps {
    /// Is a linter configured for the project?
    pub has_lint_tool: bool,
    /// Does the plan have a generated symbol manifest?
    pub has_symbol_manifest: bool,
    /// Does the plan have generated behavioural tests?
    pub has_generated_tests: bool,
    /// Does the plan have property-based tests?
    pub has_property_tests: bool,
    /// Does the plan have an integration scenario?
    pub has_integration_scenario: bool,
}

impl RungCaps {
    /// All caps enabled — every rung is available.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            has_lint_tool: true,
            has_symbol_manifest: true,
            has_generated_tests: true,
            has_property_tests: true,
            has_integration_scenario: true,
        }
    }

    /// Returns `true` if `rung` is available according to these caps.
    const fn allows(&self, rung: Rung) -> bool {
        match rung {
            Rung::Compile | Rung::Test => true, // always available
            Rung::Lint => self.has_lint_tool,
            Rung::Symbol => self.has_symbol_manifest,
            Rung::GeneratedTest => self.has_generated_tests,
            Rung::PropertyTest => self.has_property_tests,
            Rung::Integration => self.has_integration_scenario,
        }
    }
}

// ─── Selection logic ────────────────────────────────────────────────────

/// Base rungs for a given complexity level (before cap filtering).
///
/// Decision table:
///
/// | Complexity | Compile | Lint | Test | Symbol | GenTest | PropTest | Integration |
/// |---|---|---|---|---|---|---|---|
/// | Trivial    |    ✓    |      |  ✓   |        |         |          |             |
/// | Simple     |    ✓    |  ✓   |  ✓   |   ✓    |         |          |             |
/// | Standard   |    ✓    |  ✓   |  ✓   |   ✓    |    ✓    |          |             |
/// | Complex    |    ✓    |  ✓   |  ✓   |   ✓    |    ✓    |    ✓     |      ✓      |
const fn base_rungs(complexity: PlanComplexity) -> &'static [Rung] {
    match complexity {
        PlanComplexity::Trivial => &[Rung::Compile, Rung::Test],
        PlanComplexity::Simple => &[Rung::Compile, Rung::Lint, Rung::Test, Rung::Symbol],
        PlanComplexity::Standard => &[
            Rung::Compile,
            Rung::Lint,
            Rung::Test,
            Rung::Symbol,
            Rung::GeneratedTest,
        ],
        PlanComplexity::Complex => &[
            Rung::Compile,
            Rung::Lint,
            Rung::Test,
            Rung::Symbol,
            Rung::GeneratedTest,
            Rung::PropertyTest,
            Rung::Integration,
        ],
    }
}

/// Select the ordered rungs to run for a plan.
///
/// The selection is a function of three inputs:
///
/// 1. **`complexity`** — the plan's base complexity classification.
/// 2. **`caps`** — which rungs are actually available (artifacts present,
///    tools configured). Caps strictly narrow: they can only remove a rung,
///    never add one the complexity band did not select.
/// 3. **`prior_failures`** — how many times this plan has already failed.
///    Each prior failure escalates the effective complexity one tier
///    (Trivial → Simple → Standard → Complex, saturating at Complex).
///    This is the "escalation ladder" — repeated failures trigger
///    progressively more thorough verification.
///
/// Results are always in [`CANONICAL_ORDER`].
#[must_use]
pub fn select_rungs(
    complexity: PlanComplexity,
    caps: &RungCaps,
    prior_failures: u32,
) -> Vec<Rung> {
    let effective = complexity.escalate_by(prior_failures);
    base_rungs(effective)
        .iter()
        .copied()
        .filter(|r| caps.allows(*r))
        .collect()
}

/// Returns `true` if `rung` is part of the **base** selection for
/// `complexity` (ignoring caps and prior failures).
///
/// Useful for TUI colouring and router introspection.
#[must_use]
pub fn is_selected(complexity: PlanComplexity, rung: Rung) -> bool {
    base_rungs(complexity).contains(&rung)
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: all caps enabled.
    fn all_caps() -> RungCaps {
        RungCaps::all()
    }

    // ── Base selection (0 prior failures) ────────────────────────────

    #[test]
    fn trivial_selects_compile_and_test() {
        let rungs = select_rungs(PlanComplexity::Trivial, &all_caps(), 0);
        assert_eq!(rungs, vec![Rung::Compile, Rung::Test]);
    }

    #[test]
    fn simple_selects_compile_lint_test_symbol() {
        let rungs = select_rungs(PlanComplexity::Simple, &all_caps(), 0);
        assert_eq!(
            rungs,
            vec![Rung::Compile, Rung::Lint, Rung::Test, Rung::Symbol]
        );
    }

    #[test]
    fn standard_selects_five_rungs() {
        let rungs = select_rungs(PlanComplexity::Standard, &all_caps(), 0);
        assert_eq!(
            rungs,
            vec![
                Rung::Compile,
                Rung::Lint,
                Rung::Test,
                Rung::Symbol,
                Rung::GeneratedTest,
            ]
        );
    }

    #[test]
    fn complex_selects_all_seven_rungs() {
        let rungs = select_rungs(PlanComplexity::Complex, &all_caps(), 0);
        assert_eq!(rungs, CANONICAL_ORDER.to_vec());
    }

    // ── Caps filtering ───────────────────────────────────────────────

    #[test]
    fn complex_without_integration_drops_it() {
        let caps = RungCaps {
            has_integration_scenario: false,
            ..RungCaps::all()
        };
        let rungs = select_rungs(PlanComplexity::Complex, &caps, 0);
        assert!(!rungs.contains(&Rung::Integration));
        assert_eq!(rungs.len(), 6);
    }

    #[test]
    fn standard_without_symbol_drops_it() {
        let caps = RungCaps {
            has_symbol_manifest: false,
            ..RungCaps::all()
        };
        let rungs = select_rungs(PlanComplexity::Standard, &caps, 0);
        assert!(!rungs.contains(&Rung::Symbol));
        assert_eq!(
            rungs,
            vec![
                Rung::Compile,
                Rung::Lint,
                Rung::Test,
                Rung::GeneratedTest,
            ]
        );
    }

    #[test]
    fn trivial_ignores_generated_test_cap() {
        // Caps can't ADD a rung the complexity band didn't select.
        let rungs = select_rungs(PlanComplexity::Trivial, &all_caps(), 0);
        assert!(!rungs.contains(&Rung::GeneratedTest));
    }

    #[test]
    fn caps_with_no_linter_drops_lint() {
        let caps = RungCaps {
            has_lint_tool: false,
            ..RungCaps::all()
        };
        let rungs = select_rungs(PlanComplexity::Simple, &caps, 0);
        assert_eq!(rungs, vec![Rung::Compile, Rung::Test, Rung::Symbol]);
    }

    #[test]
    fn compile_always_present() {
        // Even with default (all-false) caps, Compile is always included.
        let caps = RungCaps::default();
        for complexity in [
            PlanComplexity::Trivial,
            PlanComplexity::Simple,
            PlanComplexity::Standard,
            PlanComplexity::Complex,
        ] {
            let rungs = select_rungs(complexity, &caps, 0);
            assert!(
                rungs.contains(&Rung::Compile),
                "Compile missing for {complexity:?}"
            );
        }
    }

    #[test]
    fn test_always_present() {
        let caps = RungCaps::default();
        for complexity in [
            PlanComplexity::Trivial,
            PlanComplexity::Simple,
            PlanComplexity::Standard,
            PlanComplexity::Complex,
        ] {
            let rungs = select_rungs(complexity, &caps, 0);
            assert!(
                rungs.contains(&Rung::Test),
                "Test missing for {complexity:?}"
            );
        }
    }

    // ── Escalation ladder (prior failures) ───────────────────────────

    #[test]
    fn one_failure_escalates_trivial_to_simple() {
        let rungs = select_rungs(PlanComplexity::Trivial, &all_caps(), 1);
        // Should match Simple base selection.
        assert_eq!(
            rungs,
            vec![Rung::Compile, Rung::Lint, Rung::Test, Rung::Symbol]
        );
    }

    #[test]
    fn two_failures_escalate_trivial_to_standard() {
        let rungs = select_rungs(PlanComplexity::Trivial, &all_caps(), 2);
        assert_eq!(
            rungs,
            vec![
                Rung::Compile,
                Rung::Lint,
                Rung::Test,
                Rung::Symbol,
                Rung::GeneratedTest,
            ]
        );
    }

    #[test]
    fn three_failures_escalate_trivial_to_complex() {
        let rungs = select_rungs(PlanComplexity::Trivial, &all_caps(), 3);
        assert_eq!(rungs, CANONICAL_ORDER.to_vec());
    }

    #[test]
    fn complex_saturates_on_repeated_failure() {
        let base = select_rungs(PlanComplexity::Complex, &all_caps(), 0);
        let escalated = select_rungs(PlanComplexity::Complex, &all_caps(), 5);
        assert_eq!(base, escalated, "Complex cannot escalate further");
    }

    #[test]
    fn escalation_respects_caps() {
        // Trivial + 1 failure → Simple, but no linter → Lint dropped.
        let caps = RungCaps {
            has_lint_tool: false,
            ..RungCaps::all()
        };
        let rungs = select_rungs(PlanComplexity::Trivial, &caps, 1);
        assert_eq!(rungs, vec![Rung::Compile, Rung::Test, Rung::Symbol]);
    }

    // ── is_selected (base policy, no escalation) ─────────────────────

    #[test]
    fn is_selected_trivial_compile() {
        assert!(is_selected(PlanComplexity::Trivial, Rung::Compile));
    }

    #[test]
    fn is_selected_trivial_not_property_test() {
        assert!(!is_selected(PlanComplexity::Trivial, Rung::PropertyTest));
    }

    #[test]
    fn is_selected_complex_includes_all() {
        for rung in CANONICAL_ORDER {
            assert!(
                is_selected(PlanComplexity::Complex, rung),
                "{rung:?} should be selected for Complex"
            );
        }
    }

    // ── Canonical ordering ───────────────────────────────────────────

    #[test]
    fn output_is_always_in_canonical_order() {
        // Regardless of cap order or complexity, the output must be sorted.
        for complexity in [
            PlanComplexity::Trivial,
            PlanComplexity::Simple,
            PlanComplexity::Standard,
            PlanComplexity::Complex,
        ] {
            for failures in 0..4 {
                let rungs = select_rungs(complexity, &all_caps(), failures);
                let mut sorted = rungs.clone();
                sorted.sort();
                assert_eq!(
                    rungs, sorted,
                    "Non-canonical order for {complexity:?} failures={failures}"
                );
            }
        }
    }

    // ── Decision table snapshot ──────────────────────────────────────

    #[test]
    fn decision_table_full_matrix() {
        let all = all_caps();

        // Trivial: Compile, Test
        let t = select_rungs(PlanComplexity::Trivial, &all, 0);
        assert!(t.contains(&Rung::Compile));
        assert!(!t.contains(&Rung::Lint));
        assert!(t.contains(&Rung::Test));
        assert!(!t.contains(&Rung::Symbol));
        assert!(!t.contains(&Rung::GeneratedTest));
        assert!(!t.contains(&Rung::PropertyTest));
        assert!(!t.contains(&Rung::Integration));

        // Simple: +Lint, +Symbol
        let s = select_rungs(PlanComplexity::Simple, &all, 0);
        assert!(s.contains(&Rung::Compile));
        assert!(s.contains(&Rung::Lint));
        assert!(s.contains(&Rung::Test));
        assert!(s.contains(&Rung::Symbol));
        assert!(!s.contains(&Rung::GeneratedTest));
        assert!(!s.contains(&Rung::PropertyTest));
        assert!(!s.contains(&Rung::Integration));

        // Standard: +GeneratedTest
        let st = select_rungs(PlanComplexity::Standard, &all, 0);
        assert!(st.contains(&Rung::Compile));
        assert!(st.contains(&Rung::Lint));
        assert!(st.contains(&Rung::Test));
        assert!(st.contains(&Rung::Symbol));
        assert!(st.contains(&Rung::GeneratedTest));
        assert!(!st.contains(&Rung::PropertyTest));
        assert!(!st.contains(&Rung::Integration));

        // Complex: +PropertyTest, +Integration
        let c = select_rungs(PlanComplexity::Complex, &all, 0);
        assert!(c.contains(&Rung::Compile));
        assert!(c.contains(&Rung::Lint));
        assert!(c.contains(&Rung::Test));
        assert!(c.contains(&Rung::Symbol));
        assert!(c.contains(&Rung::GeneratedTest));
        assert!(c.contains(&Rung::PropertyTest));
        assert!(c.contains(&Rung::Integration));
    }

    // ── PlanComplexity escalation unit tests ─────────────────────────

    #[test]
    fn plan_complexity_escalate() {
        assert_eq!(PlanComplexity::Trivial.escalate(), PlanComplexity::Simple);
        assert_eq!(PlanComplexity::Simple.escalate(), PlanComplexity::Standard);
        assert_eq!(PlanComplexity::Standard.escalate(), PlanComplexity::Complex);
        assert_eq!(PlanComplexity::Complex.escalate(), PlanComplexity::Complex);
    }

    #[test]
    fn plan_complexity_escalate_by_zero_is_identity() {
        for c in [
            PlanComplexity::Trivial,
            PlanComplexity::Simple,
            PlanComplexity::Standard,
            PlanComplexity::Complex,
        ] {
            assert_eq!(c.escalate_by(0), c);
        }
    }

    // ── Rung label ───────────────────────────────────────────────────

    #[test]
    fn rung_labels_are_nonempty() {
        for rung in CANONICAL_ORDER {
            assert!(!rung.label().is_empty(), "{rung:?} has empty label");
        }
    }
}
