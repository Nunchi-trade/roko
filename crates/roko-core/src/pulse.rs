//! Ephemeral events on the Bus — the live-wire counterpart to [`Engram`](crate::Engram).
//!
//! A [`Pulse`] represents transport traffic that has not been persisted. Pulses
//! flow through the [`Bus`](crate::traits) and may "graduate" to an [`Engram`]
//! through deliberate promotion. This module also defines [`Topic`] (the
//! addressing key for Pulses), [`TopicFilter`] (subscription filters), and
//! [`PolicyOutputs`] (the explicit outputs from a React's `decide()` call).

use crate::{Body, Engram, Kind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

// ─── Topic ──────────────────────────────────────────────────────────────────

/// Addressing/routing key for [`Pulse`]s on the Bus.
///
/// Topics follow a dotted hierarchy: `"gate.verdict.emitted"`,
/// `"heartbeat.gamma.tick"`, etc. Subscribers filter by topic prefix.
///
/// # Examples
///
/// ```
/// use roko_core::Topic;
///
/// let topic = Topic::new("gate.verdict.emitted");
/// assert!(topic.starts_with("gate.verdict"));
/// assert!(!topic.starts_with("heartbeat"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Topic(pub String);

impl Topic {
    /// Create a new topic from any string-like value.
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Whether this topic starts with the given prefix.
    ///
    /// Used for hierarchical matching in [`TopicFilter::Prefix`].
    #[must_use]
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.0.starts_with(prefix)
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ─── Pulse ──────────────────────────────────────────────────────────────────

/// Ephemeral event on the Bus — the live-wire counterpart to [`Engram`].
///
/// Pulses represent transport traffic that has not been persisted. A Pulse
/// may "graduate" to an Engram through deliberate promotion.
///
/// # Construction
///
/// Use [`Pulse::new`] for direct construction or [`Pulse::builder`] for
/// ergonomic multi-field construction:
///
/// ```
/// use roko_core::{Body, Kind, Pulse, Topic};
///
/// let p = Pulse::new(1, Topic::new("gate.verdict"), Kind::GateVerdict, Body::text("pass"));
/// assert_eq!(p.seq, 1);
/// assert_eq!(p.topic, Topic::new("gate.verdict"));
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pulse {
    /// Monotonic sequence number assigned by the Bus.
    pub seq: u64,
    /// Routing key for subscription matching.
    pub topic: Topic,
    /// What kind of event this pulse represents.
    pub kind: Kind,
    /// The pulse's payload.
    pub body: Body,
    /// Unix milliseconds when this pulse was created.
    pub created_at_ms: i64,
    /// Arbitrary string metadata (ordered for stable serialization).
    pub tags: BTreeMap<String, String>,
}

impl Pulse {
    /// Create a new pulse with the given fields, timestamped to now.
    #[must_use]
    pub fn new(seq: u64, topic: Topic, kind: Kind, body: Body) -> Self {
        Self {
            seq,
            topic,
            kind,
            body,
            created_at_ms: current_time_ms(),
            tags: BTreeMap::new(),
        }
    }

    /// Begin building a pulse with ergonomic chaining.
    #[must_use]
    pub fn builder(seq: u64, topic: Topic, kind: Kind) -> PulseBuilder {
        PulseBuilder::new(seq, topic, kind)
    }

    /// Get a tag value by key.
    #[must_use]
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.get(key).map(String::as_str)
    }
}

// ─── PulseBuilder ───────────────────────────────────────────────────────────

/// Ergonomic builder for [`Pulse`]s.
///
/// Fills in sensible defaults: current time, empty body, no tags.
pub struct PulseBuilder {
    seq: u64,
    topic: Topic,
    kind: Kind,
    body: Body,
    created_at_ms: Option<i64>,
    tags: BTreeMap<String, String>,
}

impl PulseBuilder {
    /// Start building a pulse with the required fields.
    #[must_use]
    pub fn new(seq: u64, topic: Topic, kind: Kind) -> Self {
        Self {
            seq,
            topic,
            kind,
            body: Body::empty(),
            created_at_ms: None,
            tags: BTreeMap::new(),
        }
    }

    /// Set the pulse's body (payload).
    #[must_use]
    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    /// Pin the pulse's creation time (mostly useful for tests).
    #[must_use]
    pub const fn created_at_ms(mut self, t: i64) -> Self {
        self.created_at_ms = Some(t);
        self
    }

    /// Set a string tag for filtering and routing.
    #[must_use]
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.insert(key.into(), value.into());
        self
    }

    /// Finalize the pulse.
    #[must_use]
    pub fn build(self) -> Pulse {
        Pulse {
            seq: self.seq,
            topic: self.topic,
            kind: self.kind,
            body: self.body,
            created_at_ms: self.created_at_ms.unwrap_or_else(current_time_ms),
            tags: self.tags,
        }
    }
}

