#![allow(missing_docs)]

use std::time::Duration;

use roko_runtime::{
    cancel::CancelToken,
    process::{ProcessSupervisor, SpawnConfig},
};
use tokio::{
    process::Command,
    time::{Instant, sleep, timeout},
};

fn scaled_test_timeout_ms(ms: u64) -> u64 {
    if std::env::var("CI").is_ok_and(|value| value == "true") {
        ms.saturating_mul(10)
    } else {
        ms
    }
}

fn scaled_test_duration(ms: u64) -> Duration {
    Duration::from_millis(scaled_test_timeout_ms(ms))
}

fn spawn_config(label: &str, grace_period: Duration) -> SpawnConfig {
    let (program, args) = long_running_command();

    SpawnConfig {
        program,
        args,
        grace_period,
        label: label.to_string(),
        ..Default::default()
    }
}

#[cfg(unix)]
fn long_running_command() -> (String, Vec<String>) {
    (
        "bash".to_string(),
        vec![
            "-c".to_string(),
            "trap '' TERM; while :; do :; done".to_string(),
        ],
    )
}

#[cfg(windows)]
fn long_running_command() -> (String, Vec<String>) {
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "while ($true) { Start-Sleep -Milliseconds 100 }".to_string(),
        ],
    )
}

async fn wait_until_process_stops(pid: u32) {
    timeout(scaled_test_duration(5_000), async {
        loop {
            if !process_is_running(pid).await {
                return;
            }
            sleep(scaled_test_duration(50)).await;
        }
    })
    .await
    .expect("process should stop within the deadline");
}

async fn process_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let output = Command::new("sh")
            .args(["-c", &format!("ps -o stat= -p {pid}")])
            .output()
            .await
            .expect("ps should be available");

        if !output.status.success() {
            return false;
        }

        let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
        !state.is_empty() && !state.starts_with('Z')
    }

    #[cfg(windows)]
    {
        let output = Command::new("cmd")
            .args(["/C", &format!(r#"tasklist /FI "PID eq {pid}" /NH"#)])
            .output()
            .await
            .expect("tasklist should be available");

        if !output.status.success() {
            return false;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.contains(&pid.to_string())
    }
}

#[tokio::test(flavor = "current_thread")]
#[cfg(unix)]
async fn sigterm_then_sigkill_escalates_when_the_child_ignores_term() {
    let cancel = CancelToken::new();
    let supervisor = ProcessSupervisor::new(cancel);

    let id = supervisor
        .spawn(spawn_config(
            "sigterm-then-sigkill",
            scaled_test_duration(50),
        ))
        .await
        .expect("spawn should succeed");

    let pid = supervisor
        .active_pids()
        .await
        .into_iter()
        .next()
        .map(|(pid, _)| pid)
        .expect("child pid should be tracked");

    sleep(scaled_test_duration(100)).await;
    let started = Instant::now();

    let outcome = supervisor
        .shutdown(id)
        .await
        .expect("shutdown should return an outcome");

    assert!(
        outcome.was_killed,
        "ignored SIGTERM should force escalation"
    );
    assert!(
        started.elapsed() >= scaled_test_duration(50),
        "shutdown should honor the grace period before escalating"
    );
    wait_until_process_stops(pid).await;
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_the_supervisor_kills_live_children() {
    let cancel = CancelToken::new();
    let supervisor = ProcessSupervisor::new(cancel);

    supervisor
        .spawn(spawn_config(
            "drop-kills-live-children",
            scaled_test_duration(50),
        ))
        .await
        .expect("spawn should succeed");

    let pid = supervisor
        .active_pids()
        .await
        .into_iter()
        .next()
        .map(|(pid, _)| pid)
        .expect("child pid should be tracked");

    drop(supervisor);

    wait_until_process_stops(pid).await;
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_token_triggers_shutdown() {
    let root = CancelToken::new();
    let cancellation = CancelToken::new();
    let supervisor = ProcessSupervisor::new(root);
    let mut config = spawn_config("per-process-cancellation", scaled_test_duration(50));
    config.cancellation = Some(cancellation.clone());

    supervisor
        .spawn(config)
        .await
        .expect("spawn should succeed");

    let pid = supervisor
        .active_pids()
        .await
        .into_iter()
        .next()
        .map(|(pid, _)| pid)
        .expect("child pid should be tracked");

    cancellation.cancel();

    timeout(scaled_test_duration(5_000), async {
        loop {
            if supervisor.count().await == 0 {
                break;
            }
            sleep(scaled_test_duration(25)).await;
        }
    })
    .await
    .expect("cancellation should remove the child from supervisor tracking");

    assert!(cancellation.is_cancelled());
    wait_until_process_stops(pid).await;
}
