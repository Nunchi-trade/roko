//! Shared support types for Phase 2+ dream stubs.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use roko_primitives::hdc::HdcVector as PrimitiveHdcVector;
use serde::{Deserialize, Serialize};

/// Re-export of the workspace HDC vector used throughout the dream docs.
pub type HdcVector = PrimitiveHdcVector;

/// Re-export of the workspace insight record type used by dream reports.
pub type InsightRecord = roko_neuro::tier_progression::InsightRecord;

/// Re-export of the workspace model-tier enum used by several Phase 2 configs.
pub type ModelTier = roko_core::agent::ModelTier;

/// Lightweight hypothesis alias used by monitoring-oriented stubs.
pub type Hypothesis = roko_learn::heuristics::Hypothesis;

/// Cross-episode pattern discovered during replay and consolidation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternRecord {
    /// Stable pattern identifier.
    pub id: String,
    /// Human-readable summary of the pattern.
    pub summary: String,
    /// Episodes that contributed to the pattern.
    pub source_episodes: Vec<String>,
    /// Confidence assigned during consolidation.
    pub confidence: f64,
}

/// Summary of emotional depotentiation applied during one dream cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DepotentiationSummary {
    /// Number of episodes whose arousal markers were reduced.
    pub episodes_processed: usize,
    /// Mean arousal delta applied across the batch.
    pub mean_arousal_delta: f64,
    /// Lowest post-dream arousal observed after clamping.
    pub minimum_post_arousal: f64,
}

/// Candidate strategy carried by EVOLUTION/MAP-Elites stubs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionaryStrategy {
    /// Stable strategy identifier.
    pub id: String,
    /// Human-readable description of the strategy.
    pub description: String,
    /// Knowledge or heuristic ids that seeded the strategy.
    pub parent_knowledge_ids: Vec<String>,
    /// Behavioral descriptor values used for archive indexing.
    pub descriptors: Vec<f64>,
}

/// Palette metadata for dream rendering surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorPalette {
    /// Palette identifier.
    pub name: String,
    /// Ordered color stops used by the renderer.
    pub colors: Vec<String>,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            name: "dream-default".to_string(),
            colors: vec![
                "mist".to_string(),
                "indigo".to_string(),
                "amber".to_string(),
                "graphite".to_string(),
            ],
        }
    }
}

/// Request passed to an image-generation provider for oneirography.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageGenRequest {
    /// Prompt describing the dream content to render.
    pub prompt: String,
    /// Requested image width in pixels.
    pub width: u32,
    /// Requested image height in pixels.
    pub height: u32,
    /// Optional provenance tags describing the source dream.
    pub tags: Vec<String>,
}

/// Privacy class for an image-generation provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyLevel {
    /// Content may leave the local machine and be retained by the provider.
    Standard,
    /// Provider promises zero retention for submitted content.
    ZeroRetention,
    /// Provider executes locally with no external transfer.
    LocalOnly,
}

/// Result returned by an image-generation provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageGenResult {
    /// Provider-generated asset identifier.
    pub id: String,
    /// Filesystem path or object path for the rendered asset.
    pub asset_path: PathBuf,
    /// Prompt actually submitted to the provider after any local shaping.
    pub prompt: String,
    /// Size of the generated image in bytes.
    pub byte_len: usize,
}

/// Snapshot of compute and plateau state embedded into dream art.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    /// Fraction of compute budget remaining.
    pub remaining_budget_fraction: f64,
    /// Whether the agent appears to be near a knowledge plateau.
    pub knowledge_plateau: bool,
    /// Backlog of unprocessed episodes at snapshot time.
    pub pending_episode_count: usize,
}

/// Compressed view of one discovered causal edge for state snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalEdgeSnap {
    /// Source feature or event.
    pub cause: String,
    /// Downstream feature or event.
    pub effect: String,
    /// Approximate temporal lag in seconds.
    pub lag_secs: u64,
    /// Confidence of the discovered edge.
    pub confidence: f64,
    /// When the edge was first discovered.
    pub discovered_at: DateTime<Utc>,
}

/// Digest of top Neuro entries embedded into oneirography state payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuroDigest {
    /// Entries retained in the digest.
    pub entries: Vec<NeuroDigestEntry>,
}

/// Lightweight digest entry for one high-confidence Neuro record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NeuroDigestEntry {
    /// Stable knowledge identifier.
    pub id: String,
    /// Human-readable title or opening sentence.
    pub label: String,
    /// Confidence score at digest time.
    pub confidence: f64,
}

/// Primitive event used by fault-tree stubs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BasicEvent {
    /// Stable event identifier.
    pub id: String,
    /// Human-readable event description.
    pub description: String,
    /// Probability-like occurrence estimate.
    pub probability: f64,
}

/// Threat novelty tier used by advanced threat-generation stubs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreatTier {
    /// Already-known failure mode with routine handling.
    Known,
    /// Emerging pattern that warrants further rehearsal.
    Emerging,
    /// Novel or especially surprising threat pattern.
    Novel,
}
