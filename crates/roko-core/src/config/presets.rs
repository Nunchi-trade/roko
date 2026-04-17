//! Named configuration presets: minimal, balanced, thorough.
//!
//! Each preset returns a fully-populated [`RokoConfig`] tuned to a
//! different cost/quality tradeoff.

use super::schema::{
    AgentConfig, BudgetConfig, ConductorConfig, GatesConfig, LearningConfig, RokoConfig,
    RoutingConfig,
};

/// Available preset names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Fastest, cheapest: haiku-class models, minimal gates, no parallelism.
    Minimal,
    /// Default: sonnet-class models, standard gates, moderate parallelism.
    Balanced,
    /// Maximum quality: opus-class models, all gates, full parallelism, all reviews.
    Thorough,
}

impl Preset {
    /// Parse from a string (case-insensitive).
    #[must_use]
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" | "min" | "fast" => Some(Self::Minimal),
            "balanced" | "default" | "normal" => Some(Self::Balanced),
            "thorough" | "max" | "full" => Some(Self::Thorough),
            _ => None,
        }
    }

    /// Human-readable label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Balanced => "balanced",
            Self::Thorough => "thorough",
        }
    }

    /// Build a [`RokoConfig`] from this preset.
    #[must_use]
    pub fn to_config(self) -> RokoConfig {
        match self {
            Self::Minimal => minimal(),
            Self::Balanced => balanced(),
            Self::Thorough => thorough(),
        }
    }

    /// All known presets, in order of increasing cost.
    pub const ALL: [Self; 3] = [Self::Minimal, Self::Balanced, Self::Thorough];
}

/// Cheapest, fastest config. Good for smoke tests and cost-sensitive runs.
fn minimal() -> RokoConfig {
    RokoConfig {
        agent: AgentConfig {
            default_model: "claude-haiku-4-5".into(),
            default_backend: "claude".into(),
            default_effort: "low".into(),
            context_limit_k: 100,
            bare_mode: true,
            fallback_model: None,
            ..AgentConfig::default()
        },
        gates: GatesConfig {
            clippy_enabled: false,
            skip_tests: true,
            max_iterations: 1,
        },
        routing: RoutingConfig {
            fast_task_model: "claude-haiku-4-5".into(),
            standard_task_model: "claude-haiku-4-5".into(),
            complex_task_model: "claude-sonnet-4-6".into(),
            ..RoutingConfig::default()
        },
        budget: BudgetConfig {
            max_plan_usd: 5.0,
            max_turn_usd: 1.0,
            prompt_token_budget: 4_000,
        },
        conductor: ConductorConfig {
            max_agents: 2,
            max_parallel_plans: 1,
            parallel_enabled: false,
            express_mode: true,
            max_auto_fix_attempts: 1,
            auto_fix_model: "claude-haiku-4-5".into(),
            ..ConductorConfig::default()
        },
        learning: LearningConfig {
            auto_playbook_refresh: false,
            knowledge_file_intel: false,
            knowledge_warnings: false,
            knowledge_wave_context: false,
            knowledge_error_patterns: false,
            learning_min_occurrences: 5,
            file_intel_max_entries: 5,
            warning_max_entries: 2,
            replan_on_gate_failure: false,
            replan_max_per_plan: 1,
            replan_gate_attempts: 3,
        },
        ..RokoConfig::default()
    }
}

/// The default balanced config. Good for everyday development.
fn balanced() -> RokoConfig {
    // Balanced is exactly the same as `RokoConfig::default()`.
    RokoConfig::default()
}

