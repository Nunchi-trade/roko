//! Runtime knowledge lifecycle facade.
//!
//! This module is intentionally file-backed and append-friendly: callers feed
//! completed runtime episodes or gate observations into one API, and the
//! facade records a durable lifecycle receipt while updating the existing
//! knowledge, admission, and heuristic stores.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use roko_learn::episode_logger::{Episode, GateVerdict};
use serde::{Deserialize, Serialize};

use crate::admission::{
    AdmissionGateOutcome, KnowledgeAdmissionOutcome, KnowledgeAdmissionStore,
    KnowledgeCandidateRecord, KnowledgeEvidence, KnowledgeEvidenceSource, KnowledgeScope,
    LightAdmissionGate,
};
use crate::tier_progression::{
    HeuristicDemotionRecord, HeuristicObservation, HeuristicStore, TierProgression,
};
use crate::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier, ReinforcementSignal};
use crate::{SourceChannel, apply_source_discount};

/// Default append-only lifecycle receipt file under `.roko/neuro/`.
pub const DEFAULT_KNOWLEDGE_LIFECYCLE_FILE: &str = "knowledge-lifecycle.jsonl";

/// Runtime lifecycle tuning knobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeLifecycleConfig {
    /// Fast-path admission gate for single runtime events.
    pub light_gate: LightAdmissionGate,
    /// Novelty factor used when reinforcing context-pack entries.
    pub reinforcement_novelty: f64,
    /// Whether entries in the runtime context pack should be promoted from
    /// successful gate observations.
    pub promote_context_entries: bool,
    /// Whether observations that miss the light gate should still be submitted
    /// to the full evidence-based admission store as candidates.
    pub submit_full_admission_on_defer: bool,
}

impl Default for KnowledgeLifecycleConfig {
    fn default() -> Self {
        Self {
            light_gate: LightAdmissionGate::default(),
            reinforcement_novelty: 0.5,
            promote_context_entries: true,
            submit_full_admission_on_defer: true,
        }
    }
}

/// Runtime observation accepted by the knowledge lifecycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEpisodeObservation {
    /// Stable episode identifier.
    pub episode_id: String,
    /// Task identifier.
    pub task_id: String,
    /// Plan identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Task category or type, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    /// Model used by the runtime event, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Agent or role identifier, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Whether the runtime gate/episode passed.
    pub gate_passed: bool,
    /// Gate verdicts observed for the episode.
    #[serde(default)]
    pub gate_verdicts: Vec<GateVerdict>,
    /// Compact gate output or failure summary.
    #[serde(default)]
    pub gate_output: String,
    /// Agent output or reflection text used for citation/quotation checks.
    #[serde(default)]
    pub agent_output: String,
    /// Knowledge entry ids present in the context pack for this runtime event.
    #[serde(default)]
    pub context_entry_ids: Vec<String>,
    /// Tags used by heuristic falsifiers and retrieval.
    #[serde(default)]
    pub task_tags: Vec<String>,
    /// Source channel for admission trust.
    pub source_channel: SourceChannel,
    /// Observation timestamp.
    pub observed_at: DateTime<Utc>,
}

impl From<&Episode> for RuntimeEpisodeObservation {
    fn from(episode: &Episode) -> Self {
        let episode_id = episode_source_id(episode).to_string();
        let plan_id = extra_string(episode, "plan_id");
        let task_type = extra_string(episode, "task_category")
            .or_else(|| extra_string(episode, "task_type"))
            .or_else(|| extra_string(episode, "complexity_band"))
            .or_else(|| non_empty(&episode.agent_template));
        let model = non_empty(&episode.model);
        let agent_id = non_empty(&episode.agent_id);
        let gate_passed = if episode.gate_verdicts.is_empty() {
            episode.success
        } else {
            episode.gate_verdicts.iter().all(|verdict| verdict.passed)
        };

        Self {
            episode_id,
            task_id: episode.task_id.clone(),
            plan_id,
            task_type,
            model,
            agent_id,
            gate_passed,
            gate_verdicts: episode.gate_verdicts.clone(),
            gate_output: gate_output_from_episode(episode),
            agent_output: agent_output_from_episode(episode),
            context_entry_ids: extract_context_entry_ids(episode),
            task_tags: task_tags_from_episode(episode),
            source_channel: SourceChannel::GateVerdict,
            observed_at: episode.completed_at,
        }
    }
}

