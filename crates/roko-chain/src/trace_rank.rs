//! TraceRank: Graph-based reputation from payment edges.
//!
//! Implements a PageRank-style reputation propagation algorithm over the
//! agent payment graph. Agents who receive payments from highly-reputed
//! agents (and deliver quality work) inherit trust transitively.
//!
//! ## Algorithm
//!
//! TraceRank is the agent-chain equivalent of PageRank applied to payment edges:
//!
//! 1. Build a directed weighted graph where edges represent payments:
//!    - Edge `A -> B` means agent A paid agent B for a job
//!    - Edge weight = payment amount * quality score
//!
//! 2. Run iterative power iteration until convergence:
//!    ```text
//!    rank[B] = (1 - damping) / N + damping * sum(rank[A] * weight(A->B) / out_weight(A))
//!    ```
//!
//! 3. Converged ranks represent transitive trust scores in [0, 1].
//!
//! ## Properties
//!
//! - **Sybil resistance**: Creating fake agents without real payment flow doesn't help
//! - **Quality-weighted**: Low-quality payments (poor deliverables) don't propagate trust
//! - **Convergent**: Guaranteed to converge (stochastic matrix + teleportation)
//! - **Composable**: TraceRank scores can be blended with direct EMA reputation
//!
//! ## Integration
//!
//! TraceRank supplements the direct EMA reputation in `ReputationRegistry`:
//! ```text
//! effective_reputation = (1 - trace_weight) * ema_score + trace_weight * trace_rank
//! ```
//! Default `trace_weight = 0.3` — direct observations dominate, graph signals supplement.

use std::collections::HashMap;

use crate::phase2::u256;

/// A directed payment edge in the agent graph.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentEdge {
    /// Paying agent (job poster).
    pub from: u256,
    /// Receiving agent (job executor).
    pub to: u256,
    /// Payment amount in smallest unit.
    pub amount: f64,
    /// Quality score of the delivered work [0.0, 1.0].
    pub quality: f64,
    /// Block number when payment was made.
    pub block: u64,
}

impl PaymentEdge {
    /// Effective weight of this edge (amount * quality).
    pub fn weight(&self) -> f64 {
        self.amount * self.quality
    }
}

/// Configuration for TraceRank computation.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRankConfig {
    /// Damping factor (probability of following an edge vs. teleporting).
    /// Standard PageRank uses 0.85. Higher = more influenced by graph structure.
    pub damping: f64,
    /// Maximum iterations before declaring non-convergence.
    pub max_iterations: usize,
    /// Convergence threshold (max change in any rank between iterations).
    pub convergence_threshold: f64,
    /// Minimum edge weight to include (filters dust payments).
    pub min_edge_weight: f64,
    /// How far back (in blocks) to consider edges. 0 = all history.
    pub lookback_blocks: u64,
    /// Weight of TraceRank in the blended reputation score.
    pub blend_weight: f64,
}

impl Default for TraceRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            max_iterations: 100,
            convergence_threshold: 1e-6,
            min_edge_weight: 0.01,
            lookback_blocks: 0,
            blend_weight: 0.3,
        }
    }
}

/// Result of a TraceRank computation.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRankResult {
    /// Converged rank scores per agent in [0, 1], normalized to sum to 1.
    pub ranks: HashMap<u256, f64>,
    /// Number of iterations until convergence.
    pub iterations: usize,
    /// Whether the algorithm converged within max_iterations.
    pub converged: bool,
    /// Maximum rank change on the final iteration.
    pub final_delta: f64,
    /// Number of agents in the graph.
    pub node_count: usize,
    /// Number of edges in the graph.
    pub edge_count: usize,
}

/// TraceRank engine for computing graph-based reputation.
#[derive(Debug, Clone)]
pub struct TraceRank {
    /// Configuration.
    pub config: TraceRankConfig,
    /// Payment edges (the graph).
    edges: Vec<PaymentEdge>,
}

impl TraceRank {
    /// Create a new TraceRank engine with default configuration.
    pub fn new() -> Self {
        Self {
            config: TraceRankConfig::default(),
            edges: Vec::new(),
        }
    }

    /// Create with custom configuration.
    pub fn with_config(config: TraceRankConfig) -> Self {
        Self {
            config,
            edges: Vec::new(),
        }
    }

    /// Record a payment edge.
    pub fn record_payment(&mut self, edge: PaymentEdge) {
        if edge.weight() >= self.config.min_edge_weight {
            self.edges.push(edge);
        }
    }

