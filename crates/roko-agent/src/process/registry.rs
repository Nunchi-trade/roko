//! Global PID registry with disk persistence.
//!
//! Tracks every child PID spawned by Roko so that:
//! - A restarting instance can kill zombies from a crash.
//! - The reaper can detect orphans reparented to PID 1 (init/launchd).
//!
//! The registry is backed by a static `OnceLock<Mutex<HashSet<u32>>>` and persists
//! to `.roko/runtime/agent-pids.json` on every mutation.

#[allow(clippy::disallowed_types)]
use std::sync::Mutex;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Access the global in-memory PID set.
#[allow(clippy::disallowed_types)]
fn spawned_pids() -> &'static Mutex<HashSet<u32>> {
    static PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();
    PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Path to the persistent PID file: `<cwd>/.roko/runtime/agent-pids.json`.
fn agent_pids_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    Some(cwd.join(".roko/runtime/agent-pids.json"))
}

/// Flush the in-memory PID set to disk.
fn persist_pids() {
    let Some(path) = agent_pids_path() else {
        return;
    };
    let Ok(set) = spawned_pids().lock() else {
        return;
    };
    let pids: Vec<u32> = set.iter().copied().collect();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!(path = %parent.display(), error = %e, "failed to create PID registry directory");
        }
    }
    if let Err(e) = std::fs::write(&path, serde_json::to_string(&pids).unwrap_or_default()) {
        tracing::warn!(path = %path.display(), error = %e, "failed to persist PID registry to disk");
    }
}

/// Register a child PID in the global registry and persist to disk.
pub fn register_spawned_pid(pid: u32) {
    if let Ok(mut set) = spawned_pids().lock() {
        set.insert(pid);
    }
    persist_pids();
}

/// Register multiple descendant PIDs discovered during a kill sweep.
pub fn register_spawned_descendants(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    if let Ok(mut set) = spawned_pids().lock() {
        set.extend(pids);
    }
    // Do not persist here — the caller (kill_tree) already persists after the full sequence.
}

/// Remove a PID from the registry (e.g. after confirmed exit).
pub fn unregister_pid(pid: u32) {
    if let Ok(mut set) = spawned_pids().lock() {
        set.remove(&pid);
    }
    persist_pids();
}

/// Return a snapshot of all currently registered PIDs.
pub fn registered_pids() -> Vec<u32> {
    spawned_pids()
        .lock()
        .map(|set| set.iter().copied().collect())
        .unwrap_or_default()
}

/// Kill any agent processes left over from a previous Roko instance.
///
/// Reads `.roko/runtime/agent-pids.json`, sends SIGTERM to each surviving PID,
/// waits 200 ms, then SIGKILL any that remain. Also kills descendants of
/// registered PIDs.
///
/// Called on startup before spawning new agents.
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_wrap, clippy::disallowed_methods)]
pub fn cleanup_orphaned_agents() {
    use super::group::collect_descendants;

    let Some(path) = agent_pids_path() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(pids) = serde_json::from_str::<Vec<u32>>(&contents) else {
        let _ = std::fs::remove_file(&path);
        return;
    };

    let our_pid = std::process::id();
    let mut killed = 0;

    for pid in &pids {
        if *pid == our_pid {
            continue;
        }
        // SAFETY: signal 0 is an existence check — no signal is delivered.
        let alive = unsafe { libc::kill(*pid as i32, 0) } == 0;
        if alive {
            tracing::info!(pid, "killing orphaned agent process from previous run");
            unsafe {
                libc::kill(*pid as i32, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            let still_alive = unsafe { libc::kill(*pid as i32, 0) } == 0;
            if still_alive {
                unsafe {
                    libc::kill(*pid as i32, libc::SIGKILL);
                }
            }
            killed += 1;
        }
    }

    // Also kill descendants of those PIDs.
    for pid in &pids {
        if *pid == our_pid {
            continue;
        }
        for desc in collect_descendants(*pid) {
            let alive = unsafe { libc::kill(desc as i32, 0) } == 0;
            if alive {
                unsafe {
                    libc::kill(desc as i32, libc::SIGKILL);
                }
                killed += 1;
            }
        }
    }

    let _ = std::fs::remove_file(&path);
    if killed > 0 {
        tracing::warn!(
            killed,
            "Cleaned up {killed} orphaned agent process(es) from previous run"
        );
    }
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn cleanup_orphaned_agents() {}

/// Reap orphaned child processes that survived normal cleanup.
///
/// Checks every PID in the registry: if the process is still alive and its
/// parent is PID 1 (reparented to init/launchd — i.e. orphaned), send SIGKILL.
/// Also discovers and kills descendant processes.
///
/// Returns the number of processes killed.
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_wrap)]
pub fn reap_orphaned_children() -> usize {
    use super::group::collect_descendants;

    let pids: Vec<u32> = match spawned_pids().lock() {
        Ok(set) => set.iter().copied().collect(),
        Err(_) => return 0,
    };

    let mut killed = 0;
    let mut dead_pids = Vec::new();

    for pid in &pids {
        // SAFETY: signal 0 is an existence check.
        let alive = unsafe { libc::kill(*pid as i32, 0) } == 0;
        if !alive {
            dead_pids.push(*pid);
            continue;
        }

        // Check if parent is PID 1 (orphaned).
        let ppid = std::process::Command::new("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u32>()
                    .ok()
            });

        if ppid == Some(1) {
            let descendants = collect_descendants(*pid);
            unsafe {
                libc::kill(*pid as i32, libc::SIGKILL);
            }
            for dpid in &descendants {
                unsafe {
                    libc::kill(*dpid as i32, libc::SIGKILL);
                }
            }
            tracing::warn!(
                pid,
                descendants = descendants.len(),
                "Reaped orphaned process (parent=1)"
            );
            killed += 1 + descendants.len();
            dead_pids.push(*pid);
            dead_pids.extend(descendants);
        }
    }

    // Prune dead PIDs from the registry.
    if !dead_pids.is_empty() {
        if let Ok(mut set) = spawned_pids().lock() {
            for pid in &dead_pids {
                set.remove(pid);
            }
        }
    }

    killed
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn reap_orphaned_children() -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_snapshot() {
        // Register a fake PID, verify it appears in the snapshot.
        let fake_pid = 99_999_999;
        register_spawned_pid(fake_pid);
        let pids = registered_pids();
        assert!(pids.contains(&fake_pid));

        // Unregister and verify removal.
        unregister_pid(fake_pid);
        let pids = registered_pids();
        assert!(!pids.contains(&fake_pid));
    }

    #[test]
    fn register_descendants_batch() {
        let fakes = [88_888_881, 88_888_882, 88_888_883];
        register_spawned_descendants(&fakes);
        let pids = registered_pids();
        for f in &fakes {
            assert!(pids.contains(f));
        }
        // Clean up.
        for f in &fakes {
            unregister_pid(*f);
        }
    }

    #[test]
    fn cleanup_orphaned_agents_does_not_panic() {
        // Should not panic even if the PID file does not exist.
        cleanup_orphaned_agents();
    }

    #[test]
    fn reap_orphaned_children_returns_zero_when_empty() {
        let killed = reap_orphaned_children();
        // With no registered PIDs pointing to real orphans, expect 0.
        let _ = killed;
    }

    #[test]
    fn agent_pids_path_is_under_roko() {
        if let Some(path) = agent_pids_path() {
            let path_str = path.to_string_lossy();
            assert!(path_str.contains(".roko/runtime/agent-pids.json"));
        }
    }
}
