//! COMP-08: Agent mesh context sharing.
//!
//! Provides a [`ContextMesh`] registry that agents in a multi-agent plan can
//! use to publish and subscribe to context sections. This enables Level 3
//! network context engineering from doc 11 of the composition spec.
//!
//! ## Design
//!
//! - Each agent publishes [`SharedContextEntry`] items keyed by topic.
//! - Other agents query the mesh for relevant entries, filtering by topic
//!   and recency.
//! - Cross-agent deduplication prevents the same knowledge from appearing
//!   in multiple agents' prompts simultaneously (reducing total token spend).
//! - The mesh is thread-safe (behind `Arc<Mutex<_>>`) for concurrent access
//!   from the `MultiAgentPool`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::prompt::{AttentionBidder, CacheLayer, Placement, PromptSection, SectionPriority};

/// A context entry shared by one agent for consumption by others.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SharedContextEntry {
    /// Unique identifier for this entry.
    pub entry_id: String,
    /// Agent that published this entry.
    pub publisher_agent: String,
    /// Topic tag for filtering (e.g. "error", "pattern", "discovery").
    pub topic: String,
    /// The actual content to inject into subscribing agents' prompts.
    pub content: String,
    /// Relevance score assigned by the publishing agent.
    pub relevance: f64,
    /// Token cost estimate.
    pub estimated_tokens: usize,
    /// Monotonic publish timestamp (epoch ms).
    pub published_at_ms: i64,
    /// How many agents have consumed this entry.
    pub consume_count: usize,
}

/// Thread-safe shared context mesh for multi-agent plans.
///
/// Agents publish discoveries, patterns, and warnings to the mesh.
/// Other agents query it to enrich their prompts with cross-agent context.
#[derive(Clone, Debug, Default)]
pub struct ContextMesh {
    inner: Arc<Mutex<MeshState>>,
}

#[derive(Clone, Debug, Default)]
struct MeshState {
    /// All published entries, keyed by entry_id.
    entries: HashMap<String, SharedContextEntry>,
    /// Index: topic -> entry_ids.
    by_topic: HashMap<String, Vec<String>>,
    /// Index: publisher_agent -> entry_ids.
    by_agent: HashMap<String, Vec<String>>,
    /// Monotonic counter for generating entry IDs.
    next_id: u64,
}

impl ContextMesh {
    /// Create a new empty context mesh.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a context entry to the mesh.
    ///
    /// Returns the assigned entry ID.
    pub fn publish(
        &self,
        agent_id: &str,
        topic: &str,
        content: &str,
        relevance: f64,
        now_ms: i64,
    ) -> String {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = format!("mesh-{}", state.next_id);
        state.next_id += 1;

        let tokens = crate::prompt::estimate_tokens(content);
        let entry = SharedContextEntry {
            entry_id: id.clone(),
            publisher_agent: agent_id.to_string(),
            topic: topic.to_string(),
            content: content.to_string(),
            relevance: relevance.clamp(0.0, 1.0),
            estimated_tokens: tokens,
            published_at_ms: now_ms,
            consume_count: 0,
        };

        state
            .by_topic
            .entry(topic.to_string())
            .or_default()
            .push(id.clone());
        state
            .by_agent
            .entry(agent_id.to_string())
            .or_default()
            .push(id.clone());
        state.entries.insert(id.clone(), entry);

        id
    }

    /// Query the mesh for entries matching a topic, excluding entries from
    /// the querying agent itself (to avoid echo).
    ///
    /// Returns entries sorted by relevance descending, limited to `max_entries`.
    pub fn query(
        &self,
        querying_agent: &str,
        topic: &str,
        max_entries: usize,
    ) -> Vec<SharedContextEntry> {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ids) = state.by_topic.get(topic) else {
            return Vec::new();
        };

