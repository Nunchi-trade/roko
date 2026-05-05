//! Integration test: the universal loop wired end-to-end with real impls.
//!
//! This proves the architecture works: seed a substrate with signals, run
//! `loop_tick`, and observe that signals flow through all six verbs and land
//! back in the substrate with provenance tracked.

use async_trait::async_trait;
use roko_core::{
    Body, Budget, Compose, Context, Decay, Engram, Kind, Provenance, Query, React, Result, Score,
    Store, Verdict, Verify, loop_tick,
};
use roko_std::{FirstRouter, MemorySubstrate, NoOpPolicy};
use std::sync::Arc;

/// A custom scorer: favors signals tagged `priority=high`.
struct PriorityScorer;
impl roko_core::traits::Score for PriorityScorer {
    fn score(&self, s: &Engram, _ctx: &Context) -> Score {
        let confidence = if s.tag("priority") == Some("high") {
            0.9
        } else {
            0.3
        };
        Score::new(confidence, 0.0, 0.0, 1.0)
    }
    fn name(&self) -> &'static str {
        "priority_scorer"
    }
}

/// A custom gate: passes if the signal body is not empty.
struct NonEmptyGate;
#[async_trait]
impl Verify for NonEmptyGate {
    async fn verify(&self, s: &Engram, _ctx: &Context) -> Verdict {
        if s.body.byte_size() > 0 {
            Verdict::pass(self.name())
        } else {
            Verdict::fail(self.name(), "body is empty")
        }
    }
    fn name(&self) -> &str {
        "non_empty"
    }
}

/// A composer that wraps the input in a "processed" kind with lineage.
struct WrapComposer;
impl Compose for WrapComposer {
    fn compose(
        &self,
        signals: &[Engram],
        _budget: &Budget,
        _scorer: &dyn roko_core::traits::Score,
        _ctx: &Context,
    ) -> Result<Engram> {
        let input = signals.first().expect("at least one input");
        Ok(input
            .derive(
                Kind::Custom("processed".into()),
                Body::text(format!("processed: {}", input.body.as_text().unwrap_or(""))),
            )
            .provenance(Provenance::trusted("wrap_composer"))
            .build())
    }
    fn name(&self) -> &str {
        "wrap_composer"
    }
}

/// A policy that emits a logging episode every time a signal passes through.
struct EpisodeLoggerPolicy;
impl React for EpisodeLoggerPolicy {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        stream
            .iter()
            .map(|s| {
                Engram::builder(Kind::Episode)
                    .body(Body::text(format!("logged: {}", s.id.short())))
                    .provenance(Provenance::agent("episode_logger"))
                    .lineage([s.id])
                    .decay(Decay::HalfLife {
                        half_life_ms: 86_400_000,
                    }) // 24h
                    .build()
            })
            .collect()
    }
    fn name(&self) -> &str {
        "episode_logger"
    }
}

#[tokio::test]
async fn universal_loop_processes_a_signal_end_to_end() {
    let substrate: Arc<dyn Store> = Arc::new(MemorySubstrate::named("test"));
    let scorer = PriorityScorer;
    let router = FirstRouter;
    let composer = WrapComposer;
    let gate = NonEmptyGate;
    let policy = EpisodeLoggerPolicy;

    // Seed the substrate with two tasks.
    let task1 = Engram::builder(Kind::Task)
        .body(Body::text("task 1 content"))
        .tag("priority", "high")
        .created_at_ms(1000)
        .build();
    let task2 = Engram::builder(Kind::Task)
        .body(Body::text("task 2 content"))
        .tag("priority", "low")
        .created_at_ms(1100)
        .build();
    substrate.put(task1.clone()).await.unwrap();
    substrate.put(task2.clone()).await.unwrap();

    let ctx = Context::at(2000);
    let query = Query::of_kind(Kind::Task);
    let budget = Budget::unlimited();

    // Run one tick of the universal loop.
    let outcome = loop_tick(
        substrate.as_ref(),
        &scorer,
        &router,
        &composer,
        &gate,
        &policy,
        &query,
        &budget,
        &ctx,
    )
    .await
    .expect("loop_tick succeeded");

    // Verify the shape of the outcome.
    assert_eq!(outcome.candidates_examined, 2, "2 tasks seeded");
    assert!(outcome.passed(), "NonEmptyGate should pass");
    assert!(outcome.did_work());

    let composed = outcome.composed.unwrap();
    assert_eq!(composed.kind, Kind::Custom("processed".into()));
    assert!(composed.body.as_text().unwrap().starts_with("processed:"));
    assert_eq!(composed.lineage.len(), 1, "composed tracks source lineage");

    // Episode was emitted by the policy.
    assert_eq!(outcome.emitted.len(), 1);
    assert_eq!(outcome.emitted[0].kind, Kind::Episode);
    assert_eq!(
        outcome.emitted[0].lineage[0], composed.id,
        "episode references composed signal"
    );

    // Written = composed + episode = 2 new signals.
    assert_eq!(outcome.written.len(), 2);

    // Store now contains original tasks + composed + episode = 4 signals.
    assert_eq!(substrate.len().await.unwrap(), 4);
}

