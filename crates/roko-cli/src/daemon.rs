//! Headless daemon mode (`--headless`).
//!
//! Runs roko as a background service that listens on a Unix-domain socket
//! for commands (status queries, signal injection, prompt dispatch). This
//! mode is used by IDE integrations and CI pipelines that want to keep a
//! long-lived agent session warm.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

use crate::load_layered;
use crate::serve_runtime::RokoCliRuntime;
use roko_core::config::load_config;
use roko_serve::{self, deploy, dispatch, feedback, fswatcher, scheduler, state::AppState};

/// State of the headless daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Daemon is initializing.
    Starting,
    /// Daemon is ready and accepting commands.
    Running,
    /// Daemon is shutting down.
    Stopping,
    /// Daemon has stopped.
    Stopped,
}

impl std::fmt::Display for DaemonState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Stopping => write!(f, "stopping"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// Configuration for the headless daemon.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// Directory for the Unix socket and PID file.
    pub runtime_dir: PathBuf,
    /// Session ID for this daemon instance.
    pub session_id: String,
}

impl DaemonConfig {
    /// Compute the socket path for this daemon session.
    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.runtime_dir
            .join(format!("roko-{}.sock", self.session_id))
    }

    /// Compute the PID file path for this daemon session.
    #[must_use]
    pub fn pid_path(&self) -> PathBuf {
        self.runtime_dir
            .join(format!("roko-{}.pid", self.session_id))
    }
}

/// Persistent daemon metadata stored in `.roko/daemon.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// Process identifier.
    pub pid: u32,
    /// HTTP listen port.
    pub port: u16,
    /// Stable session identifier for this daemon instance.
    pub session_id: String,
    /// When the daemon started.
    pub started_at: DateTime<Utc>,
    /// Current daemon state.
    pub state: DaemonState,
}

/// Headless daemon context.
#[derive(Debug)]
pub struct DaemonMode {
    /// Daemon configuration.
    pub config: DaemonConfig,
    /// Current state of the daemon.
    pub state: DaemonState,
    /// Number of commands processed.
    pub commands_processed: usize,
}

impl DaemonMode {
    /// Create a new daemon instance.
    #[must_use]
    pub const fn new(config: DaemonConfig) -> Self {
        Self {
            config,
            state: DaemonState::Starting,
            commands_processed: 0,
        }
    }

    /// Create a daemon with default runtime directory under the given workdir.
    #[must_use]
    pub fn with_workdir(workdir: &Path, session_id: String) -> Self {
        let config = DaemonConfig {
            runtime_dir: workdir.join(".roko").join("run"),
            session_id,
        };
        Self::new(config)
    }

    /// Transition the daemon to the running state.
    pub const fn start(&mut self) {
        self.state = DaemonState::Running;
    }

    /// Request the daemon to stop.
    pub const fn stop(&mut self) {
        self.state = DaemonState::Stopping;
    }

    /// Mark the daemon as fully stopped.
    pub const fn mark_stopped(&mut self) {
        self.state = DaemonState::Stopped;
    }

    /// Record that a command was processed.
    pub const fn record_command(&mut self) {
        self.commands_processed += 1;
    }

    /// Check if the daemon is in a state that accepts commands.
    #[must_use]
    pub fn is_accepting(&self) -> bool {
        self.state == DaemonState::Running
    }

    /// Generate a status summary for the daemon.
    #[must_use]
    pub fn status_summary(&self) -> DaemonStatus {
        DaemonStatus {
            session_id: self.config.session_id.clone(),
            state: self.state.clone(),
            socket_path: self.config.socket_path(),
            commands_processed: self.commands_processed,
            pid: std::process::id(),
        }
    }
}

/// Status snapshot of a running daemon, returned by `roko status`.
#[derive(Debug, Clone)]
pub struct DaemonStatus {
    /// Session ID.
    pub session_id: String,
    /// Current state.
    pub state: DaemonState,
    /// Path to the Unix socket.
    pub socket_path: PathBuf,
    /// Number of commands processed.
    pub commands_processed: usize,
    /// Process ID.
    pub pid: u32,
}

impl std::fmt::Display for DaemonStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "session : {}", self.session_id)?;
        writeln!(f, "state   : {}", self.state)?;
        writeln!(f, "socket  : {}", self.socket_path.display())?;
        writeln!(f, "commands: {}", self.commands_processed)?;
        write!(f, "pid     : {}", self.pid)
    }
}

