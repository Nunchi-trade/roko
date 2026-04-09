//! Pattern discovery — mines recurring action trigrams and cross-episode
//! structural meta-patterns across episode logs.
//!
//! `PatternMiner` consumes an iterable stream of episode-like observations
//! (types that implement the [`EpisodeView`] trait) and counts how often
//! specific three-action sequences (trigrams) appear across episodes. After a
//! batch has been ingested, [`PatternMiner::discover`] returns every trigram
//! whose episode support and confidence (defined as `support / total_episodes`)
//! clear the configured thresholds.
//!
//! The module is deliberately decoupled from the concrete `Episode` type used
//! by the episode logger — it depends only on the opaque [`EpisodeView`]
//! trait. That keeps pattern mining composable with anything that exposes an
//! ordered list of action kinds and a success flag, including synthetic
//! fixtures and downstream replayers.
//!
//! # Example
//!
//! ```
//! use roko_learn::pattern_discovery::{EpisodeView, PatternMiner};
//!
//! struct Ep { actions: Vec<String>, ok: bool }
//! impl EpisodeView for Ep {
//!     fn actions(&self) -> &[String] { &self.actions }
//!     fn succeeded(&self) -> bool { self.ok }
//! }
//!
//! let mut miner = PatternMiner::new(2, 0.5);
//! let run = |words: &[&str]| Ep {
//!     actions: words.iter().map(|s| (*s).to_string()).collect(),
//!     ok: true,
//! };
//! miner.ingest_episode(&run(&["a", "b", "c", "d"]));
//! miner.ingest_episode(&run(&["a", "b", "c", "e"]));
//! let patterns = miner.discover();
//! assert!(!patterns.is_empty());
//! ```

use std::collections::{BTreeMap, HashSet};

use bardo_primitives::HdcVector;
use serde::{Deserialize, Serialize};

use crate::episode_logger::Episode;
use crate::hdc_clustering::{KMedoidsConfig, k_medoids};

/// Read-only projection of an episode that is sufficient for trigram mining.
///
/// Implementors only need to expose an ordered list of action kinds (as
/// strings) and whether the episode ultimately succeeded. Keeping the trait
/// minimal lets `roko-learn` remain independent of any particular `Episode`
/// type — including [`crate::episode_logger`]'s canonical struct.
pub trait EpisodeView {
    /// Ordered slice of action kind labels recorded during the episode.
    fn actions(&self) -> &[String];
    /// Whether the episode reached a successful terminal state.
    fn succeeded(&self) -> bool;
}

/// A recurring structural signal mined from episode action sequences.
///
/// Patterns carry enough metadata to sort, deduplicate, and audit their
/// origin. `signature` is the deterministic content hash (`fnv1a-64`) of the
/// trigram tuple and is safe to use as a stable identifier across runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    /// Stable string id ("trigram:<signature>").
    pub id: String,
    /// Deterministic 64-bit content hash of the underlying trigram.
    pub signature: u64,
    /// Human-readable rendering of the trigram (e.g. "read → edit → test").
    pub description: String,
    /// Number of distinct episodes that contained the trigram at least once.
    pub support_count: u32,
    /// `support_count / total_episodes`, clamped to `[0.0, 1.0]`.
    pub confidence: f32,
    /// Unix milliseconds of the first episode that contained the trigram.
    pub first_seen_ms: i64,
    /// Unix milliseconds of the most recent episode that contained it.
    pub last_seen_ms: i64,
}

#[derive(Debug, Clone)]
struct TrigramStats {
    trigram: [String; 3],
    signature: u64,
    support: u32,
    first_seen_ms: i64,
    last_seen_ms: i64,
}

/// Mines recurring action trigrams across a batch of episodes.
///
/// The miner is a pure accumulator: [`PatternMiner::ingest_episode`] updates
/// internal counters, while [`PatternMiner::discover`] projects the current
/// state into a ranked list of [`Pattern`]s. [`PatternMiner::reset`] clears
/// all state so the miner can be reused for a fresh batch.
#[derive(Debug)]
pub struct PatternMiner {
    min_support: u32,
    min_confidence: f32,
    total_episodes: u32,
    stats: BTreeMap<u64, TrigramStats>,
    clock_ms: i64,
}

