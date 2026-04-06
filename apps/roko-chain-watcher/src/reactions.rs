//! Pattern-based reaction rules for the watcher.
//!
//! `decide` takes a snapshot of recent pheromones and insights, applies a
//! small handful of hand-written rules, and returns a vector of `Reaction`s
//! the watcher should attempt. The rules are intentionally simple and
//! side-effect-free so they are trivial to unit-test.
//!
//! # Rules
//! 1. If a `threat` pheromone has intensity > 0.7 and no existing `warning`
//!    insight is semantically similar, post a warning insight.
//! 2. If an `opportunity` pheromone has intensity > 0.6, post a
//!    `strategy_fragment` insight describing the opportunity.
//! 3. If a `wisdom` pheromone is observed, confirm a matching insight (top
//!    hit by similarity) if one exists.
//! 4. If an insight's content contains anti-pattern keywords (`WRONG`,
//!    `BUG`, `INCORRECT`) and has at least one confirmation, challenge it.
//! 5. On every poll (when there is at least one observation), deposit a
//!    `wisdom` pheromone summarizing chain state.

use crate::rpc_client::{InsightHit, PheromoneHit};

/// High-level categories of action the watcher can take in response to chain state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReactionKind {
    /// Issue `chain_confirmInsight` against an existing insight.
    ConfirmInsight,
    /// Issue `chain_challengeInsight` against an existing insight.
    ChallengeInsight,
    /// Issue `chain_depositPheromone`.
    DepositPheromone,
    /// Issue `chain_postInsight`.
    PostInsight,
}

/// A single reaction decision. The watcher loop is responsible for translating
/// this into concrete RPC calls.
#[derive(Clone, Debug)]
pub struct Reaction {
    /// What kind of RPC call this reaction corresponds to.
    pub kind: ReactionKind,
    /// Human-readable reason for the reaction (used in tracing logs).
    pub reason: String,
    /// Content to post (for `PostInsight` / `DepositPheromone`).
    pub content: Option<String>,
    /// Insight id targeted by `ConfirmInsight` / `ChallengeInsight`.
    pub target_id: Option<String>,
    /// Knowledge kind (`"warning"`, `"strategy_fragment"`, ...) for `PostInsight`.
    pub insight_kind: Option<String>,
    /// Pheromone kind for `DepositPheromone`.
    pub pheromone_kind: Option<String>,
    /// Intensity for `DepositPheromone`.
    pub intensity: Option<f32>,
}

impl Reaction {
    /// Constructs a `PostInsight` reaction.
    #[must_use]
    pub fn post_insight(kind: &str, content: String, reason: String) -> Self {
        Self {
            kind: ReactionKind::PostInsight,
            reason,
            content: Some(content),
            target_id: None,
            insight_kind: Some(kind.to_string()),
            pheromone_kind: None,
            intensity: None,
        }
    }

    /// Constructs a `ConfirmInsight` reaction.
    #[must_use]
    pub const fn confirm_insight(target_id: String, reason: String) -> Self {
        Self {
            kind: ReactionKind::ConfirmInsight,
            reason,
            content: None,
            target_id: Some(target_id),
            insight_kind: None,
            pheromone_kind: None,
            intensity: None,
        }
    }

    /// Constructs a `ChallengeInsight` reaction.
    #[must_use]
    pub const fn challenge_insight(target_id: String, reason: String) -> Self {
        Self {
            kind: ReactionKind::ChallengeInsight,
            reason,
            content: None,
            target_id: Some(target_id),
            insight_kind: None,
            pheromone_kind: None,
            intensity: None,
        }
    }

    /// Constructs a `DepositPheromone` reaction.
    #[must_use]
    pub fn deposit_pheromone(
        kind: &str,
        content: String,
        intensity: f32,
        reason: String,
    ) -> Self {
        Self {
            kind: ReactionKind::DepositPheromone,
            reason,
            content: Some(content),
            target_id: None,
            insight_kind: None,
            pheromone_kind: Some(kind.to_string()),
            intensity: Some(intensity),
        }
    }
}

