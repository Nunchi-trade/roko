//! `ChainSubstrate`: a `Substrate` impl wiring roko Signals onto mirage's chain
//! knowledge layer (InsightEntry with lifecycle + decay + confirmations).

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::RwLock;
use roko_core::{Body, ContentHash, Context, Engram, Query, error::Result, traits::Substrate};

use crate::{
    chain::{
        HnswConfig, InsightId, KnowledgeStore, PostOutcome,
        projection::{project_bytes, project_tokens},
    },
    roko_bridge::map_kind,
};
use roko_primitives::HdcVector;

/// Configuration for a [`ChainSubstrate`].
#[derive(Clone, Copy, Debug)]
pub struct ChainSubstrateConfig {
    /// HNSW switch-over threshold (entries above this use HNSW for search).
    pub hnsw_threshold: usize,
    /// Default stake applied to posted insights (wei).
    pub default_stake_wei: u128,
}

impl Default for ChainSubstrateConfig {
    fn default() -> Self {
        Self {
            hnsw_threshold: 100_000,
            default_stake_wei: 2_000_000_000_000_000, // 0.002 ETH
        }
    }
}

/// Roko-compatible [`Substrate`] backed by a mirage [`KnowledgeStore`].
///
/// Signals stored here are promoted to `InsightEntry`s with full lifecycle:
/// they can be confirmed by other agents (weight boost), challenged
/// (state transition), and decayed by time. The knowledge layer applies
/// HDC-based duplicate collapsing on near-identical content (>95% similarity).
///
/// Reads via `get()` return the original `Engram` byte-for-byte.
pub struct ChainSubstrate {
    state: RwLock<ChainSubstrateState>,
    name: String,
    config: ChainSubstrateConfig,
}

struct ChainSubstrateState {
    signals: HashMap<ContentHash, Engram>,
    /// ContentHash → InsightId routing (for decay/confirm/challenge APIs).
    routing: HashMap<ContentHash, InsightId>,
    reverse: HashMap<InsightId, ContentHash>,
    knowledge: KnowledgeStore,
}

impl std::fmt::Debug for ChainSubstrate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.state.read();
        f.debug_struct("ChainSubstrate")
            .field("name", &self.name)
            .field("signals", &st.signals.len())
            .field("insights", &st.knowledge.len())
            .field("config", &self.config)
            .finish()
    }
}

impl ChainSubstrate {
    /// Constructs a new chain substrate with the default config.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_config(name, ChainSubstrateConfig::default())
    }

    /// Constructs a new chain substrate with custom config.
    #[must_use]
    pub fn with_config(name: impl Into<String>, config: ChainSubstrateConfig) -> Self {
        let knowledge = KnowledgeStore::with_hnsw(HnswConfig::default(), config.hnsw_threshold);
        Self {
            state: RwLock::new(ChainSubstrateState {
                signals: HashMap::new(),
                routing: HashMap::new(),
                reverse: HashMap::new(),
                knowledge,
            }),
            name: name.into(),
            config,
        }
    }

    fn project_signal(signal: &Engram) -> HdcVector {
        match &signal.body {
            Body::Text(s) => project_tokens(s),
            Body::Json(v) => project_tokens(&v.to_string()),
            Body::Bytes(bytes) => project_bytes(bytes),
            Body::Empty => project_bytes(signal.kind.as_str().as_bytes()),
        }
    }

    fn signal_text(signal: &Engram) -> String {
        match &signal.body {
            Body::Text(s) => s.clone(),
            Body::Json(v) => v.to_string(),
            Body::Bytes(_) => signal.kind.as_str().to_owned(),
            Body::Empty => signal.kind.as_str().to_owned(),
        }
    }

    /// Records a confirmation on the insight backing `signal_hash`. No-op if the signal
    /// was never posted here or was already pruned.
    pub fn confirm(&self, signal_hash: ContentHash, confirmer: impl Into<Vec<u8>>) -> bool {
        let mut st = self.state.write();
        let Some(&insight_id) = st.routing.get(&signal_hash) else {
            return false;
        };
        st.knowledge.confirm(insight_id, confirmer.into()).is_ok()
    }

    /// Records a challenge on the insight backing `signal_hash`.
    pub fn challenge(&self, signal_hash: ContentHash, challenger: impl Into<Vec<u8>>) -> bool {
        let mut st = self.state.write();
        let Some(&insight_id) = st.routing.get(&signal_hash) else {
            return false;
        };
        st.knowledge
            .challenge(insight_id, challenger.into())
            .is_ok()
    }

    /// Applies knowledge-layer decay using `ctx.now_ms`.
    pub fn apply_decay(&self, now_secs: u64) {
        let mut st = self.state.write();
        st.knowledge.apply_decay(now_secs);
    }
}