/// Maximum quality config. All reviews, all gates, premium models.
fn thorough() -> RokoConfig {
    RokoConfig {
        agent: AgentConfig {
            default_model: "claude-opus-4-6".into(),
            default_backend: "claude".into(),
            default_effort: "high".into(),
            context_limit_k: 300,
            bare_mode: true,
            fallback_model: Some("claude-sonnet-4-6".into()),
            ..AgentConfig::default()
        },
        gates: GatesConfig {
            clippy_enabled: true,
            skip_tests: false,
            max_iterations: 5,
        },
        routing: RoutingConfig {
            fast_task_model: "claude-sonnet-4-6".into(),
            standard_task_model: "claude-opus-4-6".into(),
            complex_task_model: "claude-opus-4-6".into(),
            ..RoutingConfig::default()
        },
        budget: BudgetConfig {
            max_plan_usd: 100.0,
            max_turn_usd: 5.0,
            prompt_token_budget: 20_000,
        },
        conductor: ConductorConfig {
            max_agents: 16,
            max_parallel_plans: 4,
            parallel_enabled: true,
            express_mode: false,
            max_auto_fix_attempts: 5,
            auto_fix_model: "claude-sonnet-4-6".into(),
            warm_implementers_per_plan: 2,
            ..ConductorConfig::default()
        },
        learning: LearningConfig {
            auto_playbook_refresh: true,
            knowledge_file_intel: true,
            knowledge_warnings: true,
            knowledge_wave_context: true,
            knowledge_error_patterns: true,
            learning_min_occurrences: 1,
            file_intel_max_entries: 30,
            warning_max_entries: 10,
            replan_on_gate_failure: true,
            replan_max_per_plan: 2,
            replan_gate_attempts: 3,
        },
        ..RokoConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_roundtrip() {
        for preset in Preset::ALL {
            let cfg = preset.to_config();
            let text = cfg.to_toml().expect("serialize");
            let back = RokoConfig::from_toml(&text).expect("deserialize");
            assert_eq!(cfg, back, "roundtrip failed for preset {:?}", preset);
        }
    }

    #[test]
    fn balanced_is_default() {
        assert_eq!(Preset::Balanced.to_config(), RokoConfig::default());
    }

    #[test]
    fn minimal_is_cheapest() {
        let m = Preset::Minimal.to_config();
        let b = Preset::Balanced.to_config();
        assert!(m.budget.max_plan_usd < b.budget.max_plan_usd);
        assert!(m.conductor.max_agents < b.conductor.max_agents);
        assert!(m.gates.skip_tests);
    }

    #[test]
    fn thorough_is_most_expensive() {
        let b = Preset::Balanced.to_config();
        let t = Preset::Thorough.to_config();
        assert!(t.budget.max_plan_usd > b.budget.max_plan_usd);
        assert!(t.conductor.max_agents > b.conductor.max_agents);
        assert!(!t.gates.skip_tests);
        assert_eq!(t.conductor.max_parallel_plans, 4);
    }

    #[test]
    fn from_str_loose_parses_aliases() {
        assert_eq!(Preset::from_str_loose("minimal"), Some(Preset::Minimal));
        assert_eq!(Preset::from_str_loose("MIN"), Some(Preset::Minimal));
        assert_eq!(Preset::from_str_loose("fast"), Some(Preset::Minimal));
        assert_eq!(Preset::from_str_loose("balanced"), Some(Preset::Balanced));
        assert_eq!(Preset::from_str_loose("default"), Some(Preset::Balanced));
        assert_eq!(Preset::from_str_loose("normal"), Some(Preset::Balanced));
        assert_eq!(Preset::from_str_loose("thorough"), Some(Preset::Thorough));
        assert_eq!(Preset::from_str_loose("max"), Some(Preset::Thorough));
        assert_eq!(Preset::from_str_loose("full"), Some(Preset::Thorough));
        assert_eq!(Preset::from_str_loose("unknown"), None);
    }

    #[test]
    fn preset_labels() {
        assert_eq!(Preset::Minimal.label(), "minimal");
        assert_eq!(Preset::Balanced.label(), "balanced");
        assert_eq!(Preset::Thorough.label(), "thorough");
    }

    #[test]
    fn minimal_disables_learning() {
        let m = Preset::Minimal.to_config();
        assert!(!m.learning.auto_playbook_refresh);
        assert!(!m.learning.knowledge_file_intel);
    }

    #[test]
    fn thorough_enables_all_learning() {
        let t = Preset::Thorough.to_config();
        assert!(t.learning.auto_playbook_refresh);
        assert!(t.learning.knowledge_file_intel);
        assert!(t.learning.knowledge_warnings);
        assert!(t.learning.knowledge_wave_context);
        assert!(t.learning.knowledge_error_patterns);
    }
}
