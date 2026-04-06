//! In-memory [`Substrate`] — fast, ephemeral, test-friendly.
//!
//! `MemorySubstrate` holds signals in a `parking_lot::RwLock<HashMap>`.
//! It's the default substrate for tests and for ephemeral in-process state
//! (agent instance registry, ticker counters, ephemeral pheromones).
//!
//! For persistence across runs, use `FileSubstrate` (in `roko-fs`).
//! For shared on-chain state, use `ChainSubstrate` (in `roko-chain`).

use async_trait::async_trait;
use parking_lot::RwLock;
use roko_core::{error::Result, Context, ContentHash, Query, Signal, Substrate};
use std::collections::HashMap;

/// An in-memory, concurrent signal substrate.
///
/// Cloning is cheap — it shares the underlying storage (via Arc-like
/// semantics through the internal `parking_lot::RwLock`).
#[derive(Default)]
pub struct MemorySubstrate {
    store: RwLock<HashMap<ContentHash, Signal>>,
    #[allow(dead_code)]
    name: String,
}

impl MemorySubstrate {
    /// Construct an empty, unnamed substrate.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a name (appears in `name()`).
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            store: RwLock::new(HashMap::new()),
            name: name.into(),
        }
    }

    /// Synchronous put (bypass async — useful for test setup).
    pub fn put_sync(&self, signal: Signal) -> ContentHash {
        let id = signal.id;
        self.store.write().insert(id, signal);
        id
    }

    /// Synchronous get.
    #[must_use]
    pub fn get_sync(&self, id: &ContentHash) -> Option<Signal> {
        self.store.read().get(id).cloned()
    }

    /// Returns the current number of stored signals (sync).
    #[must_use]
    pub fn len_sync(&self) -> usize {
        self.store.read().len()
    }
}

#[async_trait]
impl Substrate for MemorySubstrate {
    async fn put(&self, signal: Signal) -> Result<ContentHash> {
        let id = signal.id;
        self.store.write().insert(id, signal);
        Ok(id)
    }

    async fn get(&self, id: &ContentHash) -> Result<Option<Signal>> {
        Ok(self.store.read().get(id).cloned())
    }

    #[allow(clippy::significant_drop_tightening)]
    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Signal>> {
        let store = self.store.read();
        let mut matching: Vec<Signal> = store
            .values()
            .filter(|s| matches_query(s, q, ctx))
            .cloned()
            .collect();
        drop(store);

        // Sort by effective weight descending (newest/highest-score first).
        matching.sort_by(|a, b| {
            b.weight_at(ctx.now_ms)
                .partial_cmp(&a.weight_at(ctx.now_ms))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if let Some(limit) = q.limit {
            matching.truncate(limit);
        }

        Ok(matching)
    }

    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize> {
        let mut store = self.store.write();
        let before = store.len();
        store.retain(|_, s| s.weight_at(ctx.now_ms) > threshold);
        Ok(before - store.len())
    }

    async fn len(&self) -> Result<usize> {
        Ok(self.store.read().len())
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}

/// Pure function: does `signal` satisfy `query` at time `ctx.now_ms`?
fn matches_query(signal: &Signal, q: &Query, ctx: &Context) -> bool {
    if let Some(kinds) = &q.kinds {
        if !kinds.contains(&signal.kind) {
            return false;
        }
    }
    if let Some(author) = &q.author {
        if &signal.provenance.author != author {
            return false;
        }
    }
    if let Some(session) = &q.session {
        if signal.provenance.session.as_ref() != Some(session) {
            return false;
        }
    }
    if let Some(since) = q.since_ms {
        if signal.created_at_ms < since {
            return false;
        }
    }
    if let Some(until) = q.until_ms {
        if signal.created_at_ms > until {
            return false;
        }
    }
    if let Some(min_w) = q.min_weight {
        if signal.weight_at(ctx.now_ms) < min_w {
            return false;
        }
    }
    for (k, v) in &q.tags {
        match signal.tags.get(k) {
            Some(value) if value == v => {}
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Decay, Kind, Provenance, Score};

    fn sig(kind: Kind, body: &str, t: i64) -> Signal {
        Signal::builder(kind)
            .body(Body::text(body))
            .created_at_ms(t)
            .build()
    }

    #[tokio::test]
    async fn put_get_roundtrip() {
        let sub = MemorySubstrate::new();
        let s = sig(Kind::Task, "a", 0);
        let id = s.id;
        sub.put(s.clone()).await.unwrap();
        let got = sub.get(&id).await.unwrap();
        assert_eq!(got, Some(s));
    }

    #[tokio::test]
    async fn idempotent_put() {
        let sub = MemorySubstrate::new();
        let s = sig(Kind::Task, "idem", 0);
        sub.put(s.clone()).await.unwrap();
        sub.put(s.clone()).await.unwrap();
        sub.put(s.clone()).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let sub = MemorySubstrate::new();
        let missing = ContentHash::of(b"nothing");
        assert!(sub.get(&missing).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn query_by_kind() {
        let sub = MemorySubstrate::new();
        sub.put(sig(Kind::Task, "task1", 0)).await.unwrap();
        sub.put(sig(Kind::Task, "task2", 0)).await.unwrap();
        sub.put(sig(Kind::Episode, "ep1", 0)).await.unwrap();

        let ctx = Context::at(0);
        let tasks = sub
            .query(&Query::of_kind(Kind::Task), &ctx)
            .await
            .unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().all(|s| s.kind == Kind::Task));
    }

    #[tokio::test]
    async fn query_by_time_window() {
        let sub = MemorySubstrate::new();
        sub.put(sig(Kind::Task, "old", 100)).await.unwrap();
        sub.put(sig(Kind::Task, "mid", 500)).await.unwrap();
        sub.put(sig(Kind::Task, "new", 1000)).await.unwrap();

        let ctx = Context::at(2000);
        let r = sub
            .query(&Query::all().since(400).until(900), &ctx)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].body.as_text().unwrap(), "mid");
    }

    #[tokio::test]
    async fn query_by_tag() {
        let sub = MemorySubstrate::new();
        sub.put(
            Signal::builder(Kind::Task)
                .body(Body::text("a"))
                .tag("env", "prod")
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();
        sub.put(
            Signal::builder(Kind::Task)
                .body(Body::text("b"))
                .tag("env", "dev")
                .created_at_ms(0)
                .build(),
        )
        .await
        .unwrap();

        let ctx = Context::at(0);
        let r = sub
            .query(&Query::all().with_tag("env", "prod"), &ctx)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].body.as_text().unwrap(), "a");
    }

    #[tokio::test]
    async fn query_limit() {
        let sub = MemorySubstrate::new();
        for i in 0..10 {
            sub.put(sig(Kind::Task, &format!("t{i}"), 0)).await.unwrap();
        }
        let ctx = Context::at(0);
        let r = sub.query(&Query::all().limit(3), &ctx).await.unwrap();
        assert_eq!(r.len(), 3);
    }

    #[tokio::test]
    async fn query_by_min_weight_applies_decay() {
        let sub = MemorySubstrate::new();
        // High score, half-life 1000ms
        sub.put(
            Signal::builder(Kind::Pheromone)
                .body(Body::text("fresh"))
                .created_at_ms(0)
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::HalfLife { half_life_ms: 1000 })
                .build(),
        )
        .await
        .unwrap();

        // At t=0, weight=1.0 — should match min_weight=0.5
        let ctx = Context::at(0);
        let r = sub
            .query(&Query::all().with_min_weight(0.5), &ctx)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);

        // At t=2000 (two half-lives), weight=0.25 — should NOT match min_weight=0.5
        let ctx = Context::at(2000);
        let r = sub
            .query(&Query::all().with_min_weight(0.5), &ctx)
            .await
            .unwrap();
        assert_eq!(r.len(), 0);
    }

