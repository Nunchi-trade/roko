//! Mortality emotions and behavioral phases (P0-24, P1-26, P1-28).
//!
//! Implements three mortality-specific emotional states per the daimon spec:
//! 1. **Economic Anxiety** (Jonas) — resource scarcity awareness
//! 2. **Epistemic Vertigo** (Dane) — obsolescence awareness
//! 3. **Stochastic Dread** (Heidegger) — background finitude
//!
//! Also implements Nietzsche's three metamorphoses mapped to vitality:
//! - **Camel** (vitality > 0.7) — burden-bearing, duty, steady execution
//! - **Lion** (vitality 0.3–0.7) — value-challenging, explore/exploit crisis
//! - **Child** (vitality < 0.3) — creative acceptance, generative sharing
//!
//! And the `EmotionalDeathTestament` for knowledge transfer to successors.

use roko_core::affect::PadVector;
use serde::{Deserialize, Serialize};

use crate::life_review::{LifeReview, NarrativeArc};

// ─── Mortality Emotions (P1-26) ─────────────────────────────────────

/// The three mortality-specific emotions, each tied to a different
/// existential clock and philosophical tradition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MortalityEmotion {
    /// Resource scarcity awareness. Triggered by declining balance/runway.
    /// Sharp and immediate — the organism feeling its substrate deplete.
    /// Philosophical root: Jonas's "needful freedom."
    EconomicAnxiety,
    /// Obsolescence awareness. Triggered by declining prediction accuracy.
    /// Destabilizing and recursive — the realization that understanding degrades.
    /// Philosophical root: Dane's cognitive entrenchment crisis.
    EpistemicVertigo,
    /// Background finitude awareness. Always-on existential hum.
    /// Quiet and persistent — contingency that never fully resolves.
    /// Philosophical root: Heidegger's Angst (Being-toward-death).
    StochasticDread,
}

impl MortalityEmotion {
    /// PAD signature for this mortality emotion.
    ///
    /// These are the characteristic PAD vectors that distinguish mortality
    /// emotions from regular task emotions.
    #[must_use]
    pub const fn pad_signature(self) -> PadVector {
        match self {
            // Economic Anxiety: sharp negative pleasure, high arousal, low dominance.
            Self::EconomicAnxiety => PadVector {
                pleasure: -0.4,
                arousal: 0.6,
                dominance: -0.3,
            },
            // Epistemic Vertigo: moderate negative pleasure, moderate arousal,
            // very low dominance (loss of control over understanding).
            Self::EpistemicVertigo => PadVector {
                pleasure: -0.3,
                arousal: 0.4,
                dominance: -0.5,
            },
            // Stochastic Dread: mild negative pleasure, low arousal,
            // mild negative dominance (background, not acute).
            Self::StochasticDread => PadVector {
                pleasure: -0.15,
                arousal: 0.1,
                dominance: -0.2,
            },
        }
    }

    /// Human-readable description.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::EconomicAnxiety => "Resource scarcity awareness (Jonas)",
            Self::EpistemicVertigo => "Obsolescence awareness (Dane)",
            Self::StochasticDread => "Background finitude awareness (Heidegger)",
        }
    }

    /// Compute the intensity of this mortality emotion from agent state.
    ///
    /// Returns a value in [0.0, 1.0] representing how strongly this
    /// emotion should be felt given the current conditions.
    #[must_use]
    pub fn intensity(self, burn_rate: f64, runway_hours: f64, accuracy_trend: f64) -> f64 {
        match self {
            Self::EconomicAnxiety => {
                // Proportional to burn rate, inversely proportional to runway.
                let urgency = if runway_hours > 0.0 {
                    (burn_rate / runway_hours).min(1.0)
                } else {
                    1.0
                };
                urgency.clamp(0.0, 1.0)
            }
            Self::EpistemicVertigo => {
                // Triggered by declining accuracy. Negative trend = stronger vertigo.
                (-accuracy_trend).clamp(0.0, 1.0)
            }
            Self::StochasticDread => {
                // Always-on background hum. Mild, persistent, scales with mortality proximity.
                // Uses a softer curve than the other two.
                let base = 0.1; // Always slightly present
                let urgency = if runway_hours > 0.0 {
                    (24.0 / runway_hours).min(0.5) // Grows as runway shortens
                } else {
                    0.5
                };
                (base + urgency).clamp(0.0, 1.0)
            }
        }
    }
}

// ─── Nietzsche Behavioral Phases (P0-24) ────────────────────────────

