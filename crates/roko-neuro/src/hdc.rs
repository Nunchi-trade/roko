use roko_primitives::hdc::HdcVector;

use crate::{KnowledgeEntry, KnowledgeKind};

const CAUSE_SHIFT: usize = 1;
const EFFECT_SHIFT: usize = 2;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct KnowledgeHdcEncoder;

/// Structured role-filler HDC encoding.
///
/// Encodes structured knowledge using HDC role-filler binding:
/// `role XOR filler` creates a composite vector that preserves structure.
/// Bundling multiple role-filler pairs produces a single vector that can be
/// queried by unbinding any role to recover its filler.
pub(crate) struct RoleFillerEncoder;

impl RoleFillerEncoder {
    /// Encode a set of role-filler string pairs into a single composite HDC vector.
    ///
    /// Each `(role, filler)` pair is bound via XOR, then all pairs are bundled
    /// via majority vote.
    pub(crate) fn encode_structured(roles_and_fillers: &[(String, String)]) -> HdcVector {
        if roles_and_fillers.is_empty() {
            return HdcVector::zeros();
        }
        let bound: Vec<HdcVector> = roles_and_fillers
            .iter()
            .map(|(role, filler)| role_hv(role).bind(&text_hv(filler)))
            .collect();
        let refs: Vec<&HdcVector> = bound.iter().collect();
        HdcVector::bundle(&refs)
    }

    /// Extract the filler for a given role by unbinding (XOR is its own inverse).
    pub(crate) fn query_role(composite: &HdcVector, role: &str) -> HdcVector {
        composite.bind(&role_hv(role))
    }
}

/// A pair of knowledge entries from different domains whose HDC vectors
/// are highly similar, indicating a structural analogy.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResonancePair {
    /// ID of the first entry.
    pub(crate) entry_a: String,
    /// ID of the second entry.
    pub(crate) entry_b: String,
    /// Hamming similarity between the two entries' HDC vectors.
    pub(crate) similarity: f64,
    /// Domain of the first entry.
    pub(crate) domain_a: String,
    /// Domain of the second entry.
    pub(crate) domain_b: String,
}

/// Detects resonant patterns across knowledge domains.
///
/// Two entries "resonate" when their HDC vectors are highly similar
/// despite coming from different source domains. This is a cross-domain
/// analogy detector: retry logic in networking is structurally similar
/// to retry logic in a database crate.
pub(crate) struct ResonanceDetector {
    /// Minimum similarity to consider a pair resonant.
    min_similarity: f64,
    /// Maximum number of pairs to return.
    max_results: usize,
}

impl Default for ResonanceDetector {
    fn default() -> Self {
        Self {
            min_similarity: 0.526,
            max_results: 20,
        }
    }
}

impl ResonanceDetector {
    /// Create a detector with the given similarity threshold and result cap.
    pub(crate) fn new(min_similarity: f64, max_results: usize) -> Self {
        Self {
            min_similarity,
            max_results,
        }
    }

    /// Detect resonant pairs across knowledge domains.
    ///
    /// Performs pairwise comparison, skipping same-domain pairs and pruning
    /// by the similarity threshold. O(n^2) — suitable for stores up to ~10K
    /// entries.
    pub(crate) fn detect_resonances(&self, entries: &[KnowledgeEntry]) -> Vec<ResonancePair> {
        let encoder = KnowledgeHdcEncoder;

        // Pre-encode all entries and extract domains.
        let encoded: Vec<(HdcVector, String)> = entries
            .iter()
            .map(|e| {
                let hv = encoder.encode_entry(e);
                let domain = extract_domain(e);
                (hv, domain)
            })
            .collect();

        let mut pairs = Vec::new();

        for i in 0..encoded.len() {
            for j in (i + 1)..encoded.len() {
                // Skip same-domain pairs.
                if encoded[i].1 == encoded[j].1 {
                    continue;
                }

                let sim = f64::from(encoded[i].0.similarity(&encoded[j].0));
                if sim >= self.min_similarity {
                    pairs.push(ResonancePair {
                        entry_a: entries[i].id.clone(),
                        entry_b: entries[j].id.clone(),
                        similarity: sim,
                        domain_a: encoded[i].1.clone(),
                        domain_b: encoded[j].1.clone(),
                    });
                }
            }
        }

        // Sort by descending similarity, then truncate to max_results.
        pairs.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        pairs.truncate(self.max_results);
        pairs
    }
}

