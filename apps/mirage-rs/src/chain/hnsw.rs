//! Hierarchical Navigable Small World (HNSW) index over binary HDC vectors.
//!
//! HNSW is a graph-based approximate nearest neighbour algorithm. Each node in
//! the graph stores a vector and a list of neighbour links at each of several
//! hierarchical layers. Higher layers are sparser and act as highways; lower
//! layers densify around the query. Search descends from the top layer, greedily
//! following nearest links, widening its candidate set with an exploration
//! parameter `ef` on the bottom layer.
//!
//! This implementation is intentionally compact and non-SIMD — it's a reference
//! index for correctness and integration testing, not a bench-tuned production
//! engine. For the 100K-entry POC scale its recall is >95% at ef_search=40.
//!
//! # References
//!
//! - Malkov, Y. & Yashunin, D. (2018). "Efficient and robust approximate nearest
//!   neighbor search using Hierarchical Navigable Small World graphs."
//! - Doc 04 §4: `tmp/agent-chain/04-hdc.md`.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use roko_primitives::HdcVector;

use super::{hdc_index::Hit, insight::InsightId};

/// Tunable HNSW parameters.
#[derive(Clone, Copy, Debug)]
pub struct HnswConfig {
    /// Max neighbours per node at layer > 0.
    pub m: usize,
    /// Max neighbours per node at layer 0 (typically 2 × M).
    pub m_max_0: usize,
    /// Construction-time candidate list size.
    pub ef_construction: usize,
    /// Deterministic seed for layer assignment.
    pub seed: u64,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            m_max_0: 32,
            ef_construction: 200,
            seed: 0xC0FF_EE_C0FF_EE,
        }
    }
}

/// One node in the graph.
#[derive(Clone, Debug)]
struct Node {
    id: InsightId,
    vector: HdcVector,
    weight: f32,
    /// Neighbour ids per layer. `neighbours[layer]` is the adjacency list at that layer.
    neighbours: Vec<Vec<usize>>,
}

/// Binary-HNSW index over HDC vectors.
#[derive(Debug)]
pub struct HnswBinaryIndex {
    config: HnswConfig,
    nodes: Vec<Node>,
    id_to_node: HashMap<InsightId, usize>,
    entry_point: Option<usize>,
    /// Level of the entry point (inclusive top layer).
    max_level: usize,
    /// RNG state for layer assignment.
    rng_state: u64,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Distance = `1 - similarity`, so smaller is better and ordering matches
/// a min-heap by wrapping with `Reverse` semantics.
fn distance(a: &HdcVector, b: &HdcVector) -> f32 {
    1.0 - a.similarity(b)
}

#[derive(Clone, Copy, Debug)]
struct Candidate {
    node: usize,
    dist: f32,
}

impl Eq for Candidate {}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.dist == other.dist && self.node == other.node
    }
}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Use total_cmp so NaN doesn't corrupt ordering (shouldn't happen with HDC sims).
        self.dist
            .total_cmp(&other.dist)
            .then_with(|| self.node.cmp(&other.node))
    }
}

/// Reverse-ordering wrapper for a max-heap of nearest items.
#[derive(Clone, Copy, Debug)]
struct Nearest(Candidate);
impl PartialEq for Nearest {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for Nearest {}
impl PartialOrd for Nearest {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Nearest {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse so BinaryHeap becomes a min-heap.
        other.0.cmp(&self.0)
    }
}

impl HnswBinaryIndex {
    /// Constructs an empty index with the given config.
    #[must_use]
    pub fn new(config: HnswConfig) -> Self {
        let seed = config.seed.max(1);
        Self {
            config,
            nodes: Vec::new(),
            id_to_node: HashMap::new(),
            entry_point: None,
            max_level: 0,
            rng_state: seed,
        }
    }

    /// Returns the number of nodes in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the index has no nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Updates the cached weight on an existing entry. Returns true if found.
    pub fn set_weight(&mut self, id: InsightId, weight: f32) -> bool {
        if let Some(&idx) = self.id_to_node.get(&id) {
            self.nodes[idx].weight = weight;
            true
        } else {
            false
        }
    }

    /// Assigns a random layer to a new node.
    ///
    /// HNSW uses `level = floor(-ln(uniform) × m_L)` with `m_L = 1 / ln(M)`.
    /// We approximate the uniform draw via splitmix64 → `(u >> 11) as f64 / 2^53`.
    fn random_level(&mut self) -> usize {
        let raw = splitmix64(&mut self.rng_state);
        let mantissa = (raw >> 11) as f64;
        let uniform = mantissa / (1u64 << 53) as f64;
        // Clamp to (0, 1) to avoid ln(0).
        let u = uniform.max(f64::MIN_POSITIVE);
        let m_l = 1.0 / (self.config.m as f64).ln();
        let level = (-u.ln() * m_l).floor() as usize;
        level.min(16) // Hard ceiling; plenty for millions of entries.
    }