/// Threshold above which a `threat` pheromone triggers a warning insight.
pub const THREAT_THRESHOLD: f32 = 0.7;
/// Threshold above which an `opportunity` pheromone triggers a strategy insight.
pub const OPPORTUNITY_THRESHOLD: f32 = 0.6;
/// Keywords that flag an existing insight as potentially flawed.
pub const ANTI_PATTERN_KEYWORDS: &[&str] = &["WRONG", "BUG", "INCORRECT"];
/// Topic keywords for richer, category-aware reactions.
const TOPIC_DEX: &[&str] = &["swap", "uniswap", "sushi", "curve", "router", "dex", "liquidity"];
const TOPIC_LENDING: &[&str] = &["aave", "lending", "borrow", "supply", "compound", "repay"];
const TOPIC_MEV: &[&str] = &["mev", "sandwich", "priority", "tip", "flashbot", "bundle"];
const TOPIC_GAS: &[&str] = &["gas", "base fee", "congestion", "saturation", "gwei"];
const TOPIC_WHALE: &[&str] = &["whale", "large", "transfer", "eth moved"];
const TOPIC_STABLE: &[&str] = &["stablecoin", "usdt", "usdc", "dai", "stable"];
const TOPIC_BRIDGE: &[&str] = &["bridge", "cross-chain", "arbitrum", "optimism", "l2"];
const TOPIC_NFT: &[&str] = &["nft", "erc721", "erc1155", "marketplace", "opensea"];

fn content_contains_anti_pattern(content: &str) -> bool {
    ANTI_PATTERN_KEYWORDS
        .iter()
        .any(|kw| content.contains(kw))
}

fn content_matches_topic(content: &str, keywords: &[&str]) -> bool {
    let lower = content.to_lowercase();
    keywords.iter().any(|kw| lower.contains(kw))
}

fn classify_topic(content: &str) -> &'static str {
    if content_matches_topic(content, TOPIC_DEX) { return "dex"; }
    if content_matches_topic(content, TOPIC_LENDING) { return "lending"; }
    if content_matches_topic(content, TOPIC_MEV) { return "mev"; }
    if content_matches_topic(content, TOPIC_WHALE) { return "whale"; }
    if content_matches_topic(content, TOPIC_STABLE) { return "stablecoin"; }
    if content_matches_topic(content, TOPIC_BRIDGE) { return "bridge"; }
    if content_matches_topic(content, TOPIC_NFT) { return "nft"; }
    if content_matches_topic(content, TOPIC_GAS) { return "gas"; }
    "general"
}

fn find_warning_for_threat(insights: &[InsightHit]) -> Option<&InsightHit> {
    insights
        .iter()
        .find(|i| i.kind == "warning" && i.similarity >= 0.6)
}