/// Path through which a runtime candidate was handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAdmissionPath {
    /// The lightweight gate admitted the candidate directly.
    LightAdmitted,
    /// The candidate was submitted to the full admission store and admitted.
    FullAdmitted,
    /// The candidate was submitted to the full admission store and deferred.
    FullDeferred,
    /// The candidate was submitted to the full admission store and rejected.
    FullRejected,
    /// The candidate was submitted to the full admission store and suppressed.
    FullSuppressed,
    /// The candidate missed the light gate and no full admission was requested.
    Deferred,
    /// No candidate could be built from the observation.
    NoCandidate,
}

/// Append-only receipt for one lifecycle ingestion call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeLifecycleRecord {
    /// Stable receipt id.
    pub record_id: String,
    /// Episode or gate observation id.
    pub episode_id: String,
    /// Timestamp when this lifecycle receipt was recorded.
    pub recorded_at: DateTime<Utc>,
    /// Candidate entry id, when one was built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_entry_id: Option<String>,
    /// How admission handled the candidate.
    pub admission_path: RuntimeAdmissionPath,
    /// Maximum similarity to existing knowledge at admission time.
    pub admission_similarity: f64,
    /// Source trust weight used by the light gate.
    pub source_trust: f64,
    /// Number of context entries reinforced as retrieved.
    pub retrieved_reinforcements: usize,
    /// Number of context entries reinforced after a passed gate.
    pub gated_reinforcements: usize,
    /// Number of context entries reinforced because they were cited by id.
    pub cited_reinforcements: usize,
    /// Number of context entries reinforced because their content was quoted.
    pub quoted_reinforcements: usize,
    /// Number of knowledge entries updated by runtime promotion evidence.
    pub promotion_updates: usize,
    /// Heuristic observation receipts generated by the same event.
    #[serde(default)]
    pub heuristic_observations: Vec<HeuristicObservation>,
    /// Heuristic demotion receipts generated by the same event.
    #[serde(default)]
    pub heuristic_demotions: Vec<HeuristicDemotionRecord>,
}

/// File-backed runtime lifecycle facade.
#[derive(Debug, Clone)]
pub struct RuntimeKnowledgeLifecycle {
    knowledge_store: KnowledgeStore,
    admission_store: KnowledgeAdmissionStore,
    heuristic_store: HeuristicStore,
    lifecycle_path: PathBuf,
    config: KnowledgeLifecycleConfig,
}

impl RuntimeKnowledgeLifecycle {
    /// Construct a lifecycle facade from an explicit knowledge store path.
    #[must_use]
    pub fn new(knowledge_store: KnowledgeStore, lifecycle_path: impl Into<PathBuf>) -> Self {
        let admission_store = KnowledgeAdmissionStore::new(knowledge_store.clone());
        let heuristic_store = knowledge_store
            .path()
            .parent()
            .map(|parent| {
                HeuristicStore::new(parent.join(crate::tier_progression::DEFAULT_HEURISTICS_FILE))
            })
            .unwrap_or_else(|| {
                HeuristicStore::new(crate::tier_progression::DEFAULT_HEURISTICS_FILE)
            });
        Self {
            knowledge_store,
            admission_store,
            heuristic_store,
            lifecycle_path: lifecycle_path.into(),
            config: KnowledgeLifecycleConfig::default(),
        }
    }

    /// Construct a lifecycle facade rooted at a `.roko/` directory.
    #[must_use]
    pub fn for_roko_dir(roko_dir: impl AsRef<Path>) -> Self {
        let roko_dir = roko_dir.as_ref();
        let knowledge_store = KnowledgeStore::for_roko_dir(roko_dir);
        Self {
            admission_store: KnowledgeAdmissionStore::new(knowledge_store.clone()),
            knowledge_store,
            heuristic_store: HeuristicStore::for_roko_dir(roko_dir),
            lifecycle_path: roko_dir
                .join("neuro")
                .join(DEFAULT_KNOWLEDGE_LIFECYCLE_FILE),
            config: KnowledgeLifecycleConfig::default(),
        }
    }