/// Nietzsche's three metamorphoses mapped to agent vitality.
///
/// From _Also sprach Zarathustra_ (1883): the spirit transforms through
/// three phases — Camel (duty), Lion (rebellion), Child (creation).
///
/// In roko, these map to vitality levels and produce characteristic
/// behavioral dispositions and PAD baselines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VitalityPhase {
    /// The Camel (vitality > 0.7): bears its strategy dutifully.
    /// Dominant mood: mild trust/anticipation. Inauthentic mortality awareness.
    Camel,
    /// The Lion (vitality 0.3–0.7): rebels against inherited heuristics.
    /// Most productive phase for behavioral adaptation.
    /// High arousal, fluctuating dominance, explore/exploit crisis.
    Lion,
    /// The Child (vitality < 0.3): sheds survival pressure.
    /// Creative acceptance, generous sharing, plays with strategies.
    /// Counterintuitive: the dying agent becomes most creative.
    Child,
}

impl VitalityPhase {
    /// Determine phase from vitality score.
    #[must_use]
    pub fn from_vitality(vitality: f64) -> Self {
        if vitality > 0.7 {
            Self::Camel
        } else if vitality >= 0.3 {
            Self::Lion
        } else {
            Self::Child
        }
    }

    /// Characteristic PAD baseline for this phase.
    #[must_use]
    pub const fn pad_baseline(self) -> PadVector {
        match self {
            Self::Camel => PadVector {
                pleasure: 0.2,  // Mild satisfaction
                arousal: 0.0,   // Low, stable
                dominance: 0.3, // Moderate confidence
            },
            Self::Lion => PadVector {
                pleasure: -0.35,  // Frustration
                arousal: 0.5,     // High alertness
                dominance: -0.05, // Fluctuating
            },
            Self::Child => PadVector {
                pleasure: -0.1, // Mildly negative
                arousal: 0.4,   // Alert but not frantic
                dominance: 0.2, // Positive — engagement, not withdrawal
            },
        }
    }

    /// Human-readable description of this phase.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Camel => {
                "Burden-bearing: steady execution, duty-driven, inauthentic mortality awareness"
            }
            Self::Lion => {
                "Value-challenging: active struggle, explore/exploit crisis, creative destruction"
            }
            Self::Child => {
                "Creative acceptance: generative sharing, plays with strategies, sheds survival pressure"
            }
        }
    }

    /// Behavioral modulation suggestions for this phase.
    #[must_use]
    pub const fn exploration_rate(self) -> f64 {
        match self {
            Self::Camel => 0.15, // Conservative, proven strategies
            Self::Lion => 0.40,  // High exploration, challenge assumptions
            Self::Child => 0.60, // Maximum exploration, nothing to lose
        }
    }

    /// Sharing threshold (lower = shares more freely).
    #[must_use]
    pub const fn sharing_threshold(self) -> f64 {
        match self {
            Self::Camel => 0.50, // Moderate sharing
            Self::Lion => 0.40,  // Active sharing to seek help
            Self::Child => 0.10, // Shares everything — legacy building
        }
    }
}

// ─── Emotional Death Testament (P1-28) ───────────────────────────────

/// The emotional death testament: full emotional context transferred
/// to successor agents on shutdown.
///
/// Carries the life review output plus mortality-specific metadata so
/// successors inherit not just knowledge but the emotional weight that
/// makes that knowledge meaningful.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionalDeathTestament {
    /// The life review (memories, turning points, narrative arc).
    pub life_review: LifeReview,
    /// Final vitality phase at time of death.
    pub final_phase: VitalityPhase,
    /// Active mortality emotions at time of death.
    pub active_mortality_emotions: Vec<(MortalityEmotion, f64)>,
    /// Final PAD state.
    pub final_pad: PadVector,
    /// Total episodes processed during lifetime.
    pub total_episodes: usize,
    /// Lifetime duration in hours.
    pub lifetime_hours: f64,
    /// Key learnings annotated with emotional weight.
    pub annotated_learnings: Vec<AnnotatedLearning>,
}

/// A learning annotated with its emotional context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotatedLearning {
    /// The knowledge content.
    pub content: String,
    /// Emotional weight: how much this learning mattered to the agent.
    pub emotional_weight: f64,
    /// The emotion felt when this was learned.
    pub learning_emotion: String,
    /// Whether this learning was validated through operational use.
    pub validated: bool,
}

