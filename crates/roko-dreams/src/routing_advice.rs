//! Dream-derived routing advice persisted for later dispatch.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use roko_learn::cascade::RoutingBias;
use roko_learn::episode_logger::Episode;
use roko_learn::pattern_discovery::CrossEpisodeConsolidationReport;
use serde::{Deserialize, Serialize};

/// Default routing advice path relative to a workspace root.
pub const DREAM_ROUTING_ADVICE_PATH: &str = ".roko/learn/dream-routing-advice.json";

/// Routing recommendations produced by dream consolidation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DreamRoutingAdvice {
    /// When this advice was generated.
    pub generated_at: DateTime<Utc>,
    /// Dream report that produced this advice.
    pub source_dream_report: String,
    /// Individual model-routing recommendations.
    pub recommendations: Vec<RoutingRecommendation>,
    /// Discovered patterns ready for prompt/context injection.
    pub pattern_summaries: Vec<PatternSummary>,
}

impl Default for DreamRoutingAdvice {
    fn default() -> Self {
        Self {
            generated_at: Utc::now(),
            source_dream_report: String::new(),
            recommendations: Vec::new(),
            pattern_summaries: Vec::new(),
        }
    }
}

/// One routing recommendation derived from a dream meta-pattern.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingRecommendation {
    /// Task category this recommendation applies to.
    pub task_category: String,
    /// Complexity band this recommendation applies to.
    pub complexity_band: String,
    /// Model to prefer.
    pub recommended_model: String,
    /// Models to deprioritize for this task shape.
    pub deprioritize: Vec<String>,
    /// Confidence in `0.0..=1.0`.
    pub confidence: f64,
    /// Number of episodes supporting the recommendation.
    pub supporting_episodes: usize,
    /// Success rate observed for the recommended model or task shape.
    pub recommended_model_success_rate: f64,
    /// Meta-pattern signature for deduplication.
    pub pattern_signature: u64,
}

/// A discovered dream pattern suitable for prompt injection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PatternSummary {
    /// Human-readable pattern description.
    pub description: String,
    /// Task categories where the pattern applies.
    pub applies_to: Vec<String>,
    /// Actionable guidance for a future agent.
    pub guidance: String,
    /// Confidence in `0.0..=1.0`.
    pub confidence: f64,
    /// Meta-pattern signature for deduplication.
    pub signature: u64,
}

/// Generate routing advice from cross-episode consolidation output.
#[must_use]
pub fn generate_routing_advice(
    report: &CrossEpisodeConsolidationReport,
    episodes: &[Episode],
    generated_at: DateTime<Utc>,
    source_dream_report: impl Into<String>,
) -> DreamRoutingAdvice {
    let mut recommendations = Vec::new();
    let mut pattern_summaries = Vec::new();

    for pattern in &report.meta_patterns {
        let cluster_episodes = pattern
            .episode_indices
            .iter()
            .filter_map(|&index| episodes.get(index))
            .collect::<Vec<_>>();
        if cluster_episodes.len() < 3 {
            continue;
        }

        let success_count = cluster_episodes
            .iter()
            .filter(|episode| episode.success)
            .count();
        let success_rate = success_count as f64 / cluster_episodes.len() as f64;
        let majority_model = majority_field(&cluster_episodes, episode_model);
        let majority_category = majority_field(&cluster_episodes, episode_task_category);
        let majority_complexity = majority_field(&cluster_episodes, episode_complexity_band);

        if let (Some(model), Some(category), Some(complexity)) = (
            majority_model.clone(),
            majority_category.clone(),
            majority_complexity.clone(),
        ) {
            if let Some((recommended_model, deprioritize)) =
                recommendation_for_model(&model, success_rate)
            {
                recommendations.push(RoutingRecommendation {
                    task_category: category,
                    complexity_band: complexity,
                    recommended_model,
                    deprioritize,
                    confidence: f64::from(pattern.coherence).clamp(0.0, 1.0),
                    supporting_episodes: cluster_episodes.len(),
                    recommended_model_success_rate: success_rate,
                    pattern_signature: pattern.signature,
                });
            }
        }

        pattern_summaries.push(PatternSummary {
            description: pattern.description.clone(),
            applies_to: majority_category.into_iter().collect(),
            guidance: generate_pattern_guidance(
                majority_model.as_deref(),
                success_rate,
                cluster_episodes.len(),
            ),
            confidence: f64::from(pattern.coherence).clamp(0.0, 1.0),
            signature: pattern.signature,
        });
    }

    dedupe_recommendations(&mut recommendations);
    dedupe_patterns(&mut pattern_summaries);

    DreamRoutingAdvice {
        generated_at,
        source_dream_report: source_dream_report.into(),
        recommendations,
        pattern_summaries,
    }
}

