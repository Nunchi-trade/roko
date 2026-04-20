//! Forensic causal chain reconstruction from content-addressed artifacts.
//!
//! Reconstructs the full causal chain for a task: which agent produced which
//! output, which gate verified it, what the verdict was, and what evidence
//! (compiler output, test results, diff) supports the verdict.
//!
//! This enables:
//! - Post-hoc auditing of any task outcome
//! - Root cause analysis when regressions appear
//! - Integrity verification via BLAKE3 hash chain
//!
//! Reference: docs/04-verification/12-forensic-ai-causal-replay.md

use roko_core::{ContentHash, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::artifact_store::ArtifactStore;

/// Metadata about a stored artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    /// Content hash of the artifact.
    pub hash: ContentHash,
    /// Size in bytes.
    pub size_bytes: usize,
    /// Kind of artifact (e.g., "compile_output", "test_output", "diff").
    pub kind: String,
    /// Whether the hash was verified against the stored content.
    pub integrity_verified: bool,
}

/// A single turn record in the causal chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnRecord {
    /// Turn index (0-based).
    pub turn_index: usize,
    /// Agent model used for this turn.
    pub agent_model: String,
    /// Verdicts produced at this turn.
    pub verdicts: Vec<Verdict>,
    /// Artifact hashes associated with this turn.
    pub artifact_hashes: Vec<ContentHash>,
}

/// The complete causal chain for a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalChain {
    /// Task identifier.
    pub task_id: String,
    /// Primary agent model used for the task.
    pub agent_model: String,
    /// Ordered turns with their verdicts and evidence.
    pub turns: Vec<TurnRecord>,
    /// All verdicts with their associated artifact hashes.
    pub verdicts: Vec<(Verdict, Option<ContentHash>)>,
    /// Artifact metadata for each referenced hash.
    pub artifacts: Vec<ArtifactMetadata>,
    /// Whether the entire hash chain has been verified.
    pub integrity_verified: bool,
}

impl CausalChain {
    /// Number of turns in the chain.
    #[must_use]
    pub fn turn_count(&self) -> usize {
        self.turns.len()
    }

    /// Total number of verdicts.
    #[must_use]
    pub fn verdict_count(&self) -> usize {
        self.verdicts.len()
    }

    /// Number of artifacts referenced.
    #[must_use]
    pub fn artifact_count(&self) -> usize {
        self.artifacts.len()
    }

    /// Whether any gate failed in the chain.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.verdicts.iter().any(|(v, _)| !v.passed)
    }

    /// Produce a compact summary of the causal chain.
    #[must_use]
    pub fn summary(&self) -> String {
        let pass_count = self.verdicts.iter().filter(|(v, _)| v.passed).count();
        let fail_count = self.verdicts.iter().filter(|(v, _)| !v.passed).count();
        format!(
            "Task {}: {} turns, {} verdicts ({} pass, {} fail), {} artifacts, integrity={}",
            self.task_id,
            self.turn_count(),
            self.verdict_count(),
            pass_count,
            fail_count,
            self.artifact_count(),
            self.integrity_verified,
        )
    }
}

/// Error type for forensic replay operations.
#[derive(Debug)]
pub enum ForensicError {
    /// The requested task was not found in the provided data.
    TaskNotFound(String),
    /// An artifact hash could not be verified.
    IntegrityCheckFailed(ContentHash),
}

impl std::fmt::Display for ForensicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TaskNotFound(id) => write!(f, "task not found: {id}"),
            Self::IntegrityCheckFailed(hash) => {
                write!(f, "integrity check failed for artifact {hash}")
            }
        }
    }
}

impl std::error::Error for ForensicError {}

/// Forensic replay builder that reconstructs causal chains.
///
/// Accumulates turn records with verdicts and artifact hashes, then
/// verifies the BLAKE3 hash chain against the artifact store.
#[derive(Debug, Default)]
pub struct ForensicReplayBuilder {
    /// Accumulated turns keyed by task_id.
    task_turns: HashMap<String, Vec<TurnRecord>>,
    /// Verdicts keyed by task_id.
    task_verdicts: HashMap<String, Vec<(Verdict, Option<ContentHash>)>>,
    /// Primary model for each task.
    task_models: HashMap<String, String>,
}

impl ForensicReplayBuilder {
    /// Create a new builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a turn's results.
    pub fn record_turn(
        &mut self,
        task_id: &str,
        turn_index: usize,
        agent_model: &str,
        verdicts: Vec<Verdict>,
        artifact_hashes: Vec<ContentHash>,
    ) {
        self.task_models
            .entry(task_id.to_string())
            .or_insert_with(|| agent_model.to_string());

        for verdict in &verdicts {
            let hash = if !artifact_hashes.is_empty() {
                Some(artifact_hashes[0])
            } else {
                None
            };
            self.task_verdicts
                .entry(task_id.to_string())
                .or_default()
                .push((verdict.clone(), hash));
        }

        self.task_turns
            .entry(task_id.to_string())
            .or_default()
            .push(TurnRecord {
                turn_index,
                agent_model: agent_model.to_string(),
                verdicts,
                artifact_hashes,
            });
    }

