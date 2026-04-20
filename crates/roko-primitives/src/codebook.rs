//! HDC Codebook — deterministic symbol allocation and role-filler composition (TA-05).
//!
//! A [`Codebook`] maps symbolic names to deterministic [`HdcVector`]s using
//! seeded generation. Role-filler binding encodes structured knowledge:
//! `role_bind(role_vec, filler_vec)` produces a compound vector that can be
//! unbound later with `unbind(compound, role_vec)` to recover the filler.
//!
//! Domain codebooks (e.g., [`CodingCodebook`]) provide pre-allocated symbols
//! for their respective analysis domains. Cross-domain resonance detection
//! identifies when patterns from different domains share structural similarity
//! beyond the chance threshold of 0.526 (for 10,240-bit BSC vectors).

use crate::HdcVector;
use std::collections::HashMap;

/// Similarity threshold for cross-domain resonance detection.
///
/// For 10,240-bit BSC vectors, random similarity is approximately 0.5.
/// A threshold of 0.526 corresponds to roughly 3 standard deviations above
/// random chance (p < 0.001), indicating genuine structural similarity.
pub const RESONANCE_THRESHOLD: f32 = 0.526;

/// A codebook mapping symbolic names to deterministic HDC vectors.
///
/// Symbols are generated from a domain-specific seed, ensuring reproducibility
/// across runs. The codebook supports role-filler binding for structured
/// knowledge representation.
#[derive(Debug, Clone)]
pub struct Codebook {
    /// Domain identifier used as the generation seed prefix.
    domain: String,
    /// Allocated symbols: name -> vector.
    symbols: HashMap<String, HdcVector>,
}

impl Codebook {
    /// Create a new empty codebook for the given domain.
    ///
    /// The domain string is used as a seed prefix so identical domain names
    /// produce identical symbol vectors across runs.
    #[must_use]
    pub fn new(domain: impl Into<String>) -> Self {
        Self {
            domain: domain.into(),
            symbols: HashMap::new(),
        }
    }

    /// Allocate a new symbol in the codebook.
    ///
    /// The vector is deterministically derived from the domain name and
    /// symbol name, so the same `(domain, name)` pair always produces the
    /// same vector.
    pub fn allocate(&mut self, name: impl Into<String>) -> &HdcVector {
        let name = name.into();
        self.symbols.entry(name.clone()).or_insert_with(|| {
            let seed = format!("{}:{}", self.domain, name);
            HdcVector::from_seed(seed.as_bytes())
        })
    }

    /// Look up a symbol by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&HdcVector> {
        self.symbols.get(name)
    }

    /// Get or allocate a symbol.
    pub fn get_or_allocate(&mut self, name: &str) -> &HdcVector {
        if !self.symbols.contains_key(name) {
            self.allocate(name.to_string());
        }
        self.symbols.get(name).unwrap()
    }

    /// Number of allocated symbols.
    #[must_use]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Whether no symbols are allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Return all symbol names.
    pub fn symbol_names(&self) -> impl Iterator<Item = &str> {
        self.symbols.keys().map(String::as_str)
    }

    /// Return the domain name.
    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }
}

// ─── Role-filler binding ──────────────────────────────────────────────────────

/// Bind a role vector to a filler vector using XOR.
///
/// This produces a compound vector encoding the relationship "role = filler".
/// The binding is involutory: `role_bind(role_bind(role, filler), role) == filler`.
#[must_use]
pub fn role_bind(role: &HdcVector, filler: &HdcVector) -> HdcVector {
    role.bind(filler)
}

/// Unbind a role from a compound vector, recovering the filler.
///
/// Since XOR is its own inverse, this is identical to `role_bind`.
#[must_use]
pub fn unbind(compound: &HdcVector, role: &HdcVector) -> HdcVector {
    compound.bind(role)
}

// ─── CodingCodebook ──────────────────────────────────────────────────────────

/// Pre-allocated coding domain codebook with 15+ symbols.
///
/// Symbols represent common software engineering concepts used in
/// pattern recognition across coding episodes.
pub struct CodingCodebook {
    codebook: Codebook,
}

