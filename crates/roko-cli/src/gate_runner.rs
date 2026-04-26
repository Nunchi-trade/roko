//! Gate-related free functions and types extracted from `orchestrate.rs`.
//!
//! This module contains gate helpers that do not require `&self` access to
//! `PlanRunner`. The heavy gate methods (`run_gate_pipeline`, `run_gate_rung`,
//! `gate_rung_config`, `enrich_rung_config`) remain on `PlanRunner` in
//! `orchestrate.rs` since they deeply access runner state.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use roko_core::{Context, Engram, Gate, TaskDomain, Verdict};
use roko_gate::generated_test_gate::ArtifactStore as GeneratedArtifactStore;
use roko_gate::rung_selector::Rung;
use roko_gate::{AcceptanceDecision, AcceptanceOutcome, NoStubEvidence};
use roko_orchestrator::GateResult;

// ─── Path helpers ────────────────────────────────────────────────────────

/// Root directory for content-addressed gate artifacts.
pub(crate) fn gate_artifact_store_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("artifacts")
}

/// Path to the persistent gate ratchet ledger.
pub(crate) fn gate_ratchet_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("learn")
        .join("gate-ratchet.json")
}

/// Directory that holds acceptance evidence for a specific plan+task pair.
pub(crate) fn acceptance_task_dir(workdir: &Path, plan_id: &str, task_id: &str) -> PathBuf {
    workdir
        .join(".roko")
        .join("acceptance")
        .join(plan_id)
        .join(task_id)
}

// ─── Domain predicates ───────────────────────────────────────────────────

/// Whether this domain uses compiled (compile/test/clippy) gates.
pub(crate) fn domain_uses_compiled_gates(domain: &TaskDomain) -> bool {
    matches!(
        domain,
        TaskDomain::Code | TaskDomain::Chain | TaskDomain::Custom(_)
    )
}

// ─── Rung mapping ────────────────────────────────────────────────────────

/// Map a primary gate phase name (e.g. "compile", "clippy", "test") to its
/// canonical [`Rung`].
pub(crate) fn primary_gate_phase_to_rung(phase: &str) -> Option<Rung> {
    match phase {
        gate if gate.starts_with("compile") => Some(Rung::Compile),
        gate if gate.starts_with("clippy") => Some(Rung::Lint),
        gate if gate.starts_with("test") => Some(Rung::Test),
        "symbol" => Some(Rung::Symbol),
        gate if gate.starts_with("generated_test") || gate == "verify_chain" => {
            Some(Rung::GeneratedTest)
        }
        gate if gate.starts_with("property_test") || gate == "fact_check" => {
            Some(Rung::PropertyTest)
        }
        gate if gate == "llm_judge" || gate.starts_with("integration") => Some(Rung::Integration),
        _ => None,
    }
}

// ─── Gate result matching ────────────────────────────────────────────────

/// Check whether a [`GateResult`] satisfies a gate requirement by name or
/// kind.
pub(crate) fn gate_result_matches_requirement(
    result: &GateResult,
    requirement: &roko_gate::GateRequirement,
) -> bool {
    let gate_name = result.gate_name.to_ascii_lowercase();
    let requirement_id = requirement.id.to_ascii_lowercase();
    if gate_name == requirement_id || gate_name.contains(&requirement_id) {
        return true;
    }
    matches!(
        (requirement.kind, gate_name.as_str()),
        (
            roko_gate::GateRequirementKind::Compile,
            "compile" | "cargo_check"
        ) | (roko_gate::GateRequirementKind::Test, "test" | "cargo_test")
            | (roko_gate::GateRequirementKind::Lint, "lint" | "clippy")
            | (
                roko_gate::GateRequirementKind::Review,
                "review" | "llm_judge"
            )
    )
}

// ─── Acceptance evidence ─────────────────────────────────────────────────

