//! Enrichment step enum and metadata.
//!
//! Ported from `apps/mori/src/support_enrich/mod.rs` lines 61-197.
//! Each variant carries associated metadata: output filename, whether it needs
//! an LLM call, its default model per backend, and whether the output is TOML.

use std::fmt;

/// LLM backend selector.
///
/// Determines which model family to use for default model selection.
/// Callers must construct this explicitly (no `Default` impl).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmBackend {
    /// Anthropic Claude models.
    Claude,
    /// `OpenAI` Codex models.
    Codex,
    /// Cursor Compose models.
    Cursor,
    /// Local models via Ollama (Gemma, Llama, Qwen, etc.).
    Ollama,
}

/// An individual enrichment step in the pipeline.
///
/// Steps are ordered by dependency: earlier steps produce artifacts consumed by
/// later steps. Use [`ALL_ORDERED`] for the canonical execution order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EnrichStep {
    /// Generate PRD context extract.
    Prd,
    /// Generate implementation brief.
    Briefs,
    /// Generate tasks.toml.
    Tasks,
    /// Generate step-by-step decomposition.
    Decompose,
    /// Generate a dense research memo for the plan/task corpus.
    Research,
    /// Generate machine-readable dependency manifest.
    Dependencies,
    /// Generate machine-readable fixture manifest.
    Fixtures,
    /// Generate integration test and sidecar guidance.
    Integration,
    /// Generate verify-tasks.toml.
    Verify,
    /// Generate review-tasks.toml.
    Reviews,
    /// Generate testing backlog.
    Tests,
    /// Generate review rubric / invariants.
    Invariants,
    /// Generate scribe-tasks.toml.
    Scribe,
}

/// All 13 enrichment steps in dependency order.
///
/// Ported from `EnrichStep::all_ordered()` (Mori line 159-175).
pub const ALL_ORDERED: &[EnrichStep] = &[
    EnrichStep::Prd,
    EnrichStep::Briefs,
    EnrichStep::Tasks,
    EnrichStep::Decompose,
    EnrichStep::Research,
    EnrichStep::Dependencies,
    EnrichStep::Fixtures,
    EnrichStep::Integration,
    EnrichStep::Verify,
    EnrichStep::Reviews,
    EnrichStep::Tests,
    EnrichStep::Invariants,
    EnrichStep::Scribe,
];