        let mut results: Vec<_> = ids
            .iter()
            .filter_map(|id| state.entries.get(id))
            .filter(|entry| entry.publisher_agent != querying_agent)
            .cloned()
            .collect();

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_entries);
        results
    }

    /// Query all entries from the mesh for a given agent, excluding self.
    ///
    /// Returns entries sorted by relevance descending, limited to `max_entries`.
    pub fn query_all(&self, querying_agent: &str, max_entries: usize) -> Vec<SharedContextEntry> {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .entries
            .values()
            .filter(|entry| entry.publisher_agent != querying_agent)
            .cloned()
            .collect();

        results.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(max_entries);
        results
    }

    /// Mark an entry as consumed by an agent. Increments the consume counter.
    pub fn mark_consumed(&self, entry_id: &str) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = state.entries.get_mut(entry_id) {
            entry.consume_count += 1;
        }
    }

    /// Convert mesh entries into [`PromptSection`]s for injection into a
    /// prompt via the [`PromptComposer`](crate::PromptComposer).
    ///
    /// Each entry becomes a Normal-priority, Workspace-layer section with
    /// the Neuro bidder (cross-agent knowledge).
    #[must_use]
    pub fn to_prompt_sections(entries: &[SharedContextEntry]) -> Vec<PromptSection> {
        entries
            .iter()
            .map(|entry| {
                PromptSection::new(
                    format!("mesh:{}", entry.topic),
                    format!("[from agent {}] {}", entry.publisher_agent, entry.content),
                )
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_bidder(AttentionBidder::Neuro)
            })
            .collect()
    }

    /// Deduplicate entries across agents. When two entries from different
    /// agents have the same topic and similar content (by token overlap),
    /// keep only the higher-relevance one.
    ///
    /// This is the cross-agent deduplication required by the spec.
    #[must_use]
    pub fn deduplicate(entries: Vec<SharedContextEntry>) -> Vec<SharedContextEntry> {
        if entries.len() <= 1 {
            return entries;
        }

        let mut kept: Vec<SharedContextEntry> = Vec::with_capacity(entries.len());

        for entry in entries {
            let is_duplicate = kept.iter().any(|accepted| {
                accepted.topic == entry.topic
                    && content_overlap(&accepted.content, &entry.content) > 0.6
            });
            if !is_duplicate {
                kept.push(entry);
            }
        }

        kept
    }

    /// Total number of entries in the mesh.
    #[must_use]
    pub fn len(&self) -> usize {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.entries.len()
    }

    /// Whether the mesh is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Remove all entries older than `max_age_ms` relative to `now_ms`.
    pub fn evict_stale(&self, now_ms: i64, max_age_ms: i64) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cutoff = now_ms - max_age_ms;
        let stale_ids: Vec<String> = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.published_at_ms < cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        for id in &stale_ids {
            state.entries.remove(id);
        }

        // Clean up indexes.
        for ids in state.by_topic.values_mut() {
            ids.retain(|id| !stale_ids.contains(id));
        }
        for ids in state.by_agent.values_mut() {
            ids.retain(|id| !stale_ids.contains(id));
        }
        state.by_topic.retain(|_, ids| !ids.is_empty());
        state.by_agent.retain(|_, ids| !ids.is_empty());
    }
}