    /// Construct a lifecycle facade rooted at a workspace directory.
    #[must_use]
    pub fn for_workdir(workdir: impl AsRef<Path>) -> Self {
        Self::for_roko_dir(workdir.as_ref().join(".roko"))
    }

    /// Override lifecycle config.
    #[must_use]
    pub fn with_config(mut self, config: KnowledgeLifecycleConfig) -> Self {
        self.config = config;
        self
    }

    /// Path of the append-only lifecycle receipt file.
    #[must_use]
    pub fn lifecycle_path(&self) -> &Path {
        &self.lifecycle_path
    }

    /// Access the durable knowledge store.
    #[must_use]
    pub fn knowledge_store(&self) -> &KnowledgeStore {
        &self.knowledge_store
    }

    /// Access the heuristic store.
    #[must_use]
    pub fn heuristic_store(&self) -> &HeuristicStore {
        &self.heuristic_store
    }

    /// Ingest a completed runtime episode.
    ///
    /// # Errors
    ///
    /// Returns an error if any backing file cannot be updated.
    pub fn ingest_episode(&self, episode: &Episode) -> Result<KnowledgeLifecycleRecord> {
        self.ingest_observation(RuntimeEpisodeObservation::from(episode))
    }

    /// Ingest a normalized runtime gate observation.
    ///
    /// # Errors
    ///
    /// Returns an error if any backing file cannot be updated.
    pub fn ingest_observation(
        &self,
        observation: RuntimeEpisodeObservation,
    ) -> Result<KnowledgeLifecycleRecord> {
        let candidate = build_runtime_entry(&observation);
        let (candidate_entry_id, admission_path, admission_similarity, source_trust) =
            self.admit_runtime_candidate(&observation, candidate.as_ref())?;

        let context_ids = clean_ids(&observation.context_entry_ids);
        let id_refs = context_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let retrieved_reinforcements = self.knowledge_store.reinforce_batch(
            &id_refs,
            ReinforcementSignal::Retrieved,
            self.config.reinforcement_novelty,
        )?;
        let gated_reinforcements = if observation.gate_passed {
            self.knowledge_store.reinforce_batch(
                &id_refs,
                ReinforcementSignal::Gated,
                self.config.reinforcement_novelty,
            )?
        } else {
            0
        };

        let cited_ids = cited_entry_ids(&context_ids, &observation.agent_output);
        let cited_refs = cited_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let cited_reinforcements = self.knowledge_store.reinforce_batch(
            &cited_refs,
            ReinforcementSignal::Cited,
            self.config.reinforcement_novelty,
        )?;

        let quoted_ids = quoted_entry_ids(&self.knowledge_store, &context_ids, &observation)?;
        let quoted_refs = quoted_ids.iter().map(String::as_str).collect::<Vec<_>>();
        let quoted_reinforcements = self.knowledge_store.reinforce_batch(
            &quoted_refs,
            ReinforcementSignal::AgentQuoted,
            self.config.reinforcement_novelty,
        )?;

        let promotion_updates = if self.config.promote_context_entries {
            let mut promotion_ids = context_ids;
            if admission_path == RuntimeAdmissionPath::LightAdmitted
                || admission_path == RuntimeAdmissionPath::FullAdmitted
            {
                if let Some(id) = candidate_entry_id.as_ref() {
                    promotion_ids.push(id.clone());
                }
            }
            promote_runtime_entries(&self.knowledge_store, &promotion_ids, &observation)?
        } else {
            0
        };

        let heuristic_observations = self.heuristic_store.evaluate_all(
            &observation.task_tags,
            &observation.gate_output,
            observation.gate_passed,
        )?;
        let heuristic_demotions = self.heuristic_store.demote_expired(&self.knowledge_store)?;

        let record = KnowledgeLifecycleRecord {
            record_id: lifecycle_record_id(&observation, candidate_entry_id.as_deref()),
            episode_id: observation.episode_id.clone(),
            recorded_at: Utc::now(),
            candidate_entry_id,
            admission_path,
            admission_similarity,
            source_trust,
            retrieved_reinforcements,
            gated_reinforcements,
            cited_reinforcements,
            quoted_reinforcements,
            promotion_updates,
            heuristic_observations,
            heuristic_demotions,
        };
        append_jsonl(&self.lifecycle_path, &record)?;
        Ok(record)
    }