/// Extract the domain from a knowledge entry.
///
/// Looks for a `domain:X` structured tag first, falls back to `source`,
/// then defaults to the knowledge kind as a domain label.
fn extract_domain(entry: &KnowledgeEntry) -> String {
    if let Some(domain) = first_structured_tag_value(&entry.tags, "domain") {
        return domain;
    }
    if let Some(source) = entry.source.as_deref() {
        let trimmed = source.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    entry.kind.as_str().to_string()
}

#[derive(Debug, Clone, PartialEq)]
struct CausalLinkParts {
    cause: String,
    effect: String,
    domain: Option<String>,
    strength: f64,
    conditions: Vec<String>,
}

impl KnowledgeHdcEncoder {
    pub(crate) fn encode_entry(self, entry: &KnowledgeEntry) -> HdcVector {
        if entry.kind == KnowledgeKind::CausalLink {
            self.encode_causal_link(entry)
        } else {
            self.encode_generic_entry(entry)
        }
    }

    pub(crate) fn encode_query(self, topic: &str) -> HdcVector {
        let topic_hv = text_hv(topic);
        let cause_probe = role_hv("cause").permute(CAUSE_SHIFT).bind(&topic_hv);
        let effect_probe = role_hv("effect").permute(EFFECT_SHIFT).bind(&topic_hv);
        bundle(vec![topic_hv, cause_probe, effect_probe])
    }

    /// Build a role-filler probe vector: `bind(role_vector, filler_vector)`.
    ///
    /// Use with `HdcVector::similarity()` against encoded entries to find
    /// entries where the given role is bound to the given filler.
    pub(crate) fn query_by_role(role: &str, filler: &str) -> HdcVector {
        role_hv(role).bind(&text_hv(filler))
    }

    /// Extract the filler component for a given role from a composite vector.
    ///
    /// Since XOR is its own inverse, `unbind(composite, role) = bind(composite, role)`.
    pub(crate) fn unbind_role(composite: &HdcVector, role: &str) -> HdcVector {
        composite.bind(&role_hv(role))
    }

    /// Encode an entry using structured role-filler bindings.
    ///
    /// Unlike `encode_generic_entry` (which optimizes for content-dominant
    /// similarity), this produces a composite vector where each metadata
    /// role can be individually queried via `unbind_role`.
    pub(crate) fn encode_structured(entry: &KnowledgeEntry) -> HdcVector {
        let mut vectors = vec![
            role_hv("content").bind(&text_hv(&entry.content)),
            role_hv("kind").bind(&text_hv(entry.kind.as_str())),
            role_hv("tier").bind(&text_hv(&format!("{:?}", entry.tier).to_ascii_lowercase())),
        ];

        let domain = extract_domain(entry);
        vectors.push(role_hv("domain").bind(&text_hv(&domain)));

        if let Some(source) = entry.source.as_deref() {
            let trimmed = source.trim();
            if !trimmed.is_empty() {
                vectors.push(role_hv("source").bind(&text_hv(trimmed)));
            }
        }

        bundle(vectors)
    }

    fn encode_generic_entry(self, entry: &KnowledgeEntry) -> HdcVector {
        let mut vectors = vec![
            text_hv(&entry.content),
            role_hv("kind").bind(&text_hv(entry.kind.as_str())),
        ];

        if !entry.tags.is_empty() {
            let tags = entry
                .tags
                .iter()
                .map(|tag| text_hv(tag))
                .collect::<Vec<_>>();
            vectors.push(bundle(tags));
        }

        if let Some(source) = entry.source.as_deref() {
            let trimmed = source.trim();
            if !trimmed.is_empty() {
                vectors.push(role_hv("source").bind(&text_hv(trimmed)));
            }
        }

        bundle(vectors)
    }

    fn encode_causal_link(self, entry: &KnowledgeEntry) -> HdcVector {
        let Some(parts) = CausalLinkParts::from_entry(entry) else {
            return self.encode_generic_entry(entry);
        };

        let mut vectors = vec![
            text_hv(&entry.content),
            role_hv("kind").bind(&text_hv(entry.kind.as_str())),
            role_hv("cause")
                .permute(CAUSE_SHIFT)
                .bind(&text_hv(&parts.cause)),
            role_hv("effect")
                .permute(EFFECT_SHIFT)
                .bind(&text_hv(&parts.effect)),
            role_hv("causal_edge").bind(
                &text_hv(&parts.cause)
                    .permute(CAUSE_SHIFT)
                    .bind(&text_hv(&parts.effect).permute(EFFECT_SHIFT)),
            ),
            role_hv("strength").bind(&strength_hv(parts.strength)),
        ];

        if let Some(domain) = parts.domain.as_deref() {
            let trimmed = domain.trim();
            if !trimmed.is_empty() {
                vectors.push(role_hv("domain").bind(&text_hv(trimmed)));
            }
        }

        if !parts.conditions.is_empty() {
            let conditions = parts
                .conditions
                .iter()
                .map(|condition| role_hv("condition").bind(&text_hv(condition)))
                .collect::<Vec<_>>();
            vectors.push(bundle(conditions));
        }

        let general_tags = entry
            .tags
            .iter()
            .filter(|tag| structured_tag(tag).is_none())
            .map(|tag| text_hv(tag))
            .collect::<Vec<_>>();
        if !general_tags.is_empty() {
            vectors.push(bundle(general_tags));
        }

        bundle(vectors)
    }
}

impl CausalLinkParts {
    fn from_entry(entry: &KnowledgeEntry) -> Option<Self> {
        let cause = first_structured_tag_value(&entry.tags, "cause")
            .or_else(|| parse_causal_content(&entry.content).map(|(cause, _)| cause))?;
        let effect = first_structured_tag_value(&entry.tags, "effect")
            .or_else(|| parse_causal_content(&entry.content).map(|(_, effect)| effect))?;
        let domain = first_structured_tag_value(&entry.tags, "domain")
            .or_else(|| entry.source.clone())
            .filter(|value| !value.trim().is_empty());
        let strength = first_structured_tag_value(&entry.tags, "strength")
            .and_then(|value| parse_strength(&value))
            .unwrap_or(entry.confidence);
        let mut conditions = structured_tag_values(&entry.tags, "condition");
        conditions.extend(structured_tag_values(&entry.tags, "conditions"));
        conditions.sort();
        conditions.dedup();

        Some(Self {
            cause,
            effect,
            domain,
            strength,
            conditions,
        })
    }
}

fn parse_causal_content(content: &str) -> Option<(String, String)> {
    for separator in ["->", "=>", "→"] {
        if let Some((cause, effect)) = split_once_trimmed(content, separator) {
            return Some((cause, effect));
        }
    }

    let lower = content.to_ascii_lowercase();
    for separator in [
        " causes ",
        " caused ",
        " leads to ",
        " lead to ",
        " results in ",
        " result in ",
        " triggers ",
        " trigger ",
        " drives ",
        " drive ",
    ] {
        if let Some(index) = lower.find(separator) {
            let cause = content[..index].trim();
            let effect = content[index + separator.len()..].trim();
            if !cause.is_empty() && !effect.is_empty() {
                return Some((cause.to_string(), effect.to_string()));
            }
        }
    }

    None
}

fn split_once_trimmed(content: &str, separator: &str) -> Option<(String, String)> {
    let (left, right) = content.split_once(separator)?;
    let left = left.trim();
    let right = right.trim();
    (!left.is_empty() && !right.is_empty()).then(|| (left.to_string(), right.to_string()))
}

fn first_structured_tag_value(tags: &[String], key: &str) -> Option<String> {
    structured_tag_values(tags, key).into_iter().next()
}

fn structured_tag_values(tags: &[String], key: &str) -> Vec<String> {
    tags.iter()
        .filter_map(|tag| structured_tag(tag))
        .filter(|(tag_key, _)| tag_key == key)
        .map(|(_, value)| value)
        .collect()
}

fn structured_tag(tag: &str) -> Option<(String, String)> {
    let (key, value) = tag.split_once(':')?;
    let key = normalize_text(key);
    let value = value.trim();
    (!key.is_empty() && !value.is_empty()).then(|| (key, value.to_string()))
}

fn parse_strength(value: &str) -> Option<f64> {
    match normalize_text(value).as_str() {
        "very_low" | "weak" | "low" => Some(0.2),
        "medium" | "moderate" => Some(0.5),
        "high" | "strong" => Some(0.8),
        "very_high" => Some(1.0),
        _ => value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|parsed| parsed.clamp(0.0, 1.0)),
    }
}

