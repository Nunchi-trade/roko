//! Graceful process-tree termination with SIGTERM → SIGKILL escalation.
//!
//! The [`kill_tree`] function implements the full shutdown sequence:
//!
//! 1. Close stdin (EOF signal to the child).
//! 2. Wait up to 1200 ms for graceful exit.
//! 3. SIGTERM the entire process group + descendants.
//! 4. Wait up to 800 ms.
//! 5. SIGKILL if still alive.

use std::time::Duration;
use tokio::process::Child;

use super::group::kill_process_group;
use super::registry::register_spawned_descendants;

/// Default grace period before SIGTERM (milliseconds).
pub const GRACE_STDIN_CLOSE_MS: u64 = 1200;

/// Grace period between SIGTERM and SIGKILL (milliseconds).
pub const GRACE_SIGTERM_MS: u64 = 800;

/// Kill a child process and its entire process group using escalation.
///
/// Sequence:
/// 1. Drop/close stdin (triggers EOF in the child).
/// 2. Wait `grace` for the child to exit on its own.
/// 3. SIGTERM the process group + all descendants.
/// 4. Wait 800 ms.
/// 5. SIGKILL the process group + all descendants + `child.kill()`.
///
/// Returns `Ok(())` when the child has exited (or was already dead).
pub async fn kill_tree(child: &mut Child, grace: Duration) -> Result<(), std::io::Error> {
    // Step 1: close stdin to signal EOF.
    // Taking stdin drops the handle, closing the pipe.
    drop(child.stdin.take());

    // Step 2: wait for graceful exit.
    let exited = tokio::time::timeout(grace, child.wait()).await;

    if exited.is_ok() && exited.as_ref().is_ok_and(Result::is_ok) {
        // Child exited gracefully.
        return Ok(());
    }

    // Step 3: SIGTERM the process group.
    #[cfg(unix)]
    {
        let descendants = kill_process_group(child, libc::SIGTERM);
        register_spawned_descendants(&descendants);
    }

    let termed = tokio::time::timeout(Duration::from_millis(GRACE_SIGTERM_MS), child.wait()).await;

    if termed.is_ok() && termed.as_ref().is_ok_and(Result::is_ok) {
        return Ok(());
    }

    // Step 5: SIGKILL escalation.
    #[cfg(unix)]
    {
        let descendants = kill_process_group(child, libc::SIGKILL);
        register_spawned_descendants(&descendants);
    }

    // Also use tokio's built-in kill (sends SIGKILL on unix, TerminateProcess on Windows).
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_millis(GRACE_SIGTERM_MS), child.wait()).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::process::Command;

    #[tokio::test]
    async fn kill_tree_terminates_quick_process() {
        let mut cmd = Command::new("sleep");
        cmd.arg("0.01");
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn sleep");
        let result = kill_tree(&mut child, Duration::from_millis(2000)).await;
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[allow(unsafe_code)]
    #[tokio::test]
    async fn kill_tree_escalates_to_sigkill() {
        use super::super::group::set_process_group;

        // spawn a process that ignores SIGTERM (trap '' TERM; sleep 30)
        let mut cmd = Command::new("bash");
        cmd.args(["-c", "trap '' TERM; sleep 30"]);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
        set_process_group(&mut cmd);

        let mut child = cmd.spawn().expect("failed to spawn bash");
        let pid = child.id();

        let result = kill_tree(&mut child, Duration::from_millis(200)).await;
        assert!(result.is_ok());

        // Verify process is actually dead.
        if let Some(pid) = pid {
            // Give OS a moment to reap.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
            assert!(!alive, "process {pid} should be dead after kill_tree");
        }
    }

    #[tokio::test]
    async fn kill_tree_handles_already_exited() {
        let mut cmd = Command::new("true");
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn true");
        // Wait for it to finish naturally.
        let _ = child.wait().await;
        // kill_tree should handle an already-exited child gracefully.
        let result = kill_tree(&mut child, Duration::from_millis(100)).await;
        assert!(result.is_ok());
    }
}
