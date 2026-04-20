//! Adaptive signal metabolism — evolutionary dynamics for signal populations.
//!
//! Signals (Engrams) are treated as organisms competing in a fitness landscape.
//! Their population dynamics follow evolutionary biology:
//!
//! - **Replicator dynamics** (Taylor & Jonker 1978): signals with above-average
//!   fitness grow; below-average signals shrink.
//! - **Hebbian learning** (Oja's rule): signal-to-outcome connection weights
//!   update in a self-normalizing fashion, converging to the principal
//!   eigenvector direction.
//! - **Fisher's fundamental theorem**: the rate of fitness increase equals
//!   the genetic variance in fitness — more diverse signal populations adapt
//!   faster.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Signal population registry
// ---------------------------------------------------------------------------

/// Opaque identifier for a signal type (e.g. hash of the signal pattern).
pub type SignalTypeId = u64;

/// Population record for a single signal type.
#[derive(Clone, Debug)]
pub struct SignalPopulation {
    /// Fraction of the total population occupied by this type, in `[0, 1]`.
    pub fraction: f64,
    /// Current fitness score (higher = better prediction accuracy).
    pub fitness: f64,
    /// Generation count (number of replicator steps survived).
    pub generation: u64,
    /// Birth timestamp (first observation).
    pub born_at: u64,
    /// Number of offspring (mutated variants spawned).
    pub offspring_count: u32,
}

/// Registry tracking a population of active signal patterns.
#[derive(Clone, Debug, Default)]
pub struct SignalRegistry {
    /// Per-type population records.
    pub populations: HashMap<SignalTypeId, SignalPopulation>,
}

impl SignalRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new signal type with initial population fraction and fitness.
    pub fn register(&mut self, id: SignalTypeId, fitness: f64, born_at: u64) {
        let n = self.populations.len() as f64 + 1.0;
        // Uniform share for new entrant; existing shares are rescaled in
        // the next replicator step.
        let pop = SignalPopulation {
            fraction: 1.0 / n,
            fitness,
            generation: 0,
            born_at,
            offspring_count: 0,
        };
        self.populations.insert(id, pop);
    }

    /// Update the fitness score for a signal type.
    pub fn update_fitness(&mut self, id: SignalTypeId, new_fitness: f64) {
        if let Some(pop) = self.populations.get_mut(&id) {
            pop.fitness = new_fitness;
        }
    }

    /// Average fitness across all types (population-weighted).
    ///
    /// `phi = Sum(x_i * f_i)` where `x_i` is fraction, `f_i` is fitness.
    pub fn average_fitness(&self) -> f64 {
        self.populations
            .values()
            .map(|p| p.fraction * p.fitness)
            .sum()
    }

    /// Remove signal types whose population fraction has dropped below
    /// `threshold`.
    pub fn cull(&mut self, threshold: f64) {
        self.populations.retain(|_, p| p.fraction >= threshold);
    }
}

// ---------------------------------------------------------------------------
// Replicator dynamics
// ---------------------------------------------------------------------------

/// Apply one step of replicator dynamics to the signal registry.
///
/// `dx_i/dt = x_i * (f_i - phi)` discretised with time-step `dt`.
///
/// After the update, fractions are re-normalised to sum to 1.0.
pub fn replicator_step(registry: &mut SignalRegistry, dt: f64) {
    if registry.populations.is_empty() {
        return;
    }

    let phi = registry.average_fitness();

    for pop in registry.populations.values_mut() {
        let dx = pop.fraction * (pop.fitness - phi) * dt;
        pop.fraction = (pop.fraction + dx).max(0.0);
        pop.generation += 1;
    }

    // Re-normalise.
    let total: f64 = registry.populations.values().map(|p| p.fraction).sum();
    if total > 0.0 {
        for pop in registry.populations.values_mut() {
            pop.fraction /= total;
        }
    }
}

// ---------------------------------------------------------------------------
// Hebbian learning (Oja's rule)
// ---------------------------------------------------------------------------

/// Apply Oja's self-normalising Hebbian update to a weight vector.
///
/// `delta_w_i = lr * (y * x_i - y^2 * w_i)`
///
/// Where `x` is the signal activation vector, `y` is the outcome
/// (verification score), `w` is the current weight vector, and `lr` is the
/// learning rate. Weights converge to the principal eigenvector direction.
pub fn hebbian_update(weights: &mut [f64], signal: &[f64], outcome: f64, lr: f64) {
    assert_eq!(
        weights.len(),
        signal.len(),
        "weight and signal vectors must match"
    );
    let y = outcome;
    let y2 = y * y;
    for (w, x) in weights.iter_mut().zip(signal.iter()) {
        *w += lr * (y * x - y2 * (*w));
    }
}

