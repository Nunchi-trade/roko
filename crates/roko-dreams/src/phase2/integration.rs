//! Phase 2 cross-system integration stubs.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Sharing mode for dream insights across the mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DreamShareMode {
    /// Broadcast eligible insights to the full mesh.
    Broadcast,
    /// Share only insights that pass selective filters.
    Selective,
    /// Share only in response to an explicit request.
    Solicited,
    /// Disable sharing.
    Disabled,
}

/// Configuration for dream-sharing behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamShareConfig {
    /// Sharing mode for this agent.
    pub mode: DreamShareMode,
    /// Minimum confidence for selective sharing.
    pub selective_confidence_threshold: f64,
    /// Minimum novelty required for selective sharing.
    pub selective_novelty_threshold: f64,
    /// Stigmergy evaporation rate per cycle.
    pub evaporation_rate: f64,
    /// Confidence decay factor per mesh hop.
    pub hop_confidence_decay: f64,
    /// Maximum number of hops allowed for one insight.
    pub max_hops: usize,
    /// Whether to sanitize episode references before sharing.
    pub sanitize_episodes: bool,
    /// Roles allowed to receive the shared insight.
    pub allowed_recipient_roles: Vec<String>,
}

/// Shared dream insight traveling across the mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedDreamInsight {
    /// Stable insight identifier.
    pub insight_id: String,
    /// Source agent identifier.
    pub source_agent_id: String,
    /// Source dream cycle identifier.
    pub source_cycle_id: String,
    /// Human-readable summary of the shared hypothesis.
    pub hypothesis_summary: String,
    /// Confidence at the source agent.
    pub original_confidence: f64,
    /// Confidence after transit and corroboration.
    pub current_confidence: f64,
    /// Number of hops traversed so far.
    pub hop_count: usize,
    /// Agents that independently corroborated the insight.
    pub corroborating_agents: Vec<String>,
    /// Current stigmergy weight.
    pub stigmergy_weight: f64,
    /// Creation time of the shared insight.
    pub created_at: DateTime<Utc>,
    /// Optional expiration time.
    pub expires_at: Option<DateTime<Utc>>,
    /// Tags carried with the insight.
    pub tags: Vec<String>,
}

/// Protocol state for dream sharing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DreamShareProtocol {
    /// Protocol configuration.
    pub config: DreamShareConfig,
    /// Insights received this cycle.
    pub inbound_buffer: Vec<SharedDreamInsight>,
    /// Insights queued for transmission.
    pub outbound_buffer: Vec<SharedDreamInsight>,
    /// Stigmergy weights for mesh-resident insights.
    pub stigmergy_map: HashMap<String, f64>,
}

impl DreamShareProtocol {
    /// Apply one evaporation step to all stigmergy weights.
    pub fn evaporate(&mut self) {
        for weight in self.stigmergy_map.values_mut() {
            *weight *= 1.0 - self.config.evaporation_rate;
        }
        self.stigmergy_map.retain(|_, value| *value >= 0.01);
    }

    /// Corroborate one mesh insight from local waking evidence.
    pub fn corroborate(&mut self, insight_id: &str, agent_id: &str, delta_tau: f64) {
        let entry = self
            .stigmergy_map
            .entry(insight_id.to_string())
            .or_insert(0.0);
        *entry = (1.0 - self.config.evaporation_rate) * *entry + delta_tau;
        if let Some(insight) = self
            .inbound_buffer
            .iter_mut()
            .chain(self.outbound_buffer.iter_mut())
            .find(|insight| insight.insight_id == insight_id)
            .filter(|insight| !insight.corroborating_agents.iter().any(|id| id == agent_id))
        {
            insight.corroborating_agents.push(agent_id.to_string());
            insight.stigmergy_weight = *entry;
        }
    }
}

/// Circadian-inspired scheduling settings for dream timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CircadianScheduler {
    /// Hours of day during which dreaming is preferred.
    pub preferred_hours: Vec<u8>,
    /// Strength of the circadian preference.
    pub circadian_strength: f64,
    /// Minimum interval between dream cycles.
    pub min_interval_mins: u64,
    /// Maximum interval between dream cycles.
    pub max_interval_mins: u64,
    /// Whether cycles should align to task boundaries.
    pub align_to_task_boundaries: bool,
}

/// Fleet-level coordination settings for dream scheduling and aggregation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FleetDreamCoordinator {
    /// Whether dream cycles should be staggered across the fleet.
    pub stagger_cycles: bool,
    /// Minimum stagger interval between two agents dreaming.
    pub min_stagger_mins: u64,
    /// Whether to aggregate dream insights across the fleet.
    pub aggregate_insights: bool,
    /// Confidence boost for independently corroborated patterns.
    pub collective_confirmation_boost: f64,
    /// Minimum number of confirming agents required for the boost.
    pub min_confirming_agents: usize,
}

/// Synaptic-homeostasis renormalization settings for consolidation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynapticRenormalization {
    /// Global scale factor applied during renormalization.
    pub global_scale_factor: f64,
    /// Entries above this confidence are protected.
    pub protection_threshold: f64,
    /// Maximum confidence reduction applied in one cycle.
    pub max_confidence_reduction: f64,
    /// Whether recent validations are exempt.
    pub exempt_recent_validations: bool,
    /// Recency window for exemptions.
    pub recency_window_hours: u64,
}