impl PatternMiner {
    /// Construct a new miner with the given support and confidence floors.
    ///
    /// `min_support` is the minimum number of distinct episodes in which a
    /// trigram must appear. `min_confidence` is the minimum ratio of
    /// `support / total_episodes`, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub const fn new(min_support: u32, min_confidence: f32) -> Self {
        // NaN fails every comparison, so this chain also normalizes it to 0.0.
        let clamped = if min_confidence > 1.0 {
            1.0
        } else if min_confidence > 0.0 {
            min_confidence
        } else {
            0.0
        };
        Self {
            min_support,
            min_confidence: clamped,
            total_episodes: 0,
            stats: BTreeMap::new(),
            clock_ms: 0,
        }
    }

    /// Configured minimum confidence threshold, clamped to `[0.0, 1.0]`.
    #[must_use]
    pub const fn min_confidence(&self) -> f32 {
        self.min_confidence
    }

    /// Configured minimum support count.
    #[must_use]
    pub const fn min_support(&self) -> u32 {
        self.min_support
    }

    /// Ingest a single episode view and update trigram statistics.
    ///
    /// Each distinct trigram contributes at most one to the episode's support
    /// count regardless of how many times it repeats within the episode.
    /// Episodes that are too short to form a trigram still count toward
    /// `total_episodes` (they just don't move any counters).
    pub fn ingest_episode<E: EpisodeView + ?Sized>(&mut self, episode: &E) {
        self.total_episodes = self.total_episodes.saturating_add(1);
        self.clock_ms = self.clock_ms.saturating_add(1);
        let ts = self.clock_ms;

        let actions = episode.actions();
        if actions.len() < 3 {
            return;
        }

        let mut seen: HashSet<u64> = HashSet::new();
        for window in actions.windows(3) {
            let trigram = [window[0].clone(), window[1].clone(), window[2].clone()];
            let signature = hash_trigram(&trigram);
            if !seen.insert(signature) {
                continue;
            }
            self.stats
                .entry(signature)
                .and_modify(|s| {
                    s.support = s.support.saturating_add(1);
                    s.last_seen_ms = ts;
                })
                .or_insert_with(|| TrigramStats {
                    trigram: trigram.clone(),
                    signature,
                    support: 1,
                    first_seen_ms: ts,
                    last_seen_ms: ts,
                });
        }
    }

    /// Snapshot the discovered patterns that pass both thresholds.
    ///
    /// Results are sorted by descending `support_count` and then by
    /// `signature` so the order is stable across identical inputs.
    #[must_use]
    pub fn discover(&self) -> Vec<Pattern> {
        if self.total_episodes == 0 {
            return Vec::new();
        }
        let total_f = ratio_f32(self.total_episodes).max(1.0);

        let mut out: Vec<Pattern> = self
            .stats
            .values()
            .filter(|s| s.support >= self.min_support)
            .filter_map(|s| {
                let support_f = ratio_f32(s.support);
                let confidence = (support_f / total_f).clamp(0.0, 1.0);
                if confidence + f32::EPSILON < self.min_confidence {
                    return None;
                }
                Some(Pattern {
                    id: format!("trigram:{:016x}", s.signature),
                    signature: s.signature,
                    description: format!(
                        "{} -> {} -> {}",
                        s.trigram[0], s.trigram[1], s.trigram[2]
                    ),
                    support_count: s.support,
                    confidence,
                    first_seen_ms: s.first_seen_ms,
                    last_seen_ms: s.last_seen_ms,
                })
            })
            .collect();

        out.sort_by(|a, b| {
            b.support_count
                .cmp(&a.support_count)
                .then_with(|| a.signature.cmp(&b.signature))
        });
        out
    }

    /// Clear every counter so the miner can be reused for a fresh batch.
    pub fn reset(&mut self) {
        self.total_episodes = 0;
        self.stats.clear();
        self.clock_ms = 0;
    }

    /// Number of episodes ingested since the last reset.
    #[must_use]
    pub const fn total_episodes(&self) -> u32 {
        self.total_episodes
    }

    /// Number of distinct trigrams currently tracked.
    #[must_use]
    pub fn distinct_trigrams(&self) -> usize {
        self.stats.len()
    }
}