    /// Read append-only lifecycle receipts.
    ///
    /// # Errors
    ///
    /// Returns an error if the receipt file exists but cannot be read.
    pub fn read_records(&self) -> Result<Vec<KnowledgeLifecycleRecord>> {
        read_jsonl(&self.lifecycle_path)
    }

    fn admit_runtime_candidate(
        &self,
        observation: &RuntimeEpisodeObservation,
        candidate: Option<&KnowledgeEntry>,
    ) -> Result<(Option<String>, RuntimeAdmissionPath, f64, f64)> {
        let Some(candidate) = candidate else {
            return Ok((None, RuntimeAdmissionPath::NoCandidate, 0.0, 0.0));
        };

        let similarity = self.knowledge_store.max_similarity(candidate)?;
        let source_trust = observation.source_channel.discount_factor();
        if self
            .config
            .light_gate
            .evaluate(candidate.confidence, similarity, source_trust)
        {
            let mut entry = candidate.clone();
            apply_source_discount(std::slice::from_mut(&mut entry), observation.source_channel);
            self.knowledge_store.ingest(vec![entry])?;
            return Ok((
                Some(candidate.id.clone()),
                RuntimeAdmissionPath::LightAdmitted,
                similarity,
                source_trust,
            ));
        }

        if !self.config.submit_full_admission_on_defer {
            return Ok((
                Some(candidate.id.clone()),
                RuntimeAdmissionPath::Deferred,
                similarity,
                source_trust,
            ));
        }

        let decision = self
            .admission_store
            .submit_candidate(candidate_for_observation(observation, candidate))?;
        let path = match decision.outcome {
            KnowledgeAdmissionOutcome::Admitted => RuntimeAdmissionPath::FullAdmitted,
            KnowledgeAdmissionOutcome::Deferred => RuntimeAdmissionPath::FullDeferred,
            KnowledgeAdmissionOutcome::Rejected => RuntimeAdmissionPath::FullRejected,
            KnowledgeAdmissionOutcome::Suppressed => RuntimeAdmissionPath::FullSuppressed,
        };
        Ok((Some(candidate.id.clone()), path, similarity, source_trust))
    }
}

fn build_runtime_entry(observation: &RuntimeEpisodeObservation) -> Option<KnowledgeEntry> {
    let summary = runtime_summary(observation);
    if summary.trim().is_empty() {
        return None;
    }

    let kind = if observation.gate_passed {
        KnowledgeKind::StrategyFragment
    } else {
        KnowledgeKind::Warning
    };
    let confidence = runtime_confidence(observation);
    let source_episodes = vec![observation.episode_id.clone()];
    let tags = runtime_tags(observation, kind);
    Some(KnowledgeEntry {
        id: derive_runtime_knowledge_id(kind, &summary, &source_episodes, &tags),
        kind,
        source: Some("runtime:gate_verdict".to_string()),
        content: summary,
        confidence,
        confidence_weight: confidence,
        refuted_insight_id: None,
        refutation_evidence: (!observation.gate_passed)
            .then(|| observation.gate_output.clone())
            .filter(|value| !value.trim().is_empty()),
        source_episodes,
        tags,
        source_model: observation.model.clone(),
        model_generality: if observation.model.is_some() {
            0.0
        } else {
            1.0
        },
        created_at: Utc::now(),
        half_life_days: kind.default_half_life_days(),
        tier: KnowledgeTier::Transient,
        emotional_tag: None,
        emotional_provenance: None,
        hdc_vector: None,
        confirmation_count: u32::from(observation.gate_passed),
        distinct_contexts: distinct_contexts(observation),
        deprecated: false,
        balance: 1.0,
        frozen: false,
        catalytic_score: 0,
    })
}

