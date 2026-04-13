use roko_primitives::hdc::HdcVector;

use crate::{KnowledgeEntry, KnowledgeKind};

const CAUSE_SHIFT: usize = 1;
const EFFECT_SHIFT: usize = 2;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct KnowledgeHdcEncoder;

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
        KnowledgeEntry {
            id: "k1".to_string(),
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
            hdc_vector: None,
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
}
