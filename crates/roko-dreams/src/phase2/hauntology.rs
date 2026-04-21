//! Phase 2 hauntology stubs.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Spectral influence metrics for hauntological analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SpectralInfluenceMetrics {
    /// Provenance depth of the deepest inherited entry.
    pub max_provenance_depth: usize,
    /// Fraction of active knowledge entries with inherited provenance.
    pub spectral_density: f64,
    /// Mean arousal delta between inherited and self-generated entries.
    pub inherited_arousal_delta: f64,
    /// Fraction of knowledge space only reachable through anti-correlated retrieval.
    pub foreclosure_index: f64,
    /// Fraction of dream insights that reference inherited entries.
    pub ghost_influence_fraction: f64,
}

impl SpectralInfluenceMetrics {
    /// Construct a spectral-influence snapshot.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_provenance_depth: 0,
            spectral_density: 0.0,
            inherited_arousal_delta: 0.0,
            foreclosure_index: 0.0,
            ghost_influence_fraction: 0.0,
        }
    }
}

/// Spectral trace provenance for a knowledge entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpectralProvenance {
    /// Originating agent identifier.
    pub original_agent_id: String,
    /// Number of hops away from the origin.
    pub generation_depth: usize,
    /// Confidence when the trace was first created.
    pub confidence_at_origin: f64,
    /// Confidence after transit through the system.
    pub confidence_after_transit: f64,
    /// Emotional charge at the origin.
    pub emotional_charge_at_origin: f64,
    /// Path the spectral trace traversed.
    pub transit_path: Vec<String>,
    /// Timestamp at the origin.
    pub created_at_origin: DateTime<Utc>,
}

impl SpectralProvenance {
    /// Construct a provenance record with neutral stub values.
    #[must_use]
    pub fn new(original_agent_id: impl Into<String>, created_at_origin: DateTime<Utc>) -> Self {
        Self {
            original_agent_id: original_agent_id.into(),
            generation_depth: 0,
            confidence_at_origin: 1.0,
            confidence_after_transit: 1.0,
            emotional_charge_at_origin: 0.0,
            transit_path: Vec::new(),
            created_at_origin,
        }
    }
}