fn strength_hv(strength: f64) -> HdcVector {
    let bin = (strength.clamp(0.0, 1.0) * 5.0).round() as u8;
    HdcVector::from_seed(format!("strength:{bin}").as_bytes())
}

fn role_hv(role: &str) -> HdcVector {
    HdcVector::from_seed(format!("role:{role}").as_bytes())
}

fn text_hv(text: &str) -> HdcVector {
    HdcVector::from_seed(normalize_text(text).as_bytes())
}

fn bundle(vectors: Vec<HdcVector>) -> HdcVector {
    let refs = vectors.iter().collect::<Vec<_>>();
    HdcVector::bundle(&refs)
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch.is_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KnowledgeEntry, KnowledgeTier};
    use chrono::Utc;

    fn entry(kind: KnowledgeKind, content: &str, tags: &[&str]) -> KnowledgeEntry {
        entry_with_id("k1", kind, content, tags)
    }

    fn entry_with_id(
        id: &str,
        kind: KnowledgeKind,
        content: &str,
        tags: &[&str],
    ) -> KnowledgeEntry {
        KnowledgeEntry {
            id: id.to_string(),
            kind,
            source: None,
            content: content.to_string(),
            confidence: 0.8,
            confidence_weight: 1.0,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: vec!["ep-1".to_string()],
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            source_model: None,
            model_generality: 1.0,
            created_at: Utc::now(),
            half_life_days: kind.default_half_life_days(),
            tier: KnowledgeTier::Working,
            emotional_tag: None,
            emotional_provenance: None,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        }
    }

    #[test]
    fn causal_content_parser_recognizes_arrow_forms() {
        assert_eq!(
            parse_causal_content("high complexity -> more review"),
            Some(("high complexity".to_string(), "more review".to_string()))
        );
        assert_eq!(
            parse_causal_content("high complexity causes more review"),
            Some(("high complexity".to_string(), "more review".to_string()))
        );
    }

    #[test]
    fn causal_link_tags_override_freeform_content() {
        let entry = entry(
            KnowledgeKind::CausalLink,
            "something vague",
            &[
                "cause:high complexity",
                "effect:more review",
                "domain:coding",
            ],
        );
        let parts = CausalLinkParts::from_entry(&entry).expect("causal parts");
        assert_eq!(parts.cause, "high complexity");
        assert_eq!(parts.effect, "more review");
        assert_eq!(parts.domain.as_deref(), Some("coding"));
    }

    #[test]
    fn directional_causal_encoding_distinguishes_reversal() {
        let encoder = KnowledgeHdcEncoder;
        let forward = encoder.encode_entry(&entry(
            KnowledgeKind::CausalLink,
            "high complexity -> more review",
            &["domain:coding"],
        ));
        let reverse = encoder.encode_entry(&entry(
            KnowledgeKind::CausalLink,
            "more review -> high complexity",
            &["domain:coding"],
        ));

        assert!(forward.similarity(&reverse) < 0.7);
    }

    #[test]
    fn causal_query_encoding_matches_both_cause_and_effect() {
        let encoder = KnowledgeHdcEncoder;
        let encoded = encoder.encode_entry(&entry(
            KnowledgeKind::CausalLink,
            "high complexity -> more review",
            &["domain:coding"],
        ));
        let cause_query = encoder.encode_query("high complexity");
        let effect_query = encoder.encode_query("more review");
        let unrelated = encoder.encode_query("postgres vacuum");

        assert!(cause_query.similarity(&encoded) > unrelated.similarity(&encoded));
        assert!(effect_query.similarity(&encoded) > unrelated.similarity(&encoded));
    }

    // ── NEURO-03: Role-filler encoding tests ─────────────────────────────

    #[test]
    fn role_filler_encode_roundtrip() {
        let pairs = vec![
            ("function".to_string(), "main".to_string()),
            ("file".to_string(), "main.rs".to_string()),
            ("language".to_string(), "rust".to_string()),
        ];
        let composite = RoleFillerEncoder::encode_structured(&pairs);

        // Unbinding the "function" role should produce a vector closer to
        // text_hv("main") than to text_hv("main.rs").
        let extracted = RoleFillerEncoder::query_role(&composite, "function");
        let main_hv = text_hv("main");
        let file_hv = text_hv("main rs");
        assert!(extracted.similarity(&main_hv) > extracted.similarity(&file_hv));
    }

    #[test]
    fn role_filler_empty_input() {
        let composite = RoleFillerEncoder::encode_structured(&[]);
        assert_eq!(composite, HdcVector::zeros());
    }

    #[test]
    fn query_by_role_finds_matching_entries() {
        let coding_entry = entry(
            KnowledgeKind::Insight,
            "Prefer small functions",
            &["domain:coding"],
        );
        let infra_entry = entry(
            KnowledgeKind::Insight,
            "Use retry for flaky networks",
            &["domain:infra"],
        );
        let coded = KnowledgeHdcEncoder::encode_structured(&coding_entry);
        let infrad = KnowledgeHdcEncoder::encode_structured(&infra_entry);

        // A probe for domain=coding should match the coding entry better.
        let probe = KnowledgeHdcEncoder::query_by_role("domain", "coding");
        assert!(probe.similarity(&coded) > probe.similarity(&infrad));
    }

    #[test]
    fn unbind_role_recovers_filler_direction() {
        let e = entry(
            KnowledgeKind::Insight,
            "Retry after transient failures",
            &["domain:networking"],
        );
        let composite = KnowledgeHdcEncoder::encode_structured(&e);

        // Unbinding "kind" should be closer to text_hv("insight") than to
        // text_hv("warning").
        let extracted = KnowledgeHdcEncoder::unbind_role(&composite, "kind");
        let insight_hv = text_hv("insight");
        let warning_hv = text_hv("warning");
        assert!(extracted.similarity(&insight_hv) > extracted.similarity(&warning_hv));
    }

    // ── NEURO-02: Resonance detection tests ──────────────────────────────

    #[test]
    fn resonance_detects_cross_domain_similarity() {
        // Two entries with the same content but different domains should resonate.
        let mut e1 = entry_with_id(
            "k1",
            KnowledgeKind::Heuristic,
            "Retry with exponential backoff on transient failures",
            &["domain:networking"],
        );
        e1.source = Some("networking".to_string());

        let mut e2 = entry_with_id(
            "k2",
            KnowledgeKind::Heuristic,
            "Retry with exponential backoff on transient failures",
            &["domain:database"],
        );
        e2.source = Some("database".to_string());

        let detector = ResonanceDetector::new(0.5, 10);
        let pairs = detector.detect_resonances(&[e1, e2]);

        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].entry_a, "k1");
        assert_eq!(pairs[0].entry_b, "k2");
        assert!(pairs[0].similarity > 0.5);
        assert_eq!(pairs[0].domain_a, "networking");
        assert_eq!(pairs[0].domain_b, "database");
    }

    #[test]
    fn resonance_skips_same_domain_pairs() {
        let e1 = entry_with_id(
            "k1",
            KnowledgeKind::Heuristic,
            "Use small functions",
            &["domain:coding"],
        );
        let e2 = entry_with_id(
            "k2",
            KnowledgeKind::Heuristic,
            "Use small functions",
            &["domain:coding"],
        );

        let detector = ResonanceDetector::new(0.3, 10);
        let pairs = detector.detect_resonances(&[e1, e2]);
        assert!(pairs.is_empty());
    }

    #[test]
    fn resonance_respects_max_results() {
        let entries: Vec<KnowledgeEntry> = (0..6)
            .map(|i| {
                let domain = if i % 2 == 0 { "alpha" } else { "beta" };
                entry_with_id(
                    &format!("k{i}"),
                    KnowledgeKind::Insight,
                    "Shared concept across domains",
                    &[&format!("domain:{domain}")],
                )
            })
            .collect();

        let detector = ResonanceDetector::new(0.3, 2);
        let pairs = detector.detect_resonances(&entries);
        assert!(pairs.len() <= 2);
    }

    #[test]
    fn resonance_filters_below_threshold() {
        let e1 = entry_with_id(
            "k1",
            KnowledgeKind::Insight,
            "Alpha centauri mission planning",
            &["domain:space"],
        );
        let e2 = entry_with_id(
            "k2",
            KnowledgeKind::Warning,
            "Database vacuum schedule every Tuesday",
            &["domain:infra"],
        );

        // Very different content + different kinds => low similarity.
        let detector = ResonanceDetector::new(0.7, 10);
        let pairs = detector.detect_resonances(&[e1, e2]);
        assert!(pairs.is_empty());
    }
}