fn candidate_for_observation(
    observation: &RuntimeEpisodeObservation,
    entry: &KnowledgeEntry,
) -> KnowledgeCandidateRecord {
    let mut candidate = KnowledgeCandidateRecord::new(
        entry.id.clone(),
        entry.kind,
        entry.source.as_deref().unwrap_or("runtime"),
        entry.content.clone(),
        entry.confidence,
    )
    .with_scope(KnowledgeScope {
        action_id: None,
        role_id: observation.agent_id.clone(),
        task_type: observation.task_type.clone(),
        crate_path: None,
        tags: observation.task_tags.clone(),
    })
    .with_tags(entry.tags.clone());
    candidate.source_episodes.clone_from(&entry.source_episodes);
    candidate.evidence = evidence_for_observation(observation);
    candidate
}

fn evidence_for_observation(observation: &RuntimeEpisodeObservation) -> Vec<KnowledgeEvidence> {
    let mut evidence = Vec::new();
    for verdict in &observation.gate_verdicts {
        let outcome = if verdict.passed {
            AdmissionGateOutcome::Passed
        } else {
            AdmissionGateOutcome::Failed
        };
        evidence.push(KnowledgeEvidence::gate(
            format!("{}:{}", observation.episode_id, verdict.gate),
            verdict.gate.clone(),
            outcome,
            0.95,
            verdict
                .signature
                .clone()
                .unwrap_or_else(|| observation.gate_output.clone()),
        ));
    }
    if evidence.is_empty() {
        let summary = if observation.agent_output.trim().is_empty() {
            runtime_summary(observation)
        } else {
            observation.agent_output.clone()
        };
        let item = if observation.gate_passed {
            KnowledgeEvidence::supporting(
                format!("{}:agent-output", observation.episode_id),
                KnowledgeEvidenceSource::AgentOutput,
                observation.agent_id.as_deref().unwrap_or("runtime"),
                0.75,
                summary,
            )
        } else {
            KnowledgeEvidence::refuting(
                format!("{}:agent-output", observation.episode_id),
                KnowledgeEvidenceSource::AgentOutput,
                observation.agent_id.as_deref().unwrap_or("runtime"),
                0.75,
                summary,
            )
        };
        evidence.push(item);
    }
    evidence
}

fn promote_runtime_entries(
    store: &KnowledgeStore,
    entry_ids: &[String],
    observation: &RuntimeEpisodeObservation,
) -> Result<usize> {
    let ids = clean_ids(entry_ids).into_iter().collect::<BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(0);
    }
    let context = context_label(observation);
    store.update_entries(|entry| {
        if !ids.contains(&entry.id) {
            return false;
        }

        if !entry.source_episodes.contains(&observation.episode_id) {
            entry.source_episodes.push(observation.episode_id.clone());
        }
        if !entry.distinct_contexts.contains(&context) {
            entry.distinct_contexts.push(context.clone());
        }

        if observation.gate_passed {
            entry.confirmation_count = entry.confirmation_count.saturating_add(1);
            entry.confidence = (entry.confidence + 0.03).clamp(0.0, 1.0);
            if let Some(tier) = TierProgression::evaluate_tier_progression_v2(
                entry,
                entry.confirmation_count as usize,
                0,
            )
            .tier()
            {
                entry.tier = tier;
            }
        } else {
            entry.confidence = (entry.confidence * 0.98).clamp(0.0, 1.0);
        }
        true
    })
}

fn quoted_entry_ids(
    store: &KnowledgeStore,
    context_ids: &[String],
    observation: &RuntimeEpisodeObservation,
) -> Result<Vec<String>> {
    if context_ids.is_empty() || observation.agent_output.trim().is_empty() {
        return Ok(Vec::new());
    }
    let output = observation.agent_output.to_ascii_lowercase();
    let ids = context_ids.iter().cloned().collect::<BTreeSet<_>>();
    let entries = store.read_all()?;
    Ok(entries
        .into_iter()
        .filter(|entry| ids.contains(&entry.id))
        .filter(|entry| quoted_content_match(&output, &entry.content))
        .map(|entry| entry.id)
        .collect())
}

