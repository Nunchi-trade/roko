//! `HdcSubstrate`: a `Substrate` impl backed by HDC similarity search.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::RwLock;
use roko_core::{Body, ContentHash, Context, Engram, Query, error::Result, traits::Substrate};
use roko_primitives::HdcVector;

use crate::chain::{
    HdcIndex, InsightId,
    projection::{project_bytes, project_tokens},
};

/// Roko-compatible [`Substrate`] backed by mirage's [`HdcIndex`].
///
/// Stores raw `Engram`s keyed by their content hash, alongside an HDC vector
/// projected from the signal's body. `query()` supports text-based semantic
/// retrieval via the `text_query` tag key (projects the value into HDC,
/// returns top-K by similarity).
///
/// This substrate does NOT apply the knowledge-layer lifecycle (no confirms,
/// challenges, or decay). For that, use [`super::ChainSubstrate`].
pub struct HdcSubstrate {
    state: RwLock<HdcSubstrateState>,
    name: String,
}

struct HdcSubstrateState {
    signals: HashMap<ContentHash, Engram>,
    /// Maps Engram content hash → HDC index entry id.
    routing: HashMap<ContentHash, InsightId>,
    /// Reverse mapping (used when results come back from the index).
    reverse: HashMap<InsightId, ContentHash>,
    index: HdcIndex,
}

impl std::fmt::Debug for HdcSubstrate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.state.read();
        f.debug_struct("HdcSubstrate")
            .field("name", &self.name)
            .field("signals", &st.signals.len())
            .finish()
    }
}

impl HdcSubstrate {
    /// Constructs an empty HDC substrate with the given debug name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            state: RwLock::new(HdcSubstrateState {
                signals: HashMap::new(),
                routing: HashMap::new(),
                reverse: HashMap::new(),
                index: HdcIndex::new(),
            }),
            name: name.into(),
        }
    }

    /// Projects a signal body into an HDC vector.
    #[must_use]
    fn project_signal(signal: &Engram) -> HdcVector {
        match &signal.body {
            Body::Text(s) => project_tokens(s),
            Body::Json(v) => project_tokens(&v.to_string()),
            Body::Bytes(bytes) => project_bytes(bytes),
            Body::Empty => project_bytes(signal.kind.as_str().as_bytes()),
        }
    }

    /// Derives a stable InsightId from a ContentHash (takes the first 16 bytes).
    #[must_use]
    fn id_from_hash(hash: &ContentHash) -> InsightId {
        let mut out = [0u8; 16];
        out.copy_from_slice(&hash.0[..16]);
        InsightId(out)
    }
}

#[async_trait]
impl Substrate for HdcSubstrate {
    async fn put(&self, signal: Engram) -> Result<ContentHash> {
        let hash = signal.content_hash();
        let vector = Self::project_signal(&signal);
        let weight = signal.score.effective().max(0.01);
        let insight_id = Self::id_from_hash(&hash);

        let mut st = self.state.write();
        st.index.insert(insight_id, vector, weight);
        st.routing.insert(hash, insight_id);
        st.reverse.insert(insight_id, hash);
        st.signals.insert(hash, signal);
        Ok(hash)
    }

    async fn get(&self, id: &ContentHash) -> Result<Option<Engram>> {
        let st = self.state.read();
        Ok(st.signals.get(id).cloned())
    }

    async fn query(&self, q: &Query, ctx: &Context) -> Result<Vec<Engram>> {
        let st = self.state.read();

        // Semantic search path: look for a `text_query` tag and use it as the
        // HDC similarity query.
        let text_query = q
            .tags
            .iter()
            .find(|(k, _)| k == "text_query")
            .map(|(_, v)| v.as_str());

        let mut results: Vec<Engram> = if let Some(text) = text_query {
            let vec = project_tokens(text);
            let k_cap = q.limit.unwrap_or(50).min(st.signals.len()).max(1);
            st.index
                .top_k(&vec, k_cap)
                .into_iter()
                .filter_map(|hit| st.reverse.get(&hit.id).copied())
                .filter_map(|h| st.signals.get(&h).cloned())
                .collect()
        } else {
            st.signals.values().cloned().collect()
        };

        // Apply standard filters.
        if let Some(kinds) = &q.kinds {
            results.retain(|s| kinds.contains(&s.kind));
        }
        if let Some(author) = &q.author {
            results.retain(|s| s.provenance.author == *author);
        }
        if let Some(session) = &q.session {
            results.retain(|s| s.provenance.session.as_deref() == Some(session.as_str()));
        }
        if let Some(since) = q.since_ms {
            results.retain(|s| s.created_at_ms >= since);
        }
        if let Some(until) = q.until_ms {
            results.retain(|s| s.created_at_ms <= until);
        }
        for (key, value) in &q.tags {
            if key == "text_query" {
                continue;
            }
            results.retain(|s| s.tag(key).map(|t| t == value.as_str()).unwrap_or(false));
        }
        if let Some(min_weight) = q.min_weight {
            let now = ctx.now_ms;
            results.retain(|s| s.weight_at(now) >= min_weight);
        }
        if let Some(limit) = q.limit {
            results.truncate(limit);
        }
        Ok(results)
    }