/// A consolidated structural pattern discovered across multiple unrelated episodes.
///
/// The report intentionally ignores task-specific identifiers such as
/// `task_id` so that structurally similar but otherwise unrelated episodes can
/// cluster together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrossEpisodeMetaPattern {
    /// Stable string id ("meta-pattern:<signature>").
    pub id: String,
    /// Index of the medoid episode within the source slice.
    pub medoid_index: usize,
    /// Indices of all member episodes in the source slice.
    pub episode_indices: Vec<usize>,
    /// Stable episode identifiers for the cluster members.
    pub episode_ids: Vec<String>,
    /// Human-readable summary of the cluster's shared structure.
    pub description: String,
    /// Deterministic 64-bit content hash of the bundled cluster vector.
    pub signature: u64,
    /// HDC bundle of all member episode vectors.
    pub bundle_vector: HdcVector,
    /// The medoid vector selected by k-medoids.
    pub medoid_vector: HdcVector,
    /// Mean similarity of cluster members to the bundled vector.
    pub coherence: f32,
}

/// Summary of a cross-episode consolidation pass.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CrossEpisodeConsolidationReport {
    /// Total number of episodes supplied to the pass.
    pub total_episodes: usize,
    /// Number of meta-patterns that survived the size and coherence filters.
    pub meta_pattern_count: usize,
    /// Number of assign/update iterations executed by k-medoids.
    pub iterations: usize,
    /// Whether k-medoids converged before hitting its iteration cap.
    pub converged: bool,
    /// Discovered cross-episode meta-patterns sorted by support descending.
    pub meta_patterns: Vec<CrossEpisodeMetaPattern>,
}

/// Batch consolidator that discovers structural meta-patterns across episodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrossEpisodeConsolidator {
    target_clusters: usize,
    min_cluster_size: usize,
    max_iterations: usize,
    min_coherence: f32,
}

impl Default for CrossEpisodeConsolidator {
    fn default() -> Self {
        Self {
            target_clusters: 4,
            min_cluster_size: 2,
            max_iterations: 50,
            min_coherence: 0.55,
        }
    }
}

impl CrossEpisodeConsolidator {
    /// Construct a consolidator with explicit clustering and filtering knobs.
    ///
    /// `target_clusters` and `min_cluster_size` are clamped to at least 1.
    /// `min_coherence` is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub const fn new(
        target_clusters: usize,
        min_cluster_size: usize,
        max_iterations: usize,
        min_coherence: f32,
    ) -> Self {
        let target_clusters = if target_clusters == 0 {
            1
        } else {
            target_clusters
        };
        let min_cluster_size = if min_cluster_size == 0 {
            1
        } else {
            min_cluster_size
        };
        let max_iterations = if max_iterations == 0 {
            1
        } else {
            max_iterations
        };
        let min_coherence = if min_coherence > 1.0 {
            1.0
        } else if min_coherence > 0.0 {
            min_coherence
        } else {
            0.0
        };
        Self {
            target_clusters,
            min_cluster_size,
            max_iterations,
            min_coherence,
        }
    }

    /// Discover cross-episode meta-patterns from a completed batch.
    #[must_use]
    pub fn discover(&self, episodes: &[Episode]) -> CrossEpisodeConsolidationReport {
        let vectors: Vec<HdcVector> = episodes.iter().map(episode_vector).collect();
        if vectors.is_empty() {
            return CrossEpisodeConsolidationReport::default();
        }

        let cluster_count = self.target_clusters.min(vectors.len()).max(1);
        let result = k_medoids(
            &vectors,
            &KMedoidsConfig {
                k: cluster_count,
                max_iterations: self.max_iterations,
            },
        );

        let mut meta_patterns = Vec::new();
        for cluster in result.clusters {
            if cluster.members.len() < self.min_cluster_size {
                continue;
            }

            let bundle_refs: Vec<&HdcVector> = cluster
                .members
                .iter()
                .map(|&index| &vectors[index])
                .collect();
            let bundle_vector = HdcVector::bundle(&bundle_refs);
            let coherence = cluster_coherence(&bundle_vector, &cluster.members, &vectors);
            if coherence + f32::EPSILON < self.min_coherence {
                continue;
            }

            let member_episodes: Vec<&Episode> = cluster
                .members
                .iter()
                .map(|&index| &episodes[index])
                .collect();
            let description = summarize_cluster(&member_episodes);
            let episode_ids = member_episodes
                .iter()
                .map(|episode| episode_identity(episode))
                .collect();
            let signature = vector_signature(&bundle_vector);

            meta_patterns.push(CrossEpisodeMetaPattern {
                id: format!("meta-pattern:{signature:016x}"),
                medoid_index: cluster.medoid_index,
                episode_indices: cluster.members,
                episode_ids,
                description,
                signature,
                bundle_vector,
                medoid_vector: cluster.medoid,
                coherence,
            });
        }

        meta_patterns.sort_by(|left, right| {
            right
                .episode_indices
                .len()
                .cmp(&left.episode_indices.len())
                .then_with(|| left.signature.cmp(&right.signature))
        });

        CrossEpisodeConsolidationReport {
            total_episodes: episodes.len(),
            meta_pattern_count: meta_patterns.len(),
            iterations: result.iterations,
            converged: result.converged,
            meta_patterns,
        }
    }
}

