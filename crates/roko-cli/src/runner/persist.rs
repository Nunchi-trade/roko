//! Atomic persistence for executor snapshots, episodes, and agent PIDs.
//!
//! All writes use write-to-tmp-then-rename for crash safety.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use roko_orchestrator::{ExecutorSnapshot, OrchestratorSnapshot};
use serde::{Deserialize, Serialize};

use super::types::RunnerEvent;

/// Schema version for the runner-owned `run-state.json` snapshot.
///
/// Bump only when the on-disk shape of [`RunStateSnapshot`] changes in a way
/// that requires migration on resume.
pub const RUN_STATE_SCHEMA_VERSION: u32 = 1;

/// Paths for all persistent state files.
#[derive(Debug, Clone)]
pub struct PersistPaths {
    /// `.roko/state/executor.json` — executor snapshot.
    pub executor_json: PathBuf,
    /// `.roko/state/orchestrator.json` — aggregate orchestrator snapshot.
    pub orchestrator_json: PathBuf,
    /// `.roko/state/run-state.json` — runner-owned cost/token/completed-task snapshot.
    pub run_state_json: PathBuf,
    /// `.roko/episodes.jsonl` — episode log.
    pub episodes_jsonl: PathBuf,
    /// `.roko/learn/efficiency.jsonl` — efficiency events.
    pub efficiency_jsonl: PathBuf,
    /// `.roko/learn/cascade-router.json` — cascade router learning state.
    pub cascade_router_json: PathBuf,
    /// `.roko/learn/gate-thresholds.json` — adaptive gate thresholds.
    pub gate_thresholds_json: PathBuf,
    /// `.roko/runtime/agent-pids.json` — live agent PIDs.
    pub agent_pids_json: PathBuf,
    /// `.roko/state/events.json` — event log for replay.
    pub events_json: PathBuf,
    /// `.roko/events.jsonl` — append-only runner event log consumed by TUI/server.
    pub events_jsonl: PathBuf,
}

impl PersistPaths {
    /// Derive all paths from a workdir, creating parent directories as needed.
    pub fn from_workdir(workdir: &Path) -> Result<Self> {
        let roko = workdir.join(".roko");
        let state = roko.join("state");
        let learn = roko.join("learn");
        let runtime = roko.join("runtime");

        for dir in [&state, &learn, &runtime] {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }

        Ok(Self {
            executor_json: state.join("executor.json"),
            orchestrator_json: state.join("orchestrator.json"),
            run_state_json: state.join("run-state.json"),
            episodes_jsonl: roko.join("episodes.jsonl"),
            efficiency_jsonl: learn.join("efficiency.jsonl"),
            cascade_router_json: learn.join("cascade-router.json"),
            gate_thresholds_json: learn.join("gate-thresholds.json"),
            agent_pids_json: runtime.join("agent-pids.json"),
            events_json: state.join("events.json"),
            events_jsonl: roko.join("events.jsonl"),
        })
    }
}

/// Runner-owned snapshot persisted alongside `executor.json`.
///
/// Captures the cost, token, and completed-task state the orchestrator-level
/// `ExecutorSnapshot` does not retain. This is the structure written to
/// `.roko/state/run-state.json` and consumed by [`super::resume`] when
/// validating a resume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunStateSnapshot {
    /// On-disk schema version. See [`RUN_STATE_SCHEMA_VERSION`].
    #[serde(default)]
    pub schema_version: u32,
    /// Stable identifier for the runner invocation that wrote this snapshot.
    pub run_id: String,
    /// UTC ms when the run started.
    #[serde(default)]
    pub started_at_ms: u64,
    /// UTC ms when the snapshot was written.
    #[serde(default)]
    pub timestamp_ms: u64,
    /// Total tasks across all plans known at snapshot time.
    pub tasks_total: usize,
    /// Number of tasks completed.
    pub tasks_completed: usize,
    /// Number of tasks that failed.
    pub tasks_failed: usize,
    /// Total input tokens across the run.
    pub total_tokens_in: u64,
    /// Total output tokens across the run.
    pub total_tokens_out: u64,
    /// Total cost in USD across the run.
    pub total_cost_usd: f64,
    /// Total agent spawn count.
    pub total_agent_calls: usize,
    /// Per-plan cost accumulation.
    #[serde(default)]
    pub plan_costs: HashMap<String, f64>,
    /// Completed task IDs per plan — the durable record used to skip
    /// already-finished work on resume.
    #[serde(default)]
    pub completed_tasks: HashMap<String, Vec<String>>,
    /// Consecutive snapshot save failures (degradation tracking).
    #[serde(default)]
    pub snapshot_fail_streak: u32,
}

