//! Config-driven model metadata registry.
//!
//! [`ModelRegistry`] wraps the `[models.*]` table from `roko.toml` and
//! provides typed accessor methods for every model property that was
//! previously inferred from substring matching on slugs. Construct once at
//! startup via [`ModelRegistry::from_config`], share via `Arc<ModelRegistry>`.

use std::collections::HashMap;

use indexmap::IndexMap;
use crate::agent::ModelTier;
use crate::config::schema::{ModelProfile, RokoConfig};

/// Canonical model metadata registry.
///
/// All model property lookups go through this struct instead of ad-hoc
/// substring matching on slugs. Built from config at startup, shared via
/// `Arc<ModelRegistry>` across all subsystems.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    /// Config key → full profile.
    profiles: IndexMap<String, ModelProfile>,
    /// API slug → config key (for slug-based lookups).
    slug_to_key: HashMap<String, String>,
    /// Ordered list of (config_key, tier) for upgrade/downgrade navigation.
    tier_ladder: Vec<(String, ModelTier)>,
}

impl ModelRegistry {
    /// Build a registry from a `RokoConfig`.
    #[must_use]
    pub fn from_config(config: &RokoConfig) -> Self {
        let profiles = config.effective_models();

        let mut slug_to_key = HashMap::with_capacity(profiles.len());
        for (key, profile) in &profiles {
            // Map the wire slug back to the config key.
            if !profile.slug.is_empty() && profile.slug != *key {
                slug_to_key.insert(profile.slug.clone(), key.clone());
            }
            // Also map the key itself for direct lookups.
            slug_to_key.insert(key.clone(), key.clone());
        }

        // Build tier ladder sorted by tier rank (Fast < Standard < Premium).
        let mut tier_ladder: Vec<(String, ModelTier)> = profiles
            .iter()
            .map(|(key, p)| (key.clone(), p.tier.unwrap_or(ModelTier::Standard)))
            .collect();
        tier_ladder.sort_by_key(|(_, tier)| tier_rank(*tier));

        Self {
            profiles,
            slug_to_key,
            tier_ladder,
        }
    }

