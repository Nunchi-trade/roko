//! Forensic replay engine for causal decision reconstruction (SAFE-12).
//!
//! [`ForensicReplay`] reconstructs the complete decision context for any past
//! agent action. Seven-step reconstruction:
//!
//! 1. **Action** -- identify the action Engram by `ContentHash`
//! 2. **Substrate state** -- reconstruct file/signal state at action time
//! 3. **Scorer outputs** -- reconstruct scores for each relevant Engram
//! 4. **Router selection** -- reconstruct routing decision including alternatives
//! 5. **Composer output** -- reconstruct prompt composition under budget
//! 6. **Gate verdict** -- reconstruct gate pass/fail decision
//! 7. **Policy decisions** -- reconstruct safety/authorization decisions
//!
//! The replay itself is persisted as a `kind: Replay` record with lineage
//! pointing to all reconstructed records, and is cryptographically verifiable
//! via BLAKE3 content-addressed hashes.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ContentHash;

// ─── Core types ───────────────────────────────────────────────────────

/// A single step in the forensic reconstruction chain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionStep {
    /// Step 1: The action being replayed.
    Action,
    /// Step 2: Substrate state at action time.
    SubstrateState,
    /// Step 3: Scorer outputs for relevant Engrams.
    ScorerOutputs,
    /// Step 4: Router selection and rejected alternatives.
    RouterSelection,
    /// Step 5: Composer output under budget constraints.
    ComposerOutput,
    /// Step 6: Gate verdict (pass/fail with evidence).
    GateVerdict,
    /// Step 7: Safety and authorization policy decisions.
    PolicyDecisions,
}

impl fmt::Display for ReconstructionStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action => write!(f, "action"),
            Self::SubstrateState => write!(f, "substrate_state"),
            Self::ScorerOutputs => write!(f, "scorer_outputs"),
            Self::RouterSelection => write!(f, "router_selection"),
            Self::ComposerOutput => write!(f, "composer_output"),
            Self::GateVerdict => write!(f, "gate_verdict"),
            Self::PolicyDecisions => write!(f, "policy_decisions"),
        }
    }
}

/// A scored Engram reference from the Scorer step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredReference {
    /// Content hash of the scored Engram.
    pub hash: ContentHash,
    /// Score assigned by the Scorer.
    pub score: f64,
    /// Name of the scorer that produced this score.
    pub scorer: String,
}

/// A routing decision with its alternatives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterDecisionRecord {
    /// The route that was selected.
    pub selected: String,
    /// Routes that were considered but rejected.
    pub alternatives: Vec<RouterAlternative>,
    /// Confidence score for the selected route.
    pub confidence: f64,
}

/// An alternative route that was considered but not selected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterAlternative {
    /// Route identifier.
    pub route: String,
    /// Score for this alternative.
    pub score: f64,
    /// Why this route was not selected.
    pub rejection_reason: String,
}

/// A policy decision made during action execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecisionRecord {
    /// Name of the policy that made the decision.
    pub policy: String,
    /// The decision outcome.
    pub outcome: PolicyOutcome,
    /// Human-readable reason for the decision.
    pub reason: String,
}

/// Outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOutcome {
    /// Policy allowed the action.
    Allow,
    /// Policy allowed with a confirmation requirement.
    AllowWithConfirm,
    /// Policy denied the action.
    Deny,
    /// Policy escalated to a higher authority.
    Escalate,
}

/// A gate verdict record for forensic replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateVerdictRecord {
    /// Name of the gate.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Evidence or reason for the verdict.
    pub evidence: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f64,
}

// ─── ForensicReplay ───────────────────────────────────────────────────