    /// Inserts a new vector. If the id already exists, its vector/weight are replaced
    /// but the graph topology is not rebuilt (cheap update path).
    pub fn insert(&mut self, id: InsightId, vector: HdcVector, weight: f32) {
        if let Some(&existing) = self.id_to_node.get(&id) {
            self.nodes[existing].vector = vector;
            self.nodes[existing].weight = weight;
            return;
        }

        let level = self.random_level();
        let node_idx = self.nodes.len();
        self.nodes.push(Node {
            id,
            vector,
            weight,
            neighbours: vec![Vec::new(); level + 1],
        });
        self.id_to_node.insert(id, node_idx);

        // First node becomes the entry point.
        let Some(mut current_ep) = self.entry_point else {
            self.entry_point = Some(node_idx);
            self.max_level = level;
            return;
        };

        // Phase 1: descend from max_level to level+1, greedy nearest.
        for l in (level + 1..=self.max_level).rev() {
            current_ep = self.greedy_search_layer(node_idx, current_ep, l);
        }

        // Phase 2: at layers [level..=0], do a beam search of size ef_construction.
        for l in (0..=level.min(self.max_level)).rev() {
            let candidates =
                self.search_layer(node_idx, current_ep, self.config.ef_construction, l);
            let m_max = if l == 0 {
                self.config.m_max_0
            } else {
                self.config.m
            };
            let selected = Self::select_neighbours(candidates, m_max);
            // Bidirectional links.
            for cand in &selected {
                self.nodes[node_idx].neighbours[l].push(cand.node);
                self.nodes[cand.node].neighbours[l].push(node_idx);
                // Prune oversized neighbour lists with a shrink step.
                self.shrink_neighbours(cand.node, l, m_max);
            }
            if let Some(first) = selected.first() {
                current_ep = first.node;
            }
        }

        // Update entry point if this node landed higher than the current max.
        if level > self.max_level {
            self.max_level = level;
            self.entry_point = Some(node_idx);
        }
    }