impl EnrichStep {
    /// Output filename within the plan directory.
    ///
    /// Ported from Mori lines 141-156.
    #[must_use]
    pub const fn output_filename(self) -> &'static str {
        match self {
            Self::Prd => "prd-extract.md",
            Self::Briefs => "brief.md",
            Self::Tasks => "tasks.toml",
            Self::Decompose => "decomposition.md",
            Self::Research => "research.md",
            Self::Dependencies => "dependency-manifest.toml",
            Self::Fixtures => "fixture-manifest.toml",
            Self::Integration => "integration.md",
            Self::Verify => "verify-tasks.toml",
            Self::Reviews => "review-tasks.toml",
            Self::Tests => "testing-backlog.md",
            Self::Invariants => "rubric.md",
            Self::Scribe => "scribe-tasks.toml",
        }
    }

    /// Whether this step requires an LLM call.
    ///
    /// Steps that return `false` run pure extraction via `generate_without_llm`.
    /// Ported from Mori lines 121-137.
    #[must_use]
    pub const fn needs_llm(self) -> bool {
        match self {
            Self::Prd
            | Self::Briefs
            | Self::Tasks
            | Self::Research
            | Self::Dependencies
            | Self::Fixtures
            | Self::Integration => false,

            Self::Decompose
            | Self::Verify
            | Self::Reviews
            | Self::Tests
            | Self::Invariants
            | Self::Scribe => true,
        }
    }

    /// Default model for this step given a backend.
    ///
    /// Heavier steps (decompose, verify, review, tests, scribe) use a stronger
    /// model; lighter steps use a smaller/cheaper model.
    /// Ported from Mori lines 93-117.
    #[must_use]
    pub const fn default_model(self, backend: LlmBackend) -> &'static str {
        match backend {
            LlmBackend::Claude => match self {
                Self::Decompose | Self::Verify | Self::Reviews | Self::Tests | Self::Scribe => {
                    "claude-sonnet-4-6"
                }
                Self::Prd
                | Self::Briefs
                | Self::Tasks
                | Self::Invariants
                | Self::Research
                | Self::Dependencies
                | Self::Fixtures
                | Self::Integration => "claude-haiku-4-5-20251001",
            },
            LlmBackend::Codex => match self {
                Self::Decompose | Self::Verify | Self::Reviews | Self::Tests | Self::Scribe => {
                    "gpt-5.4"
                }
                Self::Prd
                | Self::Briefs
                | Self::Tasks
                | Self::Invariants
                | Self::Research
                | Self::Dependencies
                | Self::Fixtures
                | Self::Integration => "gpt-5.4-mini",
            },
            LlmBackend::Cursor => "composer-2-fast",
            // Ollama: heavier steps use a capable local model, lighter use small.
            // These are common Ollama model tags; the actual model depends on what
            // the user has pulled locally.
            LlmBackend::Ollama => match self {
                Self::Decompose | Self::Verify | Self::Reviews | Self::Tests | Self::Scribe => {
                    "gemma4:27b"
                }
                Self::Prd
                | Self::Briefs
                | Self::Tasks
                | Self::Invariants
                | Self::Research
                | Self::Dependencies
                | Self::Fixtures
                | Self::Integration => "gemma4:12b",
            },
        }
    }

    /// Whether the output of this step is TOML (and therefore subject to
    /// validation and repair).
    ///
    /// Ported from Mori `is_toml_step` (lines 1624-1634).
    /// Note: the spec lists 7 TOML steps (Tasks, Decompose, Dependencies,
    /// Fixtures, Reviews, Tests, Invariants) but Mori only has 6 (Tasks,
    /// Verify, Review, Scribe, Dependencies, Fixtures). We follow Mori's
    /// actual runtime behavior here.
    #[must_use]
    pub const fn is_toml(self) -> bool {
        matches!(
            self,
            Self::Tasks
                | Self::Verify
                | Self::Reviews
                | Self::Scribe
                | Self::Dependencies
                | Self::Fixtures
        )
    }
}