// ─── TopicFilter ────────────────────────────────────────────────────────────

/// Filter for Bus subscriptions.
///
/// Used by [`Bus::subscribe`](crate::traits) to select which [`Pulse`]s
/// the subscriber receives.
///
/// # Examples
///
/// ```
/// use roko_core::{Topic, TopicFilter};
///
/// let filter = TopicFilter::Prefix("gate.".into());
/// assert!(filter.matches(&Topic::new("gate.verdict.emitted")));
/// assert!(!filter.matches(&Topic::new("heartbeat.tick")));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TopicFilter {
    /// Match a single exact topic.
    Exact(Topic),
    /// Match all topics starting with this prefix.
    Prefix(String),
    /// Match all topics.
    All,
    /// Match only if ALL inner filters match (conjunction).
    And(Vec<TopicFilter>),
    /// Match if ANY inner filter matches (disjunction).
    Or(Vec<TopicFilter>),
    /// Match if the inner filter does NOT match (negation).
    Not(Box<TopicFilter>),
}

impl TopicFilter {
    /// Whether a topic matches this filter.
    #[must_use]
    pub fn matches(&self, topic: &Topic) -> bool {
        match self {
            Self::Exact(t) => t == topic,
            Self::Prefix(p) => topic.starts_with(p),
            Self::All => true,
            Self::And(filters) => filters.iter().all(|filter| filter.matches(topic)),
            Self::Or(filters) => filters.iter().any(|filter| filter.matches(topic)),
            Self::Not(filter) => !filter.matches(topic),
        }
    }
}

// ─── PolicyOutputs ──────────────────────────────────────────────────────────

/// Explicit outputs from a [`React`](crate::React)'s `decide()` call.
///
/// Policies can both publish new [`Pulse`]s for immediate downstream reactions
/// AND persist [`Engram`]s for summaries and decisions. This struct makes both
/// output channels explicit.
///
/// # Examples
///
/// ```
/// use roko_core::{Body, Engram, Kind, PolicyOutputs, Pulse, Topic};
///
/// let outputs = PolicyOutputs::empty()
///     .with_pulse(Pulse::new(1, Topic::new("alert"), Kind::Metric, Body::text("cpu high")))
///     .with_engram(Engram::builder(Kind::Episode).body(Body::text("logged")).build());
///
/// assert_eq!(outputs.pulses.len(), 1);
/// assert_eq!(outputs.engrams.len(), 1);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyOutputs {
    /// Pulses to publish on the Bus for immediate downstream reactions.
    pub pulses: Vec<Pulse>,
    /// Engrams to persist via the Store.
    pub engrams: Vec<Engram>,
}

impl PolicyOutputs {
    /// Create empty outputs (no pulses, no engrams).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            pulses: Vec::new(),
            engrams: Vec::new(),
        }
    }

    /// Add a pulse to the outputs.
    #[must_use]
    pub fn with_pulse(mut self, pulse: Pulse) -> Self {
        self.pulses.push(pulse);
        self
    }

    /// Add an engram to the outputs.
    #[must_use]
    pub fn with_engram(mut self, engram: Engram) -> Self {
        self.engrams.push(engram);
        self
    }

    /// Whether these outputs contain any pulses or engrams.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pulses.is_empty() && self.engrams.is_empty()
    }

    /// Total count of pulses and engrams.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pulses.len() + self.engrams.len()
    }
}

impl Default for PolicyOutputs {
    fn default() -> Self {
        Self::empty()
    }
}