/// Convert persisted dream advice into a cascade routing bias.
#[must_use]
pub fn dream_advice_to_routing_bias(
    advice: &DreamRoutingAdvice,
    task_category: &str,
    complexity_band: &str,
) -> RoutingBias {
    let category = normalize_match_key(task_category);
    let complexity = normalize_match_key(complexity_band);
    let mut deprioritize = BTreeSet::new();
    let mut reasons = Vec::new();

    for recommendation in &advice.recommendations {
        if recommendation.confidence < 0.5 || recommendation.supporting_episodes < 3 {
            continue;
        }
        if normalize_match_key(&recommendation.task_category) != category {
            continue;
        }
        if normalize_match_key(&recommendation.complexity_band) != complexity {
            continue;
        }
        for model in &recommendation.deprioritize {
            deprioritize.insert(model.clone());
        }
        reasons.push(format!(
            "{} episodes support {} for {}/{}",
            recommendation.supporting_episodes,
            recommendation.recommended_model,
            recommendation.task_category,
            recommendation.complexity_band
        ));
    }

    RoutingBias {
        deprioritize: deprioritize.into_iter().collect(),
        prefer_cheaper: false,
        reason: if reasons.is_empty() {
            "no matching dream routing advice".to_string()
        } else {
            format!("dream routing advice: {}", reasons.join("; "))
        },
    }
}

/// Return pattern summaries that apply to a task category.
#[must_use]
pub fn relevant_pattern_summaries<'a>(
    advice: &'a DreamRoutingAdvice,
    task_category: &str,
    min_confidence: f64,
    limit: usize,
) -> Vec<&'a PatternSummary> {
    let category = normalize_match_key(task_category);
    let mut summaries = advice
        .pattern_summaries
        .iter()
        .filter(|summary| summary.confidence >= min_confidence)
        .filter(|summary| {
            summary
                .applies_to
                .iter()
                .any(|applies| normalize_match_key(applies) == category)
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.signature.cmp(&right.signature))
    });
    summaries.truncate(limit);
    summaries
}

/// Resolve the default advice path for a workspace.
#[must_use]
pub fn dream_routing_advice_path(workdir: impl AsRef<Path>) -> PathBuf {
    workdir.as_ref().join(DREAM_ROUTING_ADVICE_PATH)
}

/// Load dream routing advice from the default workspace path.
///
/// Missing files return an empty default advice value.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_dream_routing_advice(workdir: impl AsRef<Path>) -> Result<DreamRoutingAdvice> {
    load_dream_routing_advice_at(&dream_routing_advice_path(workdir))
}

/// Load dream routing advice from an explicit path.
///
/// Missing files return an empty default advice value.
///
/// # Errors
///
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_dream_routing_advice_at(path: &Path) -> Result<DreamRoutingAdvice> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DreamRoutingAdvice::default());
        }
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

/// Save dream routing advice to the default workspace path.
///
/// # Errors
///
/// Returns an error if the advice file cannot be written.
pub fn save_dream_routing_advice(
    workdir: impl AsRef<Path>,
    advice: &DreamRoutingAdvice,
) -> Result<()> {
    save_dream_routing_advice_at(&dream_routing_advice_path(workdir), advice)
}

