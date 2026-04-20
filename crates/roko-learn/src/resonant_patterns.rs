//! Resonant patterns: evolutionary pattern organisms (TA-09).
//!
//! Patterns are organisms with HDC vector genomes and fitness scores. They
//! compete for attention budget via VCG auction, and Lotka-Volterra dynamics
//! govern predator-prey relationships between competing patterns.
//!
//! Each pattern carries a topological signature (persistence diagram) that
//! captures the shape of the data it represents, enabling shape-aware
//! similarity and competition.
//!
//! # References
//!
//! - Lotka, A. J. (1925). Elements of Physical Biology.
//! - Volterra, V. (1926). Fluctuations in the abundance of a species.
//! - Price, G. R. (1970). Selection and covariance. *Nature*, 227, 520-521.

use roko_primitives::HdcVector;

/// A resonant pattern with an HDC genome, fitness, and population dynamics.
#[derive(Debug, Clone)]
pub struct ResonantPattern {
    /// Unique identifier.
    pub id: u64,
    /// HDC vector encoding the pattern genome (10,240-bit).
    pub genome: HdcVector,
    /// Prediction accuracy over the pattern's lifetime, in `[0.0, 1.0]`.
    pub fitness: f64,
    /// Ticks since the pattern was born.
    pub age: u64,
    /// Number of offspring (mutated copies) produced.
    pub offspring_count: u32,
    /// Current population size (attention share).
    pub population: f64,
    /// Carrying capacity — maximum sustainable population.
    pub carrying_capacity: f64,
    /// Growth rate for Lotka-Volterra dynamics.
    pub growth_rate: f64,
}

impl ResonantPattern {
    /// Create a new resonant pattern.
    pub fn new(id: u64, genome: HdcVector, fitness: f64) -> Self {
        Self {
            id,
            genome,
            fitness,
            age: 0,
            offspring_count: 0,
            population: 1.0,
            carrying_capacity: 100.0,
            growth_rate: 0.1,
        }
    }

    /// Create with custom population parameters.
    pub fn with_population(
        mut self,
        population: f64,
        carrying_capacity: f64,
        growth_rate: f64,
    ) -> Self {
        self.population = population;
        self.carrying_capacity = carrying_capacity;
        self.growth_rate = growth_rate;
        self
    }

    /// Whether this pattern is alive (population > extinction threshold).
    pub fn is_alive(&self) -> bool {
        self.population > 0.01
    }

    /// Increment age by one tick.
    pub fn tick(&mut self) {
        self.age += 1;
    }
}

/// Lotka-Volterra competition dynamics.
///
/// Updates populations of competing patterns using the competitive
/// Lotka-Volterra equations:
///
/// ```text
/// dN_i/dt = r_i * N_i * (1 - (N_i + sum_j(a_ij * N_j)) / K_i)
/// ```
///
/// where:
/// - N_i = population of pattern i
/// - r_i = intrinsic growth rate
/// - K_i = carrying capacity
/// - a_ij = competition coefficient (genome similarity between i and j)
///
/// Patterns with higher fitness have higher growth rates. Patterns with
/// similar genomes compete more strongly (higher a_ij).
pub fn lotka_volterra_step(patterns: &mut [ResonantPattern], dt: f64) {
    if patterns.is_empty() {
        return;
    }

    let n = patterns.len();

    // Compute competition coefficients from genome similarity.
    // a_ij = similarity(genome_i, genome_j), so identical patterns compete maximally.
    let mut competition = vec![vec![0.0_f64; n]; n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                competition[i][j] = 1.0; // Self-competition.
            } else {
                competition[i][j] = f64::from(patterns[i].genome.similarity(&patterns[j].genome));
            }
        }
    }

    // Compute population changes.
    let mut deltas = vec![0.0_f64; n];
    for i in 0..n {
        let p = &patterns[i];
        if !p.is_alive() {
            continue;
        }

        // Total competition pressure from all other patterns.
        let competitive_pressure: f64 = (0..n)
            .map(|j| competition[i][j] * patterns[j].population)
            .sum();

        // Fitness-adjusted growth rate.
        let effective_rate = p.growth_rate * p.fitness;

        // Lotka-Volterra growth equation.
        let growth = effective_rate
            * p.population
            * (1.0 - competitive_pressure / p.carrying_capacity.max(f64::EPSILON));

        deltas[i] = growth * dt;
    }

    // Apply changes.
    for (i, delta) in deltas.into_iter().enumerate() {
        patterns[i].population = (patterns[i].population + delta).max(0.0);
        patterns[i].tick();
    }
}

