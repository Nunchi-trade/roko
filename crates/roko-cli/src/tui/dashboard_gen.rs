//! Durable dashboard generation tracking for TUI snapshot consumers.
//!
//! The counter is persisted under `.roko/state/dashboard-gen.json` so
//! sequential process restarts can continue a monotonic generation sequence.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Persistent generation state for one dashboard root.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardGenerationState {
    /// Fingerprint of the latest observed dashboard inputs.
    pub fingerprint: u64,
    /// Monotonic generation counter for the dashboard root.
    pub generation: u64,
}

/// Durable generation counter backed by `.roko/state/dashboard-gen.json`.
#[derive(Debug)]
pub struct DurableDashboardGenerationCounter {
    path: PathBuf,
    state: Mutex<HashMap<PathBuf, DashboardGenerationState>>,
}

impl DurableDashboardGenerationCounter {
    /// Load the counter state from `workdir` or start from an empty map.
    #[must_use]
    pub fn load(workdir: impl AsRef<Path>) -> Self {
        let path = dashboard_generation_state_path(workdir.as_ref());
        let state = read_state(&path).unwrap_or_default();
        Self {
            path,
            state: Mutex::new(state),
        }
    }

    /// Return the next generation for `root`, persisting any changes atomically.
    #[must_use]
    pub fn next(&self, root: impl AsRef<Path>, fingerprint: u64) -> u64 {
        let root = root.as_ref().to_path_buf();
        let mut guard = self
            .state
            .lock()
            .expect("dashboard generation lock poisoned");
        let mut should_persist = false;
        let generation = {
            let entry = guard.entry(root).or_default();

            if entry.fingerprint != fingerprint {
                entry.fingerprint = fingerprint;
                entry.generation = entry.generation.saturating_add(1);
                should_persist = true;
            }

            entry.generation
        };

        if should_persist {
            persist_state(&self.path, &guard);
        }

        generation
    }
}

fn dashboard_generation_state_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("state")
        .join("dashboard-gen.json")
}

fn read_state(path: &Path) -> Option<HashMap<PathBuf, DashboardGenerationState>> {
    let body = fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn persist_state(path: &Path, state: &HashMap<PathBuf, DashboardGenerationState>) {
    let Some(parent) = path.parent() else {
        return;
    };
    let _ = fs::create_dir_all(parent);

    let tmp = atomic_tmp_path(path);
    let Ok(body) = serde_json::to_string_pretty(state) else {
        return;
    };

    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

fn atomic_tmp_path(path: &Path) -> PathBuf {
    let mut tmp_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("{name}.tmp"))
        .unwrap_or_else(|| String::from("dashboard-gen.json.tmp"));

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    tmp_name.push('.');
    tmp_name.push_str(&std::process::id().to_string());
    tmp_name.push('.');
    tmp_name.push_str(&unique.to_string());

    path.with_file_name(tmp_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_workdir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        path.push(format!(
            "roko-dashboard-gen-{}-{}",
            std::process::id(),
            unique
        ));
        path
    }

    #[test]
    fn sequential_instances_observe_a_monotonic_generation_counter() {
        let workdir = unique_test_workdir();
        let _ = fs::remove_dir_all(&workdir);
        fs::create_dir_all(&workdir).expect("create test workdir");

        let counter1 = DurableDashboardGenerationCounter::load(&workdir);
        assert_eq!(counter1.next(&workdir, 11), 1);

        let counter2 = DurableDashboardGenerationCounter::load(&workdir);
        assert_eq!(counter2.next(&workdir, 22), 2);

        let counter3 = DurableDashboardGenerationCounter::load(&workdir);
        assert_eq!(counter3.next(&workdir, 22), 2);

        let state_path = dashboard_generation_state_path(&workdir);
        assert!(state_path.exists());

        let _ = fs::remove_dir_all(&workdir);
    }
}
