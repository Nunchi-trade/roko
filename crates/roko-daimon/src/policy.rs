//! AffectPolicy adapter for DaimonState.
//!
//! Wraps the existing DaimonState to implement the foundation AffectPolicy
//! trait consumed by WorkflowEngine / EffectDriver.

use std::path::PathBuf;

use async_trait::async_trait;
use roko_core::{BehavioralState, RokoError};
use roko_core::foundation::{AffectContext, AffectPolicy, DispatchModulation};

use crate::{AffectEngine, AffectEvent, DaimonState, TierThresholds, adjusted_thresholds};

/// AffectPolicy implementation backed by DaimonState.
///
/// Delegates all trait methods to the underlying DaimonState and its
/// AffectEngine implementation. Owns the state and its persistence path.
pub struct DaimonPolicy {
    state: DaimonState,
    state_path: PathBuf,
}

impl DaimonPolicy {
    /// Load or create a DaimonPolicy from the given state file path.
    ///
    /// If the file exists and is valid JSON, loads it. Otherwise creates
    /// a fresh neutral DaimonState.
    #[must_use]
    pub fn new(state_path: PathBuf) -> Self {
        let state = DaimonState::load_or_new(&state_path);
        Self { state, state_path }
    }
}

#[async_trait]
impl AffectPolicy for DaimonPolicy {
    fn pre_dispatch(&self, _task_id: &str, _role: &str) -> AffectContext {
        let affect = self.state.query();
        let tag = self.state.emotional_tag("pre_dispatch");
        AffectContext {
            behavioral_state: affect.behavioral_state,
            pad: [
                affect.pad.pleasure as f32,
                affect.pad.arousal as f32,
                affect.pad.dominance as f32,
            ],
            emotional_tag: Some(tag.trigger),
        }
    }

    fn on_task_outcome(
        &mut self,
        task_id: &str,
        succeeded: bool,
        _tokens_used: u64,
        _cost_usd: f64,
    ) {
        self.state.appraise(AffectEvent::TaskOutcome {
            task_id: task_id.to_string(),
            succeeded,
        });
    }

    fn on_gate_result(&mut self, gate_name: &str, passed: bool, rung: u8, _confidence: f64) {
        self.state.appraise(AffectEvent::GateResult {
            plan_id: String::new(),
            task_id: gate_name.to_string(),
            passed,
            rung: u32::from(rung),
        });
    }

    fn modulate_dispatch(&self, _role: &str, params: &mut DispatchModulation) {
        let affect = self.state.query();
        let thresholds: TierThresholds = adjusted_thresholds(&affect.behavioral_state);

        // Map tier thresholds to a tier_bias in [-1.0, 1.0].
        // Lower t0_ceiling means the state wants to escalate (positive bias).
        // Default t0_ceiling is 0.20; Struggling is 0.10 (escalate), Coasting is 0.30 (demote).
        params.tier_bias = (0.20 - thresholds.t0_ceiling) as f32 * 5.0;
        params.tier_bias = params.tier_bias.clamp(-1.0, 1.0);

        // Modulate turn limit factor based on behavioral state.
        params.turn_limit_factor = match affect.behavioral_state {
            BehavioralState::Struggling => 1.3,
            BehavioralState::Coasting => 0.8,
            BehavioralState::Focused => 0.9,
            BehavioralState::Exploring => 1.1,
            BehavioralState::Resting | BehavioralState::Engaged => 1.0,
        };

        // Exploration rate from behavioral state.
        params.exploration_rate = match affect.behavioral_state {
            BehavioralState::Exploring => 0.6,
            BehavioralState::Coasting => 0.3,
            BehavioralState::Struggling => 0.1,
            _ => 0.2,
        };
    }

    fn behavioral_state(&self) -> BehavioralState {
        self.state.query().behavioral_state
    }

    async fn persist(&self) -> roko_core::Result<()> {
        AffectEngine::persist(&self.state, &self.state_path).map_err(RokoError::substrate)
    }
}