// ---------------------------------------------------------------------------
// Fisher's fundamental theorem — variance monitoring
// ---------------------------------------------------------------------------

/// Compute the population-weighted variance of fitness scores.
///
/// `Var(f) = Sum(x_i * (f_i - phi)^2)`.
///
/// By Fisher's fundamental theorem, this equals the rate of mean fitness
/// increase. Higher variance -> faster adaptation.
pub fn population_fitness_variance(registry: &SignalRegistry) -> f64 {
    let phi = registry.average_fitness();
    registry
        .populations
        .values()
        .map(|p| p.fraction * (p.fitness - phi).powi(2))
        .sum()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn two_type_registry() -> SignalRegistry {
        let mut r = SignalRegistry::new();
        r.populations.insert(
            1,
            SignalPopulation {
                fraction: 0.5,
                fitness: 1.0,
                generation: 0,
                born_at: 0,
                offspring_count: 0,
            },
        );
        r.populations.insert(
            2,
            SignalPopulation {
                fraction: 0.5,
                fitness: 0.5,
                generation: 0,
                born_at: 0,
                offspring_count: 0,
            },
        );
        r
    }

    #[test]
    fn average_fitness_weighted() {
        let r = two_type_registry();
        // phi = 0.5*1.0 + 0.5*0.5 = 0.75
        assert!((r.average_fitness() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn replicator_fitter_grows() {
        let mut r = two_type_registry();
        for _ in 0..20 {
            replicator_step(&mut r, 0.1);
        }
        // Type 1 (fitness=1.0) should have grown relative to type 2 (fitness=0.5).
        assert!(
            r.populations[&1].fraction > r.populations[&2].fraction,
            "fitter type should dominate"
        );
    }

    #[test]
    fn replicator_fractions_sum_to_one() {
        let mut r = two_type_registry();
        replicator_step(&mut r, 0.5);
        let total: f64 = r.populations.values().map(|p| p.fraction).sum();
        assert!((total - 1.0).abs() < 1e-12);
    }

    #[test]
    fn hebbian_converges_direction() {
        let mut w = vec![0.5, 0.5];
        let signal = vec![1.0, 0.0];
        // Repeated Hebbian updates should push w toward [1, 0].
        for _ in 0..100 {
            hebbian_update(&mut w, &signal, 1.0, 0.1);
        }
        assert!(w[0] > 0.9);
        assert!(w[1].abs() < 0.3);
    }

    #[test]
    fn hebbian_self_normalising() {
        // When y = w^T x (the actual neural output), Oja's rule
        // self-normalises to ||w|| = 1. We simulate this by computing y.
        let mut w = vec![0.3, 0.4, 0.5];
        let signal = vec![1.0, 0.0, 0.0]; // principal direction along dim 0
        for _ in 0..500 {
            let y: f64 = w.iter().zip(signal.iter()).map(|(a, b)| a * b).sum();
            hebbian_update(&mut w, &signal, y, 0.05);
        }
        let norm: f64 = w.iter().map(|x| x * x).sum::<f64>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.15,
            "weight norm should converge near 1.0, got {}",
            norm
        );
        // w should point primarily along dim 0.
        assert!(w[0] > 0.9, "w[0] should dominate, got {}", w[0]);
    }

    #[test]
    fn fitness_variance_equal_populations() {
        let mut r = SignalRegistry::new();
        r.populations.insert(
            1,
            SignalPopulation {
                fraction: 0.5,
                fitness: 1.0,
                generation: 0,
                born_at: 0,
                offspring_count: 0,
            },
        );
        r.populations.insert(
            2,
            SignalPopulation {
                fraction: 0.5,
                fitness: 1.0,
                generation: 0,
                born_at: 0,
                offspring_count: 0,
            },
        );
        // All have same fitness -> variance = 0
        assert!(population_fitness_variance(&r).abs() < 1e-12);
    }

    #[test]
    fn fitness_variance_diverse_populations() {
        let r = two_type_registry();
        // phi = 0.75; Var = 0.5*(1-0.75)^2 + 0.5*(0.5-0.75)^2 = 0.5*0.0625+0.5*0.0625 = 0.0625
        let v = population_fitness_variance(&r);
        assert!((v - 0.0625).abs() < 1e-9);
    }

    #[test]
    fn cull_removes_tiny_fractions() {
        let mut r = two_type_registry();
        r.populations.get_mut(&2).unwrap().fraction = 0.001;
        r.cull(0.01);
        assert!(!r.populations.contains_key(&2));
        assert!(r.populations.contains_key(&1));
    }

    #[test]
    fn register_new_type() {
        let mut r = SignalRegistry::new();
        r.register(42, 0.8, 100);
        assert!(r.populations.contains_key(&42));
        assert!((r.populations[&42].fitness - 0.8).abs() < 1e-9);
    }
}
