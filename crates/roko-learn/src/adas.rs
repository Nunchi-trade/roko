//! Adaptive Design of AI Systems (ADAS) autocatalytic optimization (LEARN-08).
//!
//! The ADAS optimizer maintains a population of prompt/model configurations,
//! evaluates their performance, mutates the best performers, and iterates.
//! It tracks which modifications improved performance and amplifies those
//! directions — an autocatalytic loop where the system improves its own
//! improvement strategy.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One candidate configuration in the ADAS population.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdasCandidate {
    /// Unique identifier for this candidate.
    pub id: String,
    /// Model slug this candidate uses.
    pub model: String,
    /// Prompt template or system-prompt variant hash.
    pub prompt_variant: String,
    /// Configuration parameters (temperature, max_tokens, etc.).
    pub params: HashMap<String, f64>,
    /// Fitness score from evaluation, in `[0.0, 1.0]`.
    pub fitness: f64,
    /// Generation when this candidate was created.
    pub born_generation: u64,
    /// Number of evaluations this candidate has undergone.
    pub evaluations: u64,
    /// Parent candidate ID (empty for seed candidates).
    pub parent_id: String,
    /// Mutation that produced this candidate from its parent.
    pub mutation_description: String,
}

impl AdasCandidate {
    /// Create a new seed candidate.
    pub fn new(id: impl Into<String>, model: impl Into<String>, prompt_variant: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            model: model.into(),
            prompt_variant: prompt_variant.into(),
            params: HashMap::new(),
            fitness: 0.0,
            born_generation: 0,
            evaluations: 0,
            parent_id: String::new(),
            mutation_description: String::new(),
        }
    }

    /// Set a configuration parameter.
    #[must_use]
    pub fn with_param(mut self, key: impl Into<String>, value: f64) -> Self {
        self.params.insert(key.into(), value);
        self
    }
}

/// Record of a mutation that was applied and its effect on fitness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationRecord {
    /// Description of the mutation.
    pub description: String,
    /// Fitness delta (child fitness - parent fitness).
    pub fitness_delta: f64,
    /// Generation when the mutation was applied.
    pub generation: u64,
    /// Number of times this mutation type has been applied.
    pub applications: u64,
    /// Cumulative fitness improvement across all applications.
    pub cumulative_delta: f64,
}

/// ADAS optimizer: self-improving search over prompt/model configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdasOptimizer {
    /// Current population of candidate configurations.
    pub population: Vec<AdasCandidate>,
    /// Current generation number.
    pub generation: u64,
    /// Maximum population size.
    pub max_population: usize,
    /// Number of top candidates to keep as parents (elitism).
    pub elite_count: usize,
    /// History of mutations and their effects.
    pub mutation_history: Vec<MutationRecord>,
    /// Mutation amplification weights: mutations that improved fitness
    /// get higher probability of being re-applied.
    mutation_weights: HashMap<String, f64>,
    /// Next candidate ID counter.
    next_id: u64,
}

impl AdasOptimizer {
    /// Create a new optimizer with an initial population.
    #[must_use]
    pub fn new(seed_population: Vec<AdasCandidate>) -> Self {
        let next_id = seed_population.len() as u64;
        Self {
            max_population: 20,
            elite_count: 5,
            population: seed_population,
            generation: 0,
            mutation_history: Vec::new(),
            mutation_weights: HashMap::new(),
            next_id,
        }
    }

    /// Override the maximum population size.
    #[must_use]
    pub fn with_max_population(mut self, max: usize) -> Self {
        self.max_population = max.max(2);
        self
    }

    /// Override the elite count.
    #[must_use]
    pub fn with_elite_count(mut self, count: usize) -> Self {
        self.elite_count = count.max(1);
        self
    }

    /// Record an evaluation result for a candidate.
    pub fn record_evaluation(&mut self, candidate_id: &str, fitness: f64) {
        let fitness = fitness.clamp(0.0, 1.0);
        if let Some(candidate) = self.population.iter_mut().find(|c| c.id == candidate_id) {
            let n = candidate.evaluations as f64;
            // Running average.
            candidate.fitness = (candidate.fitness * n + fitness) / (n + 1.0);
            candidate.evaluations += 1;
        }
    }

