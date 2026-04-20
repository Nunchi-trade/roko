//! Shapley-value attribution for fair credit distribution among agents.
//!
//! Implements the Shapley value from cooperative game theory to fairly attribute
//! collective outcomes to individual agent contributions. This is the theoretically
//! optimal attribution method satisfying efficiency, symmetry, null player, and
//! additivity axioms.
//!
//! ## When to use
//!
//! - Distributing rewards after a multi-agent plan succeeds
//! - Attributing blame when a collective task fails
//! - Computing fair cost-sharing for shared resources
//! - Determining which agents are essential vs. replaceable
//!
//! ## Algorithms
//!
//! - **Exact**: O(2^n * n) — practical for n <= 12 agents
//! - **Monte Carlo**: O(samples * n) — for larger groups (convergence ~1000 samples)
//!
//! ## Example
//!
//! ```rust
//! use roko_learn::shapley::{shapley_exact, Coalition};
//!
//! // 3 agents where agent 0 contributes 40% alone, agent 1 contributes 30%,
//! // and together they get a synergy bonus.
//! let values = shapley_exact(3, |coalition| {
//!     let mut v = 0.0;
//!     if coalition.contains(0) { v += 0.4; }
//!     if coalition.contains(1) { v += 0.3; }
//!     if coalition.contains(2) { v += 0.2; }
//!     // Synergy: agents 0+1 together get extra 0.1
//!     if coalition.contains(0) && coalition.contains(1) { v += 0.1; }
//!     v
//! });
//! // values[0] > values[1] > values[2] (agent 0 contributes most)
//! ```

use std::collections::HashMap;

/// A coalition represented as a bitmask of participating agents.
///
/// Agent `i` is in the coalition if bit `i` is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Coalition(pub u64);

impl Coalition {
    /// Empty coalition (no agents).
    pub const EMPTY: Self = Self(0);

    /// Create a coalition containing a single agent.
    pub const fn singleton(agent: usize) -> Self {
        Self(1 << agent)
    }

    /// Grand coalition containing all `n` agents.
    pub const fn grand(n: usize) -> Self {
        if n >= 64 {
            Self(u64::MAX)
        } else {
            Self((1u64 << n) - 1)
        }
    }

    /// Check if an agent is in this coalition.
    pub const fn contains(self, agent: usize) -> bool {
        (self.0 >> agent) & 1 == 1
    }

    /// Add an agent to the coalition.
    pub const fn with(self, agent: usize) -> Self {
        Self(self.0 | (1 << agent))
    }

    /// Remove an agent from the coalition.
    pub const fn without(self, agent: usize) -> Self {
        Self(self.0 & !(1 << agent))
    }

    /// Number of agents in the coalition.
    pub const fn size(self) -> u32 {
        self.0.count_ones()
    }

    /// Iterate over agent indices in this coalition.
    pub fn members(self) -> impl Iterator<Item = usize> {
        (0..64).filter(move |&i| self.contains(i))
    }
}

/// Contribution record for a single agent.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapleyAttribution {
    /// Agent identifier.
    pub agent_id: String,
    /// Shapley value (fair share of the total outcome).
    pub shapley_value: f64,
    /// Fractional share of the grand coalition value.
    pub share: f64,
}

/// Compute exact Shapley values for `n` agents.
///
/// The characteristic function `v(coalition) -> f64` maps each coalition to its value.
/// Shapley values satisfy: sum(shapley_values) = v(grand_coalition) - v(empty_coalition).
///
/// ## Complexity
/// O(2^n * n) — practical for n <= 12.
///
/// ## Panics
/// Panics if `n > 20` (would require >1M evaluations; use Monte Carlo instead).
pub fn shapley_exact<F>(n: usize, v: F) -> Vec<f64>
where
    F: Fn(Coalition) -> f64,
{
    assert!(n <= 20, "exact Shapley requires n <= 20; use shapley_monte_carlo for larger groups");

    let mut values = vec![0.0; n];

    // Precompute factorial lookup (small n, just use f64).
    let factorial = |k: u32| -> f64 {
        (1..=k).map(|i| i as f64).product::<f64>().max(1.0)
    };

    let n_factorial = factorial(n as u32);

    // For each agent i, iterate over all coalitions S not containing i.
    for i in 0..n {
        let others_mask = Coalition::grand(n).without(i).0;

        // Enumerate all subsets S of N \ {i}.
        let mut s_bits = 0u64;
        loop {
            let coalition_s = Coalition(s_bits);
            let coalition_s_with_i = coalition_s.with(i);

            let s_size = coalition_s.size();
            let marginal = v(coalition_s_with_i) - v(coalition_s);

            // Weight: |S|! * (n - |S| - 1)! / n!
            let weight = factorial(s_size) * factorial(n as u32 - s_size - 1) / n_factorial;
            values[i] += weight * marginal;

            // Gosper's hack for enumerating subsets of others_mask.
            if s_bits == others_mask {
                break;
            }
            // Next subset of others_mask (Carry-Ripple subset enumeration).
            s_bits = next_subset(s_bits, others_mask);
            if s_bits == 0 && others_mask != 0 {
                break;
            }
        }
    }

    values
}

