//! Tool profile and oneirography configuration sections.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---- [tools] -------------------------------------------------------------

/// Tool profile configuration section.
///
/// Parsed from the `[tools]` section in `roko.toml`:
///
/// ```toml
/// [tools]
/// # Extra tools allowed beyond the role profile.
/// allow = ["bash", "web_fetch"]
/// # Tools to deny regardless of role profile.
/// deny = ["write_file"]
///
/// [tools.profiles.coding]
/// extra_tools = ["bash", "edit_file", "write_file"]
/// excluded_tools = []
///
/// [tools.profiles.research]
/// extra_tools = ["web_search", "web_fetch"]
/// excluded_tools = ["write_file", "edit_file"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Global tool allowlist -- these tools are always available regardless of
    /// role or domain profile. Additive with profile-specific tools.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Global tool denylist -- these tools are never available regardless of
    /// role or domain profile. Takes precedence over `allow`.
    #[serde(default)]
    pub deny: Vec<String>,

    /// Named domain profiles keyed by domain label (e.g., "coding", "research", "chain").
    #[serde(default)]
    pub profiles: HashMap<String, ToolProfileConfig>,
}

/// A single named tool profile with extra/excluded tool lists.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolProfileConfig {
    /// Tools added for this domain/profile.
    #[serde(default)]
    pub extra_tools: Vec<String>,

    /// Tools excluded for this domain/profile.
    #[serde(default)]
    pub excluded_tools: Vec<String>,
}

impl ToolsConfig {
    /// Compute the effective tool set for a given domain.
    ///
    /// The result is: `(base_tools + extra_tools + global_allow) - excluded_tools - global_deny`.
    pub fn effective_tools_for_domain(&self, domain: &str, base_tools: &[String]) -> Vec<String> {
        let mut tools: std::collections::HashSet<String> = base_tools.iter().cloned().collect();

        // Add global allows.
        for tool in &self.allow {
            tools.insert(tool.clone());
        }

        // Add domain-specific extras.
        if let Some(profile) = self.profiles.get(domain) {
            for tool in &profile.extra_tools {
                tools.insert(tool.clone());
            }
            // Remove domain-specific exclusions.
            for tool in &profile.excluded_tools {
                tools.remove(tool);
            }
        }

        // Remove global denies (highest priority).
        for tool in &self.deny {
            tools.remove(tool);
        }

        let mut result: Vec<String> = tools.into_iter().collect();
        result.sort();
        result
    }

    /// Returns `true` if a specific tool is allowed for a domain, considering
    /// all profile layers.
    pub fn is_tool_allowed(&self, domain: &str, tool_name: &str) -> bool {
        // Global deny takes precedence.
        if self.deny.iter().any(|t| t == tool_name) {
            return false;
        }

        // Check domain-specific exclusion.
        if let Some(profile) = self.profiles.get(domain) {
            if profile.excluded_tools.iter().any(|t| t == tool_name) {
                return false;
            }
        }

        true
    }
}

// ---- [oneirography] ------------------------------------------------------

/// Configuration for the oneirography (dream art) pipeline (DREAM-13).
///
/// Disabled by default. Opt-in via `[oneirography]` in roko.toml:
/// ```toml
/// [oneirography]
/// enabled = true
/// provider = "dall-e-3"
/// variants = 3
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OneirographyConfig {
    /// Whether dream art generation is enabled (default `false`).
    pub enabled: bool,
    /// Image generation provider identifier (e.g., `"dall-e-3"`, `"stable-diffusion"`).
    pub provider: String,
    /// Number of image variants to generate per dream cycle.
    pub variants: usize,
    /// Base reserve price for affect-reactive auctions.
    pub base_reserve: f64,
    /// Base auction duration in seconds.
    pub base_duration_seconds: u64,
}

impl Default for OneirographyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: "disabled".to_string(),
            variants: 3,
            base_reserve: 0.01,
            base_duration_seconds: 3600,
        }
    }
}
