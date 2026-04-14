//! Process-group primitives: set process group, collect descendants, signal groups.
//!
//! On Unix, child agents are placed in their own process group via `setpgid(0,0)`
//! so that an entire agent tree (agent + MCP servers + subshells) can be signaled
//! atomically with a single negative-PID `kill()`.
//!
//! Non-Unix targets provide no-op stubs.

use tokio::process::{Child, Command};

/// Configure a [`Command`] to spawn its child in a new process group.
///
/// On Unix this installs a `pre_exec` hook calling `libc::setpgid(0, 0)`.
/// On other platforms this is a no-op.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn set_process_group(cmd: &mut Command) {
    // SAFETY: `setpgid(0, 0)` is async-signal-safe and only touches the child's
    // process group before `exec`. No shared state is accessed.
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn set_process_group(_cmd: &mut Command) {}

/// Breadth-first collection of all descendant PIDs rooted at `root_pid`.
///
/// Uses `pgrep -P <pid>` at each level. Returns PIDs in breadth-first order
/// (immediate children first, then grandchildren, etc.). The walk is capped at
/// depth 8 to prevent infinite loops from circular reparenting.
#[cfg(unix)]
pub fn collect_descendants(root_pid: u32) -> Vec<u32> {
    let mut descendants = Vec::new();
    let mut queue = vec![root_pid];
    let mut depth = 0;

    while !queue.is_empty() && depth < 8 {
        depth += 1;
        let mut next_queue = Vec::new();

        for parent in &queue {
            if let Ok(output) = std::process::Command::new("pgrep")
                .args(["-P", &parent.to_string()])
                .output()
            {
                if output.status.success() {
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        if let Ok(pid) = line.trim().parse::<u32>() {
                            descendants.push(pid);
                            next_queue.push(pid);
                        }
                    }
                }
            }
        }
        queue = next_queue;
    }
    descendants
}

/// No-op on non-Unix platforms — returns an empty vec.
#[cfg(not(unix))]
pub fn collect_descendants(_root_pid: u32) -> Vec<u32> {
    Vec::new()
}

/// Engram the entire process group rooted at `child`, plus any descendants that
/// escaped into their own process groups.
///
/// Strategy:
/// 1. **Snapshot** all descendant PIDs *before* signaling (the tree is intact while
///    the parent is alive).
/// 2. **Engram the process group** (negative PID) — covers children that stayed in-group.
/// 3. **Engram each descendant individually** — covers processes that called
///    `setpgid(0,0)` and left the group (e.g. codex zsh shells).
///
/// Returns the list of descendant PIDs that were signaled (caller may want to
/// register them for the reaper).
#[cfg(unix)]
#[allow(unsafe_code, clippy::cast_possible_wrap, clippy::cast_sign_loss)]
pub fn kill_process_group(child: &Child, signal: i32) -> Vec<u32> {
    let Some(pid) = child.id() else {
        return Vec::new();
    };

    // Step 1: snapshot descendants before killing anything.
    let descendants = collect_descendants(pid);

    // Step 2: signal the process group.
    // SAFETY: `libc::kill` with negative PID is standard POSIX signal delivery.
    unsafe {
        libc::kill(-(pid as i32), signal);
    }

    // Step 3: signal each descendant individually.
    for dpid in &descendants {
        unsafe {
            libc::kill(*dpid as i32, signal);
        }
    }

    if !descendants.is_empty() {
        tracing::debug!(
            pid,
            descendant_count = descendants.len(),
            "kill_process_group: signaled {} descendants",
            descendants.len()
        );
    }

    descendants
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn kill_process_group(_child: &Child, _signal: i32) -> Vec<u32> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_descendants_no_children() {
        // PID 1 (init/launchd) might have children, but an invalid PID should not.
        let desc = collect_descendants(u32::MAX - 1);
        // We cannot assert it is empty on all systems, but it should not panic.
        let _ = desc;
    }

    #[test]
    fn set_process_group_does_not_panic() {
        let mut cmd = Command::new("echo");
        set_process_group(&mut cmd);
    }

    #[cfg(unix)]
    #[test]
    fn kill_process_group_with_no_child_pid() {
        // A child that was already waited on has id() == None.
        // We simulate via a fresh command that we never spawn.
        // Just verify the function handles None gracefully.
        let cmd = Command::new("true");
        // We cannot get a Child without spawning, so we test the early-return path
        // indirectly through kill_tree tests.
        let _ = cmd;
    }
}