fn cited_entry_ids(context_ids: &[String], output: &str) -> Vec<String> {
    if output.trim().is_empty() {
        return Vec::new();
    }
    let output = output.to_ascii_lowercase();
    context_ids
        .iter()
        .filter(|id| {
            let lower = id.to_ascii_lowercase();
            output.contains(&lower) || output.contains(&format!("knowledge:{lower}"))
        })
        .cloned()
        .collect()
}

fn quoted_content_match(output: &str, content: &str) -> bool {
    let words = content
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|word| word.len() >= 5)
        .take(12)
        .collect::<Vec<_>>();
    if words.len() < 4 {
        return false;
    }
    let matches = words.iter().filter(|word| output.contains(*word)).count();
    matches >= 4
}

fn runtime_summary(observation: &RuntimeEpisodeObservation) -> String {
    let task = observation
        .task_type
        .as_deref()
        .or_else(|| non_empty_str(&observation.task_id))
        .unwrap_or("runtime task");
    let model = observation.model.as_deref().unwrap_or("unknown model");
    let gates = summarize_gates(&observation.gate_verdicts);
    if observation.gate_passed {
        let extra = if observation.agent_output.trim().is_empty() {
            String::new()
        } else {
            format!(
                " Runtime reflection: {}.",
                trim_to(&observation.agent_output, 240)
            )
        };
        format!("Successful runtime episode for {task} with {model} passed {gates}.{extra}")
    } else {
        let reason = if observation.gate_output.trim().is_empty() {
            "the gate failed without a recorded diagnostic".to_string()
        } else {
            trim_to(&observation.gate_output, 240)
        };
        format!("Runtime warning for {task} with {model}: {gates} did not pass because {reason}.")
    }
}

fn runtime_confidence(observation: &RuntimeEpisodeObservation) -> f64 {
    let gate_count = observation.gate_verdicts.len().max(1) as f64;
    let pass_count = observation
        .gate_verdicts
        .iter()
        .filter(|verdict| verdict.passed)
        .count() as f64;
    let pass_rate = if observation.gate_verdicts.is_empty() {
        if observation.gate_passed { 1.0 } else { 0.0 }
    } else {
        pass_count / gate_count
    };
    let content_bonus = if observation.agent_output.trim().is_empty()
        && observation.gate_output.trim().is_empty()
    {
        0.0
    } else {
        0.05
    };
    if observation.gate_passed {
        (0.62 + pass_rate * 0.25 + content_bonus).clamp(0.5, 0.95)
    } else {
        (0.58 + (1.0 - pass_rate) * 0.22 + content_bonus).clamp(0.5, 0.90)
    }
}

fn runtime_tags(observation: &RuntimeEpisodeObservation, kind: KnowledgeKind) -> Vec<String> {
    let mut tags = observation.task_tags.clone();
    tags.push(kind.as_str().to_string());
    tags.push("runtime".to_string());
    tags.push(if observation.gate_passed {
        "gate:passed".to_string()
    } else {
        "gate:failed".to_string()
    });
    tags.extend(
        observation
            .gate_verdicts
            .iter()
            .map(|verdict| format!("gate:{}", normalize_tag(&verdict.gate))),
    );
    dedupe(tags)
}

fn task_tags_from_episode(episode: &Episode) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(plan) = extra_string(episode, "plan_id") {
        tags.push(format!("plan:{plan}"));
    }
    if let Some(task_type) = extra_string(episode, "task_category")
        .or_else(|| extra_string(episode, "task_type"))
        .or_else(|| extra_string(episode, "complexity_band"))
    {
        tags.push(format!("task:{task_type}"));
    }
    if let Some(model) = non_empty(&episode.model) {
        tags.push(format!("model:{model}"));
    }
    if let Some(agent) = non_empty(&episode.agent_template).or_else(|| non_empty(&episode.agent_id))
    {
        tags.push(format!("agent:{agent}"));
    }
    tags.extend(episode.gate_verdicts.iter().map(|verdict| {
        format!(
            "gate:{}:{}",
            verdict.gate,
            if verdict.passed { "pass" } else { "fail" }
        )
    }));
    dedupe(tags)
}