    async fn prune(&self, threshold: f32, ctx: &Context) -> Result<usize> {
        let mut st = self.state.write();
        let now = ctx.now_ms;
        let victims: Vec<ContentHash> = st
            .signals
            .iter()
            .filter(|(_, s)| s.weight_at(now) < threshold)
            .map(|(h, _)| *h)
            .collect();
        let count = victims.len();
        for hash in victims {
            if let Some(id) = st.routing.remove(&hash) {
                st.index.remove(id);
                st.reverse.remove(&id);
            }
            st.signals.remove(&hash);
        }
        Ok(count)
    }

    async fn len(&self) -> Result<usize> {
        Ok(self.state.read().signals.len())
    }

    async fn is_empty(&self) -> Result<bool> {
        Ok(self.state.read().signals.is_empty())
    }

    fn name(&self) -> &'static str {
        // SAFETY: the name is always a static string literal or leaked.
        // This satisfies the roko-core Substrate trait signature.
        "hdc-substrate"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Context, Engram, Kind, Provenance, Score};

    fn signal(kind: Kind, text: &str, author: &str) -> Engram {
        Engram::builder(kind)
            .body(Body::text(text))
            .provenance(Provenance::agent(author))
            .score(Score::new(0.8, 0.5, 1.0, 1.0))
            .build()
    }

    #[tokio::test]
    async fn put_and_get_roundtrip() {
        let sub = HdcSubstrate::new("hdc-test");
        let s = signal(Kind::Insight, "eip-1967 slot layout", "alice");
        let expected_hash = s.content_hash();
        let hash = sub.put(s.clone()).await.unwrap();
        assert_eq!(hash, expected_hash);
        let got = sub.get(&hash).await.unwrap().unwrap();
        assert_eq!(got, s);
    }

    #[tokio::test]
    async fn query_semantic_returns_closest_match() {
        let sub = HdcSubstrate::new("hdc-test");
        for text in [
            "uniswap v3 STF revert insufficient allowance",
            "deploy eip-1967 upgradable proxy on optimism",
            "set arbitrum gas limit to 3x estimate",
            "permit2 batch approval signing flow",
        ] {
            sub.put(signal(Kind::Insight, text, "alice")).await.unwrap();
        }
        let q = Query::all()
            .with_tag("text_query", "uniswap v3 STF revert insufficient allowance")
            .limit(2);
        let ctx = Context::now();
        let results = sub.query(&q, &ctx).await.unwrap();
        assert!(!results.is_empty());
        assert_eq!(
            results[0].body.as_text().unwrap(),
            "uniswap v3 STF revert insufficient allowance"
        );
    }

    #[tokio::test]
    async fn query_filters_by_kind_and_author() {
        let sub = HdcSubstrate::new("hdc-test");
        sub.put(signal(Kind::Insight, "a", "alice")).await.unwrap();
        sub.put(signal(Kind::Insight, "b", "bob")).await.unwrap();
        sub.put(signal(Kind::Skill, "c", "alice")).await.unwrap();
        let ctx = Context::now();
        let q = Query::of_kind(Kind::Insight).with_author("alice");
        let results = sub.query(&q, &ctx).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].body.as_text().unwrap(), "a");
    }

    #[tokio::test]
    async fn prune_removes_low_weight_signals() {
        let sub = HdcSubstrate::new("hdc-test");
        let s = Engram::builder(Kind::Insight)
            .body(Body::text("decays fast"))
            .provenance(Provenance::agent("alice"))
            .score(Score::new(0.01, 0.0, 0.0, 0.01))
            .build();
        sub.put(s).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 1);
        let ctx = Context::now();
        let removed = sub.prune(0.1, &ctx).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(sub.len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn put_is_idempotent_on_content() {
        let sub = HdcSubstrate::new("hdc-test");
        let s = signal(Kind::Insight, "dedupe", "alice");
        let h1 = sub.put(s.clone()).await.unwrap();
        let h2 = sub.put(s).await.unwrap();
        assert_eq!(h1, h2);
        assert_eq!(sub.len().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn len_and_is_empty_reflect_state() {
        let sub = HdcSubstrate::new("hdc-test");
        assert!(sub.is_empty().await.unwrap());
        sub.put(signal(Kind::Insight, "hi", "alice")).await.unwrap();
        assert!(!sub.is_empty().await.unwrap());
        assert_eq!(sub.len().await.unwrap(), 1);
    }
}
