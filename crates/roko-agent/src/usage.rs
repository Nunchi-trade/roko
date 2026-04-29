//! Compatibility re-export for shared usage metrics.
//!
//! `Usage` remains the legacy flat counter shape from `roko-core`.
//! `UsageObservation` is the canonical telemetry-facing shape that can
//! distinguish "not reported" from zero.

use serde::{Deserialize, Serialize};

pub use roko_core::chat_types::Usage;

/// Canonical usage observation for agent attempts and model calls.
///
/// Numeric fields are optional so unknown values stay unknown rather than
/// collapsing to zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UsageObservation {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    #[serde(alias = "cache_create_tokens")]
    pub cache_creation_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub source: UsageSource,
    pub model: Option<String>,
    pub wall_ms: u64,
}

/// Provenance for a usage observation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum UsageSource {
    /// Provider-reported usage from the backend response.
    ProviderReported,
    /// Estimated from local accounting.
    Estimated,
    /// Source not known.
    #[default]
    Unknown,
}

impl From<Usage> for UsageObservation {
    fn from(usage: Usage) -> Self {
        Self {
            input_tokens: Some(u64::from(usage.input_tokens)),
            output_tokens: Some(u64::from(usage.output_tokens)),
            cache_creation_tokens: Some(u64::from(usage.cache_create_tokens)),
            cache_read_tokens: Some(u64::from(usage.cache_read_tokens)),
            cost_usd: Some(f64::from(usage.cost_usd)),
            source: UsageSource::Unknown,
            model: None,
            wall_ms: usage.wall_ms,
        }
    }
}

impl From<UsageObservation> for Usage {
    fn from(observation: UsageObservation) -> Self {
        let clamp_u32 = |value: Option<u64>| match value {
            Some(value) => u32::try_from(value).unwrap_or(u32::MAX),
            None => 0,
        };

        Self {
            input_tokens: clamp_u32(observation.input_tokens),
            output_tokens: clamp_u32(observation.output_tokens),
            cache_read_tokens: clamp_u32(observation.cache_read_tokens),
            cache_create_tokens: clamp_u32(observation.cache_creation_tokens),
            cost_usd: observation.cost_usd.map_or(0.0, |value| value as f32),
            wall_ms: observation.wall_ms,
        }
    }
}