/// Monte Carlo approximation of Shapley values using random permutations.
///
/// For each random permutation, compute the marginal contribution of each agent
/// when they "enter" the coalition in permutation order. Average over many samples.
///
/// ## Complexity
/// O(samples * n^2) characteristic function evaluations.
///
/// ## Parameters
/// - `n`: number of agents
/// - `v`: characteristic function
/// - `samples`: number of random permutations to sample (more = more accurate)
/// - `seed`: random seed for reproducibility
pub fn shapley_monte_carlo<F>(n: usize, v: F, samples: usize, seed: u64) -> Vec<f64>
where
    F: Fn(Coalition) -> f64,
{
    let mut values = vec![0.0; n];
    let mut rng_state = seed;

    for _ in 0..samples {
        // Generate a random permutation using Fisher-Yates shuffle.
        let perm = random_permutation(n, &mut rng_state);

        // Walk through the permutation, computing marginal contributions.
        let mut coalition = Coalition::EMPTY;
        for &agent in &perm {
            let v_without = v(coalition);
            coalition = coalition.with(agent);
            let v_with = v(coalition);
            values[agent] += v_with - v_without;
        }
    }

    // Average over samples.
    let inv_samples = 1.0 / samples as f64;
    for val in &mut values {
        *val *= inv_samples;
    }

    values
}

/// Compute Shapley attributions with agent IDs and shares.
///
/// This is the high-level API that wraps `shapley_exact` or `shapley_monte_carlo`
/// based on group size, and returns named attributions sorted by value.
pub fn shapley_attribution<F>(agent_ids: &[String], v: F, samples: Option<usize>) -> Vec<ShapleyAttribution>
where
    F: Fn(Coalition) -> f64,
{
    let n = agent_ids.len();
    if n == 0 {
        return Vec::new();
    }

    let values = if n <= 12 {
        shapley_exact(n, &v)
    } else {
        shapley_monte_carlo(n, &v, samples.unwrap_or(1000), 42)
    };

    let grand_value = v(Coalition::grand(n));
    let total_shapley: f64 = values.iter().sum();

    let mut attributions: Vec<ShapleyAttribution> = agent_ids
        .iter()
        .zip(values.iter())
        .map(|(id, &sv)| ShapleyAttribution {
            agent_id: id.clone(),
            shapley_value: sv,
            share: if total_shapley.abs() > 1e-12 {
                sv / total_shapley
            } else if grand_value.abs() > 1e-12 {
                sv / grand_value
            } else {
                1.0 / n as f64
            },
        })
        .collect();

    // Sort by Shapley value descending.
    attributions.sort_by(|a, b| b.shapley_value.partial_cmp(&a.shapley_value).unwrap_or(std::cmp::Ordering::Equal));
    attributions
}

/// Compute Shapley values from a precomputed value table (HashMap-based).
///
/// Useful when the characteristic function is expensive and you want to cache results.
pub fn shapley_from_table(n: usize, table: &HashMap<Coalition, f64>) -> Vec<f64> {
    shapley_exact(n, |c| table.get(&c).copied().unwrap_or(0.0))
}

// ─── Internal helpers ───────────────────────────────────────────────

/// Enumerate next subset of a mask (carry-ripple enumeration).
fn next_subset(current: u64, mask: u64) -> u64 {
    if current == 0 && mask != 0 {
        // First non-empty subset: lowest set bit of mask.
        mask & mask.wrapping_neg()
    } else {
        // Carry-ripple: add 1 to current within the mask bits.
        let ripple = current.wrapping_sub(mask) & mask;
        if ripple == 0 {
            0 // Wrapped around
        } else {
            ripple
        }
    }
}

/// Xorshift64 PRNG for Monte Carlo sampling.
fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Generate a random permutation of [0..n) using Fisher-Yates.
fn random_permutation(n: usize, rng_state: &mut u64) -> Vec<usize> {
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (xorshift64(rng_state) as usize) % (i + 1);
        perm.swap(i, j);
    }
    perm
}