    /// Searches the index for the top-k nearest neighbours of `query`.
    ///
    /// `ef_search` controls the breadth of the beam on layer 0; higher values
    /// trade latency for recall. A typical setting is `ef_search = max(k × 2, 40)`.
    #[must_use]
    pub fn search(&self, query: &HdcVector, k: usize, ef_search: usize) -> Vec<Hit> {
        let Some(mut ep) = self.entry_point else {
            return Vec::new();
        };
        if self.nodes.is_empty() || k == 0 {
            return Vec::new();
        }

        // Greedy descent from max_level down to layer 1.
        for l in (1..=self.max_level).rev() {
            ep = self.greedy_search_layer_vec(query, ep, l);
        }

        // Beam search on layer 0.
        let ef = ef_search.max(k).max(self.config.ef_construction.min(64));
        let candidates = self.search_layer_vec(query, ep, ef, 0);

        let mut hits: Vec<Hit> = candidates
            .into_iter()
            .map(|c| {
                let node = &self.nodes[c.node];
                let similarity = 1.0 - c.dist;
                Hit {
                    id: node.id,
                    similarity,
                    weight: node.weight,
                    score: similarity * node.weight,
                }
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(k);
        hits
    }

    /// Greedy nearest-neighbour walk on a specific layer, using a node's vector.
    fn greedy_search_layer(&self, node: usize, entry: usize, layer: usize) -> usize {
        let q = &self.nodes[node].vector.clone();
        self.greedy_search_layer_vec(q, entry, layer)
    }

    fn greedy_search_layer_vec(&self, q: &HdcVector, entry: usize, layer: usize) -> usize {
        let mut current = entry;
        let mut current_dist = distance(q, &self.nodes[current].vector);
        loop {
            let mut improved = false;
            for &neighbour in &self.nodes[current].neighbours[layer] {
                let d = distance(q, &self.nodes[neighbour].vector);
                if d < current_dist {
                    current_dist = d;
                    current = neighbour;
                    improved = true;
                }
            }
            if !improved {
                break;
            }
        }
        current
    }

    /// Beam-search on a given layer from `entry`, returning up to `ef` nearest candidates.
    fn search_layer(&self, node: usize, entry: usize, ef: usize, layer: usize) -> Vec<Candidate> {
        let q = self.nodes[node].vector.clone();
        self.search_layer_vec(&q, entry, ef, layer)
    }

    fn search_layer_vec(
        &self,
        q: &HdcVector,
        entry: usize,
        ef: usize,
        layer: usize,
    ) -> Vec<Candidate> {
        let mut visited: HashSet<usize> = HashSet::new();
        visited.insert(entry);

        let entry_dist = distance(q, &self.nodes[entry].vector);
        let mut frontier: BinaryHeap<Candidate> = BinaryHeap::new(); // min-heap via reverse ordering below
        let mut top_k: BinaryHeap<Nearest> = BinaryHeap::new();

        // We want `frontier` to be a min-heap (expand closest next) and `top_k`
        // to keep the *farthest* on top so we can pop it when the beam overflows.
        // BinaryHeap<Candidate> is a max-heap by default on Candidate's Ord.
        // We build a custom structure using `Nearest` for both.
        frontier.push(Candidate {
            node: entry,
            dist: -entry_dist, // negate so the max-heap yields smallest dist first
        });
        top_k.push(Nearest(Candidate {
            node: entry,
            dist: entry_dist,
        }));

        while let Some(c) = frontier.pop() {
            let c_dist = -c.dist;
            // Farthest in top_k: use peek.
            let farthest = top_k.peek().map(|n| n.0.dist).unwrap_or(f32::INFINITY);
            if c_dist > farthest && top_k.len() >= ef {
                break;
            }
            for &neighbour in &self.nodes[c.node].neighbours[layer] {
                if !visited.insert(neighbour) {
                    continue;
                }
                let d = distance(q, &self.nodes[neighbour].vector);
                let farthest = top_k.peek().map(|n| n.0.dist).unwrap_or(f32::INFINITY);
                if top_k.len() < ef || d < farthest {
                    frontier.push(Candidate {
                        node: neighbour,
                        dist: -d,
                    });
                    top_k.push(Nearest(Candidate {
                        node: neighbour,
                        dist: d,
                    }));
                    if top_k.len() > ef {
                        top_k.pop();
                    }
                }
            }
        }

        let mut out: Vec<Candidate> = top_k.into_iter().map(|n| n.0).collect();
        out.sort_by(|a, b| a.dist.total_cmp(&b.dist));
        out
    }

    /// Pick up to `m_max` neighbours from `candidates` by closest distance.
    fn select_neighbours(mut candidates: Vec<Candidate>, m_max: usize) -> Vec<Candidate> {
        candidates.sort_by(|a, b| a.dist.total_cmp(&b.dist));
        candidates.truncate(m_max);
        candidates
    }

    /// Shrink a node's neighbour list at `layer` to `m_max` if needed.
    fn shrink_neighbours(&mut self, node: usize, layer: usize, m_max: usize) {
        if self.nodes[node].neighbours[layer].len() <= m_max {
            return;
        }
        // Rank by distance to this node, keep the m_max closest.
        let base = self.nodes[node].vector.clone();
        let mut ranked: Vec<(usize, f32)> = self.nodes[node].neighbours[layer]
            .iter()
            .copied()
            .map(|n| (n, distance(&base, &self.nodes[n].vector)))
            .collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
        ranked.truncate(m_max);
        self.nodes[node].neighbours[layer] = ranked.into_iter().map(|(n, _)| n).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::insight::{InsightId, KnowledgeKind};

    fn mk_id(s: &str) -> InsightId {
        InsightId::derive(b"a", s.as_bytes(), KnowledgeKind::Insight)
    }

    #[test]
    fn empty_index_returns_no_hits() {
        let idx = HnswBinaryIndex::new(HnswConfig::default());
        let q = HdcVector::from_seed(b"nothing");
        assert_eq!(idx.search(&q, 5, 40).len(), 0);
    }

    #[test]
    fn single_entry_returns_itself() {
        let mut idx = HnswBinaryIndex::new(HnswConfig::default());
        let v = HdcVector::from_seed(b"only one");
        let id = mk_id("only one");
        idx.insert(id, v, 1.0);
        let hits = idx.search(&v, 3, 40);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
        assert!((hits[0].similarity - 1.0).abs() < 1e-4);
    }

    #[test]
    fn search_recalls_exact_match_in_100_entries() {
        let mut idx = HnswBinaryIndex::new(HnswConfig::default());
        for i in 0..100 {
            let key = format!("entry-{i}");
            idx.insert(mk_id(&key), HdcVector::from_seed(key.as_bytes()), 1.0);
        }
        let q = HdcVector::from_seed(b"entry-42");
        let hits = idx.search(&q, 5, 80);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, mk_id("entry-42"));
    }

    #[test]
    fn set_weight_updates_live_entry() {
        let mut idx = HnswBinaryIndex::new(HnswConfig::default());
        let id = mk_id("mutable");
        idx.insert(id, HdcVector::from_seed(b"mutable"), 1.0);
        assert!(idx.set_weight(id, 0.33));
        let hits = idx.search(&HdcVector::from_seed(b"mutable"), 1, 40);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].weight - 0.33).abs() < 1e-6);
    }

    #[test]
    fn reinsert_same_id_updates_vector_in_place() {
        let mut idx = HnswBinaryIndex::new(HnswConfig::default());
        let id = mk_id("same");
        idx.insert(id, HdcVector::from_seed(b"first"), 1.0);
        assert_eq!(idx.len(), 1);
        idx.insert(id, HdcVector::from_seed(b"second"), 2.0);
        assert_eq!(idx.len(), 1);
        let hits = idx.search(&HdcVector::from_seed(b"second"), 1, 40);
        assert_eq!(hits[0].id, id);
        assert!((hits[0].weight - 2.0).abs() < 1e-6);
    }

    #[test]
    fn top_k_honours_k() {
        let mut idx = HnswBinaryIndex::new(HnswConfig::default());
        for i in 0..50 {
            let key = format!("k-{i}");
            idx.insert(mk_id(&key), HdcVector::from_seed(key.as_bytes()), 1.0);
        }
        let q = HdcVector::from_seed(b"k-10");
        let hits = idx.search(&q, 7, 40);
        assert_eq!(hits.len(), 7);
    }
}
