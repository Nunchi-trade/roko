//! `FactCheckGate` — Perplexity-powered fact verification gate.
//!
//! Uses a [`SearchOracle`] (typically backed by `PerplexitySearchClient`) to
//! verify claims extracted from agent output against live web search results.
//!
//! # Motivation
//!
//! LLMs hallucinate factual claims — dates, paper titles, API names, version
//! numbers. This gate catches those by grounding each extracted claim against
//! Perplexity Sonar search results before the output is accepted.
//!
//! # Why a local trait
//!
//! `roko-gate` deliberately does **not** depend on `roko-agent` (cycle), so
//! this module defines a minimal [`SearchOracle`] trait. Callers wire in an
//! implementation backed by `PerplexitySearchClient` or a mock.
//!
//! # Configuration
//!
//! Enabled via `[gates.fact_check]` in config. Off by default.
//!
//! ```toml
//! [gates.fact_check]
//! min_confidence = 0.7   # fraction of claims that must be web-verifiable
//! ```
//!
//! # Acceptance
//!
//! - Gate **passes** when `verified / total >= min_confidence`
//! - Gate **passes** when the output contains no verifiable claims (0/0 = 1.0)
//! - Gate **fails** when fewer claims than the threshold are web-verifiable,
//!   with a reason of the form `"Fact check: 2/4 claims verified (50%)"`

use async_trait::async_trait;
use roko_core::{Context, Gate, Signal, Verdict};
use std::sync::Arc;
use std::time::Instant;

// ─── Oracle trait ──────────────────────────────────────────────────────────

/// A single web search hit returned by a [`SearchOracle`].
#[derive(Debug, Clone, Default)]
pub struct SearchHit {
    /// The snippet / page content surfaced by the search engine.
    pub content: String,
}

/// Minimal search interface the gate delegates to.
///
/// Implementors typically wrap `PerplexitySearchClient::search`. Tests use a
/// mock.
///
/// # Errors
///
/// Any transport or parsing failure should be returned as `Err(String)`.
#[async_trait]
pub trait SearchOracle: Send + Sync {
    /// Search for `query` and return a ranked list of content snippets.
    async fn search(&self, query: &str) -> Result<Vec<SearchHit>, String>;
}

// ─── Gate ─────────────────────────────────────────────────────────────────

/// Perplexity-backed fact-checking gate.
///
/// Extracts verifiable claims from the signal body, searches for each one, and
/// passes iff the fraction of web-verifiable claims meets [`min_confidence`].
///
/// [`min_confidence`]: FactCheckGate::min_confidence
pub struct FactCheckGate {
    oracle: Arc<dyn SearchOracle>,
    min_confidence: f64,
    name: String,
}

impl FactCheckGate {
    /// Default confidence threshold (70 % of claims must be verified).
    pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;

    /// Construct a gate with the given oracle and minimum confidence.
    ///
    /// `min_confidence` is clamped to `[0.0, 1.0]`.
    #[must_use]
    pub fn new(oracle: Arc<dyn SearchOracle>, min_confidence: f64) -> Self {
        Self {
            oracle,
            min_confidence: min_confidence.clamp(0.0, 1.0),
            name: "fact_check".to_string(),
        }
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The configured minimum confidence threshold (`[0.0, 1.0]`).
    #[must_use]
    pub const fn min_confidence(&self) -> f64 {
        self.min_confidence
    }

    /// Extract verifiable claims from a block of text.
    ///
    /// A "claim" is a sentence that:
    /// - Is at least 20 characters long (filters fragments)
    /// - Contains at least one word longer than 4 characters (filters filler)
    ///
    /// Splits on `.`, `!`, and `?` boundaries.
    fn extract_claims(text: &str) -> Vec<String> {
        text.split(['.', '!', '?'])
            .map(str::trim)
            .filter(|s| {
                s.len() >= 20
                    && s.split_whitespace()
                        .any(|w| w.chars().filter(|c| c.is_alphabetic()).count() > 4)
            })
            .map(str::to_string)
            .collect()
    }

    /// Return `true` if `content` contains enough keywords from `claim` to be
    /// considered supporting evidence.
    ///
    /// Uses a simple heuristic: extract alphabetic words longer than 4 chars
    /// from the claim; pass if ≥ half of them appear (case-insensitively) in
    /// the content.
    fn supports_claim(content: &str, claim: &str) -> bool {
        let keywords: Vec<String> = claim
            .split_whitespace()
            .filter_map(|w| {
                let alpha: String = w.chars().filter(|c| c.is_alphabetic()).collect();
                if alpha.len() > 4 {
                    Some(alpha.to_lowercase())
                } else {
                    None
                }
            })
            .collect();

        if keywords.is_empty() {
            return false;
        }

        let content_lower = content.to_lowercase();
        let matched = keywords
            .iter()
            .filter(|kw| content_lower.contains(kw.as_str()))
            .count();

        // Require at least half the keywords to appear in the content.
        matched * 2 >= keywords.len()
    }
}

#[async_trait]
impl Gate for FactCheckGate {
    async fn verify(&self, signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let elapsed_ms = |t: Instant| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Extract text from signal body.
        let text = match signal.body.as_text() {
            Ok(t) if !t.is_empty() => t.to_string(),
            _ => {
                return Verdict::pass(&self.name)
                    .with_detail("no text content to fact-check")
                    .with_duration(elapsed_ms(started));
            }
        };

        let claims = Self::extract_claims(&text);

        // No verifiable claims → trivially pass.
        if claims.is_empty() {
            return Verdict::pass(&self.name)
                .with_detail("no verifiable claims found")
                .with_score(1.0)
                .with_duration(elapsed_ms(started));
        }

        let mut verified: usize = 0;
        let total = claims.len();

        for claim in &claims {
            match self.oracle.search(claim).await {
                Ok(hits) => {
                    if hits.iter().any(|h| Self::supports_claim(&h.content, claim)) {
                        verified += 1;
                    }
                }
                Err(err) => {
                    // A search error on one claim doesn't abort the whole check —
                    // we simply count that claim as unverified and note the error.
                    let _ = err; // logged by caller if needed
                }
            }
        }

        let confidence = verified as f64 / total as f64;
        #[allow(clippy::cast_possible_truncation)]
        let score = confidence as f32;

        let verdict = if confidence >= self.min_confidence {
            Verdict::pass(&self.name)
                .with_score(score)
                .with_detail(format!(
                    "Fact check: {verified}/{total} claims verified ({:.0}%)",
                    confidence * 100.0
                ))
        } else {
            Verdict::fail(
                &self.name,
                format!(
                    "Fact check: {verified}/{total} claims verified ({:.0}%)",
                    confidence * 100.0
                ),
            )
            .with_score(score)
        };

        verdict.with_duration(elapsed_ms(started))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    // ── Mock oracle ──────────────────────────────────────────────────────

    /// Oracle that returns a fixed list of hits for every query.
    struct ConstOracle {
        hits: Vec<SearchHit>,
    }

    impl ConstOracle {
        fn with_content(content: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                hits: vec![SearchHit {
                    content: content.into(),
                }],
            })
        }