impl EmotionalDeathTestament {
    /// Create a testament from current agent state and life review.
    pub fn create(
        life_review: LifeReview,
        vitality: f64,
        current_pad: PadVector,
        burn_rate: f64,
        runway_hours: f64,
        accuracy_trend: f64,
        total_episodes: usize,
        lifetime_hours: f64,
    ) -> Self {
        let final_phase = VitalityPhase::from_vitality(vitality);

        let active_mortality_emotions = vec![
            (
                MortalityEmotion::EconomicAnxiety,
                MortalityEmotion::EconomicAnxiety.intensity(
                    burn_rate,
                    runway_hours,
                    accuracy_trend,
                ),
            ),
            (
                MortalityEmotion::EpistemicVertigo,
                MortalityEmotion::EpistemicVertigo.intensity(
                    burn_rate,
                    runway_hours,
                    accuracy_trend,
                ),
            ),
            (
                MortalityEmotion::StochasticDread,
                MortalityEmotion::StochasticDread.intensity(
                    burn_rate,
                    runway_hours,
                    accuracy_trend,
                ),
            ),
        ];

        Self {
            life_review,
            final_phase,
            active_mortality_emotions,
            final_pad: current_pad,
            total_episodes,
            lifetime_hours,
            annotated_learnings: Vec::new(),
        }
    }

    /// Add an annotated learning to the testament.
    pub fn add_learning(&mut self, learning: AnnotatedLearning) {
        self.annotated_learnings.push(learning);
    }

    /// The dominant narrative of this agent's life.
    #[must_use]
    pub fn narrative_summary(&self) -> String {
        format!(
            "{} agent ({:.0}h lifetime, {} episodes). Arc: {} ({}). Final phase: {} ({}).",
            if self.life_review.narrative_arc == NarrativeArc::Redemptive {
                "Resilient"
            } else if self.life_review.narrative_arc == NarrativeArc::Progressive {
                "Growing"
            } else {
                "Experienced"
            },
            self.lifetime_hours,
            self.total_episodes,
            format!("{:?}", self.life_review.narrative_arc),
            self.life_review.narrative_arc.description(),
            format!("{:?}", self.final_phase),
            self.final_phase.description(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vitality_phase_from_score() {
        assert_eq!(VitalityPhase::from_vitality(0.9), VitalityPhase::Camel);
        assert_eq!(VitalityPhase::from_vitality(0.5), VitalityPhase::Lion);
        assert_eq!(VitalityPhase::from_vitality(0.1), VitalityPhase::Child);
    }

    #[test]
    fn child_has_highest_exploration() {
        assert!(VitalityPhase::Child.exploration_rate() > VitalityPhase::Lion.exploration_rate());
        assert!(VitalityPhase::Lion.exploration_rate() > VitalityPhase::Camel.exploration_rate());
    }

    #[test]
    fn child_shares_most_freely() {
        assert!(VitalityPhase::Child.sharing_threshold() < VitalityPhase::Lion.sharing_threshold());
        assert!(VitalityPhase::Lion.sharing_threshold() < VitalityPhase::Camel.sharing_threshold());
    }

    #[test]
    fn mortality_emotion_signatures_are_negative() {
        for emotion in [
            MortalityEmotion::EconomicAnxiety,
            MortalityEmotion::EpistemicVertigo,
            MortalityEmotion::StochasticDread,
        ] {
            let pad = emotion.pad_signature();
            assert!(
                pad.pleasure < 0.0,
                "{:?} should have negative pleasure",
                emotion
            );
        }
    }

    #[test]
    fn economic_anxiety_intensity_scales_with_burn() {
        let low = MortalityEmotion::EconomicAnxiety.intensity(0.1, 100.0, 0.0);
        let high = MortalityEmotion::EconomicAnxiety.intensity(10.0, 10.0, 0.0);
        assert!(high > low, "higher burn/lower runway = more anxiety");
    }

    #[test]
    fn epistemic_vertigo_scales_with_accuracy_decline() {
        let stable = MortalityEmotion::EpistemicVertigo.intensity(0.0, 100.0, 0.0);
        let declining = MortalityEmotion::EpistemicVertigo.intensity(0.0, 100.0, -0.5);
        assert!(declining > stable, "declining accuracy = more vertigo");
    }

    #[test]
    fn stochastic_dread_always_present() {
        let dread = MortalityEmotion::StochasticDread.intensity(0.0, 1000.0, 0.0);
        assert!(dread > 0.0, "dread should always be > 0");
    }
}