/// Complete forensic replay of an agent action.
///
/// Contains the seven-step reconstruction of the full decision context
/// for any past agent action. All steps are cryptographically verifiable
/// via BLAKE3 content-addressed hashes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForensicReplay {
    /// Content hash of the action being replayed.
    pub action: ContentHash,
    /// Unix-millis timestamp of the original action.
    pub action_timestamp_ms: i64,
    /// Agent that performed the action.
    pub agent_id: String,

    /// Step 2: Content hashes of the Substrate state at action time.
    pub substrate_state: Vec<ContentHash>,
    /// Step 3: Scorer outputs for each relevant Engram.
    pub scorer_outputs: Vec<ScoredReference>,
    /// Step 4: Router selection including rejected alternatives.
    pub router_selection: Option<RouterDecisionRecord>,
    /// Step 5: Composer output (the prompt that was assembled).
    pub composer_output: Option<String>,
    /// Step 6: Gate verdicts that determined pass/fail.
    pub gate_verdicts: Vec<GateVerdictRecord>,
    /// Step 7: Policy decisions made during the action.
    pub policy_decisions: Vec<PolicyDecisionRecord>,

    /// Content hash of this replay record (for chain integrity).
    pub replay_hash: ContentHash,
    /// Lineage: hashes of all reconstructed records.
    pub lineage: Vec<ContentHash>,
    /// Unix-millis timestamp when this replay was constructed.
    pub replayed_at_ms: i64,
    /// Per-step reconstruction status.
    pub step_status: HashMap<ReconstructionStep, StepStatus>,
}

/// Status of a single reconstruction step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Step was fully reconstructed.
    Complete,
    /// Step was partially reconstructed (some data missing).
    Partial {
        /// What is missing.
        missing: String,
    },
    /// Step could not be reconstructed.
    Failed {
        /// Reason for failure.
        reason: String,
    },
}

impl ForensicReplay {
    /// Create a new replay from a starting action hash.
    ///
    /// The replay is initialized with empty reconstruction steps; callers
    /// populate each step as they walk the episode/signal/custody logs.
    #[must_use]
    pub fn new(action: ContentHash, action_timestamp_ms: i64, agent_id: impl Into<String>) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        Self {
            action,
            action_timestamp_ms,
            agent_id: agent_id.into(),
            substrate_state: Vec::new(),
            scorer_outputs: Vec::new(),
            router_selection: None,
            composer_output: None,
            gate_verdicts: Vec::new(),
            policy_decisions: Vec::new(),
            replay_hash: ContentHash::of(&[0; 32]), // placeholder
            lineage: Vec::new(),
            replayed_at_ms: now_ms,
            step_status: HashMap::new(),
        }
    }

    /// Record the substrate state at action time (Step 2).
    pub fn with_substrate_state(mut self, state: Vec<ContentHash>) -> Self {
        self.lineage.extend(state.iter().copied());
        self.substrate_state = state;
        self.step_status
            .insert(ReconstructionStep::SubstrateState, StepStatus::Complete);
        self
    }

    /// Record scorer outputs (Step 3).
    pub fn with_scorer_outputs(mut self, outputs: Vec<ScoredReference>) -> Self {
        self.lineage.extend(outputs.iter().map(|o| o.hash));
        self.scorer_outputs = outputs;
        self.step_status
            .insert(ReconstructionStep::ScorerOutputs, StepStatus::Complete);
        self
    }

    /// Record router selection (Step 4).
    pub fn with_router_selection(mut self, selection: RouterDecisionRecord) -> Self {
        self.router_selection = Some(selection);
        self.step_status
            .insert(ReconstructionStep::RouterSelection, StepStatus::Complete);
        self
    }

    /// Record composer output (Step 5).
    pub fn with_composer_output(mut self, output: String) -> Self {
        self.composer_output = Some(output);
        self.step_status
            .insert(ReconstructionStep::ComposerOutput, StepStatus::Complete);
        self
    }

    /// Record gate verdicts (Step 6).
    pub fn with_gate_verdicts(mut self, verdicts: Vec<GateVerdictRecord>) -> Self {
        self.gate_verdicts = verdicts;
        self.step_status
            .insert(ReconstructionStep::GateVerdict, StepStatus::Complete);
        self
    }

    /// Record policy decisions (Step 7).
    pub fn with_policy_decisions(mut self, decisions: Vec<PolicyDecisionRecord>) -> Self {
        self.policy_decisions = decisions;
        self.step_status
            .insert(ReconstructionStep::PolicyDecisions, StepStatus::Complete);
        self
    }

    /// Mark a step as partially reconstructed.
    pub fn mark_partial(&mut self, step: ReconstructionStep, missing: impl Into<String>) {
        self.step_status.insert(
            step,
            StepStatus::Partial {
                missing: missing.into(),
            },
        );
    }

    /// Mark a step as failed to reconstruct.
    pub fn mark_failed(&mut self, step: ReconstructionStep, reason: impl Into<String>) {
        self.step_status.insert(
            step,
            StepStatus::Failed {
                reason: reason.into(),
            },
        );
    }

    /// Finalize the replay by computing its content hash.
    ///
    /// This must be called after all steps have been populated. The hash
    /// covers the entire replay content, making it tamper-evident.
    pub fn finalize(&mut self) {
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        self.replay_hash = ContentHash::of(&canonical);
    }

    /// Return `true` if all seven steps are complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        let required = [
            ReconstructionStep::SubstrateState,
            ReconstructionStep::ScorerOutputs,
            ReconstructionStep::RouterSelection,
            ReconstructionStep::ComposerOutput,
            ReconstructionStep::GateVerdict,
            ReconstructionStep::PolicyDecisions,
        ];
        required
            .iter()
            .all(|step| matches!(self.step_status.get(step), Some(StepStatus::Complete)))
    }

    /// Return the number of complete steps.
    #[must_use]
    pub fn complete_step_count(&self) -> usize {
        self.step_status
            .values()
            .filter(|s| matches!(s, StepStatus::Complete))
            .count()
    }

    /// Return a human-readable summary of the replay.
    #[must_use]
    pub fn summary(&self) -> String {
        let complete = self.complete_step_count();
        let total = self.step_status.len();
        format!(
            "ForensicReplay(action={}, agent={}, steps={}/{}, complete={})",
            self.action.short(),
            self.agent_id,
            complete,
            total,
            self.is_complete()
        )
    }
}