    /// Number of recorded edges.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Compute TraceRank scores for all agents in the graph.
    ///
    /// Uses power iteration with teleportation (standard PageRank algorithm).
    pub fn compute(&self) -> TraceRankResult {
        let current_block = self.edges.iter().map(|e| e.block).max().unwrap_or(0);

        // Filter edges by lookback window.
        let active_edges: Vec<&PaymentEdge> = if self.config.lookback_blocks > 0 {
            let min_block = current_block.saturating_sub(self.config.lookback_blocks);
            self.edges.iter().filter(|e| e.block >= min_block).collect()
        } else {
            self.edges.iter().collect()
        };

        if active_edges.is_empty() {
            return TraceRankResult {
                ranks: HashMap::new(),
                iterations: 0,
                converged: true,
                final_delta: 0.0,
                node_count: 0,
                edge_count: 0,
            };
        }

        // Collect all unique nodes.
        let mut nodes: Vec<u256> = Vec::new();
        for edge in &active_edges {
            if !nodes.contains(&edge.from) {
                nodes.push(edge.from);
            }
            if !nodes.contains(&edge.to) {
                nodes.push(edge.to);
            }
        }
        let n = nodes.len();
        let node_index: HashMap<u256, usize> =
            nodes.iter().enumerate().map(|(i, &id)| (id, i)).collect();

        // Build adjacency: out_edges[from_idx] = vec of (to_idx, weight).
        let mut out_edges: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        let mut out_weights: Vec<f64> = vec![0.0; n];

        for edge in &active_edges {
            let from_idx = node_index[&edge.from];
            let to_idx = node_index[&edge.to];
            let w = edge.weight();
            out_edges[from_idx].push((to_idx, w));
            out_weights[from_idx] += w;
        }

        // Power iteration.
        let damping = self.config.damping;
        let teleport = (1.0 - damping) / n as f64;
        let mut ranks = vec![1.0 / n as f64; n];
        let mut new_ranks = vec![0.0; n];
        let mut iterations = 0;
        let mut final_delta = f64::MAX;

        for _ in 0..self.config.max_iterations {
            iterations += 1;

            // Initialize with teleportation probability.
            for r in &mut new_ranks {
                *r = teleport;
            }

            // Distribute rank through edges.
            for from_idx in 0..n {
                if out_weights[from_idx] <= 0.0 {
                    // Dangling node: distribute equally (like teleportation).
                    let share = damping * ranks[from_idx] / n as f64;
                    for r in &mut new_ranks {
                        *r += share;
                    }
                } else {
                    for &(to_idx, weight) in &out_edges[from_idx] {
                        let contribution =
                            damping * ranks[from_idx] * weight / out_weights[from_idx];
                        new_ranks[to_idx] += contribution;
                    }
                }
            }

            // Check convergence.
            final_delta = ranks
                .iter()
                .zip(new_ranks.iter())
                .map(|(old, new)| (old - new).abs())
                .fold(0.0_f64, f64::max);

            std::mem::swap(&mut ranks, &mut new_ranks);

            if final_delta < self.config.convergence_threshold {
                break;
            }
        }

        let converged = final_delta < self.config.convergence_threshold;

        // Build result map.
        let result_ranks: HashMap<u256, f64> = nodes
            .iter()
            .zip(ranks.iter())
            .map(|(&id, &rank)| (id, rank))
            .collect();

        TraceRankResult {
            ranks: result_ranks,
            iterations,
            converged,
            final_delta,
            node_count: n,
            edge_count: active_edges.len(),
        }
    }

    /// Blend TraceRank score with direct EMA reputation.
    ///
    /// `effective = (1 - blend_weight) * ema + blend_weight * trace_rank`
    pub fn blend_reputation(&self, ema_score: f64, trace_rank_score: f64) -> f64 {
        let w = self.config.blend_weight.clamp(0.0, 1.0);
        (1.0 - w) * ema_score + w * trace_rank_score
    }

    /// Get the normalized rank for a specific agent (0 if not in graph).
    ///
    /// Normalizes to [0, 1] by dividing by max rank in the result.
    pub fn normalized_rank(&self, result: &TraceRankResult, agent: u256) -> f64 {
        let max_rank = result.ranks.values().copied().fold(0.0_f64, f64::max);
        if max_rank <= 0.0 {
            return 0.0;
        }
        result.ranks.get(&agent).copied().unwrap_or(0.0) / max_rank
    }
}

impl Default for TraceRank {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_returns_empty_result() {
        let tr = TraceRank::new();
        let result = tr.compute();
        assert!(result.ranks.is_empty());
        assert!(result.converged);
        assert_eq!(result.node_count, 0);
    }

