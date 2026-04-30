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
    /// Forensic fingerprints of every task definition known when this
    /// snapshot was written. Read by [`super::resume::prepare_resume`]
    /// to detect drift between runs.
    #[serde(default)]
    pub fingerprints: Vec<TaskDefFingerprint>,
    /// CascadeRouter snapshot JSON captured at save time.
    ///
    /// `None` for old snapshots or when no router is configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade_router_json: Option<String>,
}

/// Forensic fingerprint of a task definition used for strict resume validation.
///
/// Hash inputs are deterministic and span the fields a plan author can mutate
/// between runs (id, title, role, tier, dependencies, verify steps, gate
/// budgets). Mismatch on resume is a hard failure: see
/// [`super::resume::ResumeReport::drifted_tasks`] for the re-queue signal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefFingerprint {
    /// Plan identifier.
    pub plan_id: String,
    /// Task identifier.
    pub task_id: String,
    /// FNV-1a hash (hex) of the canonical task definition payload.
    pub fingerprint: String,
}

/// Per-rung gate threshold statistics persisted in `.roko/learn/gate-thresholds.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateThresholdStats {
    #[serde(default)]
    pub pass_count: u64,
    #[serde(default, alias = "total_observations")]
    pub total_count: u64,
    #[serde(default = "GateThresholdStats::default_ema_pass_rate")]
    pub ema_pass_rate: f64,
}

impl GateThresholdStats {
    const fn default_ema_pass_rate() -> f64 {
        0.5
    }
}

impl Default for GateThresholdStats {
    fn default() -> Self {
        Self {
            pass_count: 0,
            total_count: 0,
            ema_pass_rate: Self::default_ema_pass_rate(),
        }
    }
}

/// Persisted adaptive gate thresholds loaded at runner startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GateThresholds {
    #[serde(default)]
    pub rungs: HashMap<u32, GateThresholdStats>,
}

impl GateThresholds {
    fn load(path: &Path) -> Result<Self> {
        let file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        serde_json::from_reader(file).with_context(|| format!("parsing {}", path.display()))
    }

    pub(crate) fn observe(&mut self, rung: u32, passed: bool) {
        let stats = self.rungs.entry(rung).or_default();
        let value = if passed { 1.0 } else { 0.0 };

        if stats.total_count == 0 {
            stats.ema_pass_rate = value;
        } else {
            stats.ema_pass_rate = 0.1_f64.mul_add(value, 0.9 * stats.ema_pass_rate);
        }

        stats.total_count += 1;
        if passed {
            stats.pass_count += 1;
        }
    }

    pub(crate) fn suggested_max_retries(&self, rung: u32) -> u32 {
        const MIN_RETRIES: u32 = 1;
        const MAX_RETRIES: u32 = 5;

        let Some(stats) = self.rungs.get(&rung) else {
            return 3;
        };

        if stats.total_count < 5 {
            return 3;
        }

        let max_f = f64::from(MAX_RETRIES);
        let range_f = f64::from(MAX_RETRIES - MIN_RETRIES);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let retries = stats.ema_pass_rate.mul_add(-range_f, max_f).round() as u32;

        retries.clamp(MIN_RETRIES, MAX_RETRIES)
    }

    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let json =
            serde_json::to_string_pretty(self).context("serializing adaptive gate thresholds")?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        atomic_write(path, json.as_bytes())
    }
}

/// Load persisted gate thresholds from disk.
pub fn load_gate_thresholds(paths: &PersistPaths) -> Result<GateThresholds> {
    GateThresholds::load(&paths.gate_thresholds_json)
}

/// Atomically write `content` to `path` via a `.tmp` sibling.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(content)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.sync_data()
            .with_context(|| format!("syncing {}", tmp.display()))?;
    }
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
    file.sync_data()
        .with_context(|| format!("syncing {}", path.display()))?;
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

/// Atomically write the runner-owned [`RunStateSnapshot`].
pub fn save_run_state(paths: &PersistPaths, snapshot: &RunStateSnapshot) -> Result<()> {
    let json = serde_json::to_string_pretty(snapshot).context("serializing run state")?;
    atomic_write(&paths.run_state_json, json.as_bytes())
}

/// Load the runner-owned [`RunStateSnapshot`] if it exists. Returns
/// `Ok(None)` when the file is missing; `Err` only on malformed payload
/// or filesystem errors so callers can distinguish "fresh run" from
/// "broken state".
pub fn load_run_state(paths: &PersistPaths) -> Result<Option<RunStateSnapshot>> {
    match fs::read_to_string(&paths.run_state_json) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .with_context(|| format!("parsing {}", paths.run_state_json.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading {}", paths.run_state_json.display())),
    }
}

/// Outcome of a JSONL recovery scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonlRecovery {
    /// File is fully consistent — every line parsed.
    Clean { lines: usize },
    /// File ended with an incomplete line; recovered by truncating after
    /// the last newline. `valid_lines` is what survives.
    TruncatedTrailing {
        valid_lines: usize,
        truncated_bytes: u64,
    },
    /// File ended with one or more malformed JSON lines that did parse as
    /// strings (have terminating `\n`) but failed serde validation.
    /// Recovered by truncating to the last valid line.
    DroppedInvalid {
        valid_lines: usize,
        dropped_lines: usize,
    },
}