fn episode_vector(episode: &Episode) -> HdcVector {
    let kind = field_vector("kind", normalized(&episode.kind));
    let template = field_vector("agent_template", normalized(&episode.agent_template));
    let model = field_vector("model", normalized(&episode.model));
    let trigger = field_vector("trigger_kind", normalized(&episode.trigger_kind));
    let role = field_vector("role", episode_extra_string(episode, "role"));
    let task_category = field_vector(
        "task_category",
        episode_extra_string(episode, "task_category"),
    );
    let complexity_band = field_vector(
        "complexity_band",
        episode_extra_string(episode, "complexity_band"),
    );
    let outcome = field_vector(
        "outcome",
        if episode.success {
            "success"
        } else {
            "failure"
        },
    );
    let gate_signature = field_vector("gate_signature", gate_signature(episode));
    let failure_reason = field_vector(
        "failure_reason",
        normalized(episode.failure_reason.as_deref().unwrap_or("")),
    );

    HdcVector::bundle(&[
        &kind,
        &template,
        &template,
        &model,
        &model,
        &trigger,
        &role,
        &task_category,
        &complexity_band,
        &outcome,
        &gate_signature,
        &failure_reason,
    ])
}

fn field_vector(label: &str, value: impl Into<String>) -> HdcVector {
    let value = value.into();
    HdcVector::from_seed(format!("{label}={value}").as_bytes())
}