/// Start the daemon in foreground or background mode.
pub async fn daemon_start(foreground: bool, port: u16) -> Result<()> {
    let workdir = std::env::current_dir().context("resolve current working directory")?;
    ensure_runtime_dirs(&workdir)?;

    if let Some(info) = read_daemon_info(&workdir)? {
        if pid_is_alive(info.pid)? {
            return Err(anyhow!(
                "daemon already running (pid {}, port {})",
                info.pid,
                info.port
            ));
        }

        cleanup_stale_runtime_files(&workdir);
    }

    if !foreground {
        spawn_detached_child(&workdir, port)?;
        return Ok(());
    }

    let core_config = load_config(&workdir)?;
    let cli_config = load_layered(&workdir)?.config;
    let runtime = RokoCliRuntime::new(cli_config).into_arc();
    let deploy_backend = Arc::from(deploy::create_backend("manual", None, None, None)?);
    let state = Arc::new(AppState::new(
        workdir.clone(),
        runtime,
        core_config,
        deploy_backend,
    ));

    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        session_id: format!("daemon-{}", uuid::Uuid::new_v4()),
        started_at: Utc::now(),
        state: DaemonState::Running,
    };
    write_daemon_info(&workdir, &info)?;

    let _scheduler = scheduler::start_scheduler(Arc::clone(&state));
    let _watchers = fswatcher::start_watchers(Arc::clone(&state));
    let _dispatch = dispatch::start_dispatch_loop(Arc::clone(&state));
    let _feedback = feedback::start_feedback_loop(Arc::clone(&state));
    let ipc_server = tokio::spawn(run_ipc_server(
        Arc::clone(&state),
        daemon_socket_path(&workdir),
        daemon_json_path(&workdir),
    ));

    let server = tokio::spawn(roko_serve::run_server_with_state(
        Arc::clone(&state),
        "0.0.0.0",
        port,
    ));

    let run_result: Result<()> = tokio::select! {
        result = server => {
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => {
                    state.shutdown().await;
                    Err(err)
                }
                Err(join_err) => {
                    state.shutdown().await;
                    Err(join_err.into())
                }
            }
        }
        result = ipc_server => {
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(err)) => {
                    state.shutdown().await;
                    Err(err)
                }
                Err(join_err) => {
                    state.shutdown().await;
                    Err(join_err.into())
                }
            }
        }
        _ = wait_for_shutdown_signal() => {
            state.shutdown().await;
            Ok(())
        }
    };

    let mut stopped = info.clone();
    stopped.state = DaemonState::Stopped;
    let _ = write_daemon_info(&workdir, &stopped);
    let _ = fs::remove_file(daemon_socket_path(&workdir));
    let _ = fs::remove_file(daemon_pid_path(&workdir));

    run_result
}

fn ensure_runtime_dirs(workdir: &Path) -> Result<()> {
    fs::create_dir_all(daemon_root_dir(workdir))
        .with_context(|| format!("create {}", daemon_root_dir(workdir).display()))?;
    fs::create_dir_all(daemon_logs_dir(workdir))
        .with_context(|| format!("create {}", daemon_logs_dir(workdir).display()))?;
    Ok(())
}

fn daemon_root_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko")
}

fn daemon_logs_dir(workdir: &Path) -> PathBuf {
    daemon_root_dir(workdir).join("logs")
}

fn daemon_json_path(workdir: &Path) -> PathBuf {
    daemon_root_dir(workdir).join("daemon.json")
}

fn daemon_pid_path(workdir: &Path) -> PathBuf {
    daemon_root_dir(workdir).join("daemon.pid")
}

fn daemon_socket_path(workdir: &Path) -> PathBuf {
    daemon_root_dir(workdir).join("daemon.sock")
}

fn log_path(workdir: &Path, name: &str) -> PathBuf {
    daemon_logs_dir(workdir).join(name)
}

fn spawn_detached_child(workdir: &Path, port: u16) -> Result<()> {
    ensure_runtime_dirs(workdir)?;
    let exe = std::env::current_exe().context("resolve current executable")?;
    let port = port.to_string();
    let stdout = File::create(log_path(workdir, "daemon.log"))
        .with_context(|| format!("open {}", log_path(workdir, "daemon.log").display()))?;
    let stderr = File::create(log_path(workdir, "daemon.err"))
        .with_context(|| format!("open {}", log_path(workdir, "daemon.err").display()))?;

    let child = Command::new(exe)
        .current_dir(workdir)
        .args(["daemon", "start", "--foreground", "--port", &port])
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .context("spawn detached daemon")?;

    println!("daemon started (pid {})", child.id());
    Ok(())
}

fn read_daemon_info(workdir: &Path) -> Result<Option<DaemonInfo>> {
    let path = daemon_json_path(workdir);
    if !path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let info = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(info))
}