/// Forensic fingerprint of a task definition used for strict resume validation.
///
/// Hash inputs are deterministic and span the fields a plan author can mutate
/// between runs (id, title, role, tier, dependencies, verify steps, gate
/// budgets). Mismatch on resume is a hard failure: see
/// [`super::resume::ResumeError::TaskMismatch`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefFingerprint {
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// FNV-1a hash (hex) of the canonical task definition payload.
    pub fingerprint: String,
}

/// Atomically write `content` to `path` via a `.tmp` sibling.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Append a JSON line to a JSONL file.
pub fn append_jsonl(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut line = serde_json::to_string(value).context("serializing JSONL value")?;
    line.push('\n');

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;

    file.write_all(line.as_bytes())
        .with_context(|| format!("appending to {}", path.display()))?;
    file.flush()?;
    Ok(())
}

/// Append a normalized runner lifecycle event to the durable JSONL log.
pub fn append_runner_event(paths: &PersistPaths, event: &RunnerEvent) -> Result<()> {
    append_jsonl(&paths.events_jsonl, event)
}

/// Save the executor snapshot atomically.
pub fn save_executor_snapshot(paths: &PersistPaths, snapshot: &ExecutorSnapshot) -> Result<()> {
    let json = serde_json::to_string_pretty(snapshot).context("serializing executor snapshot")?;
    atomic_write(&paths.executor_json, json.as_bytes())
}

/// Save the aggregate orchestrator snapshot atomically.
pub fn save_orchestrator_snapshot(
    paths: &PersistPaths,
    snapshot: &OrchestratorSnapshot,
) -> Result<()> {
    let json = snapshot
        .to_json()
        .context("serializing orchestrator snapshot")?;
    atomic_write(&paths.orchestrator_json, json.as_bytes())
}

/// Save the set of live agent PIDs.
pub fn save_agent_pids(paths: &PersistPaths, pids: &[u32]) -> Result<()> {
    let json = serde_json::to_string_pretty(&pids).context("serializing agent PIDs")?;
    atomic_write(&paths.agent_pids_json, json.as_bytes())
}

/// Read previously-saved agent PIDs and kill any that are still alive.
pub fn cleanup_orphaned_agents(paths: &PersistPaths) {
    let Ok(content) = fs::read_to_string(&paths.agent_pids_json) else {
        return;
    };
    let pids = match serde_json::from_str::<Vec<u32>>(&content) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                path = %paths.agent_pids_json.display(),
                err = %e,
                "malformed agent PID file — removing"
            );
            let _ = fs::remove_file(&paths.agent_pids_json);
            return;
        }
    };

    for pid in pids {
        // Delegate to roko-agent's registry-based cleanup.
        roko_agent::process::register_spawned_pid(pid);
    }
    roko_agent::process::cleanup_orphaned_agents();

    // Clean up the PID file.
    let _ = fs::remove_file(&paths.agent_pids_json);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_paths_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = PersistPaths::from_workdir(tmp.path()).unwrap();
        assert!(paths.executor_json.parent().unwrap().is_dir());
        assert!(paths.efficiency_jsonl.parent().unwrap().is_dir());
        assert!(paths.agent_pids_json.parent().unwrap().is_dir());
    }

    #[test]
    fn atomic_write_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.json");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn append_jsonl_multiple_values() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("log.jsonl");
        append_jsonl(&path, &serde_json::json!({"a": 1})).unwrap();
        append_jsonl(&path, &serde_json::json!({"b": 2})).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn save_agent_pids_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = PersistPaths::from_workdir(tmp.path()).unwrap();
        save_agent_pids(&paths, &[1234, 5678]).unwrap();

        let content = fs::read_to_string(&paths.agent_pids_json).unwrap();
        let pids: Vec<u32> = serde_json::from_str(&content).unwrap();
        assert_eq!(pids, vec![1234, 5678]);
    }
}