    /// Resolve a slug (config key or API wire slug) to its config key.
    fn resolve_key<'a>(&'a self, slug: &str) -> Option<&'a str> {
        // Exact match on config key.
        if let Some((key, _)) = self.profiles.get_key_value(slug) {
            return Some(key.as_str());
        }
        // Exact match on wire slug.
        if let Some(key) = self.slug_to_key.get(slug) {
            return Some(key.as_str());
        }
        // Prefix match: find the first config key whose wire slug matches.
        for (key, profile) in &self.profiles {
            if profile.slug == slug {
                return Some(key.as_str());
            }
        }
        None
    }

    /// Get the full profile for a slug, if known.
    #[must_use]
    pub fn profile(&self, slug: &str) -> Option<&ModelProfile> {
        self.resolve_key(slug)
            .and_then(|key| self.profiles.get(key))
    }

    /// Capability tier: Fast, Standard, or Premium.
    #[must_use]
    pub fn tier(&self, slug: &str) -> ModelTier {
        self.profile(slug)
            .and_then(|p| p.tier)
            .unwrap_or(ModelTier::Standard)
    }

    /// Model family for affinity grouping (e.g. "sonnet", "haiku", "gpt").
    #[must_use]
    pub fn family(&self, slug: &str) -> Option<&str> {
        self.profile(slug)
            .and_then(|p| p.family.as_deref())
    }

    /// Whether two slugs belong to the same model family.
    #[must_use]
    pub fn same_family(&self, a: &str, b: &str) -> bool {
        if a == b {
            return true;
        }
        match (self.family(a), self.family(b)) {
            (Some(fa), Some(fb)) => fa == fb,
            _ => false,
        }
    }

    /// Whether the model supports thinking/reasoning output.
    #[must_use]
    pub fn supports_thinking(&self, slug: &str) -> bool {
        self.profile(slug)
            .map(|p| p.supports_thinking)
            .unwrap_or(false)
    }

    /// Quality factor 0.0–1.0 for episode evaluation.
    /// Falls back to tier-derived defaults when not set in config.
    #[must_use]
    pub fn quality_factor(&self, slug: &str) -> f64 {
        if let Some(profile) = self.profile(slug) {
            if let Some(q) = profile.quality_factor {
                return q;
            }
            return quality_from_tier(profile.tier.unwrap_or(ModelTier::Standard));
        }
        0.5
    }

    /// Whether this model is on the provider's free/generous tier.
    #[must_use]
    pub fn is_free_tier(&self, slug: &str) -> bool {
        self.profile(slug)
            .map(|p| p.is_free_tier)
            .unwrap_or(false)
    }

    /// Approximate cost proxy for Pareto frontier calculations.
    /// Falls back to tier-derived defaults when cost data is absent.
    #[must_use]
    pub fn cost_proxy(&self, slug: &str) -> f64 {
        if let Some(profile) = self.profile(slug) {
            // Use actual costs when available.
            if let (Some(inp), Some(out)) = (profile.cost_input_per_m, profile.cost_output_per_m) {
                // Weighted blend: input tokens dominate most workloads.
                return inp * 0.7 + out * 0.3;
            }
            return cost_proxy_from_tier(profile.tier.unwrap_or(ModelTier::Standard));
        }
        cost_proxy_from_tier(ModelTier::Standard)
    }

    /// Provider config key for this model (e.g. "anthropic", "gemini", "zhipu").
    #[must_use]
    pub fn provider_for(&self, slug: &str) -> Option<&str> {
        self.profile(slug).map(|p| p.provider.as_str())
    }

    /// Context window in tokens.
    #[must_use]
    pub fn context_window(&self, slug: &str) -> u64 {
        self.profile(slug)
            .map(|p| p.context_window)
            .unwrap_or(128_000)
    }

    /// Input token cost per million tokens.
    #[must_use]
    pub fn cost_input_per_m(&self, slug: &str) -> f64 {
        self.profile(slug)
            .and_then(|p| p.cost_input_per_m)
            .unwrap_or(0.0)
    }

    /// Output token cost per million tokens.
    #[must_use]
    pub fn cost_output_per_m(&self, slug: &str) -> f64 {
        self.profile(slug)
            .and_then(|p| p.cost_output_per_m)
            .unwrap_or(0.0)
    }

    /// Find a model one tier up from the given slug.
    #[must_use]
    #[allow(unreachable_patterns)]
    pub fn upgrade_tier(&self, slug: &str) -> Option<&str> {
        let current = self.tier(slug);
        let target = match current {
            ModelTier::Fast => ModelTier::Standard,
            ModelTier::Standard => ModelTier::Premium,
            ModelTier::Premium => return None,
            _ => ModelTier::Premium,
        };

        self.tier_ladder
            .iter()
            .find(|(key, tier)| *tier == target && !self.same_family(key, slug))
            .or_else(|| self.tier_ladder.iter().find(|(_, tier)| *tier == target))
            .map(|(key, _)| key.as_str())
    }

    /// Find a model one tier down from the given slug.
    #[must_use]
    #[allow(unreachable_patterns)]
    pub fn downgrade_tier(&self, slug: &str) -> Option<&str> {
        let current = self.tier(slug);
        let target = match current {
            ModelTier::Premium => ModelTier::Standard,
            ModelTier::Standard => ModelTier::Fast,
            ModelTier::Fast => return None,
            _ => ModelTier::Fast,
        };

        self.tier_ladder
            .iter()
            .find(|(key, tier)| *tier == target && !self.same_family(key, slug))
            .or_else(|| self.tier_ladder.iter().find(|(_, tier)| *tier == target))
            .map(|(key, _)| key.as_str())
    }

    /// Iterate over all known (config_key, profile) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ModelProfile)> {
        self.profiles.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of registered models.
    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// All registered model slugs (config keys).
    #[must_use]
    pub fn model_slugs(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self {
            profiles: HashMap::new(),
            slug_to_key: HashMap::new(),
            tier_ladder: Vec::new(),
        }
    }
}

/// Numeric rank for tier ordering.
#[allow(unreachable_patterns)] // ModelTier is #[non_exhaustive]
pub fn tier_rank(tier: ModelTier) -> u8 {
    match tier {
        ModelTier::Fast => 0,
        ModelTier::Standard => 1,
        ModelTier::Premium => 2,
        _ => 1,
    }
}

#[allow(unreachable_patterns)]
fn quality_from_tier(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 0.3,
        ModelTier::Standard => 0.5,
        ModelTier::Premium => 1.0,
        _ => 0.5,
    }
}