/// Names of the standard coding domain symbols.
const CODING_SYMBOLS: &[&str] = &[
    "compile_error",
    "test_failure",
    "lint_warning",
    "type_mismatch",
    "missing_import",
    "unused_variable",
    "borrow_check",
    "lifetime_error",
    "trait_bound",
    "refactor",
    "new_function",
    "dependency_add",
    "test_added",
    "performance",
    "security",
    "documentation",
];

impl CodingCodebook {
    /// Create a coding codebook with all standard symbols pre-allocated.
    #[must_use]
    pub fn new() -> Self {
        let mut codebook = Codebook::new("coding");
        for symbol in CODING_SYMBOLS {
            codebook.allocate(*symbol);
        }
        Self { codebook }
    }

    /// Look up a coding symbol by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&HdcVector> {
        self.codebook.get(name)
    }

    /// Access the underlying codebook.
    #[must_use]
    pub fn codebook(&self) -> &Codebook {
        &self.codebook
    }

    /// Number of allocated symbols (always >= 16).
    #[must_use]
    pub fn len(&self) -> usize {
        self.codebook.len()
    }

    /// Whether the codebook is empty (never true after construction).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codebook.is_empty()
    }
}

impl Default for CodingCodebook {
    fn default() -> Self {
        Self::new()
    }
}

// ─── PatternStore ─────────────────────────────────────────────────────────────

/// A stored pattern with its HDC fingerprint and metadata.
#[derive(Debug, Clone)]
pub struct StoredPattern {
    /// Human-readable label for this pattern.
    pub label: String,
    /// HDC fingerprint of the pattern.
    pub fingerprint: HdcVector,
    /// Number of times this pattern has been observed.
    pub observation_count: u64,
    /// Domain from which this pattern originated.
    pub source_domain: String,
}

/// Store for HDC-encoded patterns with similarity-based retrieval.
///
/// Patterns are stored as `(label, HdcVector)` pairs and can be queried
/// by similarity to a probe vector.
#[derive(Debug, Clone, Default)]
pub struct PatternStore {
    patterns: Vec<StoredPattern>,
}

impl PatternStore {
    /// Create an empty pattern store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a pattern into the store.
    pub fn insert(&mut self, pattern: StoredPattern) {
        // Check if pattern already exists (by label) and update.
        if let Some(existing) = self.patterns.iter_mut().find(|p| p.label == pattern.label) {
            existing.observation_count += pattern.observation_count;
            existing.fingerprint = pattern.fingerprint;
            return;
        }
        self.patterns.push(pattern);
    }

    /// Retrieve patterns similar to the probe, above the given threshold.
    ///
    /// Returns `(label, similarity)` pairs sorted by similarity descending.
    #[must_use]
    pub fn query_similar(&self, probe: &HdcVector, threshold: f32) -> Vec<(&str, f32)> {
        let mut results: Vec<(&str, f32)> = self
            .patterns
            .iter()
            .map(|p| (p.label.as_str(), p.fingerprint.similarity(probe)))
            .filter(|(_, sim)| *sim >= threshold)
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results
    }