/// Inspect a JSONL file for partial-append corruption and recover by
/// truncating at the last successfully-parsed line.
///
/// Strategy: read the file as bytes, try to parse each line through
/// `validator`. If any tail line fails (or the file ends mid-line),
/// the file is rewritten atomically with everything up through the last
/// validated line.
pub fn recover_jsonl<T, F>(path: &Path, validator: F) -> Result<JsonlRecovery>
where
    T: for<'de> Deserialize<'de>,
    F: Fn(&str) -> std::result::Result<T, serde_json::Error>,
{
    let original = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JsonlRecovery::Clean { lines: 0 });
        }
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", path.display()));
        }
    };
    if original.is_empty() {
        return Ok(JsonlRecovery::Clean { lines: 0 });
    }

    let text = match std::str::from_utf8(&original) {
        Ok(text) => text,
        Err(_) => {
            // Non-utf8 — refuse to silently destroy it.
            anyhow::bail!("{} is not valid UTF-8; refusing to recover", path.display());
        }
    };

    let trailing_partial = !text.ends_with('\n');
    let mut last_good_byte = 0_u64;
    let mut valid_lines = 0_usize;
    let mut dropped_lines = 0_usize;

    let mut byte_offset = 0_u64;
    for raw_line in text.split_inclusive('\n') {
        let trimmed = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let is_complete = raw_line.ends_with('\n');
        if !is_complete {
            // Trailing partial line — stop here without counting it as
            // dropped.
            break;
        }
        if trimmed.trim().is_empty() {
            byte_offset += raw_line.len() as u64;
            last_good_byte = byte_offset;
            continue;
        }
        match validator(trimmed) {
            Ok(_) => {
                byte_offset += raw_line.len() as u64;
                last_good_byte = byte_offset;
                valid_lines += 1;
            }
            Err(_) => {
                dropped_lines += 1;
                // Stop on first malformed entry — don't trust the tail.
                break;
            }
        }
    }

    let truncated_bytes = original.len() as u64 - last_good_byte;
    if truncated_bytes == 0 && !trailing_partial && dropped_lines == 0 {
        return Ok(JsonlRecovery::Clean { lines: valid_lines });
    }

    // Truncate to the last validated line.
    if last_good_byte == 0 {
        // Nothing valid — leave file alone, surface as dropped.
        if dropped_lines > 0 {
            return Ok(JsonlRecovery::DroppedInvalid {
                valid_lines: 0,
                dropped_lines,
            });
        }
        return Ok(JsonlRecovery::TruncatedTrailing {
            valid_lines: 0,
            truncated_bytes,
        });
    }

    let kept = &original[..last_good_byte as usize];
    let tmp = path.with_extension("recover.tmp");
    fs::write(&tmp, kept).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;

    if dropped_lines > 0 {
        Ok(JsonlRecovery::DroppedInvalid {
            valid_lines,
            dropped_lines,
        })
    } else {
        Ok(JsonlRecovery::TruncatedTrailing {
            valid_lines,
            truncated_bytes,
        })
    }
}

impl TaskDefFingerprint {
    /// Compute a forensic fingerprint for `task` in `plan_id`.
    ///
    /// The hash spans the fields a plan author can change between runs;
    /// downstream resume validation rejects mismatches as a hard
    /// failure.
    #[must_use]
    pub fn from_task(task: &crate::task_parser::TaskDef, plan_id: &str) -> Self {
        let canonical = canonical_task_payload(task);
        Self {
            plan_id: plan_id.to_string(),
            task_id: task.id.clone(),
            fingerprint: fnv1a_hex(&canonical),
        }
    }
}

fn canonical_task_payload(task: &crate::task_parser::TaskDef) -> String {
    let depends_on = task.depends_on.join(",");
    let depends_on_plan = task.depends_on_plan.join(",");
    let verify = task
        .verify
        .iter()
        .map(|step| format!("{}:{}:{}", step.phase, step.command, step.timeout_ms))
        .collect::<Vec<_>>()
        .join("|");
    let acceptance = task.acceptance.join("|");
    let role = task.role.clone().unwrap_or_default();
    let domain = task
        .domain
        .as_ref()
        .map(|d| d.label().to_string())
        .unwrap_or_default();
    let max_loc = task.max_loc.map(|n| n.to_string()).unwrap_or_default();
    format!(
        "id={};title={};role={};tier={};domain={};depends_on={};depends_on_plan={};verify={};acceptance={};max_loc={};max_retries={};timeout_secs={}",
        task.id,
        task.title,
        role,
        task.tier,
        domain,
        depends_on,
        depends_on_plan,
        verify,
        acceptance,
        max_loc,
        task.max_retries,
        task.timeout_secs,
    )
}

fn fnv1a_hex(payload: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = FNV_OFFSET;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
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

    #[test]
    fn load_run_state_defaults_missing_cascade_router_json() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = PersistPaths::from_workdir(tmp.path()).unwrap();
        let payload = serde_json::json!({
            "schema_version": RUN_STATE_SCHEMA_VERSION,
            "run_id": "run-1",
            "started_at_ms": 1,
            "timestamp_ms": 2,
            "tasks_total": 3,
            "tasks_completed": 1,
            "tasks_failed": 0,
            "total_tokens_in": 10,
            "total_tokens_out": 20,
            "total_cost_usd": 0.25,
            "total_agent_calls": 2,
            "plan_costs": {},
            "completed_tasks": {},
            "snapshot_fail_streak": 0,
            "fingerprints": []
        });
        fs::write(
            &paths.run_state_json,
            serde_json::to_string(&payload).unwrap(),
        )
        .unwrap();

        let snapshot = load_run_state(&paths).unwrap().unwrap();
        assert!(snapshot.cascade_router_json.is_none());
    }
}