    #[tokio::test]
    async fn prune_removes_decayed_signals() {
        let sub = MemorySubstrate::new();
        sub.put(
            Signal::builder(Kind::Pheromone)
                .body(Body::text("fresh"))
                .created_at_ms(0)
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::HalfLife { half_life_ms: 100 })
                .build(),
        )
        .await
        .unwrap();
        sub.put(
            Signal::builder(Kind::Task)
                .body(Body::text("permanent"))
                .created_at_ms(0)
                .score(Score::new(1.0, 0.0, 0.0, 1.0))
                .decay(Decay::None)
                .build(),
        )
        .await
        .unwrap();

        // At t=10000 (many half-lives), pheromone should be prunable.
        let ctx = Context::at(10_000);
        let pruned = sub.prune(0.01, &ctx).await.unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn query_sorts_by_weight_descending() {
        let sub = MemorySubstrate::new();
        sub.put(
            Signal::builder(Kind::Task)
                .body(Body::text("low"))
                .created_at_ms(0)
                .score(Score::new(0.2, 0.0, 0.0, 1.0))
                .build(),
        )
        .await
        .unwrap();
        sub.put(
            Signal::builder(Kind::Task)
                .body(Body::text("high"))
                .created_at_ms(0)
                .score(Score::new(0.9, 0.0, 0.0, 1.0))
                .build(),
        )
        .await
        .unwrap();

        let ctx = Context::at(0);
        let r = sub.query(&Query::all(), &ctx).await.unwrap();
        assert_eq!(r[0].body.as_text().unwrap(), "high");
        assert_eq!(r[1].body.as_text().unwrap(), "low");
    }

    #[tokio::test]
    async fn query_by_author() {
        let sub = MemorySubstrate::new();
        sub.put(
            Signal::builder(Kind::Task)
                .provenance(Provenance::agent("alice"))
                .created_at_ms(0)
                .body(Body::text("a"))
                .build(),
        )
        .await
        .unwrap();
        sub.put(
            Signal::builder(Kind::Task)
                .provenance(Provenance::agent("bob"))
                .created_at_ms(0)
                .body(Body::text("b"))
                .build(),
        )
        .await
        .unwrap();

        let ctx = Context::at(0);
        let r = sub
            .query(&Query::all().with_author("alice"), &ctx)
            .await
            .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].body.as_text().unwrap(), "a");
    }
}