        fn empty() -> Arc<Self> {
            Arc::new(Self { hits: vec![] })
        }
    }

    #[async_trait]
    impl SearchOracle for ConstOracle {
        async fn search(&self, _query: &str) -> Result<Vec<SearchHit>, String> {
            Ok(self.hits.clone())
        }
    }

    /// Oracle that always returns an error.
    struct ErrOracle;

    #[async_trait]
    impl SearchOracle for ErrOracle {
        async fn search(&self, _query: &str) -> Result<Vec<SearchHit>, String> {
            Err("simulated search failure".to_string())
        }
    }

    /// Oracle that returns content matching only specific queries.
    struct SelectiveOracle {
        match_keyword: String,
        hit_content: String,
    }

    impl SelectiveOracle {
        fn new(match_keyword: impl Into<String>, hit_content: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                match_keyword: match_keyword.into(),
                hit_content: hit_content.into(),
            })
        }
    }

    #[async_trait]
    impl SearchOracle for SelectiveOracle {
        async fn search(&self, query: &str) -> Result<Vec<SearchHit>, String> {
            if query.to_lowercase().contains(&self.match_keyword.to_lowercase()) {
                Ok(vec![SearchHit {
                    content: self.hit_content.clone(),
                }])
            } else {
                Ok(vec![])
            }
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn text_signal(text: &str) -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::text(text))
            .build()
    }

    fn empty_signal() -> Signal {
        Signal::builder(Kind::AgentOutput)
            .body(Body::empty())
            .build()
    }

    // ── claim extraction ─────────────────────────────────────────────────

    #[test]
    fn extract_claims_splits_on_sentence_terminators() {
        let text = "Rust is a systems programming language. It provides memory safety! \
                    Tokio is an async runtime?";
        let claims = FactCheckGate::extract_claims(text);
        assert_eq!(claims.len(), 3);
        assert!(claims[0].contains("Rust"));
        assert!(claims[1].contains("memory safety"));
        assert!(claims[2].contains("Tokio"));
    }

    #[test]
    fn extract_claims_filters_short_fragments() {
        let text = "Hello. Rust is a systems programming language. OK? Yes.";
        let claims = FactCheckGate::extract_claims(text);
        // Only the long sentence survives the length filter.
        assert_eq!(claims.len(), 1);
        assert!(claims[0].contains("systems programming"));
    }

    #[test]
    fn extract_claims_empty_input_returns_empty() {
        assert!(FactCheckGate::extract_claims("").is_empty());
    }

    // ── supports_claim ───────────────────────────────────────────────────

    #[test]
    fn supports_claim_returns_true_when_keywords_present() {
        let content = "Rust provides memory safety without garbage collection";
        let claim = "Rust provides memory safety";
        assert!(FactCheckGate::supports_claim(content, claim));
    }

    #[test]
    fn supports_claim_returns_false_when_keywords_absent() {
        let content = "Python is a dynamically typed language";
        let claim = "Rust provides memory safety through ownership";
        assert!(!FactCheckGate::supports_claim(content, claim));
    }

    #[test]
    fn supports_claim_is_case_insensitive() {
        let content = "RUST PROVIDES MEMORY SAFETY";
        let claim = "Rust provides memory safety";
        assert!(FactCheckGate::supports_claim(content, claim));
    }

    #[test]
    fn supports_claim_short_words_only_returns_false() {
        // All words ≤ 4 chars → no keywords extracted → false
        assert!(!FactCheckGate::supports_claim("a b c", "a b c"));
    }

    // ── gate: passing cases ──────────────────────────────────────────────

    #[tokio::test]
    async fn passes_when_all_claims_verified() {
        let oracle = ConstOracle::with_content(
            "Rust is a systems programming language with memory safety features",
        );
        let gate = FactCheckGate::new(oracle, 0.7);
        let text = "Rust is a systems programming language. \
                    Rust provides memory safety features through ownership.";
        let v = gate.verify(&text_signal(text), &Context::at(0)).await;
        assert!(v.passed, "should pass: {}", v.reason);
        assert!(v.score >= 0.7);
    }

    #[tokio::test]
    async fn passes_with_empty_body() {
        let gate = FactCheckGate::new(ConstOracle::empty(), 0.7);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(v.passed, "empty body should pass");
    }

    #[tokio::test]
    async fn passes_when_no_verifiable_claims() {
        let oracle = ConstOracle::empty();
        let gate = FactCheckGate::new(oracle, 0.7);
        // Short sentences that don't pass the claim filter.
        let v = gate.verify(&text_signal("Hi. Yes. OK."), &Context::at(0)).await;
        assert!(v.passed);
        assert!((v.score - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn passes_at_exact_confidence_threshold() {
        // 3 claims, 1 verified → 33% → threshold of 0.25 should pass.
        let oracle = SelectiveOracle::new(
            "ownership",
            "Rust ownership enforces memory safety through its borrow checker",
        );
        let gate = FactCheckGate::new(oracle, 0.25);
        // One claim mentions ownership (verifiable), the other two do not.
        let text = "Rust ownership enforces memory safety completely. \
                    Tokio implements async runtime scheduling features. \
                    Python is dynamically typed and interpreted language.";
        let v = gate.verify(&text_signal(text), &Context::at(0)).await;
        assert!(v.passed, "should pass at low threshold: {}", v.reason);
    }

    // ── gate: failing cases ──────────────────────────────────────────────

    #[tokio::test]
    async fn fails_when_no_claims_verified() {
        let oracle = ConstOracle::empty(); // no search results → no verifications
        let gate = FactCheckGate::new(oracle, 0.7);
        let text = "Rust is a systems programming language. \
                    Tokio is an async runtime for Rust applications.";
        let v = gate.verify(&text_signal(text), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("Fact check"), "reason: {}", v.reason);
        assert!(v.reason.contains("0/"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn fails_with_descriptive_reason() {
        let oracle = ConstOracle::empty();
        let gate = FactCheckGate::new(oracle, 0.7);
        let text = "Rust is a systems programming language with memory safety. \
                    The Tokio runtime provides async scheduling capabilities.";
        let v = gate.verify(&text_signal(text), &Context::at(0)).await;
        assert!(!v.passed);
        // Reason should include verified/total and percentage.
        assert!(v.reason.contains('%'), "reason: {}", v.reason);
    }

    // ── gate: error handling ─────────────────────────────────────────────

    #[tokio::test]
    async fn search_error_counts_claim_as_unverified() {
        let gate = FactCheckGate::new(Arc::new(ErrOracle), 0.7);
        let text = "Rust is a systems programming language with memory safety. \
                    Tokio is an async runtime for Rust applications.";
        let v = gate.verify(&text_signal(text), &Context::at(0)).await;
        // All searches failed → 0 verified → should fail at 0.7 threshold.
        assert!(!v.passed);
        assert!(v.reason.contains("0/"));
    }

    // ── gate: metadata ───────────────────────────────────────────────────

    #[tokio::test]
    async fn verdict_records_gate_name() {
        let gate = FactCheckGate::new(ConstOracle::empty(), 0.7).with_name("perplexity_check");
        assert_eq!(gate.name(), "perplexity_check");
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert_eq!(v.gate, "perplexity_check");
    }

    #[tokio::test]
    async fn verdict_records_duration() {
        let gate = FactCheckGate::new(ConstOracle::empty(), 0.7);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert_ne!(v.duration_ms, u64::MAX, "duration must be set");
    }

    #[tokio::test]
    async fn min_confidence_is_clamped() {
        let lo = FactCheckGate::new(ConstOracle::empty(), -1.0);
        assert!((lo.min_confidence() - 0.0).abs() < 1e-9);
        let hi = FactCheckGate::new(ConstOracle::empty(), 5.0);
        assert!((hi.min_confidence() - 1.0).abs() < 1e-9);
    }
}