#[async_trait]
impl Substrate for ChainSubstrate {
    async fn put(&self, signal: Engram) -> Result<ContentHash> {
        let hash = signal.content_hash();
        let text = Self::signal_text(&signal);
        let vector = Self::project_signal(&signal);
        let kind = map_kind(&signal.kind);
        let author = signal.provenance.author.clone().into_bytes();
        let now_secs = (signal.created_at_ms / 1000).max(0) as u64;

        let mut st = self.state.write();
        if st.signals.contains_key(&hash) {
            return Ok(hash);
        }
        let outcome = st.knowledge.post(
            author,
            kind,
            text,
            vector,
            Vec::new(),
            now_secs,
            self.config.default_stake_wei,
        );
        let insight_id = match outcome {
            PostOutcome::Accepted { id }
            | PostOutcome::Duplicate {
                existing_id: id, ..
            }
            | PostOutcome::ExactMatch { id } => id,
        };
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

        let text_query = q
            .tags
            .iter()
            .find(|(k, _)| k == "text_query")
            .map(|(_, v)| v.as_str());

        let mut results: Vec<Engram> = if let Some(text) = text_query {
            let vec = project_tokens(text);
            let k_cap = q.limit.unwrap_or(50).min(st.signals.len().max(1));
            st.knowledge
                .search(&vec, k_cap)
                .into_iter()
                .filter_map(|hit| st.reverse.get(&hit.id).copied())
                .filter_map(|h| st.signals.get(&h).cloned())
                .collect()
        } else {
            st.signals.values().cloned().collect()
        };

        // Standard filters.
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
        let now_ms = ctx.now_ms;

        // Apply knowledge-layer decay first to update state machine.
        let now_secs = (now_ms / 1000).max(0) as u64;
        st.knowledge.apply_decay(now_secs);

        // Walk signals and drop any whose Engram.weight_at(now) is below threshold
        // OR whose backing insight was pruned/stale.
        let victims: Vec<ContentHash> = st
            .signals
            .iter()
            .filter(|(h, s)| {
                if s.weight_at(now_ms) < threshold {
                    return true;
                }
                if let Some(id) = st.routing.get(h) {
                    if let Some(entry) = st.knowledge.get(*id) {
                        matches!(
                            entry.state,
                            crate::chain::KnowledgeState::Pruned
                                | crate::chain::KnowledgeState::Stale
                        )
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .map(|(h, _)| *h)
            .collect();
        let count = victims.len();
        for hash in victims {
            if let Some(id) = st.routing.remove(&hash) {
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
        "chain-substrate"
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
        let sub = ChainSubstrate::new("chain-test");
        let s = signal(Kind::Insight, "permit2 batch approval dance", "alice");
        let hash = sub.put(s.clone()).await.unwrap();
        let got = sub.get(&hash).await.unwrap().unwrap();
        assert_eq!(got, s);
    }

    #[tokio::test]
    async fn semantic_query_returns_match() {
        let sub = ChainSubstrate::new("chain-test");
        for text in [
            "uniswap v3 STF revert insufficient allowance",
            "deploy eip-1967 upgradable proxy on optimism",
            "set arbitrum gas limit to 3x estimate",
            "permit2 batch approval signing flow",
        ] {
            sub.put(signal(Kind::Insight, text, "alice")).await.unwrap();
        }
        let q = Query::all()
            .with_tag("text_query", "deploy eip-1967 upgradable proxy on optimism")
            .limit(1);
        let ctx = Context::now();
        let results = sub.query(&q, &ctx).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].body.as_text().unwrap(),
            "deploy eip-1967 upgradable proxy on optimism"
        );
    }

    #[tokio::test]
    async fn confirm_records_confirmation_on_backing_insight() {
        let sub = ChainSubstrate::new("chain-test");
        let s = signal(Kind::Insight, "gas buffer idea", "alice");
        let hash = sub.put(s).await.unwrap();
        assert!(sub.confirm(hash, "bob".as_bytes().to_vec()));
        // Second confirm from same agent is a no-op.
        assert!(!sub.confirm(hash, "bob".as_bytes().to_vec()));
        assert!(sub.confirm(hash, "carol".as_bytes().to_vec()));
    }

    #[tokio::test]
    async fn challenge_records_challenge() {
        let sub = ChainSubstrate::new("chain-test");
        let s = signal(
            Kind::Custom("com.example.anti_insight".into()),
            "WRONG: needs approve(0)",
            "alice",
        );
        let hash = sub.put(s).await.unwrap();
        assert!(sub.challenge(hash, b"bob".to_vec()));
    }

    #[tokio::test]
    async fn prune_drops_pruned_insights() {
        let sub = ChainSubstrate::new("chain-test");
        // Warnings have 180s half-life — 20 half-lives drives them below 0.01× initial.
        let s = Engram::builder(Kind::CompileDiagnostic)
            .body(Body::text("temp oracle drift"))
            .provenance(Provenance::agent("alice"))
            .score(Score::NEUTRAL)
            .created_at_ms(0)
            .build();
        let hash = sub.put(s).await.unwrap();
        assert_eq!(sub.len().await.unwrap(), 1);
        // Move knowledge layer forward 20 half-lives.
        sub.apply_decay(180 * 20);
        // Query knowledge directly to check state.
        let ctx = Context::at(180_000 * 20);
        let removed = sub.prune(f32::MIN, &ctx).await.unwrap();
        assert!(
            removed >= 1,
            "expected stale/pruned backing insight to prune signal, removed={removed}"
        );
        assert!(sub.get(&hash).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn kind_mapping_applies_correctly() {
        let sub = ChainSubstrate::new("chain-test");
        let skill = signal(Kind::Skill, "use anvil_mine to skip blocks", "alice");
        let h = sub.put(skill).await.unwrap();
        // The backing insight should be a Heuristic.
        let st = sub.state.read();
        let insight_id = st.routing.get(&h).unwrap();
        let entry = st.knowledge.get(*insight_id).unwrap();
        assert_eq!(entry.kind, crate::chain::KnowledgeKind::Heuristic);
    }
}