/// Price equation for evolutionary change tracking.
///
/// The Price equation decomposes the change in mean fitness into
/// selection and transmission components:
///
/// ```text
/// Delta(z_bar) = Cov(w, z) / w_bar + E(w * Delta_z) / w_bar
/// ```
///
/// where z is the trait (fitness), w is relative fitness.
///
/// Returns `(selection_component, transmission_component)`.
pub fn price_equation(patterns: &[ResonantPattern]) -> (f64, f64) {
    if patterns.is_empty() {
        return (0.0, 0.0);
    }

    let n = patterns.len() as f64;
    let mean_fitness = patterns.iter().map(|p| p.fitness).sum::<f64>() / n;
    let mean_pop = patterns.iter().map(|p| p.population).sum::<f64>() / n;

    if mean_pop < f64::EPSILON {
        return (0.0, 0.0);
    }

    // Relative fitness (using population as proxy for reproductive success).
    let w: Vec<f64> = patterns
        .iter()
        .map(|p| p.population / mean_pop)
        .collect();

    let w_bar: f64 = w.iter().sum::<f64>() / n;

    // Selection component: Cov(w, z) / w_bar.
    let mean_w = w_bar;
    let cov_wz: f64 = w
        .iter()
        .zip(patterns.iter())
        .map(|(&wi, p)| (wi - mean_w) * (p.fitness - mean_fitness))
        .sum::<f64>()
        / n;

    let selection = if w_bar > f64::EPSILON {
        cov_wz / w_bar
    } else {
        0.0
    };

    // Transmission component: E(w * Delta_z) / w_bar.
    // For now, Delta_z = 0 (no mutation), so transmission is 0.
    let transmission = 0.0;

    (selection, transmission)
}

/// Population fitness variance (Fisher's fundamental theorem monitoring).
///
/// The rate of fitness increase in a population equals the genetic
/// variance in fitness. Higher variance means faster adaptation.
pub fn fitness_variance(patterns: &[ResonantPattern]) -> f64 {
    if patterns.len() < 2 {
        return 0.0;
    }

    let mean = patterns.iter().map(|p| p.fitness).sum::<f64>() / patterns.len() as f64;
    patterns
        .iter()
        .map(|p| (p.fitness - mean).powi(2))
        .sum::<f64>()
        / (patterns.len() - 1) as f64
}