fn distinct_contexts(observation: &RuntimeEpisodeObservation) -> Vec<String> {
    vec![context_label(observation)]
}

fn context_label(observation: &RuntimeEpisodeObservation) -> String {
    format!(
        "{}:{}:{}",
        observation.plan_id.as_deref().unwrap_or("unknown-plan"),
        observation.task_id,
        observation.episode_id
    )
}

fn summarize_gates(verdicts: &[GateVerdict]) -> String {
    if verdicts.is_empty() {
        return "the recorded outcome".to_string();
    }
    verdicts
        .iter()
        .map(|verdict| {
            format!(
                "{}:{}",
                verdict.gate,
                if verdict.passed { "pass" } else { "fail" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn gate_output_from_episode(episode: &Episode) -> String {
    let mut parts = Vec::new();
    if let Some(reason) = episode
        .failure_reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(reason.to_string());
    }
    for verdict in &episode.gate_verdicts {
        if let Some(signature) = verdict
            .signature
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            parts.push(format!("{}: {signature}", verdict.gate));
        }
    }
    parts.join("\n")
}

fn agent_output_from_episode(episode: &Episode) -> String {
    let mut parts = Vec::new();
    if let Some(reflection) = episode
        .reflection
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(reflection.to_string());
    }
    if let Some(summary) = episode
        .reasoning_summary
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(summary.to_string());
    }
    parts.join("\n")
}

fn extract_context_entry_ids(episode: &Episode) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["knowledge_entry_ids", "context_entry_ids", "knowledge_ids"] {
        if let Some(value) = episode.extra.get(key) {
            collect_string_values(value, &mut ids);
        }
    }
    if let Some(composition) = episode.prompt_composition.as_ref() {
        collect_prompt_knowledge_ids(composition, &mut ids);
    }
    clean_ids(&ids)
}

fn collect_prompt_knowledge_ids(value: &serde_json::Value, ids: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if key.contains("knowledge") && key.contains("id") {
                    collect_string_values(value, ids);
                }
                collect_prompt_knowledge_ids(value, ids);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_prompt_knowledge_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn collect_string_values(value: &serde_json::Value, ids: &mut Vec<String>) {
    match value {
        serde_json::Value::String(id) => ids.push(id.clone()),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_string_values(value, ids);
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["id", "entry_id", "knowledge_id"] {
                if let Some(value) = map.get(key) {
                    collect_string_values(value, ids);
                }
            }
        }
        _ => {}
    }
}

fn lifecycle_record_id(
    observation: &RuntimeEpisodeObservation,
    candidate_id: Option<&str>,
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    observation.episode_id.hash(&mut hasher);
    observation.observed_at.timestamp_millis().hash(&mut hasher);
    candidate_id.unwrap_or("").hash(&mut hasher);
    format!("knowledge-lifecycle:{:016x}", hasher.finish())
}

fn derive_runtime_knowledge_id(
    kind: KnowledgeKind,
    content: &str,
    source_episodes: &[String],
    tags: &[String],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.as_str().hash(&mut hasher);
    content.hash(&mut hasher);
    for episode in source_episodes {
        episode.hash(&mut hasher);
    }
    for tag in tags {
        tag.hash(&mut hasher);
    }
    format!("runtime_{:016x}", hasher.finish())
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(non_empty_str)
        .map(ToOwned::to_owned)
}

fn non_empty(value: &str) -> Option<String> {
    non_empty_str(value).map(ToOwned::to_owned)
}

fn non_empty_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn normalize_tag(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '/') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn clean_ids(ids: &[String]) -> Vec<String> {
    dedupe(
        ids.iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect(),
    )
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn trim_to(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    let mut end = limit;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &trimmed[..end])
}

fn append_jsonl<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let line = serde_json::to_string(value).context("serialize lifecycle record")?;
    writeln!(file, "{line}").context("write lifecycle record")?;
    file.flush().context("flush lifecycle record")?;
    file.sync_all().context("sync lifecycle record")?;
    Ok(())
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    let reader = BufReader::new(file);
    let mut values = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("read {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<T>(&line) {
            values.push(value);
        }
    }
    Ok(values)
}
