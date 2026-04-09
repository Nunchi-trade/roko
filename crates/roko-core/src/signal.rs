//! The universal `Signal` type.
//!
//! A [`Signal`] is every event, every piece of data, every agent output, every
//! gate verdict in the Roko system. Signals are:
//!
//! - **Addressable** — content-hashed via BLAKE3
//! - **Decaying** — every signal has a decay function; weight fades over time
//! - **Scored** — multi-dimensional confidence/novelty/utility/reputation
//! - **Traced** — lineage tracks which signals this derived from
//! - **Composable** — signals combine into new signals via [`Composer`]s

use crate::{Body, ContentHash, Decay, Kind, Provenance, Score};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The universal datum of the Roko system.
///
/// See [crate-level docs](crate) for the architectural role of Signal.
///
/// # Identity
///
/// A signal's identity is its [`ContentHash`], computed from its kind, body,
/// author, and tags (see [`Signal::content_hash`]). Score and decay are
/// **excluded** from the hash — they can change without changing identity.
///
/// # Construction
///
/// Use [`Signal::builder`] for ergonomic construction:
///
/// ```
/// use roko_core::{Signal, Kind, Body};
///
/// let s = Signal::builder(Kind::Task)
///     .body(Body::text("implement login"))
///     .tag("priority", "high")
///     .build();
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// Content-addressed identity (computed from kind + body + author + tags).
    pub id: ContentHash,
    /// What kind of signal this is.
    pub kind: Kind,
    /// The signal's payload.
    pub body: Body,
    /// Unix milliseconds when this signal was first emitted.
    pub created_at_ms: i64,
    /// How this signal's weight decays over time.
    pub decay: Decay,
    /// Producer attribution and trust.
    pub provenance: Provenance,
    /// Quality score at emission time (may be recomputed by scorers).
    pub score: Score,
    /// `ContentHash`es of signals this derived from (forms a DAG for auditing
    /// and autocatalytic metrics).
    pub lineage: Vec<ContentHash>,
    /// Arbitrary string metadata (ordered for stable hashing).
    pub tags: BTreeMap<String, String>,
}

impl Signal {
    /// Begin building a signal.
    #[must_use]
    pub fn builder(kind: Kind) -> SignalBuilder {
        SignalBuilder::new(kind)
    }

    /// Compute the content hash of this signal's identity fields.
    ///
    /// The hash covers: kind, body, author, taint, lineage, and tags.
    /// It does NOT cover: score, decay, timestamp — these can change over
    /// time without changing what the signal fundamentally is.
    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.kind.as_str().as_bytes());
        hasher.update(b"|");
        hasher.update(&self.body.canonical_bytes());
        hasher.update(b"|");
        hasher.update(self.provenance.author.as_bytes());
        hasher.update(b"|");
        hasher.update(&[u8::from(self.provenance.tainted)]);
        hasher.update(b"|");
        for h in &self.lineage {
            hasher.update(&h.0);
        }
        hasher.update(b"|");
        for (k, v) in &self.tags {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b";");
        }
        ContentHash(*hasher.finalize().as_bytes())
    }

    /// The effective weight of this signal at the given current time.
    /// Combines score × decay.
    #[must_use]
    pub fn weight_at(&self, now_ms: i64) -> f32 {
        let age = now_ms - self.created_at_ms;
        self.score.effective() * self.decay.apply(age)
    }

    /// Age of this signal in milliseconds relative to a reference time.
    #[must_use]
    pub fn age_ms(&self, now_ms: i64) -> i64 {
        (now_ms - self.created_at_ms).max(0)
    }

    /// Get a tag value by key.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }

    /// Check if this signal's kind matches the given kind.
    #[must_use]
    pub fn is(&self, kind: &Kind) -> bool {
        &self.kind == kind
    }

    /// Emit a derived signal — new kind/body, but tracks this signal as lineage.
    /// Useful when a gate/composer/policy produces a new signal from an input.
    pub fn derive(&self, kind: Kind, body: Body) -> SignalBuilder {
        SignalBuilder::new(kind)
            .body(body)
            .lineage([self.id])
            .provenance(Provenance::agent("derived"))
    }
}

// ─── Builder ───────────────────────────────────────────────────────────────

/// Ergonomic builder for [`Signal`]s.
///
/// Fills in sensible defaults: current time, neutral score, no decay, trusted
/// roko provenance, empty lineage and tags.
pub struct SignalBuilder {
    kind: Kind,
    body: Body,
    created_at_ms: Option<i64>,
    decay: Decay,
    provenance: Provenance,
    score: Score,
    lineage: Vec<ContentHash>,
    tags: BTreeMap<String, String>,
}