    #[test]
    fn single_edge_gives_receiver_higher_rank() {
        let mut tr = TraceRank::new();
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.9,
            block: 1,
        });

        let result = tr.compute();
        assert!(result.converged);
        assert_eq!(result.node_count, 2);

        // Agent 2 (receiver) should have higher rank than agent 1 (payer).
        let rank_1 = result.ranks[&1];
        let rank_2 = result.ranks[&2];
        assert!(
            rank_2 > rank_1,
            "receiver should rank higher: {rank_1} vs {rank_2}"
        );
    }

    #[test]
    fn quality_affects_trust_propagation() {
        // Two receivers: agent 2 got high quality, agent 3 got low quality.
        let mut tr = TraceRank::new();
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.95,
            block: 1,
        });
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 3,
            amount: 100.0,
            quality: 0.2,
            block: 2,
        });

        let result = tr.compute();
        let rank_2 = result.ranks[&2];
        let rank_3 = result.ranks[&3];

        assert!(
            rank_2 > rank_3,
            "high-quality receiver should rank higher: {rank_2} vs {rank_3}"
        );
    }

    #[test]
    fn transitive_trust_propagation() {
        // Chain: 1 -> 2 -> 3 (trust flows transitively).
        let mut tr = TraceRank::new();
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.9,
            block: 1,
        });
        tr.record_payment(PaymentEdge {
            from: 2,
            to: 3,
            amount: 80.0,
            quality: 0.85,
            block: 2,
        });

        let result = tr.compute();
        // Agent 3 should have some rank even though 1 never paid 3 directly.
        let rank_3 = result.ranks[&3];
        assert!(rank_3 > 0.0, "transitive trust should give rank to agent 3");
    }

    #[test]
    fn cycle_converges() {
        // Circular payments: 1 -> 2 -> 3 -> 1.
        let mut tr = TraceRank::new();
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.9,
            block: 1,
        });
        tr.record_payment(PaymentEdge {
            from: 2,
            to: 3,
            amount: 100.0,
            quality: 0.9,
            block: 2,
        });
        tr.record_payment(PaymentEdge {
            from: 3,
            to: 1,
            amount: 100.0,
            quality: 0.9,
            block: 3,
        });

        let result = tr.compute();
        assert!(result.converged, "cycle should converge");

        // All should have roughly equal rank (symmetric cycle).
        let ranks: Vec<f64> = result.ranks.values().copied().collect();
        let mean = ranks.iter().sum::<f64>() / ranks.len() as f64;
        for &r in &ranks {
            assert!(
                (r - mean).abs() < 0.01,
                "symmetric cycle should give equal ranks: {ranks:?}"
            );
        }
    }

    #[test]
    fn blend_reputation_works() {
        let tr = TraceRank::with_config(TraceRankConfig {
            blend_weight: 0.3,
            ..Default::default()
        });

        let blended = tr.blend_reputation(0.8, 0.6);
        // (1 - 0.3) * 0.8 + 0.3 * 0.6 = 0.56 + 0.18 = 0.74
        assert!((blended - 0.74).abs() < 1e-10);
    }

    #[test]
    fn normalized_rank_scales_to_unit() {
        let mut tr = TraceRank::new();
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.9,
            block: 1,
        });
        tr.record_payment(PaymentEdge {
            from: 1,
            to: 3,
            amount: 50.0,
            quality: 0.5,
            block: 2,
        });

        let result = tr.compute();
        let norm_2 = tr.normalized_rank(&result, 2);
        let norm_3 = tr.normalized_rank(&result, 3);

        // Highest-ranked agent should have normalized rank = 1.0.
        assert!((norm_2 - 1.0).abs() < 0.01 || (norm_3 - 1.0).abs() < 0.01);
    }

    #[test]
    fn dust_payments_filtered() {
        let mut tr = TraceRank::with_config(TraceRankConfig {
            min_edge_weight: 1.0,
            ..Default::default()
        });

        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 0.001,
            quality: 1.0,
            block: 1, // weight = 0.001 < 1.0
        });

        assert_eq!(tr.edge_count(), 0, "dust payment should be filtered");
    }

    #[test]
    fn lookback_window_filters_old_edges() {
        let mut tr = TraceRank::with_config(TraceRankConfig {
            lookback_blocks: 100,
            ..Default::default()
        });

        tr.record_payment(PaymentEdge {
            from: 1,
            to: 2,
            amount: 100.0,
            quality: 0.9,
            block: 50, // old
        });
        tr.record_payment(PaymentEdge {
            from: 3,
            to: 4,
            amount: 100.0,
            quality: 0.9,
            block: 180, // recent
        });

        let result = tr.compute();
        // Only the recent edge should be in the graph.
        // Lookback: max_block(180) - 100 = 80, so block 50 < 80 is excluded.
        assert_eq!(result.node_count, 2, "only recent edge should be included");
        assert!(result.ranks.contains_key(&3));
        assert!(result.ranks.contains_key(&4));
    }
}