fn write_daemon_info(workdir: &Path, info: &DaemonInfo) -> Result<()> {
    let json_path = daemon_json_path(workdir);
    let pid_path = daemon_pid_path(workdir);
    if let Some(parent) = json_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(info).context("serialize daemon metadata")?;
    fs::write(&json_path, text).with_context(|| format!("write {}", json_path.display()))?;
    fs::write(&pid_path, info.pid.to_string()).with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

fn cleanup_stale_runtime_files(workdir: &Path) {
    let json_path = daemon_json_path(workdir);
    let pid_path = daemon_pid_path(workdir);
    let socket_path = daemon_socket_path(workdir);
    let _ = fs::remove_file(json_path);
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(socket_path);
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> Result<bool> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    Ok(system.process(Pid::from_u32(pid)).is_some())
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> Result<bool> {
    Ok(false)
}

async fn run_ipc_server(
    state: Arc<AppState>,
    socket_path: PathBuf,
    daemon_json_path: PathBuf,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create {}", parent.display()))?;
    }
    if socket_path.exists() {
        let _ = tokio::fs::remove_file(&socket_path).await;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;

    loop {
        tokio::select! {
            _ = state.cancel.cancelled() => break,
            accept = listener.accept() => {
                let (stream, _) = match accept {
                    Ok(pair) => pair,
                    Err(err) => {
                        if state.cancel.is_cancelled() {
                            break;
                        }
                        return Err(err).context("accept daemon IPC connection");
                    }
                };

                let state = Arc::clone(&state);
                let daemon_json_path = daemon_json_path.clone();
                tokio::spawn(async move {
                    let _ = handle_ipc_connection(stream, state, daemon_json_path).await;
                });
            }
        }
    }

    let _ = tokio::fs::remove_file(&socket_path).await;
    Ok(())
}

async fn handle_ipc_connection(
    mut stream: UnixStream,
    state: Arc<AppState>,
    daemon_json_path: PathBuf,
) -> Result<()> {
    let mut buf = vec![0u8; 1024];
    let n = stream
        .read(&mut buf)
        .await
        .context("read daemon IPC request")?;
    let request = String::from_utf8_lossy(&buf[..n]).trim().to_ascii_lowercase();

    if request == "shutdown" {
        let state_for_shutdown = Arc::clone(&state);
        tokio::spawn(async move {
            state_for_shutdown.shutdown().await;
        });
        stream.write_all(b"{\"ok\":true,\"command\":\"shutdown\"}\n").await?;
        return Ok(());
    }

    let response = match read_daemon_info_from_path(&daemon_json_path) {
        Ok(Some(info)) => serde_json::to_string(&info)?,
        Ok(None) => "{\"ok\":false,\"error\":\"daemon metadata unavailable\"}".to_string(),
        Err(err) => serde_json::json!({
            "ok": false,
            "error": err.to_string(),
        })
        .to_string(),
    };
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    Ok(())
}

fn read_daemon_info_from_path(path: &Path) -> Result<Option<DaemonInfo>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let info = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(info))
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
        return;
    }

    #[cfg(not(unix))]
    {
    let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_config() -> DaemonConfig {
        DaemonConfig {
            runtime_dir: PathBuf::from("/tmp/roko-test"),
            session_id: "test-session".into(),
        }
    }

    #[test]
    fn socket_path_includes_session_id() {
        let cfg = test_config();
        let path = cfg.socket_path();
        assert_eq!(path, PathBuf::from("/tmp/roko-test/roko-test-session.sock"));
    }

    #[test]
    fn pid_path_includes_session_id() {
        let cfg = test_config();
        let path = cfg.pid_path();
        assert_eq!(path, PathBuf::from("/tmp/roko-test/roko-test-session.pid"));
    }

    #[test]
    fn daemon_lifecycle() {
        let mut daemon = DaemonMode::new(test_config());
        assert_eq!(daemon.state, DaemonState::Starting);
        assert!(!daemon.is_accepting());

        daemon.start();
        assert_eq!(daemon.state, DaemonState::Running);
        assert!(daemon.is_accepting());

        daemon.record_command();
        daemon.record_command();
        assert_eq!(daemon.commands_processed, 2);

        daemon.stop();
        assert_eq!(daemon.state, DaemonState::Stopping);
        assert!(!daemon.is_accepting());

        daemon.mark_stopped();
        assert_eq!(daemon.state, DaemonState::Stopped);
    }

    #[test]
    fn with_workdir_creates_config() {
        let daemon = DaemonMode::with_workdir(Path::new("/project"), "abc".into());
        assert_eq!(
            daemon.config.runtime_dir,
            PathBuf::from("/project/.roko/run")
        );
        assert_eq!(daemon.config.session_id, "abc");
    }

    #[test]
    fn status_summary_reflects_state() {
        let mut daemon = DaemonMode::new(test_config());
        daemon.start();
        daemon.record_command();

        let status = daemon.status_summary();
        assert_eq!(status.session_id, "test-session");
        assert_eq!(status.state, DaemonState::Running);
        assert_eq!(status.commands_processed, 1);
        assert!(status.pid > 0);
    }

    #[test]
    fn daemon_state_display() {
        assert_eq!(DaemonState::Starting.to_string(), "starting");
        assert_eq!(DaemonState::Running.to_string(), "running");
        assert_eq!(DaemonState::Stopping.to_string(), "stopping");
        assert_eq!(DaemonState::Stopped.to_string(), "stopped");
    }

    #[test]
    fn daemon_status_display() {
        let status = DaemonStatus {
            session_id: "s1".into(),
            state: DaemonState::Running,
            socket_path: PathBuf::from("/tmp/s1.sock"),
            commands_processed: 5,
            pid: 12345,
        };
        let text = status.to_string();
        assert!(text.contains("s1"));
        assert!(text.contains("running"));
        assert!(text.contains("/tmp/s1.sock"));
        assert!(text.contains("5"));
        assert!(text.contains("12345"));
    }
}
