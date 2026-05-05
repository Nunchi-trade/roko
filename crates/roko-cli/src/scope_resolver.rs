//! Prompt scope classification for `roko do`.

use roko_core::agent::resolve_model;
use roko_core::config::schema::RokoConfig;
use roko_gate::PlanComplexity;

/// Resolves a user prompt into the existing gate pipeline complexity type.
pub struct ScopeResolver;

impl ScopeResolver {
    /// Classify a prompt, trying the LLM extension point before falling back to
    /// a deterministic heuristic.
    pub async fn resolve(prompt: &str, config: &RokoConfig) -> PlanComplexity {
        match Self::llm_classify(prompt, config).await {
            Ok(complexity) => complexity,
            Err(_) => Self::heuristic_classify(prompt),
        }
    }

    /// LLM classification hook.
    ///
    /// The CLI already builds model-call services in `run.rs`; this method keeps
    /// the resolver wired to the same config/model resolution path so the next
    /// step can call that service without inventing a second dispatch stack.
    async fn llm_classify(prompt: &str, config: &RokoConfig) -> Result<PlanComplexity, String> {
        let model_key = config
            .agent
            .tier_models
            .get("fast")
            .or_else(|| config.agent.tier_models.get("mechanical"))
            .map(String::as_str)
            .unwrap_or(config.agent.default_model.as_str());

        let resolved = resolve_model(config, model_key);
        if resolved.slug.trim().is_empty() {
            return Err("scope classifier model resolved to an empty slug".to_string());
        }

        let _classifier_prompt = format!(
            "Classify this request as one of Trivial, Simple, Standard, Complex:\n\n{prompt}"
        );
        Err(format!(
            "scope classifier model `{}` resolved but the shared ModelCaller hook is not wired yet",
            resolved.slug
        ))
    }

    fn heuristic_classify(prompt: &str) -> PlanComplexity {
        let lower = prompt.to_ascii_lowercase();
        let words = lower.split_whitespace().count();

        let trivial_markers = [
            "fix typo",
            "typo",
            "spelling",
            "rename",
            "update comment",
            "format",
            "readme",
        ];
        let complex_markers = [
            "architecture",
            "redesign",
            "refactor",
            "migration",
            "cross-crate",
            "cross crate",
            "multi-agent",
            "concurrency",
            "security",
            "authentication",
            "database schema",
            "streaming",
            "event architecture",
        ];
        let standard_markers = [
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

        if words <= 14 && trivial_markers.iter().any(|marker| lower.contains(marker)) {
            return PlanComplexity::Trivial;
        }

        if words >= 80 || complex_markers.iter().any(|marker| lower.contains(marker)) {
            return PlanComplexity::Complex;
        }

        if words >= 30 || standard_markers.iter().any(|marker| lower.contains(marker)) {
            return PlanComplexity::Standard;
        }

        PlanComplexity::Simple
    }
}
