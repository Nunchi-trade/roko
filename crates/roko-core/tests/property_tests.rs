//! Property-based tests for roko-core kernel types.

use proptest::prelude::*;
use roko_core::{Body, Datum, Engram, Kind, Pulse, Score, Topic, TopicFilter};

// ─── Arbitrary strategies ───────────────────────────────────────────────────

fn arb_kind() -> impl Strategy<Value = Kind> {
    prop_oneof![
        Just(Kind::Task),
        Just(Kind::Episode),
        Just(Kind::GateVerdict),
        Just(Kind::Metric),
        Just(Kind::Prompt),
        Just(Kind::Pheromone),
        Just(Kind::ProcessSpawn),
        "\\PC{1,32}".prop_map(Kind::Custom),
    ]
}

fn arb_body() -> impl Strategy<Value = Body> {
    prop_oneof![
        Just(Body::Empty),
        "\\PC{0,128}".prop_map(Body::text),
        any::<Vec<u8>>()
            .prop_map(|bytes| Body::bytes(bytes.into_iter().take(64).collect::<Vec<_>>())),
    ]
}

fn arb_topic() -> impl Strategy<Value = Topic> {
    "[a-z]{1,8}(\\.[a-z]{1,8}){0,3}".prop_map(Topic::new)
}

fn arb_pulse() -> impl Strategy<Value = Pulse> {
    (any::<u64>(), arb_topic(), arb_kind(), arb_body()).prop_map(|(seq, topic, kind, body)| {
        Pulse::builder(seq, topic, kind)
            .body(body)
            .created_at_ms(1_000_000)
            .build()
    })
}

fn arb_score() -> impl Strategy<Value = Score> {
    (
        0.0f32..=1.0,
        0.0f32..=1.0,
        0.0f32..=100.0,
        0.0f32..=100.0,
    )
        .prop_map(|(c, n, u, r)| Score::new(c, n, u, r))
}

fn arb_engram() -> impl Strategy<Value = Engram> {
    (arb_kind(), arb_body(), arb_score()).prop_map(|(kind, body, score)| {
        Engram::builder(kind)
            .body(body)
            .score(score)
            .created_at_ms(1_000_000)
            .build()
    })
}

// ─── Pulse tests ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn pulse_serde_roundtrip(pulse in arb_pulse()) {
        let json = serde_json::to_string(&pulse).expect("serialize");
        let parsed: Pulse = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(pulse, parsed);
    }
}

// ─── Topic tests ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn topic_starts_with_empty_is_always_true(topic in arb_topic()) {
        prop_assert!(topic.starts_with(""));
    }

    #[test]
    fn topic_starts_with_self(topic in arb_topic()) {
        prop_assert!(topic.starts_with(&topic.0));
    }

    #[test]
    fn topic_serde_roundtrip(topic in arb_topic()) {
        let json = serde_json::to_string(&topic).expect("serialize");
        let parsed: Topic = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(topic, parsed);
    }
}

// ─── TopicFilter tests ─────────────────────────────────────────────────────

proptest! {
    #[test]
    fn filter_all_matches_everything(topic in arb_topic()) {
        prop_assert!(TopicFilter::All.matches(&topic));
    }

    #[test]
    fn filter_exact_matches_same_topic(s in "[a-z]{1,16}") {
        let topic = Topic::new(&s);
        let filter = TopicFilter::Exact(topic.clone());
        prop_assert!(filter.matches(&topic));
    }
}

// ─── Datum accessor tests ───────────────────────────────────────────────────

proptest! {
    #[test]
    fn datum_engram_kind_matches(engram in arb_engram()) {
        let datum = Datum::Engram(&engram);
        prop_assert_eq!(datum.kind(), &engram.kind);
    }

    #[test]
    fn datum_engram_body_matches(engram in arb_engram()) {
        let datum = Datum::Engram(&engram);
        prop_assert_eq!(datum.body(), &engram.body);
    }

    #[test]
    fn datum_pulse_kind_matches(pulse in arb_pulse()) {
        let datum = Datum::Pulse(&pulse);
        prop_assert_eq!(datum.kind(), &pulse.kind);
    }
}

// ─── Score tests ────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn score_effective_is_finite(score in arb_score()) {
        let eff = score.effective();
        prop_assert!(eff.is_finite());
        prop_assert!(eff >= 0.0);
    }

    #[test]
    fn score_zero_confidence_means_zero_effective(
        n in 0.0f32..=1.0,
        u in 0.0f32..=100.0,
        r in 0.0f32..=100.0,
    ) {
        let score = Score::new(0.0, n, u, r);
        prop_assert!((score.effective() - 0.0).abs() < 1e-6);
    }

    #[test]
    fn score_ordering_by_effective(
        a in arb_score(),
        b in arb_score(),
    ) {
        let ea = a.effective();
        let eb = b.effective();
        if ea > eb {
            prop_assert!(a.exceeds(eb - 0.001));
        }
        if eb > ea {
            prop_assert!(b.exceeds(ea - 0.001));
        }
    }

    #[test]
    fn score_serde_roundtrip(score in arb_score()) {
        let json = serde_json::to_string(&score).expect("serialize");
        let parsed: Score = serde_json::from_str(&json).expect("deserialize");
        prop_assert!((score.confidence - parsed.confidence).abs() < 1e-6);
        prop_assert!((score.novelty - parsed.novelty).abs() < 1e-6);
        prop_assert!((score.utility - parsed.utility).abs() < 1e-6);
        prop_assert!((score.reputation - parsed.reputation).abs() < 1e-6);
    }
}

// ─── Engram tests ───────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn engram_content_hash_deterministic(kind in arb_kind(), body in arb_body()) {
        let a = Engram::builder(kind.clone())
            .body(body.clone())
            .created_at_ms(1_000)
            .build();
        let b = Engram::builder(kind)
            .body(body)
            .created_at_ms(1_000)
            .build();
        prop_assert_eq!(a.id, b.id);
    }

    #[test]
    fn engram_serde_roundtrip(engram in arb_engram()) {
        let json = serde_json::to_string(&engram).expect("serialize");
        let parsed: Engram = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(engram, parsed);
    }
}