impl SignalBuilder {
    /// Start building a signal of the given kind.
    #[must_use]
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            body: Body::empty(),
            created_at_ms: None,
            decay: Decay::None,
            provenance: Provenance::default(),
            score: Score::NEUTRAL,
            lineage: Vec::new(),
            tags: BTreeMap::new(),
        }
    }

    /// Set the signal's body (payload).
    #[must_use]
    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    /// Set the signal's decay function.
    #[must_use]
    pub const fn decay(mut self, decay: Decay) -> Self {
        self.decay = decay;
        self
    }

    /// Set the signal's provenance (author + trust).
    #[must_use]
    pub fn provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    /// Set the signal's score.
    #[must_use]
    pub const fn score(mut self, score: Score) -> Self {
        self.score = score;
        self
    }

    /// Pin the signal's creation time (mostly useful for tests).
    #[must_use]
    pub const fn created_at_ms(mut self, t: i64) -> Self {
        self.created_at_ms = Some(t);
        self
    }

    /// Add content-hashes of parent signals to the lineage chain.
    #[must_use]
    pub fn lineage(mut self, hashes: impl IntoIterator<Item = ContentHash>) -> Self {
        self.lineage.extend(hashes);
        self
    }

    /// Set a string tag for filtering and indexing.
    #[must_use]
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Finalize the signal, computing its content hash.
    #[must_use]
    pub fn build(self) -> Signal {
        let created_at_ms = self.created_at_ms.unwrap_or_else(current_time_ms);
        let mut signal = Signal {
            id: ContentHash([0; 32]), // placeholder
            kind: self.kind,
            body: self.body,
            created_at_ms,
            decay: self.decay,
            provenance: self.provenance,
            score: self.score,
            lineage: self.lineage,
            tags: self.tags,
        };
        signal.id = signal.content_hash();
        signal
    }
}

/// Current Unix time in milliseconds.
fn current_time_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults() {
        let s = Signal::builder(Kind::Task).build();
        assert_eq!(s.kind, Kind::Task);
        assert_eq!(s.body, Body::Empty);
        assert_eq!(s.decay, Decay::None);
        assert!(s.lineage.is_empty());
        assert!(s.tags.is_empty());
    }

    #[test]
    fn content_hash_is_deterministic() {
        let a = Signal::builder(Kind::Task)
            .body(Body::text("same"))
            .created_at_ms(0)
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("same"))
            .created_at_ms(0)
            .build();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn content_hash_ignores_score_and_decay() {
        // Two signals with same identity fields but different score/decay
        // should have the SAME id (score/decay don't affect identity).
        let a = Signal::builder(Kind::Task)
            .body(Body::text("same"))
            .created_at_ms(0)
            .score(Score::new(0.1, 0.0, 0.0, 1.0))
            .decay(Decay::None)
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("same"))
            .created_at_ms(0)
            .score(Score::new(0.9, 1.0, 5.0, 2.0))
            .decay(Decay::HalfLife { half_life_ms: 1000 })
            .build();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn content_hash_includes_body() {
        let a = Signal::builder(Kind::Task)
            .body(Body::text("a"))
            .created_at_ms(0)
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("b"))
            .created_at_ms(0)
            .build();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn content_hash_includes_tags() {
        let a = Signal::builder(Kind::Task)
            .body(Body::text("x"))
            .created_at_ms(0)
            .tag("priority", "high")
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("x"))
            .created_at_ms(0)
            .tag("priority", "low")
            .build();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn tag_order_does_not_affect_hash() {
        // BTreeMap stores keys in sorted order, so insertion order is irrelevant.
        let a = Signal::builder(Kind::Task)
            .body(Body::text("x"))
            .created_at_ms(0)
            .tag("a", "1")
            .tag("b", "2")
            .build();
        let b = Signal::builder(Kind::Task)
            .body(Body::text("x"))
            .created_at_ms(0)
            .tag("b", "2")
            .tag("a", "1")
            .build();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn weight_at_combines_score_and_decay() {
        let s = Signal::builder(Kind::Pheromone)
            .decay(Decay::HalfLife { half_life_ms: 1000 })
            .score(Score::new(1.0, 0.0, 0.0, 1.0)) // effective = 1.0
            .created_at_ms(0)
            .build();
        assert!((s.weight_at(0) - 1.0).abs() < 1e-6);
        assert!((s.weight_at(1000) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn derive_tracks_lineage() {
        let parent = Signal::builder(Kind::Task)
            .body(Body::text("parent"))
            .created_at_ms(0)
            .build();
        let child = parent.derive(Kind::GateVerdict, Body::text("pass")).build();
        assert_eq!(child.lineage, vec![parent.id]);
        assert_eq!(child.kind, Kind::GateVerdict);
    }

    #[test]
    fn serde_roundtrip() {
        let s = Signal::builder(Kind::Episode)
            .body(Body::text("an episode happened"))
            .decay(Decay::HalfLife {
                half_life_ms: 60_000,
            })
            .tag("run", "42")
            .build();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Signal = serde_json::from_str(&json).unwrap();
        assert_eq!(s, parsed);
    }

    #[test]
    fn tag_accessor() {
        let s = Signal::builder(Kind::Task).tag("foo", "bar").build();
        assert_eq!(s.tag("foo"), Some("bar"));
        assert_eq!(s.tag("missing"), None);
    }

    #[test]
    fn is_matches_kind() {
        let s = Signal::builder(Kind::GateVerdict).build();
        assert!(s.is(&Kind::GateVerdict));
        assert!(!s.is(&Kind::Task));
    }
}
