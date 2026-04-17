#![allow(missing_docs)]

use roko_gate::adaptive_threshold::AdaptiveThresholds;

#[test]
fn unknown_rung_defaults_to_neutral_threshold() {
    let thresholds = AdaptiveThresholds::new();
    assert_eq!(thresholds.threshold_for(42), 0.5);
}

#[test]
fn adaptive_thresholds_persist_across_sessions() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("gate-thresholds.json");

    let mut first_session = AdaptiveThresholds::load(&path).unwrap_or_default();
    first_session.observe(2, true);
    first_session.observe(2, true);

    let expected = first_session.threshold_for(2);
    first_session.save(&path).unwrap();

    let second_session = AdaptiveThresholds::load(&path).unwrap();
    assert!((second_session.threshold_for(2) - expected).abs() < 1e-6);
    assert_eq!(
        second_session
            .rung_stats(2)
            .expect("rung stats should round-trip")
            .total_observations,
        2
    );
}
