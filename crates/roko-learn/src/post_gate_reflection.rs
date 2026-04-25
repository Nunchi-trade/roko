//! Structured post-gate reflections and playbook candidate extraction.
//!
//! Reflections are durable audit records derived from gate outcomes. They can
//! accumulate repeated evidence into playbook candidates, but they do not
//! mutate active prompt or playbook policy by themselves.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::episode_logger::Episode;
use crate::playbook_rules::{ReflectionPlaybookCandidate, Triggers};

const MAX_LESSON_CHARS: usize = 600;
const MAX_EVIDENCE_ITEMS: usize = 10;
const MAX_EVIDENCE_CHARS: usize = 160;
const MAX_RECORDS: usize = 1_000;
const MAX_CANDIDATES: usize = 256;

/// Terminal gate outcome that triggered a reflection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionGateOutcome {
    /// Gate or task passed.
    Passed,
    /// Gate or task failed.
    Failed,
}

/// Admission state for a reflection-derived lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflectionAdmissionStatus {
    /// Evidence was captured, but there is not enough support to admit it.
    Candidate,
    /// Evidence is sufficient for a human or policy layer to admit it.
    Admissible,
    /// The lesson has been explicitly admitted into an active playbook store.
    Admitted,
    /// The lesson has too little evidence for admission.
    RejectedLowEvidence,
    /// The reflection did not contain an actionable lesson.
    RejectedNoActionableLesson,
}

/// A bounded post-gate reflection record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostGateReflectionRecord {
    /// Stable id for this reflection observation.
    pub reflection_id: String,
    /// Optional plan id associated with the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Optional task id associated with the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Optional source episode id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_id: Option<String>,
    /// Gate that triggered the reflection.
    pub trigger_gate: String,
    /// Whether the triggering gate passed or failed.
    pub outcome: ReflectionGateOutcome,
    /// Failure pattern ids that explain a failed gate.
    #[serde(default)]
    pub failure_pattern_ids: Vec<String>,
    /// Bounded positive evidence for passing gates.
    #[serde(default)]
    pub pass_evidence: Vec<String>,
    /// Short lesson proposed by the reflection.
    pub proposed_lesson: String,
    /// Confidence in `[0.0, 0.95]`, derived from evidence count.
    pub confidence: f64,
    /// Number of observations supporting this lesson cluster.
    pub evidence_count: u32,
    /// Admission status for the lesson.
    pub admission_status: ReflectionAdmissionStatus,
    /// Timestamp when the record was created.
    pub created_at: DateTime<Utc>,
}

/// Input used to create a post-gate reflection record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionInput {
    /// Optional plan id associated with the gate.
    pub plan_id: Option<String>,
    /// Optional task id associated with the gate.
    pub task_id: Option<String>,
    /// Optional source episode id.
    pub episode_id: Option<String>,
    /// Gate that triggered the reflection.
    pub trigger_gate: String,
    /// Whether the triggering gate passed or failed.
    pub outcome: ReflectionGateOutcome,
    /// Failure pattern ids that explain a failed gate.
    pub failure_pattern_ids: Vec<String>,
    /// Bounded positive evidence for passing gates.
    pub pass_evidence: Vec<String>,
    /// Short lesson proposed by the reflection.
    pub proposed_lesson: String,
}

impl ReflectionInput {
    /// Build a reflection input from an episode with gate verdicts.
    #[must_use]
    pub fn from_episode(episode: &Episode) -> Option<Self> {
        let failed = episode.gate_verdicts.iter().find(|verdict| !verdict.passed);
        let passed = episode.gate_verdicts.iter().find(|verdict| verdict.passed);
        let verdict = failed.or(passed)?;
        let outcome = if verdict.passed {
            ReflectionGateOutcome::Passed
        } else {
            ReflectionGateOutcome::Failed
        };

        let proposed_lesson = episode
            .reflection
            .as_deref()
            .or(episode.reasoning_summary.as_deref())
            .or(episode.failure_reason.as_deref())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| deterministic_lesson(episode, &verdict.gate, outcome));

        let failure_pattern_ids = episode
            .gate_verdicts
            .iter()
            .filter(|verdict| !verdict.passed)
            .filter_map(|verdict| verdict.signature.clone())
            .collect();
        let pass_evidence = episode
            .gate_verdicts
            .iter()
            .filter(|verdict| verdict.passed)
            .map(|verdict| {
                verdict
                    .signature
                    .as_deref()
                    .filter(|sig| !sig.trim().is_empty())
                    .map_or_else(|| format!("{}:passed", verdict.gate), str::to_string)
            })
            .collect();