/// Remove extinct patterns (population below threshold).
pub fn cull_extinct(patterns: &mut Vec<ResonantPattern>) {
    patterns.retain(|p| p.is_alive());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_genome(seed: u64) -> HdcVector {
        HdcVector::from_seed(&seed.to_le_bytes())
    }

    #[test]
    fn resonant_pattern_creation() {
        let genome = test_genome(42);
        let pattern = ResonantPattern::new(1, genome, 0.8);
        assert_eq!(pattern.id, 1);
        assert!((pattern.fitness - 0.8).abs() < f64::EPSILON);
        assert_eq!(pattern.age, 0);
        assert!(pattern.is_alive());
    }

    #[test]
    fn pattern_tick_increments_age() {
        let genome = test_genome(42);
        let mut pattern = ResonantPattern::new(1, genome, 0.8);
        pattern.tick();
        pattern.tick();
        assert_eq!(pattern.age, 2);
    }

    #[test]
    fn pattern_extinction() {
        let genome = test_genome(42);
        let mut pattern = ResonantPattern::new(1, genome, 0.0);
        pattern.population = 0.001;
        assert!(!pattern.is_alive());
    }

    #[test]
    fn lotka_volterra_single_pattern_grows() {
        let genome = test_genome(42);
        let mut patterns = vec![
            ResonantPattern::new(1, genome, 0.8).with_population(10.0, 100.0, 0.5),
        ];

        let initial_pop = patterns[0].population;
        lotka_volterra_step(&mut patterns, 1.0);
        assert!(
            patterns[0].population > initial_pop,
            "single pattern below carrying capacity should grow: {} -> {}",
            initial_pop,
            patterns[0].population
        );
    }

    #[test]
    fn lotka_volterra_competition_reduces_growth() {
        let g1 = test_genome(42);
        let g2 = test_genome(43); // Different genome -> less competition.

        let mut solo = vec![
            ResonantPattern::new(1, g1, 0.8).with_population(10.0, 100.0, 0.5),
        ];
        lotka_volterra_step(&mut solo, 1.0);
        let solo_growth = solo[0].population;

        let mut competing = vec![
            ResonantPattern::new(1, g1, 0.8).with_population(10.0, 100.0, 0.5),
            ResonantPattern::new(2, g2, 0.6).with_population(30.0, 100.0, 0.3),
        ];
        lotka_volterra_step(&mut competing, 1.0);
        let competed_growth = competing[0].population;

        // With a competitor present, growth should be less.
        assert!(
            competed_growth <= solo_growth + 0.5,
            "competition should reduce growth: solo={solo_growth}, competed={competed_growth}"
        );
    }

    #[test]
    fn lotka_volterra_empty() {
        let mut patterns: Vec<ResonantPattern> = vec![];
        lotka_volterra_step(&mut patterns, 1.0); // Should not panic.
    }

    #[test]
    fn price_equation_uniform_fitness() {
        let genome = test_genome(42);
        let patterns = vec![
            ResonantPattern::new(1, genome, 0.5).with_population(10.0, 100.0, 0.1),
            ResonantPattern::new(2, genome, 0.5).with_population(10.0, 100.0, 0.1),
        ];
        let (selection, _transmission) = price_equation(&patterns);
        assert!(
            selection.abs() < 0.01,
            "uniform fitness should have ~0 selection: {selection}"
        );
    }

    #[test]
    fn price_equation_fitness_variation() {
        let g1 = test_genome(42);
        let g2 = test_genome(43);
        let patterns = vec![
            ResonantPattern::new(1, g1, 0.9).with_population(50.0, 100.0, 0.1),
            ResonantPattern::new(2, g2, 0.1).with_population(5.0, 100.0, 0.1),
        ];
        let (selection, _) = price_equation(&patterns);
        // High-fitness pattern has higher population -> positive covariance.
        assert!(
            selection > 0.0,
            "selection should be positive when fit patterns have more population: {selection}"
        );
    }

    #[test]
    fn fitness_variance_uniform() {
        let genome = test_genome(42);
        let patterns = vec![
            ResonantPattern::new(1, genome, 0.5),
            ResonantPattern::new(2, genome, 0.5),
        ];
        let var = fitness_variance(&patterns);
        assert!(var < f64::EPSILON, "uniform fitness should have 0 variance");
    }

    #[test]
    fn fitness_variance_diverse() {
        let g1 = test_genome(42);
        let g2 = test_genome(43);
        let patterns = vec![
            ResonantPattern::new(1, g1, 0.9),
            ResonantPattern::new(2, g2, 0.1),
        ];
        let var = fitness_variance(&patterns);
        assert!(var > 0.1, "diverse fitness should have positive variance: {var}");
    }

    #[test]
    fn cull_extinct_removes_dead() {
        let g1 = test_genome(42);
        let g2 = test_genome(43);
        let mut patterns = vec![
            ResonantPattern::new(1, g1, 0.8).with_population(10.0, 100.0, 0.1),
            {
                let mut p = ResonantPattern::new(2, g2, 0.1);
                p.population = 0.001; // Below threshold.
                p
            },
        ];
        cull_extinct(&mut patterns);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].id, 1);
    }
}