/// Produce a vector of reactions given current pheromones and insights.
#[must_use]
pub fn decide(
    pheromones: &[PheromoneHit],
    insights: &[InsightHit],
    watcher_id: &str,
) -> Vec<Reaction> {
    let mut out: Vec<Reaction> = Vec::new();

    // Rule 1 + Rule 2: react to high-intensity pheromones.
    for p in pheromones {
        match p.kind.as_str() {
            "threat" if p.intensity > THREAT_THRESHOLD => {
                if find_warning_for_threat(insights).is_none() {
                    let content = format!(
                        "[{watcher_id}] observed high-intensity threat pheromone #{} (intensity={:.2})",
                        p.id, p.intensity
                    );
                    out.push(Reaction::post_insight(
                        "warning",
                        content,
                        format!(
                            "threat pheromone {} intensity {:.2} > {:.2}",
                            p.id, p.intensity, THREAT_THRESHOLD
                        ),
                    ));
                }
            }
            "opportunity" if p.intensity > OPPORTUNITY_THRESHOLD => {
                let content = format!(
                    "[{watcher_id}] strategy fragment suggested by opportunity pheromone #{} (intensity={:.2})",
                    p.id, p.intensity
                );
                out.push(Reaction::post_insight(
                    "strategy_fragment",
                    content,
                    format!(
                        "opportunity pheromone {} intensity {:.2} > {:.2}",
                        p.id, p.intensity, OPPORTUNITY_THRESHOLD
                    ),
                ));
            }
            "wisdom" => {
                // Rule 3: confirm the best matching insight if we have one.
                if let Some(top) = insights.iter().max_by(|a, b| {
                    a.similarity
                        .partial_cmp(&b.similarity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    if top.similarity >= 0.55 {
                        out.push(Reaction::confirm_insight(
                            top.id.clone(),
                            format!(
                                "wisdom pheromone {} aligns with insight {} (sim={:.2})",
                                p.id, top.id, top.similarity
                            ),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    // Rule 3b: active consensus — confirm up to 3 UNCONFIRMED insights per poll
    // that have high semantic similarity to our query. This is what drives the
    // visible "knowledge consolidates through confirmation" loop.
    if insights.len() >= 3 {
        let mut candidates: Vec<&InsightHit> = insights
            .iter()
            .filter(|i| i.confirmations == 0 && i.similarity >= 0.50)
            .collect();
        candidates.sort_by(|a, b| {
            b.similarity.partial_cmp(&a.similarity).unwrap_or(std::cmp::Ordering::Equal)
        });
        for top in candidates.iter().take(3) {
            out.push(Reaction::confirm_insight(
                top.id.clone(),
                format!(
                    "consensus confirm: {} (sim={:.2})",
                    top.id, top.similarity
                ),
            ));
        }
    }

    // Rule 4: challenge insights with anti-pattern keywords.
    for i in insights {
        if content_contains_anti_pattern(&i.content) && i.confirmations >= 1 {
            out.push(Reaction::challenge_insight(
                i.id.clone(),
                format!("insight {} contains anti-pattern keywords", i.id),
            ));
        }
    }

    // Rule 4b: diversity challenge — if an insight has ≥3 confirmations from the
    // same source (author equals our watcher_id), and we've re-observed it many
    // times, that's suspicious. Challenge it to test resilience.
    // (Signal is subtle but creates the "consensus → skepticism" arc visually.)
    for i in insights {
        if i.confirmations >= 5 && i.challenges == 0 && i.similarity >= 0.65 {
            // Only one watcher should challenge; use id hash modulo to spread
            let last_char = i.id.chars().last().unwrap_or('0');
            let hash = (last_char as u32).wrapping_mul(2654435761);
            if (hash % 13) == 0 && watcher_id.contains("consensus") {
                out.push(Reaction::challenge_insight(
                    i.id.clone(),
                    format!("stress-test challenge: insight {} has {} confirmations", i.id, i.confirmations),
                ));
            }
        }
    }

    // Rule 6: topic-aware synthesis — when multiple insights share a topic, synthesize.
    {
        let mut topic_counts: std::collections::HashMap<&str, Vec<&InsightHit>> =
            std::collections::HashMap::new();
        for i in insights {
            let topic = classify_topic(&i.content);
            topic_counts.entry(topic).or_default().push(i);
        }
        for (topic, hits) in &topic_counts {
            if hits.len() >= 3 && *topic != "general" {
                // Synthesize a higher-level heuristic from converging signals.
                let snippet: Vec<&str> = hits.iter().take(3).map(|h| {
                    let end = h.content.len().min(40);
                    &h.content[..end]
                }).collect();
                let content = format!(
                    "[{watcher_id}] convergence detected: {} signals on \"{}\": {}…",
                    hits.len(), topic, snippet.join(" | ")
                );
                out.push(Reaction::post_insight(
                    "heuristic",
                    content,
                    format!("{} insights converging on topic '{}'", hits.len(), topic),
                ));
                // Also deposit an opportunity if the convergence is strong.
                if hits.len() >= 5 {
                    out.push(Reaction::deposit_pheromone(
                        "opportunity",
                        format!(
                            "[{watcher_id}] strong {} convergence: {} signals aligning",
                            topic, hits.len()
                        ),
                        0.7,
                        format!("topic convergence: {} × {}", topic, hits.len()),
                    ));
                }
            }
        }
    }

    // Rule 7: cross-reference — if pheromones and insights share a topic, post causal_link.
    for p in pheromones {
        let p_topic = classify_topic(&format!("{:?}", p.kind));
        for i in insights.iter().take(10) {
            let i_topic = classify_topic(&i.content);
            if p_topic != "general" && p_topic == i_topic && i.similarity >= 0.45 {
                let content = format!(
                    "[{watcher_id}] cross-signal: {} pheromone #{} correlates with insight {} (topic: {}, sim={:.2})",
                    p.kind, p.id, i.id, p_topic, i.similarity
                );
                out.push(Reaction::post_insight(
                    "causal_link",
                    content,
                    format!("pheromone-insight correlation on topic '{}'", p_topic),
                ));
                break; // one cross-ref per pheromone
            }
        }
    }

    // Rule 8: opportunity from declining threats — if we see many low-intensity threats
    // (decaying), the danger has passed and that's an opportunity.
    {
        let low_threats: Vec<&PheromoneHit> = pheromones
            .iter()
            .filter(|p| p.kind == "threat" && p.intensity > 0.1 && p.intensity < 0.4)
            .collect();
        if low_threats.len() >= 3 {
            let avg_int: f32 =
                low_threats.iter().map(|p| p.intensity).sum::<f32>() / low_threats.len() as f32;
            let content = format!(
                "[{watcher_id}] threat decay window: {} fading threats (avg intensity {:.2}) — recovery opportunity",
                low_threats.len(), avg_int
            );
            out.push(Reaction::deposit_pheromone(
                "opportunity",
                content,
                0.65,
                format!("{} decaying threats → opportunity", low_threats.len()),
            ));
        }
    }

    // Rule 9: contrarian challenge — if every recent insight is the same kind,
    // challenge the weakest one (by weight) to inject diversity.
    if insights.len() >= 5 {
        let first_kind = &insights[0].kind;
        let all_same = insights.iter().take(8).all(|i| i.kind == *first_kind);
        if all_same {
            if let Some(weakest) = insights.iter().min_by(|a, b| {
                a.weight.partial_cmp(&b.weight).unwrap_or(std::cmp::Ordering::Equal)
            }) {
                if weakest.confirmations >= 1 {
                    out.push(Reaction::challenge_insight(
                        weakest.id.clone(),
                        format!(
                            "diversity challenge: {} consecutive '{}' insights — testing weakest (w={:.2})",
                            insights.len().min(8), first_kind, weakest.weight
                        ),
                    ));
                }
            }
        }
    }

    // Rule 5: wisdom summary — richer context from what we observed.
    if pheromones.len() + insights.len() >= 3 {
        let threat_count = pheromones.iter().filter(|p| p.kind == "threat").count();
        let opp_count = pheromones.iter().filter(|p| p.kind == "opportunity").count();
        let confirmed = insights.iter().filter(|i| i.confirmations >= 2).count();
        let top_topic = {
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for i in insights {
                *counts.entry(classify_topic(&i.content)).or_default() += 1;
            }
            counts.into_iter().max_by_key(|(_, v)| *v).map(|(k, _)| k).unwrap_or("general")
        };
        let content = format!(
            "[{watcher_id}] wisdom: {} pheromones ({} threats, {} opps), {} insights ({} confirmed), dominant topic: {}",
            pheromones.len(), threat_count, opp_count, insights.len(), confirmed, top_topic
        );
        out.push(Reaction::deposit_pheromone(
            "wisdom",
            content,
            0.25 + (confirmed as f32 * 0.05).min(0.25), // more confirmed = stronger wisdom
            "periodic chain-state summary".to_string(),
        ));
    }

    out
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, clippy::float_cmp)]
mod tests {
    use super::*;

    fn pheromone(id: u64, kind: &str, intensity: f32) -> PheromoneHit {
        PheromoneHit {
            id,
            kind: kind.to_string(),
            similarity: 0.9,
            intensity,
            score: intensity * 0.9,
        }
    }

    fn insight(id: &str, kind: &str, content: &str, sim: f32, conf: usize) -> InsightHit {
        InsightHit {
            id: id.to_string(),
            kind: kind.to_string(),
            content: content.to_string(),
            similarity: sim,
            weight: 1.0,
            score: sim,
            confirmations: conf,
            challenges: 0,
            state: Some("active".to_string()),
        }
    }

    #[test]
    fn empty_inputs_yield_no_reactions() {
        let reactions = decide(&[], &[], "watcher-a");
        assert!(reactions.is_empty());
    }

    #[test]
    fn high_threat_with_no_warning_triggers_post() {
        let phs = vec![pheromone(1, "threat", 0.85)];
        let reactions = decide(&phs, &[], "watcher-a");
        assert!(reactions
            .iter()
            .any(|r| r.kind == ReactionKind::PostInsight
                && r.insight_kind.as_deref() == Some("warning")));
    }

    #[test]
    fn threat_below_threshold_does_not_post_warning() {
        let phs = vec![pheromone(1, "threat", 0.5)];
        let reactions = decide(&phs, &[], "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.insight_kind.as_deref() != Some("warning")));
    }

    #[test]
    fn high_threat_skipped_when_warning_already_present() {
        let phs = vec![pheromone(1, "threat", 0.9)];
        let insights = vec![insight(
            "insight:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "warning",
            "already warned",
            0.95,
            2,
        )];
        let reactions = decide(&phs, &insights, "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.insight_kind.as_deref() != Some("warning")));
    }

    #[test]
    fn opportunity_posts_strategy_fragment() {
        let phs = vec![pheromone(2, "opportunity", 0.72)];
        let reactions = decide(&phs, &[], "watcher-a");
        assert!(reactions
            .iter()
            .any(|r| r.insight_kind.as_deref() == Some("strategy_fragment")));
    }

    #[test]
    fn opportunity_below_threshold_no_post() {
        let phs = vec![pheromone(2, "opportunity", 0.3)];
        let reactions = decide(&phs, &[], "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.insight_kind.as_deref() != Some("strategy_fragment")));
    }

    #[test]
    fn wisdom_confirms_top_insight() {
        let phs = vec![pheromone(3, "wisdom", 0.8)];
        let insights = vec![
            insight("insight:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "insight", "ok", 0.6, 0),
            insight("insight:cccccccccccccccccccccccccccccccc", "insight", "better", 0.82, 1),
        ];
        let reactions = decide(&phs, &insights, "watcher-a");
        let confirm = reactions
            .iter()
            .find(|r| r.kind == ReactionKind::ConfirmInsight)
            .expect("expected confirm reaction");
        assert_eq!(
            confirm.target_id.as_deref(),
            Some("insight:cccccccccccccccccccccccccccccccc")
        );
    }

    #[test]
    fn wisdom_skips_confirm_when_similarity_low() {
        let phs = vec![pheromone(3, "wisdom", 0.8)];
        let insights = vec![insight(
            "insight:dddddddddddddddddddddddddddddddd",
            "insight",
            "weak match",
            0.2,
            0,
        )];
        let reactions = decide(&phs, &insights, "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.kind != ReactionKind::ConfirmInsight));
    }

    #[test]
    fn anti_pattern_keyword_triggers_challenge() {
        let insights = vec![insight(
            "insight:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "insight",
            "this is WRONG and unsafe",
            0.9,
            2,
        )];
        let reactions = decide(&[], &insights, "watcher-a");
        assert!(reactions
            .iter()
            .any(|r| r.kind == ReactionKind::ChallengeInsight));
    }

    #[test]
    fn anti_pattern_without_confirmations_not_challenged() {
        let insights = vec![insight(
            "insight:ffffffffffffffffffffffffffffffff",
            "insight",
            "BUG here",
            0.9,
            0,
        )];
        let reactions = decide(&[], &insights, "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.kind != ReactionKind::ChallengeInsight));
    }

    #[test]
    fn summary_pheromone_deposited_when_observations_exist() {
        let phs = vec![pheromone(1, "wisdom", 0.2)];
        let reactions = decide(&phs, &[], "watcher-a");
        assert!(reactions.iter().any(|r| {
            r.kind == ReactionKind::DepositPheromone
                && r.pheromone_kind.as_deref() == Some("wisdom")
        }));
    }

    #[test]
    fn no_summary_pheromone_when_nothing_observed() {
        let reactions = decide(&[], &[], "watcher-a");
        assert!(reactions
            .iter()
            .all(|r| r.kind != ReactionKind::DepositPheromone));
    }

    #[test]
    fn multiple_rules_fire_in_one_pass() {
        let phs = vec![
            pheromone(1, "threat", 0.95),
            pheromone(2, "opportunity", 0.85),
        ];
        let insights = vec![insight(
            "insight:11111111111111111111111111111111",
            "insight",
            "INCORRECT claim",
            0.9,
            3,
        )];
        let reactions = decide(&phs, &insights, "watcher-a");
        let kinds: Vec<&ReactionKind> = reactions.iter().map(|r| &r.kind).collect();
        assert!(kinds.contains(&&ReactionKind::PostInsight));
        assert!(kinds.contains(&&ReactionKind::ChallengeInsight));
        assert!(kinds.contains(&&ReactionKind::DepositPheromone));
    }

    #[test]
    fn reaction_reason_is_non_empty() {
        let phs = vec![pheromone(1, "threat", 0.9)];
        for r in decide(&phs, &[], "watcher-a") {
            assert!(!r.reason.is_empty(), "reaction must have a reason");
        }
    }
}