        Some(Self {
            plan_id: extra_string(episode, "plan_id"),
            task_id: (!episode.task_id.trim().is_empty()).then(|| episode.task_id.clone()),
            episode_id: (!episode.id.trim().is_empty()).then(|| episode.id.clone()),
            trigger_gate: verdict.gate.clone(),
            outcome,
            failure_pattern_ids,
            pass_evidence,
            proposed_lesson,
        })
    }
}

/// Promotion thresholds for reflection-derived playbook candidates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReflectionPromotionConfig {
    /// Required repeated evidence before a candidate becomes admissible.
    pub min_evidence_count: u32,
    /// Required confidence before a candidate becomes admissible.
    pub min_confidence: f64,
}

impl Default for ReflectionPromotionConfig {
    fn default() -> Self {
        Self {
            min_evidence_count: 3,
            min_confidence: 0.65,
        }
    }
}

/// Result of recording one reflection.
#[derive(Debug, Clone, PartialEq)]
pub struct ReflectionObservation {
    /// Newly appended reflection record.
    pub record: PostGateReflectionRecord,
    /// Candidate updated by the reflection, if the lesson was actionable.
    pub candidate: Option<ReflectionPlaybookCandidate>,
}

/// JSON-backed reflection store.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PostGateReflectionStore {
    /// Bounded reflection records in append order.
    #[serde(default)]
    pub records: Vec<PostGateReflectionRecord>,
    /// Reflection-derived playbook candidates.
    #[serde(default)]
    pub candidates: Vec<ReflectionPlaybookCandidate>,
}

