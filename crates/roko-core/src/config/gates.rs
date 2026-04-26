//! Verify (verification) and pipeline configuration sections.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};

use super::agent::default_true;

// ---- [gates] -------------------------------------------------------------

/// Verify (verification) settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatesConfig {
    /// Enable clippy / lint gate.
    #[serde(default = "default_true")]
    pub clippy_enabled: bool,
    /// Skip test gate entirely.
    #[serde(default)]
    pub skip_tests: bool,
    /// Max gate retry iterations before giving up.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Per-domain gate overrides. Keys are domain labels (e.g. "research", "docs"),
    /// values are shell commands to run as gates (e.g. `["shell:true"]`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub domain_gates: HashMap<String, Vec<String>>,
}

const fn default_max_iterations() -> u32 {
    3
}

impl Default for GatesConfig {
    fn default() -> Self {
        Self {
            clippy_enabled: default_true(),
            skip_tests: false,
            max_iterations: default_max_iterations(),
            domain_gates: HashMap::new(),
        }
    }
}

// ---- [pipeline] ---------------------------------------------------------

/// Reviewer composition for a pipeline band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PipelineReviewerMode {
    /// Single quick-pass reviewer.
    Quick,
    /// Full review suite (architect, auditor, scribe).
    Full,
}

impl PipelineReviewerMode {
    /// Stable config label used in TOML.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Full => "full",
        }
    }
}

impl Default for PipelineReviewerMode {
    fn default() -> Self {
        Self::Quick
    }
}

/// Effective pipeline settings for one complexity band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineBandConfig {
    /// Whether the strategist stage runs before implementation.
    #[serde(default)]
    pub strategist: bool,
    /// Whether reviewer agents run after implementation.
    #[serde(default)]
    pub reviewers: bool,
    /// Which reviewer composition to use when reviewers are enabled.
    #[serde(default)]
    pub reviewer_mode: PipelineReviewerMode,
    /// Maximum implementation-review iterations before stopping.
    #[serde(default = "default_pipeline_band_iterations")]
    pub max_iterations: u32,
}

const fn default_pipeline_band_iterations() -> u32 {
    1
}

impl PipelineBandConfig {
    /// Defaults for the `mechanical` tier.
    #[must_use]
    pub const fn mechanical() -> Self {
        Self {
            strategist: false,
            reviewers: false,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 1,
        }
    }

    /// Defaults for the `focused` tier.
    #[must_use]
    pub const fn focused() -> Self {
        Self {
            strategist: false,
            reviewers: false,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 2,
        }
    }

    /// Defaults for the `integrative` tier.
    #[must_use]
    pub const fn integrative() -> Self {
        Self {
            strategist: true,
            reviewers: true,
            reviewer_mode: PipelineReviewerMode::Quick,
            max_iterations: 2,
        }
    }

    /// Defaults for the `architectural` tier.
    #[must_use]
    pub const fn architectural() -> Self {
        Self {
            strategist: true,
            reviewers: true,
            reviewer_mode: PipelineReviewerMode::Full,
            max_iterations: 3,
        }
    }
}

impl Default for PipelineBandConfig {
    fn default() -> Self {
        Self::mechanical()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
struct PipelineBandConfigOverride {
    #[serde(default)]
    strategist: Option<bool>,
    #[serde(default)]
    reviewers: Option<bool>,
    #[serde(default)]
    reviewer_mode: Option<PipelineReviewerMode>,
    #[serde(default)]
    max_iterations: Option<u32>,
}

impl PipelineBandConfigOverride {
    fn resolve(self, defaults: PipelineBandConfig) -> PipelineBandConfig {
        PipelineBandConfig {
            strategist: self.strategist.unwrap_or(defaults.strategist),
            reviewers: self.reviewers.unwrap_or(defaults.reviewers),
            reviewer_mode: self.reviewer_mode.unwrap_or(defaults.reviewer_mode),
            max_iterations: self.max_iterations.unwrap_or(defaults.max_iterations),
        }
    }
}

fn deserialize_pipeline_band_with_defaults<'de, D>(
    deserializer: D,
    defaults: PipelineBandConfig,
) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    let override_cfg = PipelineBandConfigOverride::deserialize(deserializer)?;
    Ok(override_cfg.resolve(defaults))
}

fn default_mechanical_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::mechanical()
}

fn deserialize_mechanical_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::mechanical())
}

fn default_focused_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::focused()
}

fn deserialize_focused_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::focused())
}

fn default_integrative_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::integrative()
}

fn deserialize_integrative_pipeline<'de, D>(deserializer: D) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::integrative())
}

fn default_architectural_pipeline() -> PipelineBandConfig {
    PipelineBandConfig::architectural()
}

fn deserialize_architectural_pipeline<'de, D>(
    deserializer: D,
) -> Result<PipelineBandConfig, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_pipeline_band_with_defaults(deserializer, PipelineBandConfig::architectural())
}

/// Complexity-to-pipeline mapping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Mechanical tasks: skip strategist and reviewers.
    #[serde(
        default = "default_mechanical_pipeline",
        deserialize_with = "deserialize_mechanical_pipeline"
    )]
    pub mechanical: PipelineBandConfig,
    /// Focused tasks: implement directly, allow one extra loop.
    #[serde(
        default = "default_focused_pipeline",
        deserialize_with = "deserialize_focused_pipeline"
    )]
    pub focused: PipelineBandConfig,
    /// Integrative tasks: strategist plus a quick reviewer.
    #[serde(
        default = "default_integrative_pipeline",
        deserialize_with = "deserialize_integrative_pipeline"
    )]
    pub integrative: PipelineBandConfig,
    /// Architectural tasks: strategist plus the full reviewer suite.
    #[serde(
        default = "default_architectural_pipeline",
        deserialize_with = "deserialize_architectural_pipeline"
    )]
    pub architectural: PipelineBandConfig,
}

impl PipelineConfig {
    /// Resolve the pipeline settings for a named complexity tier.
    #[must_use]
    pub fn for_tier(&self, tier: &str) -> PipelineBandConfig {
        match tier {
            "mechanical" => self.mechanical,
            "focused" => self.focused,
            "integrative" => self.integrative,
            "architectural" => self.architectural,
            _ => self.focused,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            mechanical: PipelineBandConfig::mechanical(),
            focused: PipelineBandConfig::focused(),
            integrative: PipelineBandConfig::integrative(),
            architectural: PipelineBandConfig::architectural(),
        }
    }
}
