//! Prompt scope classification for `roko do`.

use roko_core::config::schema::RokoConfig;
use roko_gate::PlanComplexity;

/// Resolves a user prompt into the existing gate pipeline complexity type.
pub struct ScopeResolver;

impl ScopeResolver {
    /// Classify a prompt with deterministic heuristics.
    pub async fn resolve(prompt: &str, config: &RokoConfig) -> PlanComplexity {
        let _ = config;
        Self::classify_prompt_complexity(prompt)
    }

    /// Classify a prompt into the existing gate-pipeline complexity type.
    #[must_use]
    pub fn classify_prompt_complexity(prompt: &str) -> PlanComplexity {
        let lower = prompt.to_ascii_lowercase();
        let words = lower.split_whitespace().count();
        let chars = lower.chars().count();
        let sentence_count = lower.matches(['.', '!', '?']).count();

        let trivial_markers = [
            "fix typo",
            "typo",
            "spelling",
            "rename",
            "update comment",
            "format",
            "readme",
            "single file",
        ];
        let complex_markers = [
            "architecture",
            "redesign",
            "refactor",
            "migration",
            "migrate",
            "cross-crate",
            "cross crate",
            "multi-agent",
            "multiple components",
            "entire",
            "sharding",
            "concurrency",
            "security",
            "authentication",
            "database schema",
            "streaming",
            "event architecture",
        ];
        let standard_markers = [
            "add ",
            "add feature",
            "implement",
            "new endpoint",
            "api",
            "cli command",
            "tests",
            "integration",
            "multiple files",
            "workflow",
        ];

        if chars <= 100
            && words <= 16
            && trivial_markers.iter().any(|marker| lower.contains(marker))
        {
            return PlanComplexity::Trivial;
        }

        if words >= 60
            || sentence_count >= 3
            || complex_markers.iter().any(|marker| lower.contains(marker))
        {
            return PlanComplexity::Complex;
        }

        if words >= 18 || standard_markers.iter().any(|marker| lower.contains(marker)) {
            return PlanComplexity::Standard;
        }

        PlanComplexity::Simple
    }
}