#[tokio::test]
async fn loop_tick_does_nothing_when_query_matches_nothing() {
    let substrate: Arc<dyn Store> = Arc::new(MemorySubstrate::new());
    let scorer = PriorityScorer;
    let router = FirstRouter;
    let composer = WrapComposer;
    let gate = NonEmptyGate;
    let policy = NoOpPolicy;

    let ctx = Context::at(0);
    // Query for tasks, but substrate is empty.
    let query = Query::of_kind(Kind::Task);
    let budget = Budget::unlimited();

    let outcome = loop_tick(
        substrate.as_ref(),
        &scorer,
        &router,
        &composer,
        &gate,
        &policy,
        &query,
        &budget,
        &ctx,
    )
    .await
    .unwrap();

    assert_eq!(outcome.candidates_examined, 0);
    assert!(outcome.composed.is_none());
    assert!(outcome.verdict.is_none());
    assert!(outcome.written.is_empty());
    assert!(!outcome.did_work());
}

#[tokio::test]
async fn failing_gate_prevents_writeback() {
    let substrate: Arc<dyn Store> = Arc::new(MemorySubstrate::new());
    let scorer = PriorityScorer;
    let router = FirstRouter;
    // Compose that produces an empty-body signal — will fail NonEmptyGate.
    struct EmptyComposer;
    impl Compose for EmptyComposer {
        fn compose(
            &self,
            _s: &[Engram],
            _b: &Budget,
            _sc: &dyn roko_core::traits::Score,
            _c: &Context,
        ) -> Result<Engram> {
            Ok(Engram::builder(Kind::Custom("empty".into()))
                .body(Body::empty())
                .build())
        }
        fn name(&self) -> &str {
            "empty"
        }
    }
    let composer = EmptyComposer;
    let gate = NonEmptyGate;
    let policy = EpisodeLoggerPolicy;

    // Seed with one task.
    substrate
        .put(
            Engram::builder(Kind::Task)
                .body(Body::text("source"))
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();

    let outcome = loop_tick(
        substrate.as_ref(),
        &scorer,
        &router,
        &composer,
        &gate,
        &policy,
        &Query::of_kind(Kind::Task),
        &Budget::unlimited(),
        &Context::at(0),
    )
    .await
    .unwrap();

    assert!(!outcome.passed(), "NonEmptyGate should fail on empty body");
    assert!(outcome.written.is_empty(), "failed gate prevents writeback");
    assert!(outcome.emitted.is_empty(), "policy didn't fire");
    // Store still has just the original task.
    assert_eq!(substrate.len().await.unwrap(), 1);
}

#[tokio::test]
async fn decayed_signals_prune_away() {
    let substrate = MemorySubstrate::new();

    // Add a pheromone with 1s half-life.
    substrate
        .put(
            Engram::builder(Kind::Pheromone)
                .body(Body::text("transient"))
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::HalfLife { half_life_ms: 1000 })
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();

    // At t=0, weight=1.0, still present.
    assert_eq!(substrate.len().await.unwrap(), 1);

    // Prune at t=20s — should be essentially zero after 20 half-lives.
    let ctx = Context::at(20_000);
    let pruned = substrate.prune(0.01, &ctx).await.unwrap();
    assert_eq!(pruned, 1);
    assert_eq!(substrate.len().await.unwrap(), 0);
}

#[tokio::test]
async fn content_hash_deduplicates() {
    let substrate = MemorySubstrate::new();
    // Two identical signals should collapse to one.
    let make = || {
        Engram::builder(Kind::Task)
            .body(Body::text("identical"))
            .created_at_ms(12_345)
            .build()
    };
    let a = make();
    let b = make();
    assert_eq!(a.id, b.id, "identical content → identical hash");

    substrate.put(a).await.unwrap();
    substrate.put(b).await.unwrap();
    assert_eq!(substrate.len().await.unwrap(), 1);
}