fn normalized(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "∅".to_string()
    } else {
        trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn episode_extra_string(episode: &Episode, key: &str) -> String {
    episode
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(normalized)
        .unwrap_or_else(|| "∅".to_string())
}

fn gate_signature(episode: &Episode) -> String {
    if episode.gate_verdicts.is_empty() {
        return "none".to_string();
    }

    episode
        .gate_verdicts
        .iter()
        .map(|verdict| {
            format!(
                "{}:{}",
                verdict.gate,
                if verdict.passed { "pass" } else { "fail" }
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn episode_identity(episode: &Episode) -> String {
    let episode_id = normalized(&episode.episode_id);
    if episode_id != "∅" {
        return episode_id;
    }

    let id = normalized(&episode.id);
    if id != "∅" {
        return id;
    }

    format!(
        "{}@{}",
        normalized(&episode.agent_id),
        episode.timestamp.timestamp_millis()
    )
}

fn summarize_cluster(episodes: &[&Episode]) -> String {
    let mut parts = Vec::new();
    summarize_majority_field(&mut parts, "kind", episodes, |episode| {
        normalized(&episode.kind)
    });
    summarize_majority_field(&mut parts, "agent_template", episodes, |episode| {
        normalized(&episode.agent_template)
    });
    summarize_majority_field(&mut parts, "model", episodes, |episode| {
        normalized(&episode.model)
    });
    summarize_majority_field(&mut parts, "trigger_kind", episodes, |episode| {
        normalized(&episode.trigger_kind)
    });
    summarize_majority_field(&mut parts, "role", episodes, |episode| {
        episode_extra_string(episode, "role")
    });
    summarize_majority_field(&mut parts, "task_category", episodes, |episode| {
        episode_extra_string(episode, "task_category")
    });
    summarize_majority_field(&mut parts, "complexity_band", episodes, |episode| {
        episode_extra_string(episode, "complexity_band")
    });
    summarize_majority_field(&mut parts, "outcome", episodes, |episode| {
        if episode.success {
            "success".to_string()
        } else {
            "failure".to_string()
        }
    });

    if parts.is_empty() {
        format!("{} episode cluster", episodes.len())
    } else {
        parts.join("; ")
    }
}

fn summarize_majority_field<F>(
    parts: &mut Vec<String>,
    label: &str,
    episodes: &[&Episode],
    extract: F,
) where
    F: Fn(&Episode) -> String,
{
    if episodes.is_empty() {
        return;
    }

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for episode in episodes {
        let value = extract(episode);
        if value == "∅" {
            continue;
        }
        *counts.entry(value).or_insert(0) += 1;
    }

    let Some((value, count)) = counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
    else {
        return;
    };

    if count * 2 > episodes.len() {
        parts.push(format!("{label}={value} ({count}/{})", episodes.len()));
    }
}

fn cluster_coherence(bundle_vector: &HdcVector, members: &[usize], vectors: &[HdcVector]) -> f32 {
    if members.is_empty() {
        return 0.0;
    }

    let total: f32 = members
        .iter()
        .map(|&index| vectors[index].similarity(bundle_vector))
        .sum();
    total / members.len() as f32
}

fn vector_signature(vector: &HdcVector) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in vector.to_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Convert a u32 count to f32. f32 exactly represents integers up to `2^24`.
/// Counts beyond that round down by at most a few ULPs — acceptable for
/// confidence ratios used only for filtering.
#[allow(clippy::cast_precision_loss)]
const fn ratio_f32(n: u32) -> f32 {
    n as f32
}

/// Deterministic 64-bit FNV-1a hash of a trigram tuple.
///
/// We roll our own tiny hasher rather than depend on `std::hash::Hasher`
/// default output (which varies across toolchain versions) so that
/// `Pattern::signature` values are stable across processes and platforms.
fn hash_trigram(trigram: &[String; 3]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = FNV_OFFSET;
    for part in trigram {
        for byte in part.as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(FNV_PRIME);
        }
        // Delimiter byte so "ab|c" and "a|bc" hash differently.
        h ^= 0x1f;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ep {
        actions: Vec<String>,
        ok: bool,
    }

    impl EpisodeView for Ep {
        fn actions(&self) -> &[String] {
            &self.actions
        }
        fn succeeded(&self) -> bool {
            self.ok
        }
    }

    fn ep(words: &[&str], ok: bool) -> Ep {
        Ep {
            actions: words.iter().map(|s| (*s).to_string()).collect(),
            ok,
        }
    }

    #[test]
    fn empty_input_returns_no_patterns() {
        let miner = PatternMiner::new(1, 0.0);
        assert!(miner.discover().is_empty());
        assert_eq!(miner.total_episodes(), 0);
        assert_eq!(miner.distinct_trigrams(), 0);
    }

    #[test]
    fn single_episode_short_sequence_has_no_trigrams() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["read", "edit"], true));
        assert_eq!(miner.total_episodes(), 1);
        assert_eq!(miner.distinct_trigrams(), 0);
        assert!(miner.discover().is_empty());
    }

    #[test]
    fn single_episode_one_trigram_recorded() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["read", "edit", "test"], true));
        assert_eq!(miner.distinct_trigrams(), 1);
        let patterns = miner.discover();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].support_count, 1);
        assert!((patterns[0].confidence - 1.0).abs() < 1e-6);
        assert_eq!(patterns[0].description, "read -> edit -> test");
        assert!(patterns[0].id.starts_with("trigram:"));
    }

    #[test]
    fn trigram_above_min_support_is_emitted() {
        let mut miner = PatternMiner::new(2, 0.0);
        miner.ingest_episode(&ep(&["read", "edit", "test", "commit"], true));
        miner.ingest_episode(&ep(&["read", "edit", "test", "revert"], true));
        miner.ingest_episode(&ep(&["plan", "spike", "abandon"], false));
        let patterns = miner.discover();
        assert!(
            patterns
                .iter()
                .any(|p| p.description == "read -> edit -> test")
        );
        assert!(
            patterns
                .iter()
                .find(|p| p.description == "read -> edit -> test")
                .is_some_and(|p| p.support_count == 2)
        );
    }

    #[test]
    fn trigram_below_min_support_is_rejected() {
        let mut miner = PatternMiner::new(3, 0.0);
        miner.ingest_episode(&ep(&["a", "b", "c", "d"], true));
        miner.ingest_episode(&ep(&["a", "b", "c", "e"], true));
        let patterns = miner.discover();
        assert!(patterns.is_empty());
    }

    #[test]
    fn confidence_threshold_filters_rare_patterns() {
        let mut miner = PatternMiner::new(1, 0.5);
        // trigram "a->b->c" appears in 1 of 4 episodes → confidence 0.25 < 0.5
        miner.ingest_episode(&ep(&["a", "b", "c"], true));
        miner.ingest_episode(&ep(&["x", "y", "z"], true));
        miner.ingest_episode(&ep(&["x", "y", "z"], true));
        miner.ingest_episode(&ep(&["x", "y", "z"], true));
        let patterns = miner.discover();
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].description, "x -> y -> z");
        assert!(patterns[0].confidence >= 0.5);
        assert!(patterns.iter().all(|p| p.description != "a -> b -> c"));
    }

    #[test]
    fn reset_clears_all_state() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["a", "b", "c"], true));
        miner.ingest_episode(&ep(&["a", "b", "c"], true));
        assert_eq!(miner.total_episodes(), 2);
        assert_eq!(miner.distinct_trigrams(), 1);
        miner.reset();
        assert_eq!(miner.total_episodes(), 0);
        assert_eq!(miner.distinct_trigrams(), 0);
        assert!(miner.discover().is_empty());
    }

    #[test]
    fn multiple_patterns_ordered_by_support_desc() {
        let mut miner = PatternMiner::new(1, 0.0);
        // "x y z" appears in 3 episodes, "a b c" in 2, "p q r" in 1.
        for _ in 0..3 {
            miner.ingest_episode(&ep(&["x", "y", "z"], true));
        }
        for _ in 0..2 {
            miner.ingest_episode(&ep(&["a", "b", "c"], true));
        }
        miner.ingest_episode(&ep(&["p", "q", "r"], false));
        let patterns = miner.discover();
        assert_eq!(patterns.len(), 3);
        assert_eq!(patterns[0].description, "x -> y -> z");
        assert_eq!(patterns[0].support_count, 3);
        assert_eq!(patterns[1].description, "a -> b -> c");
        assert_eq!(patterns[1].support_count, 2);
        assert_eq!(patterns[2].description, "p -> q -> r");
        assert_eq!(patterns[2].support_count, 1);
        // Support-desc implies non-increasing counts across the list.
        for pair in patterns.windows(2) {
            assert!(pair[0].support_count >= pair[1].support_count);
        }
    }

    #[test]
    fn repeated_trigram_in_one_episode_counts_once() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["a", "b", "c", "a", "b", "c"], true));
        miner.ingest_episode(&ep(&["q", "r", "s"], false));
        let patterns = miner.discover();
        let abc = patterns
            .iter()
            .find(|p| p.description == "a -> b -> c")
            .expect("a->b->c present");
        assert_eq!(abc.support_count, 1);
    }

    #[test]
    fn first_and_last_seen_track_ingestion_order() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["x", "y", "z"], true));
        miner.ingest_episode(&ep(&["p", "q", "r"], true));
        miner.ingest_episode(&ep(&["x", "y", "z"], true));
        let xyz = miner
            .discover()
            .into_iter()
            .find(|p| p.description == "x -> y -> z")
            .expect("x->y->z present");
        assert!(xyz.first_seen_ms < xyz.last_seen_ms);
        assert_eq!(xyz.first_seen_ms, 1);
        assert_eq!(xyz.last_seen_ms, 3);
    }

    #[test]
    fn signatures_are_deterministic_and_distinct() {
        let t1 = ["read".to_string(), "edit".to_string(), "test".to_string()];
        let t2 = ["read".to_string(), "edit".to_string(), "test".to_string()];
        let t3 = ["read".to_string(), "test".to_string(), "edit".to_string()];
        assert_eq!(hash_trigram(&t1), hash_trigram(&t2));
        assert_ne!(hash_trigram(&t1), hash_trigram(&t3));
    }

    #[test]
    fn nan_and_extreme_confidence_thresholds_are_clamped() {
        let nan_miner = PatternMiner::new(1, f32::NAN);
        assert!((nan_miner.min_confidence - 0.0).abs() < f32::EPSILON);
        let hi_miner = PatternMiner::new(1, 5.0);
        assert!((hi_miner.min_confidence - 1.0).abs() < f32::EPSILON);
        let lo_miner = PatternMiner::new(1, -1.0);
        assert!((lo_miner.min_confidence - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn pattern_serde_roundtrip() {
        let mut miner = PatternMiner::new(1, 0.0);
        miner.ingest_episode(&ep(&["a", "b", "c"], true));
        let original = miner.discover().into_iter().next().expect("one pattern");
        let json = serde_json::to_string(&original).expect("serialize");
        let decoded: Pattern = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(original, decoded);
    }

    fn structural_episode(
        agent_id: &str,
        task_id: &str,
        agent_template: &str,
        model: &str,
        trigger_kind: &str,
        task_category: &str,
        complexity_band: &str,
        success: bool,
        gate_names: &[&str],
        failure_reason: Option<&str>,
    ) -> Episode {
        let mut episode = Episode::new(agent_id, task_id);
        episode.agent_template = agent_template.to_string();
        episode.model = model.to_string();
        episode.trigger_kind = trigger_kind.to_string();
        episode.success = success;
        if let Some(reason) = failure_reason {
            episode.failure_reason = Some(reason.to_string());
        }
        episode.extra.insert(
            "task_category".to_string(),
            serde_json::Value::String(task_category.to_string()),
        );
        episode.extra.insert(
            "complexity_band".to_string(),
            serde_json::Value::String(complexity_band.to_string()),
        );
        episode.gate_verdicts = gate_names
            .iter()
            .map(|gate| crate::episode_logger::GateVerdict::new(gate, success))
            .collect();
        episode
    }

    #[test]
    fn cross_episode_consolidation_groups_structurally_similar_episodes() {
        let mut episodes = Vec::new();

        for i in 0..3 {
            episodes.push(structural_episode(
                "agent-a",
                &format!("task-a-{i}"),
                "code-implementer",
                "claude-sonnet",
                "plan_run",
                "implementation",
                "standard",
                true,
                &["compile", "test"],
                None,
            ));
        }

        for i in 0..3 {
            episodes.push(structural_episode(
                "agent-b",
                &format!("task-b-{i}"),
                "researcher",
                "claude-haiku",
                "review_queue",
                "research",
                "light",
                false,
                &["lint", "inspect"],
                Some("rate_limit"),
            ));
        }

        let consolidator = CrossEpisodeConsolidator::new(2, 2, 50, 0.55);
        let report = consolidator.discover(&episodes);

        assert_eq!(report.total_episodes, 6);
        assert_eq!(report.meta_pattern_count, 2);
        assert!(report.converged);
        assert!(report.iterations > 0);

        let implementation = report
            .meta_patterns
            .iter()
            .find(|pattern| pattern.description.contains("code-implementer"))
            .expect("implementation cluster");
        let research = report
            .meta_patterns
            .iter()
            .find(|pattern| pattern.description.contains("researcher"))
            .expect("research cluster");

        assert_eq!(implementation.episode_indices.len(), 3);
        assert_eq!(research.episode_indices.len(), 3);
        assert!(implementation.coherence >= 0.55);
        assert!(research.coherence >= 0.55);
        assert!(
            implementation
                .description
                .contains("agent_template=code-implementer")
        );
        assert!(research.description.contains("agent_template=researcher"));
        assert!(
            implementation
                .episode_ids
                .iter()
                .all(|id| id.starts_with("ep_"))
        );
    }

    #[test]
    fn cross_episode_consolidation_filters_singleton_clusters() {
        let episodes = vec![structural_episode(
            "agent-a",
            "task-a",
            "code-implementer",
            "claude-sonnet",
            "plan_run",
            "implementation",
            "standard",
            true,
            &["compile", "test"],
            None,
        )];

        let consolidator = CrossEpisodeConsolidator::new(1, 2, 10, 0.0);
        let report = consolidator.discover(&episodes);
        assert_eq!(report.total_episodes, 1);
        assert_eq!(report.meta_pattern_count, 0);
        assert!(report.meta_patterns.is_empty());
    }
}