impl PostGateReflectionStore {
    /// Load a reflection store from disk. Missing or malformed stores are
    /// treated as empty so gate recording fails closed at the caller boundary.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => return Self::default(),
        };
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    /// Save this store to disk using a temporary file and rename.
    ///
    /// # Errors
    ///
    /// Returns filesystem or serialization errors.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Record a reflection and update its playbook candidate cluster.
    pub fn observe(
        &mut self,
        input: ReflectionInput,
        config: ReflectionPromotionConfig,
    ) -> ReflectionObservation {
        let now = Utc::now();
        let proposed_lesson = truncate_chars(&input.proposed_lesson, MAX_LESSON_CHARS);
        let cluster_id = reflection_cluster_id(&input);
        let evidence_count = self
            .records
            .iter()
            .filter(|record| reflection_record_cluster_id(record) == cluster_id)
            .count()
            .saturating_add(1) as u32;
        let confidence = confidence_for_evidence(evidence_count);
        let mut admission_status = status_for_evidence(evidence_count, confidence, config);
        if proposed_lesson.trim().is_empty() {
            admission_status = ReflectionAdmissionStatus::RejectedNoActionableLesson;
        }

        let record = PostGateReflectionRecord {
            reflection_id: format!(
                "reflection-{:016x}",
                stable_hash(&(cluster_id.clone(), now))
            ),
            plan_id: input.plan_id.clone(),
            task_id: input.task_id.clone(),
            episode_id: input.episode_id.clone(),
            trigger_gate: truncate_chars(&input.trigger_gate, MAX_EVIDENCE_CHARS),
            outcome: input.outcome,
            failure_pattern_ids: bound_items(input.failure_pattern_ids),
            pass_evidence: bound_items(input.pass_evidence),
            proposed_lesson: proposed_lesson.clone(),
            confidence,
            evidence_count,
            admission_status,
            created_at: now,
        };

        let candidate = candidate_from_record(&record).map(|candidate| {
            self.merge_candidate(candidate, config);
            self.candidates
                .iter()
                .find(|existing| existing.candidate_id == candidate_id_for_record(&record))
                .cloned()
                .expect("candidate was just merged")
        });

        self.records.push(record.clone());
        if self.records.len() > MAX_RECORDS {
            let overflow = self.records.len() - MAX_RECORDS;
            self.records.drain(0..overflow);
        }
        if self.candidates.len() > MAX_CANDIDATES {
            self.candidates.sort_by(|left, right| {
                right
                    .evidence_count
                    .cmp(&left.evidence_count)
                    .then_with(|| {
                        right
                            .confidence
                            .partial_cmp(&left.confidence)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            self.candidates.truncate(MAX_CANDIDATES);
        }

        ReflectionObservation { record, candidate }
    }

    fn merge_candidate(
        &mut self,
        mut candidate: ReflectionPlaybookCandidate,
        config: ReflectionPromotionConfig,
    ) {
        if let Some(existing) = self
            .candidates
            .iter_mut()
            .find(|existing| existing.candidate_id == candidate.candidate_id)
        {
            existing.evidence_count = existing.evidence_count.saturating_add(1);
            existing.confidence = confidence_for_evidence(existing.evidence_count);
            existing.updated_at = Utc::now();
            push_unique_all(
                &mut existing.source_reflection_ids,
                candidate.source_reflection_ids,
            );
            existing.admission_status =
                status_for_evidence(existing.evidence_count, existing.confidence, config);
            return;
        }

        candidate.admission_status =
            status_for_evidence(candidate.evidence_count, candidate.confidence, config);
        self.candidates.push(candidate);
    }
}

fn candidate_from_record(record: &PostGateReflectionRecord) -> Option<ReflectionPlaybookCandidate> {
    if record.proposed_lesson.trim().is_empty()
        || record.admission_status == ReflectionAdmissionStatus::RejectedNoActionableLesson
    {
        return None;
    }

    let triggers = triggers_from_record(record);
    if triggers.file_globs.is_empty()
        && triggers.tags.is_empty()
        && triggers.categories.is_empty()
        && triggers.error_signatures.is_empty()
        && triggers.roles.is_empty()
    {
        return None;
    }

    let candidate_id = candidate_id_for_record(record);
    Some(ReflectionPlaybookCandidate {
        candidate_id: candidate_id.clone(),
        rule_id: format!("reflection-{candidate_id}"),
        title: truncate_chars(
            &format!("{} {}", record.trigger_gate, record.proposed_lesson),
            80,
        ),
        body: record.proposed_lesson.clone(),
        triggers,
        confidence: record.confidence,
        evidence_count: record.evidence_count,
        admission_status: record.admission_status,
        source_reflection_ids: vec![record.reflection_id.clone()],
        created_at: record.created_at,
        updated_at: record.created_at,
    })
}

fn triggers_from_record(record: &PostGateReflectionRecord) -> Triggers {
    let mut triggers = Triggers::default();
    triggers.file_globs = extract_file_mentions(&record.proposed_lesson);
    triggers.tags = extract_error_tags(&record.proposed_lesson);
    triggers
        .error_signatures
        .extend(record.failure_pattern_ids.iter().cloned());
    if record.outcome == ReflectionGateOutcome::Passed {
        triggers.tags.push("gate-pass".to_string());
    }
    dedup(&mut triggers.file_globs);
    dedup(&mut triggers.tags);
    dedup(&mut triggers.error_signatures);
    triggers
}

fn reflection_cluster_id(input: &ReflectionInput) -> String {
    let lesson = normalize_text(&input.proposed_lesson);
    let patterns = normalized_join(&input.failure_pattern_ids);
    let passes = normalized_join(&input.pass_evidence);
    format!(
        "{}|{:?}|{}|{}|{}",
        input.trigger_gate, input.outcome, patterns, passes, lesson
    )
}

fn reflection_record_cluster_id(record: &PostGateReflectionRecord) -> String {
    let lesson = normalize_text(&record.proposed_lesson);
    let patterns = normalized_join(&record.failure_pattern_ids);
    let passes = normalized_join(&record.pass_evidence);
    format!(
        "{}|{:?}|{}|{}|{}",
        record.trigger_gate, record.outcome, patterns, passes, lesson
    )
}

fn candidate_id_for_record(record: &PostGateReflectionRecord) -> String {
    format!(
        "{:016x}",
        stable_hash(&(
            &record.trigger_gate,
            record.outcome,
            normalized_join(&record.failure_pattern_ids),
            normalize_text(&record.proposed_lesson),
        ))
    )
}

fn status_for_evidence(
    evidence_count: u32,
    confidence: f64,
    config: ReflectionPromotionConfig,
) -> ReflectionAdmissionStatus {
    if evidence_count >= config.min_evidence_count && confidence >= config.min_confidence {
        ReflectionAdmissionStatus::Admissible
    } else {
        ReflectionAdmissionStatus::Candidate
    }
}

fn confidence_for_evidence(evidence_count: u32) -> f64 {
    (0.4 + f64::from(evidence_count) * 0.1).min(0.95)
}

fn deterministic_lesson(episode: &Episode, gate: &str, outcome: ReflectionGateOutcome) -> String {
    match outcome {
        ReflectionGateOutcome::Failed => {
            let details = episode
                .gate_verdicts
                .iter()
                .find(|verdict| !verdict.passed)
                .and_then(|verdict| verdict.signature.as_deref())
                .unwrap_or("unclassified failure");
            format!("Investigate {gate} failure before retrying; pattern: {details}")
        }
        ReflectionGateOutcome::Passed => {
            format!("Preserve the approach that satisfied {gate} in the next similar task")
        }
    }
}

fn extract_file_mentions(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let clean = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']'
                )
            });
            let looks_like_file = clean.contains('/')
                && [".rs", ".toml", ".json", ".md", ".ts", ".tsx", ".js", ".jsx"]
                    .iter()
                    .any(|suffix| clean.ends_with(suffix));
            looks_like_file.then(|| clean.to_string())
        })
        .take(MAX_EVIDENCE_ITEMS)
        .collect()
}

