//! PE09 regression coverage for shipped daimon public contracts.

use chrono::{Duration, Utc};
use roko_core::{BehavioralState, PadVector};
use roko_daimon::{
    AffectEngine, AffectEvent, DaimonState, StrategySpaceDefinition, TaskStrategyObservation,
};
use tempfile::TempDir;

#[test]
fn emotional_tag_serializes_the_shipping_single_layer_schema() {
    let mut state = DaimonState::new();
    state.state.pad = PadVector::new(0.4, -0.2, 0.6);
    // Set ALMA mood layer to match so mood_snapshot is correct.
    state.state.alma.mood = PadVector::new(0.4, -0.2, 0.6);

    let tag = state.emotional_tag("gate_failure");
    let value = serde_json::to_value(&tag).expect("serialize emotional tag");
    let object = value.as_object().expect("emotional tag object");

    assert_eq!(object.len(), 4);
    assert!(object.contains_key("pad"));
    assert!(object.contains_key("intensity"));
    assert!(object.contains_key("trigger"));
    assert!(object.contains_key("mood_snapshot"));
    assert!(!object.contains_key("plutchik"));
    assert!(!object.contains_key("discovery_emotion"));
    assert_eq!(tag.pad, state.state.pad);
    // mood_snapshot now comes from the ALMA mood layer.
    assert_eq!(tag.mood_snapshot, state.state.alma.mood);
}

#[test]
fn confidence_decay_moves_toward_midpoint_from_both_extremes() {
    let mut high = DaimonState::with_half_life_hours(1.0);
    high.state.confidence = 1.0;
    high.state.updated_at = Utc::now() - Duration::hours(4);
    let _ = high.appraise(AffectEvent::TimePressure {
        task_id: "task-high".to_string(),
        deadline_proximity: 0.0,
    });

    let mut low = DaimonState::with_half_life_hours(1.0);
    low.state.confidence = 0.0;
    low.state.updated_at = Utc::now() - Duration::hours(4);
    let _ = low.appraise(AffectEvent::TimePressure {
        task_id: "task-low".to_string(),
        deadline_proximity: 0.0,
    });

    assert!(high.query_state().confidence < 1.0);
    assert!(high.query_state().confidence > 0.5);
    assert!(low.query_state().confidence > 0.0);
    assert!(low.query_state().confidence < 0.5);
}

#[test]
fn load_or_new_reclassifies_behavioral_state_from_current_pad_and_confidence() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("daimon-state.json");

    let mut state = DaimonState::new();
    state.state.pad = PadVector::new(0.6, 0.0, 0.0);
    state.state.confidence = 0.9;
    state.state.behavioral_state = BehavioralState::Struggling;
    state.persist(&path).expect("persist daimon");

    let reloaded = DaimonState::load_or_new(&path);

    assert_eq!(
        reloaded.query_state().behavioral_state,
        BehavioralState::Coasting
    );
    assert_eq!(
        reloaded.query_state().behavioral_state,
        BehavioralState::classify(
            reloaded.query_state().pad,
            reloaded.query_state().confidence
        )
    );
}

#[test]
fn unknown_non_coding_strategy_labels_fall_back_to_index_roles() {
    let observation = TaskStrategyObservation {
        task_tier: "architectural".to_string(),
        file_count: 6,
        verification_count: 3,
        dependency_count: 4,
        max_loc: 320,
        familiarity: 0.2,
        confidence: 0.65,
        failure_pressure: 0.6,
        urgency_pressure: 1.0,
    };
    let baseline = StrategySpaceDefinition::coding()
        .computer()
        .task_coords(&observation);
    let custom = StrategySpaceDefinition {
        domain: "research".to_string(),
        dimensions: [
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
            "delta".to_string(),
            "epsilon".to_string(),
            "zeta".to_string(),
            "eta".to_string(),
            "theta".to_string(),
        ],
    }
    .validate()
    .expect("validate custom strategy space")
    .computer()
    .task_coords(&observation);

    assert_eq!(custom, baseline);
}