/// Current Unix time in milliseconds.
fn current_time_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Topic ───────────────────────────────────────────────────────────

    #[test]
    fn topic_new_and_display() {
        let t = Topic::new("gate.verdict.emitted");
        assert_eq!(t.0, "gate.verdict.emitted");
        assert_eq!(format!("{t}"), "gate.verdict.emitted");
    }

    #[test]
    fn topic_starts_with_hierarchical() {
        let t = Topic::new("gate.verdict.emitted");
        assert!(t.starts_with("gate"));
        assert!(t.starts_with("gate.verdict"));
        assert!(t.starts_with("gate.verdict.emitted"));
        assert!(!t.starts_with("heartbeat"));
        assert!(!t.starts_with("gate.compile"));
    }

    #[test]
    fn topic_equality_and_hash() {
        use std::collections::HashSet;
        let a = Topic::new("x.y");
        let b = Topic::new("x.y");
        let c = Topic::new("x.z");
        assert_eq!(a, b);
        assert_ne!(a, c);

        let mut set = HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn topic_serde_roundtrip() {
        let t = Topic::new("heartbeat.gamma.tick");
        let json = serde_json::to_string(&t).unwrap();
        let parsed: Topic = serde_json::from_str(&json).unwrap();
        assert_eq!(t, parsed);
    }

    // ── Pulse ───────────────────────────────────────────────────────────

    #[test]
    fn pulse_new_sets_timestamp() {
        let p = Pulse::new(1, Topic::new("test"), Kind::Task, Body::text("hi"));
        assert_eq!(p.seq, 1);
        assert_eq!(p.topic, Topic::new("test"));
        assert_eq!(p.kind, Kind::Task);
        assert!(p.created_at_ms > 0);
        assert!(p.tags.is_empty());
    }

    #[test]
    fn pulse_builder_defaults() {
        let p = Pulse::builder(42, Topic::new("t"), Kind::Metric).build();
        assert_eq!(p.seq, 42);
        assert_eq!(p.kind, Kind::Metric);
        assert_eq!(p.body, Body::Empty);
        assert!(p.tags.is_empty());
    }

    #[test]
    fn pulse_builder_full() {
        let p = Pulse::builder(10, Topic::new("gate.compile"), Kind::GateVerdict)
            .body(Body::text("pass"))
            .created_at_ms(12345)
            .tag("plan_id", "plan-1")
            .tag("gate", "compile")
            .build();

        assert_eq!(p.seq, 10);
        assert_eq!(p.topic, Topic::new("gate.compile"));
        assert_eq!(p.body, Body::Text("pass".into()));
        assert_eq!(p.created_at_ms, 12345);
        assert_eq!(p.tag("plan_id"), Some("plan-1"));
        assert_eq!(p.tag("gate"), Some("compile"));
        assert_eq!(p.tag("missing"), None);
    }

    #[test]
    fn pulse_serde_roundtrip() {
        let p = Pulse::builder(7, Topic::new("episode.logged"), Kind::Episode)
            .body(Body::text("recorded"))
            .created_at_ms(99999)
            .tag("run", "42")
            .build();
        let json = serde_json::to_string(&p).unwrap();
        let parsed: Pulse = serde_json::from_str(&json).unwrap();
        assert_eq!(p, parsed);
    }

    // ── TopicFilter ─────────────────────────────────────────────────────

    #[test]
    fn filter_exact_matches() {
        let f = TopicFilter::Exact(Topic::new("gate.verdict"));
        assert!(f.matches(&Topic::new("gate.verdict")));
        assert!(!f.matches(&Topic::new("gate.verdict.emitted")));
        assert!(!f.matches(&Topic::new("gate")));
    }

    #[test]
    fn filter_prefix_matches() {
        let f = TopicFilter::Prefix("gate.".into());
        assert!(f.matches(&Topic::new("gate.verdict")));
        assert!(f.matches(&Topic::new("gate.compile.started")));
        assert!(!f.matches(&Topic::new("heartbeat.tick")));
        assert!(!f.matches(&Topic::new("gate")));
    }

    #[test]
    fn filter_all_matches_everything() {
        let f = TopicFilter::All;
        assert!(f.matches(&Topic::new("anything")));
        assert!(f.matches(&Topic::new("")));
        assert!(f.matches(&Topic::new("deeply.nested.topic.here")));
    }

    #[test]
    fn filter_serde_roundtrip() {
        for f in [
            TopicFilter::Exact(Topic::new("x.y")),
            TopicFilter::Prefix("gate.".into()),
            TopicFilter::All,
        ] {
            let json = serde_json::to_string(&f).unwrap();
            let parsed: TopicFilter = serde_json::from_str(&json).unwrap();
            assert_eq!(f, parsed);
        }
    }

    #[test]
    fn filter_and_with_mixed_filters() {
        // Both must match: prefix "gate." AND exact "gate.verdict"
        let f = TopicFilter::And(vec![
            TopicFilter::Prefix("gate.".into()),
            TopicFilter::Exact(Topic::new("gate.verdict")),
        ]);
        assert!(f.matches(&Topic::new("gate.verdict")));
        // Matches prefix but not exact
        assert!(!f.matches(&Topic::new("gate.compile")));
        // Matches neither
        assert!(!f.matches(&Topic::new("heartbeat.tick")));
    }

    #[test]
    fn filter_or_with_disjoint_filters() {
        // Either matches: exact "gate.verdict" OR prefix "heartbeat."
        let f = TopicFilter::Or(vec![
            TopicFilter::Exact(Topic::new("gate.verdict")),
            TopicFilter::Prefix("heartbeat.".into()),
        ]);
        assert!(f.matches(&Topic::new("gate.verdict")));
        assert!(f.matches(&Topic::new("heartbeat.tick")));
        assert!(!f.matches(&Topic::new("episode.logged")));
    }

    #[test]
    fn filter_not_inverts_exact_match() {
        let f = TopicFilter::Not(Box::new(TopicFilter::Exact(Topic::new("gate.verdict"))));
        // The exact topic should NOT match (inverted)
        assert!(!f.matches(&Topic::new("gate.verdict")));
        // Everything else matches
        assert!(f.matches(&Topic::new("gate.compile")));
        assert!(f.matches(&Topic::new("heartbeat.tick")));
    }

    #[test]
    fn filter_nested_combinators() {
        // Or([And([Prefix("gate."), Prefix("gate.verdict")]), Not(Exact("heartbeat.tick"))])
        let f = TopicFilter::Or(vec![
            TopicFilter::And(vec![
                TopicFilter::Prefix("gate.".into()),
                TopicFilter::Prefix("gate.verdict".into()),
            ]),
            TopicFilter::Not(Box::new(TopicFilter::Exact(Topic::new("heartbeat.tick")))),
        ]);
        // Matches first branch (both prefixes)
        assert!(f.matches(&Topic::new("gate.verdict.emitted")));
        // Does not match first branch (only one prefix), but matches second (not "heartbeat.tick")
        assert!(f.matches(&Topic::new("gate.compile")));
        // "heartbeat.tick" fails second branch (Not inverts to false), check first branch: fails
        assert!(!f.matches(&Topic::new("heartbeat.tick")));
    }

    #[test]
    fn filter_and_empty_is_vacuous_truth() {
        let f = TopicFilter::And(vec![]);
        // Empty And: all of zero filters match (vacuous truth)
        assert!(f.matches(&Topic::new("anything")));
        assert!(f.matches(&Topic::new("")));
    }

    #[test]
    fn filter_or_empty_is_false() {
        let f = TopicFilter::Or(vec![]);
        // Empty Or: none of zero filters match
        assert!(!f.matches(&Topic::new("anything")));
        assert!(!f.matches(&Topic::new("")));
    }

    // ── PolicyOutputs ───────────────────────────────────────────────────

    #[test]
    fn policy_outputs_empty() {
        let o = PolicyOutputs::empty();
        assert!(o.is_empty());
        assert_eq!(o.len(), 0);
        assert!(o.pulses.is_empty());
        assert!(o.engrams.is_empty());
    }

    #[test]
    fn policy_outputs_default_is_empty() {
        let o = PolicyOutputs::default();
        assert!(o.is_empty());
    }

    #[test]
    fn policy_outputs_with_pulse() {
        let p = Pulse::new(1, Topic::new("alert"), Kind::Metric, Body::text("cpu high"));
        let o = PolicyOutputs::empty().with_pulse(p.clone());
        assert!(!o.is_empty());
        assert_eq!(o.len(), 1);
        assert_eq!(o.pulses.len(), 1);
        assert_eq!(o.pulses[0], p);
        assert!(o.engrams.is_empty());
    }

    #[test]
    fn policy_outputs_with_engram() {
        let e = Engram::builder(Kind::Episode)
            .body(Body::text("logged"))
            .created_at_ms(0)
            .build();
        let o = PolicyOutputs::empty().with_engram(e.clone());
        assert!(!o.is_empty());
        assert_eq!(o.len(), 1);
        assert!(o.pulses.is_empty());
        assert_eq!(o.engrams.len(), 1);
        assert_eq!(o.engrams[0], e);
    }

    #[test]
    fn policy_outputs_chained() {
        let p = Pulse::new(1, Topic::new("a"), Kind::Metric, Body::empty());
        let e = Engram::builder(Kind::Task).created_at_ms(0).build();
        let o = PolicyOutputs::empty()
            .with_pulse(p)
            .with_engram(e)
            .with_pulse(Pulse::new(2, Topic::new("b"), Kind::Episode, Body::empty()));
        assert_eq!(o.len(), 3);
        assert_eq!(o.pulses.len(), 2);
        assert_eq!(o.engrams.len(), 1);
    }

    #[test]
    fn policy_outputs_serde_roundtrip() {
        let o = PolicyOutputs::empty()
            .with_pulse(
                Pulse::builder(1, Topic::new("x"), Kind::Task)
                    .created_at_ms(100)
                    .build(),
            )
            .with_engram(
                Engram::builder(Kind::Episode)
                    .body(Body::text("ep"))
                    .created_at_ms(200)
                    .build(),
            );
        let json = serde_json::to_string(&o).unwrap();
        let parsed: PolicyOutputs = serde_json::from_str(&json).unwrap();
        assert_eq!(o, parsed);
    }
}