fn extract_error_tags(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let clean = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | ',' | ';' | ':' | ')' | '(' | '[' | ']'
                )
            });
            if clean.starts_with('E')
                && clean.len() == 5
                && clean.chars().skip(1).all(|ch| ch.is_ascii_digit())
            {
                Some(clean.to_string())
            } else if clean.contains('_')
                && clean
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                Some(clean.to_lowercase())
            } else {
                None
            }
        })
        .take(MAX_EVIDENCE_ITEMS)
        .collect()
}

fn extra_string(episode: &Episode, key: &str) -> Option<String> {
    episode
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn bound_items(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|item| truncate_chars(&item, MAX_EVIDENCE_CHARS))
        .filter(|item| !item.trim().is_empty())
        .take(MAX_EVIDENCE_ITEMS)
        .collect()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn normalized_join(items: &[String]) -> String {
    let mut items: Vec<String> = items.iter().map(|item| normalize_text(item)).collect();
    items.sort();
    items.dedup();
    items.join(",")
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn dedup(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}

fn push_unique_all(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_logger::GateVerdict;
    use crate::playbook_rules::{
        CandidateAdmissionConfig, CandidateAdmissionDecision, PlaybookRules,
    };
    use tempfile::TempDir;

    fn failed_episode() -> Episode {
        let mut episode = Episode::new("agent", "task-1");
        episode
            .gate_verdicts
            .push(GateVerdict::new("compile", false).with_signature("E0308:type_mismatch"));
        episode.reflection = Some(
            "Fix crates/roko-learn/src/lib.rs before retrying E0308 type_mismatch".to_string(),
        );
        episode
    }

    #[test]
    fn gate_failure_creates_bounded_reflection_record() {
        let input = ReflectionInput::from_episode(&failed_episode()).expect("reflection input");
        let mut store = PostGateReflectionStore::default();
        let observed = store.observe(input, ReflectionPromotionConfig::default());

        assert_eq!(observed.record.trigger_gate, "compile");
        assert_eq!(observed.record.outcome, ReflectionGateOutcome::Failed);
        assert_eq!(
            observed.record.failure_pattern_ids,
            vec!["E0308:type_mismatch"]
        );
        assert_eq!(
            observed.record.admission_status,
            ReflectionAdmissionStatus::Candidate
        );
        assert!(observed.record.proposed_lesson.len() <= MAX_LESSON_CHARS);
        assert_eq!(store.records.len(), 1);
    }

    #[test]
    fn repeated_evidence_promotes_candidate_to_admissible() {
        let mut store = PostGateReflectionStore::default();
        let config = ReflectionPromotionConfig::default();
        for _ in 0..3 {
            let input = ReflectionInput::from_episode(&failed_episode()).expect("reflection input");
            store.observe(input, config);
        }

        assert_eq!(store.candidates.len(), 1);
        let candidate = &store.candidates[0];
        assert_eq!(candidate.evidence_count, 3);
        assert_eq!(
            candidate.admission_status,
            ReflectionAdmissionStatus::Admissible
        );
    }

    #[test]
    fn low_evidence_candidate_is_rejected_by_playbook_admission() {
        let mut store = PostGateReflectionStore::default();
        let input = ReflectionInput::from_episode(&failed_episode()).expect("reflection input");
        let observed = store.observe(input, ReflectionPromotionConfig::default());
        let candidate = observed.candidate.expect("candidate");

        let dir = TempDir::new().expect("tempdir");
        let rules = PlaybookRules::open(dir.path().join("rules.toml")).expect("rules");
        let decision = rules
            .admit_reflection_candidate(&candidate, CandidateAdmissionConfig::default())
            .expect("admission");

        assert!(matches!(
            decision,
            CandidateAdmissionDecision::RejectedLowEvidence { .. }
        ));
        assert_eq!(rules.count(), 0);
    }
}