    /// Find the single most similar pattern to the probe.
    #[must_use]
    pub fn nearest(&self, probe: &HdcVector) -> Option<(&str, f32)> {
        self.patterns
            .iter()
            .map(|p| (p.label.as_str(), p.fingerprint.similarity(probe)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Number of stored patterns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Whether the store is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

// ─── Cross-domain resonance detection ────────────────────────────────────────

/// Result of a cross-domain resonance check.
#[derive(Debug, Clone)]
pub struct ResonanceResult {
    /// Pattern from domain A.
    pub pattern_a: String,
    /// Domain of pattern A.
    pub domain_a: String,
    /// Pattern from domain B.
    pub pattern_b: String,
    /// Domain of pattern B.
    pub domain_b: String,
    /// Similarity between the two patterns.
    pub similarity: f32,
}

/// Detect cross-domain resonance between two pattern stores.
///
/// Returns all pairs of patterns from different domains whose similarity
/// exceeds [`RESONANCE_THRESHOLD`] (0.526), indicating genuine structural
/// similarity beyond random chance.
#[must_use]
pub fn detect_cross_domain_resonance(
    store_a: &PatternStore,
    domain_a: &str,
    store_b: &PatternStore,
    domain_b: &str,
) -> Vec<ResonanceResult> {
    let mut results = Vec::new();
    for a in &store_a.patterns {
        for b in &store_b.patterns {
            let sim = a.fingerprint.similarity(&b.fingerprint);
            if sim >= RESONANCE_THRESHOLD {
                results.push(ResonanceResult {
                    pattern_a: a.label.clone(),
                    domain_a: domain_a.to_string(),
                    pattern_b: b.label.clone(),
                    domain_b: domain_b.to_string(),
                    similarity: sim,
                });
            }
        }
    }
    results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codebook_deterministic_allocation() {
        let mut cb1 = Codebook::new("test-domain");
        let mut cb2 = Codebook::new("test-domain");

        cb1.allocate("symbol_a");
        cb2.allocate("symbol_a");

        let v1 = cb1.get("symbol_a").unwrap();
        let v2 = cb2.get("symbol_a").unwrap();

        // Same domain + name -> identical vectors.
        assert_eq!(v1.similarity(v2), 1.0);
    }

    #[test]
    fn codebook_different_symbols_are_dissimilar() {
        let mut cb = Codebook::new("test");
        cb.allocate("alpha");
        cb.allocate("beta");

        let alpha = cb.get("alpha").unwrap();
        let beta = cb.get("beta").unwrap();

        // Random vectors should have ~0.5 similarity.
        let sim = alpha.similarity(beta);
        assert!(
            (sim - 0.5).abs() < 0.1,
            "different symbols should be near-orthogonal: {sim}"
        );
    }

    #[test]
    fn role_bind_unbind_recovers_filler() {
        let role = HdcVector::from_seed(b"role:agent_type");
        let filler = HdcVector::from_seed(b"filler:code_review");

        let compound = role_bind(&role, &filler);

        // Unbinding should recover the filler.
        let recovered = unbind(&compound, &role);
        assert_eq!(recovered.similarity(&filler), 1.0);
    }

    #[test]
    fn role_bind_is_involutory() {
        let a = HdcVector::from_seed(b"vec_a");
        let b = HdcVector::from_seed(b"vec_b");

        let bound = role_bind(&a, &b);
        let unbound = role_bind(&bound, &a);
        assert_eq!(unbound.similarity(&b), 1.0);
    }

    #[test]
    fn coding_codebook_has_standard_symbols() {
        let cb = CodingCodebook::new();
        assert!(cb.len() >= 16);
        assert!(!cb.is_empty());

        // Check a few standard symbols exist.
        assert!(cb.get("compile_error").is_some());
        assert!(cb.get("test_failure").is_some());
        assert!(cb.get("borrow_check").is_some());
        assert!(cb.get("refactor").is_some());
    }

    #[test]
    fn coding_codebook_symbols_are_near_orthogonal() {
        let cb = CodingCodebook::new();
        let compile = cb.get("compile_error").unwrap();
        let test = cb.get("test_failure").unwrap();

        let sim = compile.similarity(test);
        assert!(
            (sim - 0.5).abs() < 0.1,
            "coding symbols should be near-orthogonal: {sim}"
        );
    }

    #[test]
    fn pattern_store_insert_and_query() {
        let mut store = PatternStore::new();

        let fp = HdcVector::from_seed(b"pattern_1");
        store.insert(StoredPattern {
            label: "compile-then-fix".to_string(),
            fingerprint: fp,
            observation_count: 5,
            source_domain: "coding".to_string(),
        });

        let fp2 = HdcVector::from_seed(b"pattern_2");
        store.insert(StoredPattern {
            label: "test-then-refactor".to_string(),
            fingerprint: fp2,
            observation_count: 3,
            source_domain: "coding".to_string(),
        });

        assert_eq!(store.len(), 2);

        // Query with pattern_1's fingerprint should return pattern_1 as nearest.
        let probe = HdcVector::from_seed(b"pattern_1");
        let nearest = store.nearest(&probe).unwrap();
        assert_eq!(nearest.0, "compile-then-fix");
        assert_eq!(nearest.1, 1.0);
    }

    #[test]
    fn pattern_store_similarity_threshold() {
        let mut store = PatternStore::new();

        // Insert two very different patterns.
        store.insert(StoredPattern {
            label: "a".to_string(),
            fingerprint: HdcVector::from_seed(b"pattern_a"),
            observation_count: 1,
            source_domain: "coding".to_string(),
        });
        store.insert(StoredPattern {
            label: "b".to_string(),
            fingerprint: HdcVector::from_seed(b"pattern_b"),
            observation_count: 1,
            source_domain: "coding".to_string(),
        });

        // High threshold should only return exact matches.
        let probe = HdcVector::from_seed(b"pattern_a");
        let results = store.query_similar(&probe, 0.9);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "a");
    }

    #[test]
    fn cross_domain_resonance_same_pattern_detected() {
        let mut store_a = PatternStore::new();
        let mut store_b = PatternStore::new();

        // Same seed -> identical fingerprint -> should detect resonance.
        let shared_fp = HdcVector::from_seed(b"shared_pattern");
        store_a.insert(StoredPattern {
            label: "coding_pattern".to_string(),
            fingerprint: shared_fp,
            observation_count: 5,
            source_domain: "coding".to_string(),
        });
        store_b.insert(StoredPattern {
            label: "research_pattern".to_string(),
            fingerprint: shared_fp,
            observation_count: 3,
            source_domain: "research".to_string(),
        });

        let resonances = detect_cross_domain_resonance(&store_a, "coding", &store_b, "research");
        assert!(!resonances.is_empty());
        assert_eq!(resonances[0].similarity, 1.0);
        assert_eq!(resonances[0].pattern_a, "coding_pattern");
        assert_eq!(resonances[0].pattern_b, "research_pattern");
    }

    #[test]
    fn cross_domain_no_resonance_for_random_patterns() {
        let mut store_a = PatternStore::new();
        let mut store_b = PatternStore::new();

        // Different seeds -> random fingerprints -> unlikely to resonate.
        store_a.insert(StoredPattern {
            label: "a".to_string(),
            fingerprint: HdcVector::from_seed(b"unique_a_seed_123"),
            observation_count: 1,
            source_domain: "coding".to_string(),
        });
        store_b.insert(StoredPattern {
            label: "b".to_string(),
            fingerprint: HdcVector::from_seed(b"unique_b_seed_456"),
            observation_count: 1,
            source_domain: "chain".to_string(),
        });

        let resonances = detect_cross_domain_resonance(&store_a, "coding", &store_b, "chain");
        // Random vectors have ~0.5 similarity, below the 0.526 threshold.
        assert!(
            resonances.is_empty(),
            "random patterns should not resonate: found {} matches",
            resonances.len()
        );
    }

    #[test]
    fn pattern_store_duplicate_label_updates() {
        let mut store = PatternStore::new();

        store.insert(StoredPattern {
            label: "compile-fix".to_string(),
            fingerprint: HdcVector::from_seed(b"v1"),
            observation_count: 3,
            source_domain: "coding".to_string(),
        });
        store.insert(StoredPattern {
            label: "compile-fix".to_string(),
            fingerprint: HdcVector::from_seed(b"v2"),
            observation_count: 2,
            source_domain: "coding".to_string(),
        });

        // Should have 1 pattern, not 2.
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn codebook_get_or_allocate() {
        let mut cb = Codebook::new("test");
        let v1 = cb.get_or_allocate("symbol_x") as *const HdcVector;
        let v2 = cb.get_or_allocate("symbol_x") as *const HdcVector;
        assert_eq!(v1, v2, "same symbol should return same pointer");
        assert_eq!(cb.len(), 1);
    }
}