// ─── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_agent_gets_full_value() {
        let values = shapley_exact(1, |c| if c.contains(0) { 10.0 } else { 0.0 });
        assert!((values[0] - 10.0).abs() < 1e-10);
    }

    #[test]
    fn symmetric_agents_get_equal_shares() {
        // Two agents, each contributes 5 alone, together they get 10 (no synergy).
        let values = shapley_exact(2, |c| {
            let mut v = 0.0;
            if c.contains(0) { v += 5.0; }
            if c.contains(1) { v += 5.0; }
            v
        });
        assert!((values[0] - 5.0).abs() < 1e-10);
        assert!((values[1] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn null_agent_gets_zero() {
        // Agent 2 contributes nothing.
        let values = shapley_exact(3, |c| {
            let mut v = 0.0;
            if c.contains(0) { v += 4.0; }
            if c.contains(1) { v += 3.0; }
            // Agent 2 adds nothing.
            v
        });
        assert!((values[2]).abs() < 1e-10, "null agent should get 0, got {}", values[2]);
    }

    #[test]
    fn efficiency_property() {
        // Sum of Shapley values = v(grand) - v(empty).
        let v = |c: Coalition| -> f64 {
            let mut val = 0.0;
            if c.contains(0) { val += 3.0; }
            if c.contains(1) { val += 2.0; }
            if c.contains(2) { val += 1.0; }
            if c.contains(0) && c.contains(1) { val += 2.0; }
            val
        };

        let values = shapley_exact(3, v);
        let sum: f64 = values.iter().sum();
        let expected = v(Coalition::grand(3)) - v(Coalition::EMPTY);

        assert!(
            (sum - expected).abs() < 1e-10,
            "efficiency: sum={sum}, v(grand)={expected}"
        );
    }

    #[test]
    fn synergy_distributes_fairly() {
        // Agent 0 alone: 0, Agent 1 alone: 0, together: 10.
        // Each should get 5.0 (symmetric synergy).
        let values = shapley_exact(2, |c| {
            if c.contains(0) && c.contains(1) { 10.0 } else { 0.0 }
        });
        assert!((values[0] - 5.0).abs() < 1e-10);
        assert!((values[1] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn essential_agent_gets_full_credit() {
        // Only the grand coalition has value (all agents essential).
        let values = shapley_exact(3, |c| {
            if c.contains(0) && c.contains(1) && c.contains(2) { 12.0 } else { 0.0 }
        });
        // Each gets 12/3 = 4.0 (symmetric essential agents).
        for &v in &values {
            assert!((v - 4.0).abs() < 1e-10, "essential agent should get 4.0, got {v}");
        }
    }

    #[test]
    fn monte_carlo_approximates_exact() {
        let v = |c: Coalition| -> f64 {
            let mut val = 0.0;
            if c.contains(0) { val += 4.0; }
            if c.contains(1) { val += 3.0; }
            if c.contains(2) { val += 1.0; }
            if c.contains(0) && c.contains(1) { val += 2.0; }
            val
        };

        let exact = shapley_exact(3, v);
        let mc = shapley_monte_carlo(3, v, 10_000, 12345);

        for i in 0..3 {
            assert!(
                (exact[i] - mc[i]).abs() < 0.1,
                "MC should approximate exact: agent {i}: exact={}, mc={}",
                exact[i], mc[i]
            );
        }
    }

    #[test]
    fn attribution_sorts_by_value() {
        let agents = vec!["a".into(), "b".into(), "c".into()];
        let attrs = shapley_attribution(&agents, |c| {
            let mut v = 0.0;
            if c.contains(0) { v += 1.0; }
            if c.contains(1) { v += 5.0; }
            if c.contains(2) { v += 3.0; }
            v
        }, None);

        assert_eq!(attrs[0].agent_id, "b"); // highest
        assert_eq!(attrs[1].agent_id, "c");
        assert_eq!(attrs[2].agent_id, "a"); // lowest
    }

    #[test]
    fn shares_sum_to_one() {
        let agents = vec!["x".into(), "y".into(), "z".into()];
        let attrs = shapley_attribution(&agents, |c| {
            let mut v = 0.0;
            if c.contains(0) { v += 2.0; }
            if c.contains(1) { v += 3.0; }
            if c.contains(2) { v += 5.0; }
            v
        }, None);

        let total_share: f64 = attrs.iter().map(|a| a.share).sum();
        assert!(
            (total_share - 1.0).abs() < 1e-10,
            "shares should sum to 1.0, got {total_share}"
        );
    }

    #[test]
    fn coalition_operations() {
        let c = Coalition::EMPTY.with(0).with(2);
        assert!(c.contains(0));
        assert!(!c.contains(1));
        assert!(c.contains(2));
        assert_eq!(c.size(), 2);

        let c2 = c.without(0);
        assert!(!c2.contains(0));
        assert!(c2.contains(2));
        assert_eq!(c2.size(), 1);

        let grand = Coalition::grand(4);
        assert_eq!(grand.size(), 4);
        assert!(grand.contains(0));
        assert!(grand.contains(3));
        assert!(!grand.contains(4));
    }
}