/// Save dream routing advice to an explicit path.
///
/// # Errors
///
/// Returns an error if the advice file cannot be written.
pub fn save_dream_routing_advice_at(path: &Path, advice: &DreamRoutingAdvice) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(advice).context("serialize dream routing advice")?;
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

fn recommendation_for_model(model: &str, success_rate: f64) -> Option<(String, Vec<String>)> {
    if success_rate < 0.4 {
        Some((next_tier_model(model), vec![model.to_string()]))
    } else if success_rate > 0.8 {
        Some((model.to_string(), Vec::new()))
    } else {
        None
    }
}

fn next_tier_model(current: &str) -> String {
    let lower = current.to_ascii_lowercase();
    if lower.contains("haiku") {
        replace_model_tier(current, "haiku", "sonnet")
    } else if lower.contains("sonnet") {
        replace_model_tier(current, "sonnet", "opus")
    } else {
        current.to_string()
    }
}

fn replace_model_tier(current: &str, from: &str, to: &str) -> String {
    current
        .replace(from, to)
        .replace(&capitalize(from), &capitalize(to))
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => {
            let mut out = first.to_uppercase().collect::<String>();
            out.push_str(chars.as_str());
            out
        }
        None => String::new(),
    }
}

fn generate_pattern_guidance(
    majority_model: Option<&str>,
    success_rate: f64,
    episode_count: usize,
) -> String {
    let model = majority_model.unwrap_or("the assigned model");
    if success_rate < 0.4 {
        format!(
            "Historical dream consolidation shows {model} has a {:.0}% success rate for this task shape across {episode_count} episodes; consider a more capable model or narrower context.",
            success_rate * 100.0
        )
    } else if success_rate > 0.8 {
        format!(
            "Historical dream consolidation shows {model} has a {:.0}% success rate for this task shape across {episode_count} episodes; this model/task pairing is reliable.",
            success_rate * 100.0
        )
    } else {
        format!(
            "Dream consolidation found mixed results ({:.0}% success) for this task shape across {episode_count} episodes; verify edge cases early.",
            success_rate * 100.0
        )
    }
}

fn majority_field<F>(episodes: &[&Episode], extract: F) -> Option<String>
where
    F: Fn(&Episode) -> String,
{
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for episode in episodes {
        let value = extract(episode);
        if value.trim().is_empty() {
            continue;
        }
        *counts.entry(value).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .filter(|(_, count)| *count * 2 > episodes.len())
        .map(|(value, _)| value)
}

fn episode_model(episode: &Episode) -> String {
    if !episode.model.trim().is_empty() {
        episode.model.trim().to_string()
    } else {
        extra_string(episode, "model").unwrap_or_default()
    }
}

fn episode_task_category(episode: &Episode) -> String {
    extra_string(episode, "task_category")
        .or_else(|| extra_string(episode, "task_type"))
        .or_else(|| {
            (!episode.agent_template.trim().is_empty()).then(|| episode.agent_template.clone())
        })
        .unwrap_or_else(|| "unknown-task".to_string())
}

fn episode_complexity_band(episode: &Episode) -> String {
    extra_string(episode, "complexity_band").unwrap_or_else(|| "standard".to_string())
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_match_key(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn dedupe_recommendations(recommendations: &mut Vec<RoutingRecommendation>) {
    recommendations.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.supporting_episodes.cmp(&left.supporting_episodes))
            .then_with(|| left.pattern_signature.cmp(&right.pattern_signature))
    });
    let mut seen = BTreeSet::new();
    recommendations.retain(|recommendation| {
        seen.insert((
            normalize_match_key(&recommendation.task_category),
            normalize_match_key(&recommendation.complexity_band),
            recommendation.recommended_model.clone(),
        ))
    });
}

fn dedupe_patterns(patterns: &mut Vec<PatternSummary>) {
    patterns.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.signature.cmp(&right.signature))
    });
    let mut seen = BTreeSet::new();
    patterns.retain(|pattern| seen.insert(pattern.signature));
}