/// Scan production paths for stub markers (`todo!`, `unimplemented!`, etc.)
/// and return evidence of whether any were found.
pub(crate) fn scan_no_stub_evidence(workdir: &Path, production_paths: &[String]) -> NoStubEvidence {
    let mut scanned_paths = Vec::new();
    let mut findings = Vec::new();
    for path in production_paths {
        let full_path = workdir.join(path);
        if !full_path.exists() {
            findings.push(format!("{path}: path missing"));
            continue;
        }
        scanned_paths.push(path.clone());
        if full_path.is_file()
            && let Ok(content) = std::fs::read_to_string(&full_path)
        {
            let lower = content.to_ascii_lowercase();
            for marker in ["todo!", "unimplemented!", "noop", "stub"] {
                if lower.contains(marker) {
                    findings.push(format!("{path}: contains marker `{marker}`"));
                }
            }
        }
    }
    NoStubEvidence {
        outcome: if findings.is_empty() {
            AcceptanceOutcome::Passed
        } else {
            AcceptanceOutcome::Failed
        },
        scanned_paths,
        findings,
    }
}

/// Format a human-readable summary of an acceptance decision that did not pass.
pub(crate) fn format_acceptance_decision(
    task_id: &str,
    decision: &AcceptanceDecision,
) -> String {
    let mut out = format!(
        "task {task_id} acceptance outcome {:?} did not pass",
        decision.outcome
    );
    for issue in &decision.issues {
        out.push_str(&format!(
            "\n- {}: {}{}",
            issue.code,
            issue.message,
            if issue.blocking { " (blocking)" } else { "" }
        ));
    }
    out
}

// ─── Neuro-gate bridge ───────────────────────────────────────────────────

/// INT-15: Query neuro knowledge for gate-related failure and stability
// ─── Recording gate wrapper ──────────────────────────────────────────────

/// A verdict captured by [`RecordingGate`] for post-pipeline analysis.
#[derive(Clone)]
pub(crate) struct RecordedGateVerdict {
    pub(crate) rung: Rung,
    pub(crate) verdict: Verdict,
}

/// Decorator around a [`Gate`] that records every verdict into a shared sink.
pub(crate) struct RecordingGate {
    rung: Rung,
    inner: Box<dyn Gate>,
    sink: Arc<Mutex<Vec<RecordedGateVerdict>>>,
}

impl RecordingGate {
    pub(crate) fn new(
        rung: Rung,
        inner: Box<dyn Gate>,
        sink: Arc<Mutex<Vec<RecordedGateVerdict>>>,
    ) -> Self {
        Self { rung, inner, sink }
    }
}

#[async_trait::async_trait]
impl Gate for RecordingGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let verdict = self.inner.verify(signal, ctx).await;
        self.sink
            .lock()
            .expect("recorded gate sink poisoned")
            .push(RecordedGateVerdict {
                rung: self.rung,
                verdict: verdict.clone(),
            });
        verdict
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

// ─── Generated-test artifact store ───────────────────────────────────────

/// Filesystem-backed store for generated test artifacts, keyed by plan.
#[derive(Clone, Debug)]
pub(crate) struct FsGeneratedArtifactStore {
    root: PathBuf,
}

impl FsGeneratedArtifactStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn artifact_dir(&self) -> PathBuf {
        self.root.join("generated-tests")
    }

    pub(crate) fn matching_entries(&self, prefix: &str) -> Vec<String> {
        let dir = self.artifact_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };

        let mut names: Vec<String> = entries
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                entry.file_type().ok().filter(|kind| kind.is_file())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let logical = format!("generated-tests/{name}");
                logical.starts_with(prefix).then_some(logical)
            })
            .collect();
        names.sort();
        names
    }
}

impl GeneratedArtifactStore for FsGeneratedArtifactStore {
    fn list(&self, _plan: &str, prefix: &str) -> Vec<String> {
        self.matching_entries(prefix)
    }

    fn read(&self, _plan: &str, name: &str) -> Option<Vec<u8>> {
        let relative = name.strip_prefix("generated-tests/")?;
        if relative.contains("..") || relative.contains('/') {
            return None;
        }
        std::fs::read(self.artifact_dir().join(relative)).ok()
    }
}