#[allow(unreachable_patterns)]
fn cost_proxy_from_tier(tier: ModelTier) -> f64 {
    match tier {
        ModelTier::Fast => 1.0,
        ModelTier::Standard => 3.0,
        ModelTier::Premium => 9.0,
        _ => 3.0,
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> RokoConfig {
        let toml_str = r#"
[models.haiku]
provider = "anthropic"
slug = "claude-haiku-4-5"
context_window = 128000
supports_tools = true
supports_thinking = false
tool_format = "openai_json"
tier = "fast"
family = "haiku"
quality_factor = 0.3
is_free_tier = false

[models.sonnet]
provider = "anthropic"
slug = "claude-sonnet-4-6"
context_window = 200000
supports_tools = true
supports_thinking = false
tool_format = "openai_json"
tier = "standard"
family = "sonnet"
quality_factor = 0.7
cost_input_per_m = 3.0
cost_output_per_m = 15.0

[models.opus]
provider = "anthropic"
slug = "claude-opus-4-6"
context_window = 200000
supports_tools = true
supports_thinking = true
tool_format = "openai_json"
tier = "premium"
family = "opus"
quality_factor = 1.0
cost_input_per_m = 15.0
cost_output_per_m = 75.0

[models.gemini-flash]
provider = "gemini"
slug = "gemini-2.5-flash"
context_window = 1048576
supports_tools = true
supports_thinking = true
tool_format = "openai_json"
tier = "fast"
family = "gemini-flash"
is_free_tier = true
"#;
        toml::from_str(toml_str).expect("test config parses")
    }

    #[test]
    fn known_slug_returns_config_values() {
        let reg = ModelRegistry::from_config(&test_config());
        assert_eq!(reg.tier("haiku"), ModelTier::Fast);
        assert_eq!(reg.tier("sonnet"), ModelTier::Standard);
        assert_eq!(reg.tier("opus"), ModelTier::Premium);
    }

    #[test]
    fn lookup_by_api_slug() {
        let reg = ModelRegistry::from_config(&test_config());
        assert_eq!(reg.tier("claude-haiku-4-5"), ModelTier::Fast);
        assert_eq!(reg.tier("claude-sonnet-4-6"), ModelTier::Standard);
        assert_eq!(reg.tier("claude-opus-4-6"), ModelTier::Premium);
    }

    #[test]
    fn unknown_slug_returns_defaults() {
        let reg = ModelRegistry::from_config(&test_config());
        assert_eq!(reg.tier("totally-unknown-model"), ModelTier::Standard);
        assert_eq!(reg.quality_factor("totally-unknown-model"), 0.5);
        assert_eq!(reg.context_window("totally-unknown-model"), 128_000);
        assert!(!reg.supports_thinking("totally-unknown-model"));
        assert!(reg.provider_for("totally-unknown-model").is_none());
    }

    #[test]
    fn tier_derived_quality_factor() {
        let reg = ModelRegistry::from_config(&test_config());
        // gemini-flash has no quality_factor → derived from Fast tier
        assert!((reg.quality_factor("gemini-flash") - 0.3).abs() < f64::EPSILON);
        // sonnet has explicit quality_factor = 0.7
        assert!((reg.quality_factor("sonnet") - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn upgrade_and_downgrade() {
        let reg = ModelRegistry::from_config(&test_config());

        // Fast → Standard
        let up = reg.upgrade_tier("haiku");
        assert!(up.is_some());
        assert_eq!(reg.tier(up.unwrap()), ModelTier::Standard);

        // Premium → None (can't go higher)
        assert!(reg.upgrade_tier("opus").is_none());

        // Premium → Standard
        let down = reg.downgrade_tier("opus");
        assert!(down.is_some());
        assert_eq!(reg.tier(down.unwrap()), ModelTier::Standard);

        // Fast → None (can't go lower)
        assert!(reg.downgrade_tier("haiku").is_none());
    }

    #[test]
    fn same_family() {
        let reg = ModelRegistry::from_config(&test_config());
        assert!(reg.same_family("haiku", "haiku"));
        assert!(!reg.same_family("haiku", "sonnet"));
        assert!(!reg.same_family("haiku", "unknown"));
    }

    #[test]
    fn is_free_tier() {
        let reg = ModelRegistry::from_config(&test_config());
        assert!(reg.is_free_tier("gemini-flash"));
        assert!(!reg.is_free_tier("sonnet"));
        assert!(!reg.is_free_tier("unknown"));
    }

    #[test]
    fn supports_thinking() {
        let reg = ModelRegistry::from_config(&test_config());
        assert!(reg.supports_thinking("opus"));
        assert!(reg.supports_thinking("gemini-flash"));
        assert!(!reg.supports_thinking("haiku"));
    }

    #[test]
    fn cost_proxy_uses_real_costs() {
        let reg = ModelRegistry::from_config(&test_config());
        // sonnet has cost data: 3.0 * 0.7 + 15.0 * 0.3 = 6.6
        let proxy = reg.cost_proxy("sonnet");
        assert!((proxy - 6.6).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_proxy_falls_back_to_tier() {
        let reg = ModelRegistry::from_config(&test_config());
        // haiku has no cost data → Fast tier → 1.0
        assert!((reg.cost_proxy("haiku") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn provider_for() {
        let reg = ModelRegistry::from_config(&test_config());
        assert_eq!(reg.provider_for("sonnet"), Some("anthropic"));
        assert_eq!(reg.provider_for("gemini-flash"), Some("gemini"));
    }

    #[test]
    fn context_window() {
        let reg = ModelRegistry::from_config(&test_config());
        assert_eq!(reg.context_window("gemini-flash"), 1_048_576);
        assert_eq!(reg.context_window("haiku"), 128_000);
    }
}
