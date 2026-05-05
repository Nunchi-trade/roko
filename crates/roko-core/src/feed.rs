//! Feed trait for agent-produced data streams.
//!
//! Feeds are typed data channels produced by agents. Each feed has a kind
//! (Raw, Derived, Composite, Meta), an access level, and an optional JSON
//! schema describing the payload shape. The [`FeedRegistry`] tracks all
//! registered feeds and supports queries by kind, agent, and free-text search.
//!
//! **Migration note (Phase 1, §1.12):** Feeds will become Pulse streams on
//! the Bus, managed via the `Connect` + `Trigger` protocols defined in
//! `docs/v2/11-CONNECTIVITY.md`. The `FeedRegistry` is actively used by
//! `roko-serve` HTTP routes and will be migrated in M037. Do not add new
//! callers — prefer Bus-based Pulse streams once available.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Enums ─────────────────────────────────────────────────────────

/// Classification of a feed's data lineage.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FeedKind {
    /// Unprocessed source data (e.g. price ticks, log lines).
    Raw,
    /// Computed from one or more raw feeds (e.g. moving average).
    Derived,
    /// Assembled from multiple derived feeds (e.g. portfolio risk).
    Composite,
    /// Metadata about other feeds (e.g. schema registry, lineage graph).
    Meta,
}

/// Visibility / payment gate for a feed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FeedAccess {
    /// Readable by any agent.
    Public,
    /// Restricted to the producing agent and explicit subscribers.
    Private,
    /// Requires payment or staking to access.
    Paid,
}

// ── Structs ───────────────────────────────────────────────────────

/// Full descriptor for a registered feed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedInfo {
    /// Unique feed identifier (assigned by the registry on registration).
    pub id: String,
    /// Human-readable feed name.
    pub name: String,
    /// Data lineage classification.
    pub kind: FeedKind,
    /// Visibility / access level.
    pub access: FeedAccess,
    /// Agent that produces this feed.
    pub agent_id: String,
    /// Short description of what the feed contains.
    #[serde(default)]
    pub description: String,
    /// Optional JSON Schema describing individual feed payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    /// When the feed was first registered.
    pub created_at: DateTime<Utc>,
}

// ── Registry ──────────────────────────────────────────────────────

/// In-memory registry of [`FeedInfo`] entries.
///
/// **Migration (M037):** Will be replaced by Bus-based Pulse streams
/// with the `Connect` + `Trigger` protocols (Phase 1 §1.12).
/// See `docs/v2/11-CONNECTIVITY.md` for the replacement design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeedRegistry {
    feeds: Vec<FeedInfo>,
    /// Monotonic counter used to generate unique feed IDs.
    #[serde(default)]
    next_id: u64,
}

impl FeedRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            feeds: Vec::new(),
            next_id: 1,
        }
    }

    /// Register a new feed and return its assigned ID.
    ///
    /// The `id` field on the incoming [`FeedInfo`] is **overwritten** with a
    /// registry-assigned value to guarantee uniqueness.
    pub fn register(&mut self, mut feed: FeedInfo) -> String {
        let id = format!("feed-{}", self.next_id);
        self.next_id += 1;
        feed.id = id.clone();
        self.feeds.push(feed);
        id
    }

    /// Remove a feed by its ID. Returns `true` if it was present.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.feeds.len();
        self.feeds.retain(|f| f.id != id);
        self.feeds.len() != before
    }

    /// Look up a feed by its ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&FeedInfo> {
        self.feeds.iter().find(|f| f.id == id)
    }

    /// List all registered feeds.
    #[must_use]
    pub fn list(&self) -> &[FeedInfo] {
        &self.feeds
    }

    /// List feeds filtered by kind.
    #[must_use]
    pub fn list_by_kind(&self, kind: FeedKind) -> Vec<&FeedInfo> {
        self.feeds.iter().filter(|f| f.kind == kind).collect()
    }

    /// List feeds produced by a specific agent.
    #[must_use]
    pub fn list_by_agent(&self, agent_id: &str) -> Vec<&FeedInfo> {
        self.feeds
            .iter()
            .filter(|f| f.agent_id == agent_id)
            .collect()
    }

    /// Simple substring search across feed name and description.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&FeedInfo> {
        let q = query.to_lowercase();
        self.feeds
            .iter()
            .filter(|f| {
                f.name.to_lowercase().contains(&q) || f.description.to_lowercase().contains(&q)
            })
            .collect()
    }
}

// ── Runtime status ───────────────────────────────────────────────

