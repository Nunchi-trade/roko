//! Atomic persistence for executor snapshots, episodes, and agent PIDs.
//!
//! All writes use write-to-tmp-then-rename for crash safety.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use roko_orchestrator::ExecutorSnapshot;
use serde::Serialize;

/// Paths for all persistent state files.
#[derive(Debug, Clone)]
pub struct PersistPaths {
    /// `.roko/state/executor.json` — executor snapshot.
    pub executor_json: PathBuf,
    /// `.roko/episodes.jsonl` — episode log.
    pub episodes_jsonl: PathBuf,
    /// `.roko/learn/efficiency.jsonl` — efficiency events.
    pub efficiency_jsonl: PathBuf,
    /// `.roko/runtime/agent-pids.json` — live agent PIDs.
    pub agent_pids_json: PathBuf,
    /// `.roko/state/events.json` — event log for replay.
    pub events_json: PathBuf,
}

impl PersistPaths {
    /// Derive all paths from a workdir, creating parent directories as needed.
    pub fn from_workdir(workdir: &Path) -> Result<Self> {
        let roko = workdir.join(".roko");
        let state = roko.join("state");
        let learn = roko.join("learn");
        let runtime = roko.join("runtime");

        for dir in [&state, &learn, &runtime] {
            fs::create_dir_all(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }

        Ok(Self {
            executor_json: state.join("executor.json"),
            episodes_jsonl: roko.join("episodes.jsonl"),
            efficiency_jsonl: learn.join("efficiency.jsonl"),
            agent_pids_json: runtime.join("agent-pids.json"),
            events_json: state.join("events.json"),
        })
    }
}

/// Atomically write `content` to `path` via a `.tmp` sibling.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content)
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} → {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Append a JSON line to a JSONL file.
pub fn append_jsonl(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut line = serde_json::to_string(value)
        .context("serializing JSONL value")?;
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

/// Save the executor snapshot atomically.
pub fn save_executor_snapshot(paths: &PersistPaths, snapshot: &ExecutorSnapshot) -> Result<()> {
    let json = serde_json::to_string_pretty(snapshot)
        .context("serializing executor snapshot")?;
    atomic_write(&paths.executor_json, json.as_bytes())
}

/// Save the set of live agent PIDs.
pub fn save_agent_pids(paths: &PersistPaths, pids: &[u32]) -> Result<()> {
    let json = serde_json::to_string_pretty(&pids)
        .context("serializing agent PIDs")?;
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
