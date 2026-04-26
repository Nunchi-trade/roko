//! Per-section prompt cost attribution helpers.

use serde::{Deserialize, Serialize};

use crate::{AttentionBidder, CompositionStrategy};

/// Per-section cost attribution computed after an agent turn completes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostAttribution {
    /// Turn identifier, usually episode id plus turn index.
    pub turn_id: String,
    /// Actual input tokens reported by the runtime/provider.
    pub total_input_tokens: u64,
    /// Actual prompt/input cost for the turn.
    pub total_cost_usd: f64,
    /// Per-section proportional attribution.
    pub sections: Vec<SectionCost>,
    /// Composition strategy selected for the prompt.
    pub strategy: CompositionStrategy,
    /// VCG payments keyed by stable section id or section name.
    pub vcg_payments: Vec<(String, f64)>,
}

/// One section's attributed share of a completed turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SectionCost {
    /// Stable section id when available.
    pub section_id: String,
    /// Human-readable section name.
    pub section_name: String,
    /// Bidder/subsystem that owned this section.
    pub bidder: AttentionBidder,
    /// Estimated tokens used during prompt assembly.
    pub estimated_tokens: usize,
    /// Fraction of the included-section token estimate.
    pub token_fraction: f64,
    /// Share of total cost attributed to this section.
    pub attributed_cost_usd: f64,
    /// Gate result once known.
    pub gate_passed: Option<bool>,
}

impl CostAttribution {
    /// Compute proportional attribution from the sections included in a turn.
    #[must_use]
    pub fn from_turn(
        turn_id: impl Into<String>,
        total_input_tokens: u64,
        total_cost_usd: f64,
        included_sections: &[(String, String, AttentionBidder, usize)],
        strategy: CompositionStrategy,
        vcg_payments: Vec<(String, f64)>,
    ) -> Self {
        let total_estimated = included_sections
            .iter()
            .map(|(_, _, _, tokens)| *tokens)
            .sum::<usize>()
            .max(1);
        let total_estimated = total_estimated as f64;

        let sections = included_sections
            .iter()
            .map(|(section_id, section_name, bidder, estimated_tokens)| {
                let token_fraction = *estimated_tokens as f64 / total_estimated;
                SectionCost {
                    section_id: section_id.clone(),
                    section_name: section_name.clone(),
                    bidder: *bidder,
                    estimated_tokens: *estimated_tokens,
                    token_fraction,
                    attributed_cost_usd: total_cost_usd * token_fraction,
                    gate_passed: None,
                }
            })
            .collect();

        Self {
            turn_id: turn_id.into(),
            total_input_tokens,
            total_cost_usd,
            sections,
            strategy,
            vcg_payments,
        }
    }

    /// Apply a downstream gate result to all included sections.
    pub fn stamp_gate_result(&mut self, gate_passed: bool) {
        for section in &mut self.sections {
            section.gate_passed = Some(gate_passed);
        }
    }

    /// Compute simple effectiveness per attributed dollar for each section.
    #[must_use]
    pub fn cost_effectiveness(&self) -> Vec<(String, Option<f64>)> {
        self.sections
            .iter()
            .map(|section| {
                let effectiveness = section.gate_passed.map(|passed| {
                    let value = if passed { 1.0 } else { 0.0 };
                    value / section.attributed_cost_usd.max(f64::EPSILON)
                });
                (section.section_id.clone(), effectiveness)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_attribution_is_proportional() {
        let attribution = CostAttribution::from_turn(
            "turn-1",
            300,
            0.03,
            &[
                (
                    "prompt:role".into(),
                    "role".into(),
                    AttentionBidder::TaskContext,
                    100,
                ),
                (
                    "prompt:ctx".into(),
                    "ctx".into(),
                    AttentionBidder::CodeIntelligence,
                    200,
                ),
            ],
            CompositionStrategy::DensityGreedy,
            Vec::new(),
        );

        let total_fraction = attribution
            .sections
            .iter()
            .map(|section| section.token_fraction)
            .sum::<f64>();
        assert!((total_fraction - 1.0).abs() < 0.000_001);
        assert!((attribution.sections[1].attributed_cost_usd - 0.02).abs() < 0.000_001);
    }

    #[test]
    fn cost_attribution_stamps_gate_result() {
        let mut attribution = CostAttribution::from_turn(
            "turn-1",
            100,
            0.01,
            &[(
                "prompt:role".into(),
                "role".into(),
                AttentionBidder::TaskContext,
                100,
            )],
            CompositionStrategy::Vcg,
            vec![("prompt:role".into(), 0.2)],
        );

        attribution.stamp_gate_result(true);

        assert_eq!(attribution.sections[0].gate_passed, Some(true));
        assert!(attribution.cost_effectiveness()[0].1.unwrap() > 0.0);
    }
}