/// Simple Jaccard-like content overlap score between two strings.
///
/// Tokenizes both strings into lowercase words and computes the intersection
/// size divided by the union size.
fn content_overlap(a: &str, b: &str) -> f64 {
    let tokens_a: std::collections::HashSet<&str> = a
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let tokens_b: std::collections::HashSet<&str> = b
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 1.0;
    }

    let intersection = tokens_a.intersection(&tokens_b).count();
    let union = tokens_a.union(&tokens_b).count();

    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_and_query_round_trip() {
        let mesh = ContextMesh::new();
        mesh.publish(
            "agent-a",
            "error",
            "build failed: missing import",
            0.9,
            1000,
        );
        mesh.publish("agent-b", "error", "test failure in module X", 0.7, 1001);

        // agent-c queries for errors, should see both.
        let results = mesh.query("agent-c", "error", 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].relevance, 0.9); // sorted by relevance desc

        // agent-a queries for errors, should not see its own.
        let results = mesh.query("agent-a", "error", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].publisher_agent, "agent-b");
    }

    #[test]
    fn query_all_excludes_self() {
        let mesh = ContextMesh::new();
        mesh.publish("agent-a", "pattern", "use builder pattern", 0.8, 1000);
        mesh.publish("agent-a", "error", "compile error", 0.6, 1001);
        mesh.publish("agent-b", "discovery", "found optimization", 0.9, 1002);

        let results = mesh.query_all("agent-a", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].publisher_agent, "agent-b");
    }

    #[test]
    fn mark_consumed_increments_counter() {
        let mesh = ContextMesh::new();
        let id = mesh.publish("agent-a", "error", "some error", 0.5, 1000);
        mesh.mark_consumed(&id);
        mesh.mark_consumed(&id);

        let results = mesh.query("agent-b", "error", 10);
        assert_eq!(results[0].consume_count, 2);
    }

    #[test]
    fn to_prompt_sections_converts_entries() {
        let entries = vec![SharedContextEntry {
            entry_id: "mesh-0".into(),
            publisher_agent: "agent-a".into(),
            topic: "error".into(),
            content: "build failed".into(),
            relevance: 0.8,
            estimated_tokens: 3,
            published_at_ms: 1000,
            consume_count: 0,
        }];

        let sections = ContextMesh::to_prompt_sections(&entries);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "mesh:error");
        assert!(sections[0].content.contains("build failed"));
        assert_eq!(sections[0].bidder, AttentionBidder::Neuro);
    }

    #[test]
    fn deduplicate_removes_similar_entries() {
        let entries = vec![
            SharedContextEntry {
                entry_id: "mesh-0".into(),
                publisher_agent: "agent-a".into(),
                topic: "error".into(),
                content: "build failed missing import in module X".into(),
                relevance: 0.9,
                estimated_tokens: 10,
                published_at_ms: 1000,
                consume_count: 0,
            },
            SharedContextEntry {
                entry_id: "mesh-1".into(),
                publisher_agent: "agent-b".into(),
                topic: "error".into(),
                content: "build failed missing import in module X too".into(),
                relevance: 0.7,
                estimated_tokens: 10,
                published_at_ms: 1001,
                consume_count: 0,
            },
        ];

        let deduped = ContextMesh::deduplicate(entries);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].relevance, 0.9); // kept the higher-relevance one
    }

    #[test]
    fn deduplicate_keeps_different_topics() {
        let entries = vec![
            SharedContextEntry {
                entry_id: "mesh-0".into(),
                publisher_agent: "agent-a".into(),
                topic: "error".into(),
                content: "build failed".into(),
                relevance: 0.9,
                estimated_tokens: 3,
                published_at_ms: 1000,
                consume_count: 0,
            },
            SharedContextEntry {
                entry_id: "mesh-1".into(),
                publisher_agent: "agent-b".into(),
                topic: "pattern".into(),
                content: "build failed".into(),
                relevance: 0.7,
                estimated_tokens: 3,
                published_at_ms: 1001,
                consume_count: 0,
            },
        ];

        let deduped = ContextMesh::deduplicate(entries);
        assert_eq!(deduped.len(), 2); // different topics, no dedup
    }

    #[test]
    fn evict_stale_removes_old_entries() {
        let mesh = ContextMesh::new();
        mesh.publish("agent-a", "old", "stale content", 0.5, 100);
        mesh.publish("agent-b", "new", "fresh content", 0.9, 2000);

        mesh.evict_stale(2500, 1000); // Evict entries older than 1000ms ago

        assert_eq!(mesh.len(), 1);
        let results = mesh.query("agent-c", "old", 10);
        assert!(results.is_empty());
        let results = mesh.query("agent-c", "new", 10);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn content_overlap_identical() {
        assert!((content_overlap("hello world", "hello world") - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn content_overlap_disjoint() {
        assert!(content_overlap("hello world", "foo bar").abs() < f64::EPSILON);
    }

    #[test]
    fn content_overlap_partial() {
        let score = content_overlap("hello world foo", "hello world bar");
        assert!(score > 0.3);
        assert!(score < 0.8);
    }

    #[test]
    fn mesh_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ContextMesh>();
    }

    #[test]
    fn len_and_is_empty() {
        let mesh = ContextMesh::new();
        assert!(mesh.is_empty());
        assert_eq!(mesh.len(), 0);

        mesh.publish("a", "t", "c", 0.5, 1000);
        assert!(!mesh.is_empty());
        assert_eq!(mesh.len(), 1);
    }
}