/// Runtime status snapshot for an active feed.
///
/// Returned by the `/api/feeds/runtime/{id}` endpoint and consumed by
/// `roko feed status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedRuntimeStatus {
    /// Stable feed identifier (e.g. `"file-watch-roko-dir"`).
    pub id: String,
    /// Topic string (e.g. `"fs.changed"`, `"provider.health"`).
    pub topic: String,
    /// Feed kind label (`"Raw"`, `"Derived"`, `"Composite"`, `"Meta"`).
    pub kind: String,
    /// Whether the feed is currently connected and producing pulses.
    pub connected: bool,
    /// Approximate output rate in Hz.
    #[serde(default)]
    pub rate_hz: f64,
    /// Total number of pulses emitted since startup.
    #[serde(default)]
    pub pulses_produced: u64,
    /// Epoch-ms timestamp of the last pulse, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_ms: Option<u64>,
    /// Error string if the feed is in a degraded state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_feed(name: &str, kind: FeedKind, agent: &str) -> FeedInfo {
        FeedInfo {
            id: String::new(), // will be overwritten by register()
            name: name.to_string(),
            kind,
            access: FeedAccess::Public,
            agent_id: agent.to_string(),
            description: format!("Test feed: {name}"),
            schema: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn register_assigns_id() {
        let mut reg = FeedRegistry::new();
        let id = reg.register(sample_feed("prices", FeedKind::Raw, "agent-1"));
        assert_eq!(id, "feed-1");
        assert_eq!(reg.list().len(), 1);
        assert_eq!(reg.list()[0].id, "feed-1");
    }

    #[test]
    fn ids_are_monotonic() {
        let mut reg = FeedRegistry::new();
        let id1 = reg.register(sample_feed("a", FeedKind::Raw, "x"));
        let id2 = reg.register(sample_feed("b", FeedKind::Derived, "y"));
        assert_eq!(id1, "feed-1");
        assert_eq!(id2, "feed-2");
    }

    #[test]
    fn unregister_returns_true_when_present() {
        let mut reg = FeedRegistry::new();
        let id = reg.register(sample_feed("prices", FeedKind::Raw, "agent-1"));
        assert!(reg.unregister(&id));
        assert!(reg.list().is_empty());
    }

    #[test]
    fn unregister_returns_false_when_absent() {
        let mut reg = FeedRegistry::new();
        assert!(!reg.unregister("feed-999"));
    }

    #[test]
    fn get_returns_entry() {
        let mut reg = FeedRegistry::new();
        let id = reg.register(sample_feed("prices", FeedKind::Raw, "agent-1"));
        let entry = reg.get(&id).expect("should find feed");
        assert_eq!(entry.name, "prices");
        assert!(reg.get("feed-999").is_none());
    }

    #[test]
    fn list_by_kind_filters() {
        let mut reg = FeedRegistry::new();
        reg.register(sample_feed("raw1", FeedKind::Raw, "a"));
        reg.register(sample_feed("derived1", FeedKind::Derived, "b"));
        reg.register(sample_feed("raw2", FeedKind::Raw, "c"));

        let raws = reg.list_by_kind(FeedKind::Raw);
        assert_eq!(raws.len(), 2);
        assert!(raws.iter().all(|f| f.kind == FeedKind::Raw));

        let derived = reg.list_by_kind(FeedKind::Derived);
        assert_eq!(derived.len(), 1);
    }

    #[test]
    fn list_by_agent_filters() {
        let mut reg = FeedRegistry::new();
        reg.register(sample_feed("a", FeedKind::Raw, "agent-1"));
        reg.register(sample_feed("b", FeedKind::Raw, "agent-2"));
        reg.register(sample_feed("c", FeedKind::Derived, "agent-1"));

        let agent1_feeds = reg.list_by_agent("agent-1");
        assert_eq!(agent1_feeds.len(), 2);
    }

    #[test]
    fn search_matches_name_and_description() {
        let mut reg = FeedRegistry::new();
        reg.register(sample_feed("ETH prices", FeedKind::Raw, "a"));
        reg.register(sample_feed("BTC volume", FeedKind::Raw, "b"));

        // Matches name
        let results = reg.search("eth");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "ETH prices");

        // Matches description (all descriptions contain "Test feed:")
        let results = reg.search("test feed");
        assert_eq!(results.len(), 2);

        // No match
        let results = reg.search("zzzzz");
        assert!(results.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let mut reg = FeedRegistry::new();
        reg.register(sample_feed("x", FeedKind::Composite, "a"));
        let json = serde_json::to_string(&reg).expect("serialize");
        let restored: FeedRegistry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.list().len(), 1);
        assert_eq!(restored.list()[0].name, "x");
        assert_eq!(restored.next_id, 2);
    }
}
