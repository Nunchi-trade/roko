#![allow(missing_docs)]

use std::time::Duration;

use roko_runtime::{
    cancel::CancelToken,
    process::{
        ProcessResumeError, ProcessResumePolicy, ProcessSessionConfig, ProcessSessionLedger,
        ProcessSessionState, ProcessSupervisor, SpawnConfig,
    },
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

fn session_config(path: std::path::PathBuf, suffix: &str) -> ProcessSessionConfig {
    ProcessSessionConfig {
        session_id: format!("session-{suffix}"),
        invocation_id: format!("invocation-{suffix}"),
        backend_id: "test-backend".to_string(),
        task_id: Some(format!("task-{suffix}")),
        reuse_policy_id: Some("test-policy".to_string()),
        resumable: true,
        timeout_ms: Some(50),
        ledger_path: path,
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

#[tokio::test(flavor = "current_thread")]
async fn wait_timeout_records_resumable_timeout_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ledger_path = tmp.path().join("process-sessions.json");
    let root = CancelToken::new();
    let supervisor = ProcessSupervisor::new(root);
    let mut config = spawn_config("timeout-ledger", scaled_test_duration(50));
    config.session = Some(session_config(ledger_path.clone(), "timeout"));

    supervisor
        .spawn(config)
        .await
        .expect("spawn should succeed");
    let completed = supervisor.wait_all(scaled_test_duration(10)).await;
    assert!(completed.is_empty(), "long-running process should time out");

    let ledger = ProcessSessionLedger::load(&ledger_path).expect("load ledger");
    let record = ledger
        .latest_for_session("session-timeout")
        .expect("session record");
    assert_eq!(record.state, ProcessSessionState::TimedOut);
    assert_eq!(record.backend_id, "test-backend");
    assert!(ledger.validate_resume("session-timeout").is_ok());

    let killed = supervisor.kill_all().await;
    assert_eq!(killed.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_records_cancelled_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ledger_path = tmp.path().join("process-sessions.json");
    let root = CancelToken::new();
    let cancellation = CancelToken::new();
    let supervisor = ProcessSupervisor::new(root);
    let mut config = spawn_config("cancel-ledger", scaled_test_duration(50));
    config.session = Some(session_config(ledger_path.clone(), "cancel"));
    config.cancellation = Some(cancellation.clone());

    supervisor
        .spawn(config)
        .await
        .expect("spawn should succeed");
    cancellation.cancel();

    timeout(scaled_test_duration(5_000), async {
        loop {
            let ledger = ProcessSessionLedger::load(&ledger_path).expect("load ledger");
            if ledger
                .latest_for_session("session-cancel")
                .is_some_and(|record| record.state == ProcessSessionState::Cancelled)
            {
                return;
            }
            sleep(scaled_test_duration(25)).await;
        }
    })
    .await
    .expect("cancelled state should be persisted");

    let ledger = ProcessSessionLedger::load(&ledger_path).expect("load ledger");
    assert!(ledger.validate_resume("session-cancel").is_ok());
}

#[test]
fn resume_validation_rejects_terminal_success() {
    let mut ledger = ProcessSessionLedger::default();
    ledger.upsert(roko_runtime::process::ProcessSessionRecord {
        session_id: "session-done".to_string(),
        invocation_id: "invocation-done".to_string(),
        backend_id: "test-backend".to_string(),
        task_id: None,
        reuse_policy_id: None,
        resumable: true,
        process_id: roko_runtime::process::ProcessId(1),
        os_pid: None,
        label: "done".to_string(),
        program: "true".to_string(),
        args: Vec::new(),
        started_at_ms: 1,
        updated_at_ms: 2,
        ended_at_ms: Some(2),
        timeout_ms: None,
        state: ProcessSessionState::Succeeded,
        reason: None,
    });

    assert_eq!(
        ledger.validate_resume("session-done"),
        Err(ProcessResumeError::TerminalState(
            ProcessSessionState::Succeeded
        ))
    );
}

#[test]
fn resume_validation_rejects_incompatible_backend_and_stale_state() {
    let mut ledger = ProcessSessionLedger::default();
    ledger.upsert(roko_runtime::process::ProcessSessionRecord {
        session_id: "session-live".to_string(),
        invocation_id: "invocation-live".to_string(),
        backend_id: "backend-a".to_string(),
        task_id: Some("task-a".to_string()),
        reuse_policy_id: None,
        resumable: true,
        process_id: roko_runtime::process::ProcessId(7),
        os_pid: None,
        label: "live".to_string(),
        program: "sleep".to_string(),
        args: Vec::new(),
        started_at_ms: 10,
        updated_at_ms: 100,
        ended_at_ms: None,
        timeout_ms: Some(1_000),
        state: ProcessSessionState::Started,
        reason: None,
    });

    let backend_policy = ProcessResumePolicy {
        expected_backend_id: Some("backend-b".to_string()),
        ..Default::default()
    };
    assert_eq!(
        ledger.validate_resume_with_policy("session-live", &backend_policy),
        Err(ProcessResumeError::BackendMismatch {
            expected: "backend-b".to_string(),
            actual: "backend-a".to_string(),
        })
    );

    let stale_policy = ProcessResumePolicy {
        max_staleness_ms: Some(50),
        now_ms: Some(200),
        ..Default::default()
    };
    assert_eq!(
        ledger.validate_resume_with_policy("session-live", &stale_policy),
        Err(ProcessResumeError::Stale {
            max_staleness_ms: 50,
            age_ms: 100,
        })
    );
}

#[test]
fn process_session_summary_counts_terminal_resumable_and_stale_records() {
    let mut ledger = ProcessSessionLedger::default();
    for (idx, state) in [
        ProcessSessionState::Started,
        ProcessSessionState::TimedOut,
        ProcessSessionState::Cancelled,
        ProcessSessionState::Failed,
        ProcessSessionState::Succeeded,
    ]
    .into_iter()
    .enumerate()
    {
        ledger.upsert(roko_runtime::process::ProcessSessionRecord {
            session_id: format!("session-{idx}"),
            invocation_id: format!("invocation-{idx}"),
            backend_id: "backend".to_string(),
            task_id: None,
            reuse_policy_id: None,
            resumable: true,
            process_id: roko_runtime::process::ProcessId(idx as u64),
            os_pid: None,
            label: format!("record-{idx}"),
            program: "test".to_string(),
            args: Vec::new(),
            started_at_ms: 1,
            updated_at_ms: if state == ProcessSessionState::Started {
                10
            } else {
                100
            },
            ended_at_ms: state.is_terminal().then_some(100),
            timeout_ms: None,
            state,
            reason: None,
        });
    }

    let summary = ledger.state_summary(Some(50), 100);
    assert_eq!(summary.total, 5);
    assert_eq!(summary.started, 1);
    assert_eq!(summary.timed_out, 1);
    assert_eq!(summary.cancelled, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.succeeded, 1);
    assert_eq!(summary.resumable, 3);
    assert_eq!(summary.stale, 1);
    assert_eq!(summary.latest_updated_at_ms, Some(100));
}