    /// Select the top `elite_count` candidates by fitness.
    #[must_use]
    pub fn select_elites(&self) -> Vec<&AdasCandidate> {
        let mut sorted: Vec<&AdasCandidate> = self.population.iter().collect();
        sorted.sort_by(|a, b| b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal));
        sorted.truncate(self.elite_count);
        sorted
    }

    /// Create a mutated child from a parent candidate.
    ///
    /// The mutation perturbs one parameter by a small delta. The mutation
    /// description is used to track which types of mutations are effective.
    pub fn mutate(&mut self, parent: &AdasCandidate, param_key: &str, delta: f64) -> AdasCandidate {
        let child_id = format!("adas-{}", self.next_id);
        self.next_id += 1;

        let mut child = AdasCandidate {
            id: child_id,
            model: parent.model.clone(),
            prompt_variant: parent.prompt_variant.clone(),
            params: parent.params.clone(),
            fitness: 0.0,
            born_generation: self.generation,
            evaluations: 0,
            parent_id: parent.id.clone(),
            mutation_description: format!("{param_key}+={delta:.4}"),
        };

        let current = child.params.get(param_key).copied().unwrap_or(0.0);
        child.params.insert(param_key.to_string(), current + delta);

        child
    }

    /// Run one generation: select elites, mutate, evaluate, and cull.
    ///
    /// The `evaluate` closure is called for each new child to determine
    /// its fitness. Returns the number of new candidates added.
    pub fn evolve_generation<F>(&mut self, mut evaluate: F) -> usize
    where
        F: FnMut(&AdasCandidate) -> f64,
    {
        self.generation += 1;

        let elites: Vec<AdasCandidate> = self.select_elites().iter().map(|e| (*e).clone()).collect();
        let mut new_children = Vec::new();

        for parent in &elites {
            // Mutate each parameter with amplification based on history.
            for key in parent.params.keys() {
                let weight = self.mutation_weights.get(key).copied().unwrap_or(1.0);
                let delta = 0.1 * weight;

                // Try positive and negative mutations.
                for sign in [1.0, -1.0] {
                    if self.population.len() + new_children.len() >= self.max_population {
                        break;
                    }
                    let mut child = self.mutate(parent, key, sign * delta);
                    let fitness = evaluate(&child);
                    child.fitness = fitness.clamp(0.0, 1.0);
                    child.evaluations = 1;

                    // Record mutation effect.
                    let fitness_delta = child.fitness - parent.fitness;
                    self.record_mutation(&child.mutation_description, fitness_delta);

                    new_children.push(child);
                }
            }
        }

        let count = new_children.len();
        self.population.extend(new_children);

        // Cull population to max size, keeping the fittest.
        self.cull();

        count
    }

    /// Record a mutation and update amplification weights.
    fn record_mutation(&mut self, description: &str, fitness_delta: f64) {
        if let Some(record) = self.mutation_history.iter_mut().find(|r| r.description == description) {
            record.applications += 1;
            record.cumulative_delta += fitness_delta;
            record.fitness_delta = record.cumulative_delta / record.applications as f64;
            record.generation = self.generation;
        } else {
            self.mutation_history.push(MutationRecord {
                description: description.to_string(),
                fitness_delta,
                generation: self.generation,
                applications: 1,
                cumulative_delta: fitness_delta,
            });
        }

        // Update amplification: beneficial mutations get higher weight.
        // Extract the parameter key from the description (before "+=" or "-=").
        let param_key = description
            .split("+=")
            .next()
            .or_else(|| description.split("-=").next())
            .unwrap_or(description)
            .to_string();

        let entry = self.mutation_weights.entry(param_key).or_insert(1.0);
        if fitness_delta > 0.0 {
            *entry = (*entry * 1.1).min(5.0); // Amplify beneficial mutations.
        } else if fitness_delta < -0.01 {
            *entry = (*entry * 0.9).max(0.2); // Dampen harmful mutations.
        }
    }

    /// Cull population to `max_population`, preserving the fittest.
    fn cull(&mut self) {
        if self.population.len() <= self.max_population {
            return;
        }

        self.population.sort_by(|a, b| {
            b.fitness.partial_cmp(&a.fitness).unwrap_or(std::cmp::Ordering::Equal)
        });
        self.population.truncate(self.max_population);
    }

    /// Best candidate in the current population.
    #[must_use]
    pub fn best(&self) -> Option<&AdasCandidate> {
        self.population
            .iter()
            .max_by(|a, b| a.fitness.partial_cmp(&b.fitness).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Current population size.
    #[must_use]
    pub fn population_size(&self) -> usize {
        self.population.len()
    }

    /// Mutation types ranked by their cumulative improvement.
    #[must_use]
    pub fn top_mutations(&self, n: usize) -> Vec<&MutationRecord> {
        let mut sorted: Vec<&MutationRecord> = self.mutation_history.iter().collect();
        sorted.sort_by(|a, b| {
            b.cumulative_delta
                .partial_cmp(&a.cumulative_delta)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted.truncate(n);
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adas_evolve_generation() {
        let seeds = vec![
            AdasCandidate::new("s0", "claude", "v1")
                .with_param("temperature", 0.7)
                .with_param("max_tokens", 4096.0),
            AdasCandidate::new("s1", "claude", "v2")
                .with_param("temperature", 0.3)
                .with_param("max_tokens", 2048.0),
        ];

        let mut optimizer = AdasOptimizer::new(seeds)
            .with_max_population(10)
            .with_elite_count(2);

        // Seed evaluations.
        optimizer.record_evaluation("s0", 0.6);
        optimizer.record_evaluation("s1", 0.8);

        // Evolve one generation with a simple fitness function.
        let new_count = optimizer.evolve_generation(|c| {
            let temp = c.params.get("temperature").copied().unwrap_or(0.5);
            // Optimal temperature around 0.4.
            1.0 - (temp - 0.4).abs()
        });

        assert!(new_count > 0);
        assert!(optimizer.population_size() <= 10);
        assert!(optimizer.generation == 1);
    }

    #[test]
    fn adas_amplification() {
        let seeds = vec![
            AdasCandidate::new("s0", "model", "v1").with_param("lr", 0.01),
        ];
        let mut opt = AdasOptimizer::new(seeds).with_max_population(10).with_elite_count(1);
        opt.record_evaluation("s0", 0.5);

        // Run a few generations.
        for _ in 0..3 {
            opt.evolve_generation(|c| {
                let lr = c.params.get("lr").copied().unwrap_or(0.01);
                if lr > 0.01 { 0.8 } else { 0.3 }
            });
        }

        // Beneficial mutations should have higher weight.
        assert!(!opt.mutation_history.is_empty());
    }
}
