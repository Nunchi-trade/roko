//! Property tests for `Score` and `Decay` numerical stability.

use proptest::prelude::*;
use roko_core::{Decay, Score};

proptest! {
    #[test]
    fn score_constructors_keep_axes_finite(
        confidence in any::<f32>(),
        novelty in any::<f32>(),
        utility in any::<f32>(),
        reputation in any::<f32>(),
        precision in any::<f32>(),
        salience in any::<f32>(),
        coherence in any::<f32>(),
    ) {
        let score = Score::new_extended(
            confidence,
            novelty,
            utility,
            reputation,
            precision,
            salience,
            coherence,
        );

        prop_assert!(score.is_finite());
        prop_assert!((0.0..=1.0).contains(&score.confidence));
        prop_assert!((0.0..=1.0).contains(&score.novelty));
        prop_assert!(score.utility >= 0.0);
        prop_assert!(score.reputation >= 0.0);
        prop_assert!((0.0..=1.0).contains(&score.precision));
        prop_assert!((0.0..=1.0).contains(&score.salience));
        prop_assert!((0.0..=1.0).contains(&score.coherence));
    }

    #[test]
    fn decay_outputs_stay_bounded(
        age_ms in any::<i64>(),
        half_life_ms in any::<u64>(),
        ttl_ms in any::<u64>(),
        strength in any::<f32>(),
        scale_ms in any::<u64>(),
    ) {
        let decays = [
            Decay::None,
            Decay::HalfLife { half_life_ms },
            Decay::Ttl { ttl_ms },
            Decay::Ebbinghaus { strength, scale_ms },
        ];

        for decay in decays {
            let weight = decay.apply(age_ms);
            prop_assert!(weight.is_finite());
            prop_assert!((0.0..=1.0).contains(&weight));
        }
    }
}