    /// Reconstruct the causal chain for a task and verify integrity.
    ///
    /// Walks the content-hash links in the artifact store to verify each
    /// referenced artifact's BLAKE3 hash matches its content.
    ///
    /// # Errors
    ///
    /// Returns `ForensicError::TaskNotFound` if no turns have been recorded
    /// for the given task.
    pub fn replay_task(
        &self,
        task_id: &str,
        artifact_store: &ArtifactStore,
    ) -> Result<CausalChain, ForensicError> {
        let turns = self
            .task_turns
            .get(task_id)
            .ok_or_else(|| ForensicError::TaskNotFound(task_id.to_string()))?;

        let verdicts = self
            .task_verdicts
            .get(task_id)
            .cloned()
            .unwrap_or_default();

        let agent_model = self
            .task_models
            .get(task_id)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        // Collect all artifact hashes and verify them.
        let mut artifacts = Vec::new();
        let mut all_verified = true;

        for turn in turns {
            for hash in &turn.artifact_hashes {
                let (size, verified) = match artifact_store.retrieve(hash) {
                    Some(content) => {
                        let recomputed = ContentHash::of(content);
                        let ok = recomputed == *hash;
                        if !ok {
                            all_verified = false;
                        }
                        (content.len(), ok)
                    }
                    None => {
                        all_verified = false;
                        (0, false)
                    }
                };

                artifacts.push(ArtifactMetadata {
                    hash: *hash,
                    size_bytes: size,
                    kind: "gate_evidence".to_string(),
                    integrity_verified: verified,
                });
            }
        }

        Ok(CausalChain {
            task_id: task_id.to_string(),
            agent_model,
            turns: turns.clone(),
            verdicts,
            artifacts,
            integrity_verified: all_verified,
        })
    }

    /// Verify the hash chain integrity for all artifacts in a causal chain.
    #[must_use]
    pub fn verify_chain_integrity(
        chain: &CausalChain,
        artifact_store: &ArtifactStore,
    ) -> bool {
        for artifact in &chain.artifacts {
            match artifact_store.retrieve(&artifact.hash) {
                Some(content) => {
                    let recomputed = ContentHash::of(content);
                    if recomputed != artifact.hash {
                        return false;
                    }
                }
                None => return false,
            }
        }
        true
    }

    /// Return all task IDs that have been recorded.
    pub fn task_ids(&self) -> Vec<&str> {
        self.task_turns.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::Verdict;

    #[test]
    fn forensic_replay_round_trip() {
        let mut store = ArtifactStore::new();
        let content = b"error[E0308]: mismatched types";
        let hash = store.store(content).unwrap();

        let mut builder = ForensicReplayBuilder::new();
        builder.record_turn(
            "task-1",
            0,
            "claude-sonnet",
            vec![Verdict::fail("compile", "type error")],
            vec![hash],
        );
        builder.record_turn(
            "task-1",
            1,
            "claude-sonnet",
            vec![Verdict::pass("compile")],
            vec![],
        );

        let chain = builder.replay_task("task-1", &store).unwrap();

        assert_eq!(chain.task_id, "task-1");
        assert_eq!(chain.turn_count(), 2);
        assert_eq!(chain.verdict_count(), 2);
        assert_eq!(chain.artifact_count(), 1);
        assert!(chain.integrity_verified);
        assert!(chain.has_failures());
    }

    #[test]
    fn forensic_replay_task_not_found() {
        let store = ArtifactStore::new();
        let builder = ForensicReplayBuilder::new();
        let result = builder.replay_task("nonexistent", &store);
        assert!(result.is_err());
    }

    #[test]
    fn forensic_verify_chain_integrity() {
        let mut store = ArtifactStore::new();
        let hash = store.store(b"test output: 42 passed, 0 failed").unwrap();

        let mut builder = ForensicReplayBuilder::new();
        builder.record_turn(
            "task-2",
            0,
            "claude-opus",
            vec![Verdict::pass("test")],
            vec![hash],
        );

        let chain = builder.replay_task("task-2", &store).unwrap();
        assert!(ForensicReplayBuilder::verify_chain_integrity(&chain, &store));
    }

    #[test]
    fn forensic_summary_format() {
        let chain = CausalChain {
            task_id: "task-3".to_string(),
            agent_model: "claude-sonnet".to_string(),
            turns: vec![],
            verdicts: vec![
                (Verdict::pass("compile"), None),
                (Verdict::fail("test", "assertion failed"), None),
            ],
            artifacts: vec![],
            integrity_verified: true,
        };

        let summary = chain.summary();
        assert!(summary.contains("task-3"));
        assert!(summary.contains("1 pass"));
        assert!(summary.contains("1 fail"));
        assert!(summary.contains("integrity=true"));
    }

    #[test]
    fn forensic_empty_chain_no_failures() {
        let chain = CausalChain {
            task_id: "task-4".to_string(),
            agent_model: "test".to_string(),
            turns: vec![],
            verdicts: vec![
                (Verdict::pass("compile"), None),
                (Verdict::pass("test"), None),
            ],
            artifacts: vec![],
            integrity_verified: true,
        };
        assert!(!chain.has_failures());
    }
}