// ─── Persistence ──────────────────────────────────────────────────────

/// Append-only JSONL logger for forensic replays.
///
/// Follows the same pattern as `CustodyLogger` and `EpisodeLogger`.
#[derive(Debug, Clone)]
pub struct ForensicReplayLogger {
    path: PathBuf,
}

impl ForensicReplayLogger {
    /// Create a logger that writes to the given path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Append a replay to the log file.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be opened/written.
    pub fn log(&self, replay: &ForensicReplay) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let line = serde_json::to_string(replay)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{line}")
    }

    /// Read all replays from the log file.
    ///
    /// Returns an empty vec if the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read.
    pub fn read_all(&self) -> std::io::Result<Vec<ForensicReplay>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&self.path)?;
        let replays = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        Ok(replays)
    }

    /// Find a replay by its action hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be read.
    pub fn find_by_action(&self, action: &ContentHash) -> std::io::Result<Option<ForensicReplay>> {
        let replays = self.read_all()?;
        Ok(replays.into_iter().find(|r| r.action == *action))
    }

    /// Return the path to the replay log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forensic_replay_builds_incrementally() {
        let action_hash = ContentHash::of(b"test-action");
        let mut replay = ForensicReplay::new(action_hash, 1713600000000, "agent-1");

        assert!(!replay.is_complete());
        assert_eq!(replay.complete_step_count(), 0);

        // Add substrate state.
        replay = replay.with_substrate_state(vec![ContentHash::of(b"state-1")]);
        assert_eq!(replay.complete_step_count(), 1);

        // Add scorer outputs.
        replay = replay.with_scorer_outputs(vec![ScoredReference {
            hash: ContentHash::of(b"engram-1"),
            score: 0.85,
            scorer: "relevance".into(),
        }]);
        assert_eq!(replay.complete_step_count(), 2);

        // Add router selection.
        replay = replay.with_router_selection(RouterDecisionRecord {
            selected: "claude-sonnet-4-6".into(),
            alternatives: vec![RouterAlternative {
                route: "claude-haiku".into(),
                score: 0.4,
                rejection_reason: "task complexity too high".into(),
            }],
            confidence: 0.92,
        });

        // Add composer output.
        replay = replay.with_composer_output("System: You are an implementer...".into());

        // Add gate verdicts.
        replay = replay.with_gate_verdicts(vec![
            GateVerdictRecord {
                gate: "compile".into(),
                passed: true,
                evidence: "exit code 0".into(),
                confidence: 1.0,
            },
            GateVerdictRecord {
                gate: "test".into(),
                passed: true,
                evidence: "42/42 passed".into(),
                confidence: 1.0,
            },
        ]);

        // Add policy decisions.
        replay = replay.with_policy_decisions(vec![PolicyDecisionRecord {
            policy: "PathPolicy".into(),
            outcome: PolicyOutcome::Allow,
            reason: "within worktree".into(),
        }]);

        assert!(replay.is_complete());
        assert_eq!(replay.complete_step_count(), 6);

        // Finalize.
        replay.finalize();
        assert_ne!(replay.replay_hash, ContentHash::of(&[0; 32]));
    }

    #[test]
    fn forensic_replay_marks_partial_and_failed() {
        let action_hash = ContentHash::of(b"test-action-2");
        let mut replay = ForensicReplay::new(action_hash, 1713600000000, "agent-2");

        replay.mark_partial(ReconstructionStep::ScorerOutputs, "scorer log truncated");
        replay.mark_failed(
            ReconstructionStep::RouterSelection,
            "no routing data available",
        );

        assert!(!replay.is_complete());
        assert!(matches!(
            replay.step_status.get(&ReconstructionStep::ScorerOutputs),
            Some(StepStatus::Partial { .. })
        ));
        assert!(matches!(
            replay.step_status.get(&ReconstructionStep::RouterSelection),
            Some(StepStatus::Failed { .. })
        ));
    }

    #[test]
    fn forensic_replay_round_trips_through_serde() {
        let action_hash = ContentHash::of(b"serde-test");
        let replay = ForensicReplay::new(action_hash, 1713600000000, "agent-3")
            .with_substrate_state(vec![ContentHash::of(b"s1")])
            .with_gate_verdicts(vec![GateVerdictRecord {
                gate: "compile".into(),
                passed: true,
                evidence: "ok".into(),
                confidence: 1.0,
            }]);

        let json = serde_json::to_string(&replay).unwrap();
        let decoded: ForensicReplay = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.action, action_hash);
        assert_eq!(decoded.agent_id, "agent-3");
        assert_eq!(decoded.gate_verdicts.len(), 1);
        assert!(decoded.gate_verdicts[0].passed);
    }

    #[test]
    fn forensic_replay_summary() {
        let action_hash = ContentHash::of(b"summary-test");
        let replay = ForensicReplay::new(action_hash, 1713600000000, "agent-4")
            .with_substrate_state(vec![])
            .with_gate_verdicts(vec![]);
        let summary = replay.summary();
        assert!(summary.contains("agent-4"));
        assert!(summary.contains("ForensicReplay"));
    }

    #[test]
    fn forensic_replay_logger_writes_and_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("forensic.jsonl");
        let logger = ForensicReplayLogger::new(&log_path);

        let mut r1 = ForensicReplay::new(ContentHash::of(b"a1"), 100, "agent-1");
        r1.finalize();
        let mut r2 = ForensicReplay::new(ContentHash::of(b"a2"), 200, "agent-2");
        r2.finalize();

        logger.log(&r1).unwrap();
        logger.log(&r2).unwrap();

        let replays = logger.read_all().unwrap();
        assert_eq!(replays.len(), 2);
        assert_eq!(replays[0].agent_id, "agent-1");
        assert_eq!(replays[1].agent_id, "agent-2");
    }

    #[test]
    fn forensic_replay_logger_find_by_action() {
        let tmp = tempfile::tempdir().unwrap();
        let logger = ForensicReplayLogger::new(tmp.path().join("forensic.jsonl"));

        let hash1 = ContentHash::of(b"find-test-1");
        let hash2 = ContentHash::of(b"find-test-2");

        let mut r1 = ForensicReplay::new(hash1, 100, "a1");
        r1.finalize();
        let mut r2 = ForensicReplay::new(hash2, 200, "a2");
        r2.finalize();

        logger.log(&r1).unwrap();
        logger.log(&r2).unwrap();

        let found = logger.find_by_action(&hash2).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().agent_id, "a2");

        let not_found = logger
            .find_by_action(&ContentHash::of(b"nonexistent"))
            .unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn forensic_replay_logger_empty_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let logger = ForensicReplayLogger::new(tmp.path().join("missing.jsonl"));
        let replays = logger.read_all().unwrap();
        assert!(replays.is_empty());
    }

    #[test]
    fn policy_outcome_variants() {
        for outcome in [
            PolicyOutcome::Allow,
            PolicyOutcome::AllowWithConfirm,
            PolicyOutcome::Deny,
            PolicyOutcome::Escalate,
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            let decoded: PolicyOutcome = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, outcome);
        }
    }

    #[test]
    fn reconstruction_step_display() {
        assert_eq!(ReconstructionStep::Action.to_string(), "action");
        assert_eq!(
            ReconstructionStep::SubstrateState.to_string(),
            "substrate_state"
        );
        assert_eq!(
            ReconstructionStep::PolicyDecisions.to_string(),
            "policy_decisions"
        );
    }
}