impl fmt::Display for EnrichStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Prd => "prd",
            Self::Briefs => "briefs",
            Self::Tasks => "tasks",
            Self::Decompose => "decompose",
            Self::Research => "research",
            Self::Dependencies => "dependencies",
            Self::Fixtures => "fixtures",
            Self::Integration => "integration",
            Self::Verify => "verify",
            Self::Reviews => "reviews",
            Self::Tests => "tests",
            Self::Invariants => "invariants",
            Self::Scribe => "scribe",
        };
        write!(f, "{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ordered_has_13_entries() {
        assert_eq!(ALL_ORDERED.len(), 13);
    }

    #[test]
    fn all_ordered_contains_every_variant() {
        let all = [
            EnrichStep::Prd,
            EnrichStep::Briefs,
            EnrichStep::Tasks,
            EnrichStep::Decompose,
            EnrichStep::Research,
            EnrichStep::Dependencies,
            EnrichStep::Fixtures,
            EnrichStep::Integration,
            EnrichStep::Verify,
            EnrichStep::Reviews,
            EnrichStep::Tests,
            EnrichStep::Invariants,
            EnrichStep::Scribe,
        ];
        for variant in &all {
            assert!(
                ALL_ORDERED.contains(variant),
                "ALL_ORDERED missing {variant}"
            );
        }
    }

    #[test]
    fn output_filename_table() {
        assert_eq!(EnrichStep::Prd.output_filename(), "prd-extract.md");
        assert_eq!(EnrichStep::Briefs.output_filename(), "brief.md");
        assert_eq!(EnrichStep::Tasks.output_filename(), "tasks.toml");
        assert_eq!(EnrichStep::Decompose.output_filename(), "decomposition.md");
        assert_eq!(EnrichStep::Research.output_filename(), "research.md");
        assert_eq!(
            EnrichStep::Dependencies.output_filename(),
            "dependency-manifest.toml"
        );
        assert_eq!(
            EnrichStep::Fixtures.output_filename(),
            "fixture-manifest.toml"
        );
        assert_eq!(EnrichStep::Integration.output_filename(), "integration.md");
        assert_eq!(EnrichStep::Verify.output_filename(), "verify-tasks.toml");
        assert_eq!(EnrichStep::Reviews.output_filename(), "review-tasks.toml");
        assert_eq!(EnrichStep::Tests.output_filename(), "testing-backlog.md");
        assert_eq!(EnrichStep::Invariants.output_filename(), "rubric.md");
        assert_eq!(EnrichStep::Scribe.output_filename(), "scribe-tasks.toml");
    }

    #[test]
    fn needs_llm_table() {
        // Non-LLM steps (pure extraction).
        assert!(!EnrichStep::Prd.needs_llm());
        assert!(!EnrichStep::Briefs.needs_llm());
        assert!(!EnrichStep::Tasks.needs_llm());
        assert!(!EnrichStep::Research.needs_llm());
        assert!(!EnrichStep::Dependencies.needs_llm());
        assert!(!EnrichStep::Fixtures.needs_llm());
        assert!(!EnrichStep::Integration.needs_llm());

        // LLM-required steps.
        assert!(EnrichStep::Decompose.needs_llm());
        assert!(EnrichStep::Verify.needs_llm());
        assert!(EnrichStep::Reviews.needs_llm());
        assert!(EnrichStep::Tests.needs_llm());
        assert!(EnrichStep::Invariants.needs_llm());
        assert!(EnrichStep::Scribe.needs_llm());
    }

    #[test]
    fn is_toml_table() {
        assert!(EnrichStep::Tasks.is_toml());
        assert!(EnrichStep::Verify.is_toml());
        assert!(EnrichStep::Reviews.is_toml());
        assert!(EnrichStep::Scribe.is_toml());
        assert!(EnrichStep::Dependencies.is_toml());
        assert!(EnrichStep::Fixtures.is_toml());

        // Non-TOML steps.
        assert!(!EnrichStep::Prd.is_toml());
        assert!(!EnrichStep::Briefs.is_toml());
        assert!(!EnrichStep::Decompose.is_toml());
        assert!(!EnrichStep::Research.is_toml());
        assert!(!EnrichStep::Integration.is_toml());
        assert!(!EnrichStep::Tests.is_toml());
        assert!(!EnrichStep::Invariants.is_toml());
    }

    #[test]
    fn default_model_claude_heavy_steps_use_sonnet() {
        let heavy = [
            EnrichStep::Decompose,
            EnrichStep::Verify,
            EnrichStep::Reviews,
            EnrichStep::Tests,
            EnrichStep::Scribe,
        ];
        for step in heavy {
            assert_eq!(
                step.default_model(LlmBackend::Claude),
                "claude-sonnet-4-6",
                "expected sonnet for {step}"
            );
        }
    }

    #[test]
    fn default_model_claude_light_steps_use_haiku() {
        let light = [
            EnrichStep::Prd,
            EnrichStep::Briefs,
            EnrichStep::Tasks,
            EnrichStep::Invariants,
            EnrichStep::Research,
            EnrichStep::Dependencies,
            EnrichStep::Fixtures,
            EnrichStep::Integration,
        ];
        for step in light {
            assert_eq!(
                step.default_model(LlmBackend::Claude),
                "claude-haiku-4-5-20251001",
                "expected haiku for {step}"
            );
        }
    }

    #[test]
    fn display_round_trips() {
        for step in ALL_ORDERED {
            let s = format!("{step}");
            assert!(!s.is_empty(), "display should be non-empty for {step:?}");
        }
    }
}
