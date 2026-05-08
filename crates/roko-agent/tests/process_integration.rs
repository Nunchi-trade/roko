//! Integration test: spawn `sleep 30` with setpgid, kill_tree, assert child exits.

fn scaled_test_timeout_ms(ms: u64) -> u64 {
    if std::env::var("CI").is_ok_and(|value| value == "true") {
        ms.saturating_mul(10)
    } else {
        ms
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
#[tokio::test]
async fn kill_tree_terminates_sleep_with_setpgid() {
    use std::process::Stdio;
    use std::time::Duration;

    use roko_agent::process::{kill_tree, set_process_group};
    use tokio::process::Command;

    let mut cmd = Command::new("sleep");
    cmd.arg("30");
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    set_process_group(&mut cmd);

    let mut child = cmd.spawn().expect("failed to spawn sleep 30");
    let pid = child.id().expect("child should have a PID");

    // Verify the process is alive.
    let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
    assert!(alive, "child should be alive immediately after spawn");

    // Kill the tree with a short grace period (sleep won't exit on stdin close).
    let result = kill_tree(
        &mut child,
        Duration::from_millis(scaled_test_timeout_ms(200)),
    )
    .await;
    assert!(result.is_ok(), "kill_tree should succeed");

    // Give the OS a moment to fully reap the process.
    tokio::time::sleep(Duration::from_millis(scaled_test_timeout_ms(200))).await;

    // Verify the process is dead.
    let still_alive = unsafe { libc::kill(pid as i32, 0) } == 0;
    assert!(
        !still_alive,
        "child pid {pid} should be dead after kill_tree"
    );
}

#[test]
fn cleanup_orphaned_agents_removes_dead_pids_and_keeps_live_pid() {
    use roko_agent::process::{
        cleanup_orphaned_agents, register_spawned_pid, registered_pids, unregister_pid,
    };

    let fake_pid = 99_999_999;
    let live_pid = std::process::id();

    register_spawned_pid(fake_pid);
    register_spawned_pid(live_pid);
    cleanup_orphaned_agents();

    let pids = registered_pids();
    assert!(
        !pids.contains(&fake_pid),
        "dead process pid should be removed from registry"
    );
    assert!(
        pids.contains(&live_pid),
        "current process pid should remain registered"
    );

    unregister_pid(live_pid);
}
