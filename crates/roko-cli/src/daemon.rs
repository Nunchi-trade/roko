//! Headless daemon mode (`--headless`).
//!
//! Runs roko as a background service that listens on a Unix-domain socket
//! for commands (status queries, signal injection, prompt dispatch). This
//! mode is used by IDE integrations and CI pipelines that want to keep a
//! long-lived agent session warm.

use std::fs::{self, File as StdFile};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use axum::serve;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader, SeekFrom};
use tokio::net::{TcpListener, UnixListener, UnixStream};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

/// macOS LaunchAgents plist helpers for daemon installation.
#[cfg(target_os = "macos")]
pub mod launchd;
/// Linux systemd user-service helpers for daemon installation.
#[cfg(target_os = "linux")]
pub mod systemd;

// ─── DaemonCmd IPC protocol ──────────────────────────────────────────────────

/// Commands accepted over the Unix-domain-socket IPC protocol.
///
/// The daemon listens on `$ROKO_STATE_DIR/daemon.sock` (typically
/// `~/.roko/daemon.sock`). Clients send a JSON-encoded `DaemonCmd` and
/// receive a JSON-encoded [`DaemonResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonCmd {
    /// Return daemon status: uptime, active agents, subscription count,
    /// memory usage.
    Status,
    /// Graceful shutdown: flush pending, backup state, deregister.
    Stop,
    /// Stop then restart with config reload.
    Restart,
    /// Reload configuration, templates, and subscriptions without restart.
    Reload,
    /// Return a snapshot of all monitored subscriptions.
    ListSubscriptions,
    /// Temporarily pause monitoring for a subscription by ID.
    PauseSubscription {
        /// Subscription ID to pause.
        id: String,
    },
    /// Resume a previously paused subscription.
    ResumeSubscription {
        /// Subscription ID to resume.
        id: String,
    },
}

/// Response returned over the daemon IPC socket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonResponse {
    /// Whether the command succeeded.
    pub ok: bool,
    /// The command that was executed.
    pub command: String,
    /// Optional payload (command-specific).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
    /// Error message if `ok` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DaemonResponse {
    fn success(command: &str) -> Self {
        Self {
            ok: true,
            command: command.to_string(),
            data: serde_json::Value::Null,
            error: None,
        }
    }

    fn success_with_data(command: &str, data: serde_json::Value) -> Self {
        Self {
            ok: true,
            command: command.to_string(),
            data,
            error: None,
        }
    }

    fn failure(command: &str, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            command: command.to_string(),
            data: serde_json::Value::Null,
            error: Some(error.into()),
        }
    }
}

/// Subscription summary returned by `ListSubscriptions` and the IPC
/// pause/resume commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionSummary {
    /// Subscription ID.
    pub id: String,
    /// Agent template name.
    pub template: String,
    /// Trigger pattern.
    pub trigger: String,
    /// Whether the subscription is currently enabled.
    pub enabled: bool,
    /// Maximum concurrent dispatches.
    pub concurrency_limit: usize,
}

use crate::config::RepoRegistry;
use crate::load_layered;
use crate::serve_runtime::RokoCliRuntime;
use roko_core::config::loader::load_config_unified;
use roko_serve::{self, deploy, dispatch, dreams, feedback, fswatcher, scheduler, state::AppState};

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
#[instrument(skip_all, fields(workdir = %workdir.display(), foreground, port))]
pub async fn daemon_start(workdir: &Path, foreground: bool, port: u16) -> Result<()> {
    ensure_runtime_dirs(workdir)?;

    if let Some(info) = read_daemon_info(workdir)? {
        if pid_is_alive(info.pid)? {
            return Err(anyhow!(
                "daemon already running (pid {}, port {})",
                info.pid,
                info.port
            ));
        }

        cleanup_stale_runtime_files(workdir);
    }

    if !foreground {
        spawn_detached_child(workdir, port)?;
        return Ok(());
    }

    let core_config = load_config_unified(workdir)?;
    let cli_config = load_layered(workdir)?.config;
    let dream_settings = cli_config.dreams.clone();
    let agent_settings = cli_config.agent.clone();
    let daimon_strategy_space = cli_config.daimon.strategy_space.clone();
    let repo_registry = RepoRegistry::load(&cli_config, workdir).unwrap_or_default();
    let state_hub = AppState::state_hub_for_workdir(workdir);
    let runtime = RokoCliRuntime::new_with_state_hub(
        cli_config,
        repo_registry,
        state_hub.clone(),
    )
    .into_arc();
    let deploy_backend = Arc::from(deploy::create_backend("manual", None, None, None)?);
    let state = Arc::new(AppState::new_with_daimon_strategy_and_state_hub(
        workdir.to_path_buf(),
        runtime,
        core_config,
        deploy_backend,
        daimon_strategy_space,
        Some(state_hub),
    )?);

    let dream_config = dreams::DreamLoopConfig {
        auto_dream: dream_settings.auto_dream,
        idle_threshold_mins: dream_settings.idle_threshold_mins,
        min_episodes_for_dream: dream_settings.min_episodes_for_dream,
        agent: dreams::DreamAgentConfig {
            command: agent_settings.command,
            args: agent_settings.args,
            model: agent_settings.model,
            bare_mode: agent_settings.bare_mode,
            effort: agent_settings.effort,
            fallback_model: agent_settings.fallback_model,
            timeout_ms: agent_settings.timeout_ms,
            env: agent_settings.env,
        },
    };

    let info = DaemonInfo {
        pid: std::process::id(),
        port,
        session_id: format!("daemon-{}", uuid::Uuid::new_v4()),
        started_at: Utc::now(),
        state: DaemonState::Running,
    };
    write_daemon_info(workdir, &info)?;

    let _scheduler = scheduler::start_scheduler(Arc::clone(&state));
    let _watchers = fswatcher::start_watchers(Arc::clone(&state));
    let _dispatch = dispatch::start_dispatch_loop(Arc::clone(&state));
    let _feedback = feedback::start_feedback_loop(Arc::clone(&state));
    let _dreams = dreams::start_dream_loop(Arc::clone(&state), dream_config);
    let shutdown_request = CancellationToken::new();
    let http_shutdown = CancellationToken::new();
    let ipc_server = match start_ipc_server(Arc::clone(&state), shutdown_request.clone()).await {
        Ok(handle) => handle,
        Err(err) => {
            shutdown_request.cancel();
            state.shutdown().await;
            return Err(err);
        }
    };

    let server_shutdown = shutdown_request.clone();
    let server_state = Arc::clone(&state);
    let server_http_shutdown = http_shutdown.clone();
    let server = tokio::spawn(async move {
        match run_daemon_http_server(server_state, port, server_http_shutdown).await {
            Ok(()) => Ok(()),
            Err(err) => {
                if !server_shutdown.is_cancelled() {
                    server_shutdown.cancel();
                }
                Err(err)
            }
        }
    });
    let reload_signal_task = tokio::spawn(wait_for_reload_signal(Arc::clone(&state)));
    let signal_task = tokio::spawn(wait_for_shutdown_signal());

    tokio::select! {
        _ = shutdown_request.cancelled() => {}
        result = signal_task => {
            match result {
                Ok(()) => shutdown_request.cancel(),
                Err(join_err) => {
                    shutdown_request.cancel();
                    state.shutdown().await;
                    return Err(join_err.into());
                }
            }
        }
    }

    graceful_shutdown_daemon(
        workdir,
        Arc::clone(&state),
        &info,
        shutdown_request,
        http_shutdown,
        reload_signal_task,
        server,
        ipc_server,
    )
    .await
}

/// Stop the active daemon for the current working directory.
#[instrument(skip_all, fields(workdir = %workdir.display()))]
pub async fn daemon_stop(workdir: &Path) -> Result<()> {
    let info = match read_daemon_info(&workdir)? {
        Some(info) => info,
        None => {
            cleanup_daemon_files(&workdir);
            return Ok(());
        }
    };

    let socket_path = daemon_socket_path(&workdir);
    let mut ipc_stopped = false;
    if let Ok(mut stream) = UnixStream::connect(&socket_path).await {
        if stream.write_all(b"stop").await.is_ok() {
            let _ = stream.shutdown().await;
            ipc_stopped = true;
        }
    }

    if !ipc_stopped {
        #[cfg(unix)]
        {
            if let Err(err) = send_signal(info.pid, "TERM") {
                warn!(error = %err, pid = info.pid, "failed to send SIGTERM to daemon");
            }
        }
    }

    let graceful_timeout = Duration::from_secs(90);
    let poll_interval = Duration::from_millis(250);
    let started = std::time::Instant::now();
    while started.elapsed() < graceful_timeout {
        if !pid_is_alive(info.pid)? {
            cleanup_shutdown_runtime_files(&workdir);
            return Ok(());
        }
        sleep(poll_interval).await;
    }

    if pid_is_alive(info.pid)? {
        #[cfg(unix)]
        {
            if let Err(err) = send_signal(info.pid, "KILL") {
                warn!(error = %err, pid = info.pid, "failed to send SIGKILL to daemon");
            }
        }
    }

    cleanup_shutdown_runtime_files(&workdir);
    Ok(())
}

/// Restart the active daemon for the current working directory.
#[instrument(skip_all, fields(workdir = %workdir.display(), port))]
pub async fn daemon_restart(workdir: &Path, port: u16) -> Result<()> {
    daemon_stop(workdir).await?;
    daemon_start(workdir, false, port).await
}

/// Install the daemon under the host service manager.
pub fn daemon_install() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return daemon_install_launchd();
    }

    #[cfg(target_os = "linux")]
    {
        return systemd::install_systemd(crate::DEFAULT_SERVE_PORT);
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(anyhow!(
            "daemon install is only supported on macOS (launchd) and Linux (systemd)"
        ))
    }
}

/// Uninstall the daemon from the host service manager.
pub fn daemon_uninstall() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        return daemon_uninstall_launchd();
    }

    #[cfg(target_os = "linux")]
    {
        return systemd::uninstall_systemd();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Err(anyhow!(
            "daemon uninstall is only supported on macOS (launchd) and Linux (systemd)"
        ))
    }
}

#[cfg(target_os = "macos")]
fn daemon_install_launchd() -> Result<()> {
    let plist_path = launchd::plist_path();
    let plist_dir = plist_path
        .parent()
        .context("resolve LaunchAgents directory")?;
    fs::create_dir_all(plist_dir).with_context(|| format!("create {}", plist_dir.display()))?;

    let home_dir = dirs::home_dir().context("resolve home directory")?;
    let logs_dir = home_dir.join(".roko").join("logs");
    fs::create_dir_all(&logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;

    let plist = launchd::generate_plist(crate::DEFAULT_SERVE_PORT);
    fs::write(&plist_path, plist).with_context(|| format!("write {}", plist_path.display()))?;

    let status = Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()
        .with_context(|| format!("run launchctl load -w {}", plist_path.display()))?;

    if !status.success() {
        return Err(anyhow!(
            "launchctl load -w {} failed with {}",
            plist_path.display(),
            status
        ));
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn daemon_uninstall_launchd() -> Result<()> {
    let plist_path = launchd::plist_path();

    let status = Command::new("launchctl")
        .arg("unload")
        .arg(&plist_path)
        .status()
        .with_context(|| format!("run launchctl unload {}", plist_path.display()))?;

    if !status.success() {
        return Err(anyhow!(
            "launchctl unload {} failed with {}",
            plist_path.display(),
            status
        ));
    }

    if plist_path.exists() {
        fs::remove_file(&plist_path).with_context(|| format!("remove {}", plist_path.display()))?;
    }

    Ok(())
}

/// Print daemon status for the current working directory.
#[instrument(skip_all, fields(workdir = %workdir.display()))]
pub async fn daemon_status(workdir: &Path) -> Result<()> {
    let info = match read_daemon_info(&workdir)? {
        Some(info) => info,
        None => {
            print_daemon_status_table(
                DaemonState::Stopped,
                None,
                None,
                None,
                None,
                None,
                count_signals_processed(&workdir).await?,
            );
            return Ok(());
        }
    };

    let pid_alive = pid_is_alive(info.pid)?;
    let state = if pid_alive {
        info.state.clone()
    } else {
        DaemonState::Stopped
    };
    let port = Some(info.port);
    let signals_processed = count_signals_processed(&workdir).await?;

    if !pid_alive {
        print_daemon_status_table(
            state,
            Some(info.pid),
            port,
            None,
            None,
            None,
            signals_processed,
        );
        return Ok(());
    }

    let socket_path = daemon_socket_path(&workdir);
    let mut stream = UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!("connect {}", socket_path.display()))?;
    stream
        .write_all(b"status")
        .await
        .context("send daemon status request")?;
    stream
        .shutdown()
        .await
        .context("close daemon status request")?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .context("read daemon status response")?;
    let response: DaemonStatusResponse = serde_json::from_slice(&buf).with_context(|| {
        format!(
            "parse daemon status response from {}",
            socket_path.display()
        )
    })?;

    print_daemon_status_table(
        state,
        Some(info.pid),
        port,
        Some(response.uptime_secs),
        Some(response.active_agents),
        Some(response.subscriptions),
        signals_processed,
    );
    Ok(())
}

/// Reload daemon templates and subscriptions without restarting active agents.
#[instrument(skip_all, fields(workdir = %workdir.display()))]
pub async fn daemon_reload(workdir: &Path) -> Result<()> {
    let socket_path = daemon_socket_path(&workdir);
    let mut stream = UnixStream::connect(&socket_path)
        .await
        .with_context(|| format!("connect {}", socket_path.display()))?;
    stream
        .write_all(b"reload")
        .await
        .context("send daemon reload request")?;
    stream
        .shutdown()
        .await
        .context("close daemon reload request")?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .context("read daemon reload response")?;
    let response: DaemonReloadResponse = serde_json::from_slice(&buf).with_context(|| {
        format!(
            "parse daemon reload response from {}",
            socket_path.display()
        )
    })?;

    println!(
        "daemon {}: subscriptions={}, templates={}, loaded={}",
        response.command, response.subscriptions, response.templates, response.loaded
    );

    if !response.warnings.is_empty() {
        println!("warnings: {}", response.warnings.join("; "));
    }

    if response.ok {
        Ok(())
    } else {
        Err(anyhow!(
            response
                .error
                .unwrap_or_else(|| "daemon reload failed".to_string())
        ))
    }
}

/// Print daemon logs for the current working directory.
#[instrument(skip_all, fields(workdir = %workdir.display(), follow, lines))]
pub async fn daemon_logs(workdir: &Path, follow: bool, lines: usize) -> Result<()> {
    let path = log_path(&workdir, "daemon.log");

    if follow {
        follow_daemon_log(&path).await
    } else {
        print_recent_daemon_logs(&path, lines).await
    }
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
    let stdout = StdFile::create(log_path(workdir, "daemon.log"))
        .with_context(|| format!("open {}", log_path(workdir, "daemon.log").display()))?;
    let stderr = StdFile::create(log_path(workdir, "daemon.err"))
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
    write_daemon_json(workdir, info)?;
    let pid_path = daemon_pid_path(workdir);
    fs::write(&pid_path, info.pid.to_string())
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

fn write_daemon_json(workdir: &Path, info: &DaemonInfo) -> Result<()> {
    let json_path = daemon_json_path(workdir);
    if let Some(parent) = json_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(info).context("serialize daemon metadata")?;
    fs::write(&json_path, text).with_context(|| format!("write {}", json_path.display()))?;
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

fn cleanup_daemon_files(workdir: &Path) {
    let json_path = daemon_json_path(workdir);
    let pid_path = daemon_pid_path(workdir);
    let socket_path = daemon_socket_path(workdir);
    let _ = fs::remove_file(json_path);
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(socket_path);
}

fn cleanup_shutdown_runtime_files(workdir: &Path) {
    let pid_path = daemon_pid_path(workdir);
    let socket_path = daemon_socket_path(workdir);
    let _ = fs::remove_file(pid_path);
    let _ = fs::remove_file(socket_path);
}

fn pid_is_alive(pid: u32) -> Result<bool> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    Ok(system.process(Pid::from_u32(pid)).is_some())
}

async fn follow_daemon_log(path: &Path) -> Result<()> {
    let poll_interval = Duration::from_millis(200);

    loop {
        let file = match File::open(path).await {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                sleep(poll_interval).await;
                continue;
            }
            Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
        };

        let mut reader = BufReader::new(file);
        reader
            .seek(SeekFrom::End(0))
            .await
            .with_context(|| format!("seek {}", path.display()))?;

        loop {
            let mut line = String::new();
            let bytes = reader
                .read_line(&mut line)
                .await
                .with_context(|| format!("read {}", path.display()))?;
            if bytes == 0 {
                sleep(poll_interval).await;
                continue;
            }

            print!("{line}");
            std::io::stdout()
                .flush()
                .context("flush daemon log output")?;
        }
    }
}

async fn print_recent_daemon_logs(path: &Path, lines: usize) -> Result<()> {
    if lines == 0 {
        return Ok(());
    }

    let mut file = match File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err).with_context(|| format!("open {}", path.display())),
    };

    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .await
        .with_context(|| format!("read {}", path.display()))?;
    if contents.is_empty() {
        return Ok(());
    }

    let mut end = contents.len();
    let mut out = Vec::new();

    while end > 0 && contents[end - 1] == b'\n' {
        end -= 1;
    }

    while out.len() < lines {
        let start = contents[..end]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map(|idx| idx + 1)
            .unwrap_or(0);

        let mut line = contents[start..end].to_vec();
        if matches!(line.last(), Some(b'\r')) {
            line.pop();
        }
        out.push(String::from_utf8_lossy(&line).into_owned());

        if start == 0 {
            break;
        }
        end = start - 1;
    }

    out.reverse();
    for line in out {
        println!("{line}");
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct DaemonStatusResponse {
    active_agents: usize,
    subscriptions: usize,
    uptime_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct DaemonReloadResponse {
    ok: bool,
    command: String,
    subscriptions: usize,
    templates: usize,
    loaded: usize,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

fn print_daemon_status_table(
    state: DaemonState,
    pid: Option<u32>,
    port: Option<u16>,
    uptime_secs: Option<u64>,
    active_agents: Option<usize>,
    subscriptions: Option<usize>,
    total_signals_processed: usize,
) {
    println!("{:<26}{}", "field", "value");
    println!("{:-<26}{}", "", "");
    println!("{:<26}{}", "state", state);
    println!(
        "{:<26}{}",
        "PID",
        pid.map_or_else(|| "n/a".to_string(), |value| value.to_string())
    );
    println!(
        "{:<26}{}",
        "port",
        port.map_or_else(|| "n/a".to_string(), |value| value.to_string())
    );
    println!(
        "{:<26}{}",
        "uptime",
        uptime_secs.map_or_else(|| "n/a".to_string(), format_uptime)
    );
    println!(
        "{:<26}{}",
        "active agents",
        active_agents.map_or_else(|| "n/a".to_string(), |value| value.to_string())
    );
    println!(
        "{:<26}{}",
        "subscriptions",
        subscriptions.map_or_else(|| "n/a".to_string(), |value| value.to_string())
    );
    println!(
        "{:<26}{}",
        "total signals processed", total_signals_processed
    );
}

fn format_uptime(total_secs: u64) -> String {
    let days = total_secs / 86_400;
    let hours = (total_secs % 86_400) / 3_600;
    let minutes = (total_secs % 3_600) / 60;
    let seconds = total_secs % 60;

    if days > 0 {
        format!("{days}d {hours:02}h {minutes:02}m {seconds:02}s")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

async fn count_signals_processed(workdir: &Path) -> Result<usize> {
    let path = workdir.join(".roko").join("engrams.jsonl");
    match tokio::fs::read_to_string(&path).await {
        Ok(text) => Ok(text.lines().filter(|line| !line.trim().is_empty()).count()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

#[cfg(unix)]
fn send_signal(pid: u32, signal: &str) -> Result<()> {
    let pid = pid.to_string();
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .arg(&pid)
        .status()
        .with_context(|| format!("run kill -{signal} {pid}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("kill -{signal} {pid} exited with {status}"))
    }
}

async fn start_ipc_server(
    state: Arc<AppState>,
    shutdown_request: CancellationToken,
) -> Result<JoinHandle<()>> {
    let socket_path = daemon_socket_path(&state.workdir);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if socket_path.exists() {
        fs::remove_file(&socket_path)
            .with_context(|| format!("remove {}", socket_path.display()))?;
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;

    Ok(tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_request.cancelled() => {
                    break;
                }
                result = listener.accept() => {
                    let (stream, _) = match result {
                        Ok(pair) => pair,
                        Err(err) => {
                            warn!(error = %err, path = %socket_path.display(), "daemon IPC accept failed");
                            shutdown_request.cancel();
                            break;
                        }
                    };

                    let state = Arc::clone(&state);
                    let shutdown_request = shutdown_request.clone();
                    tokio::spawn(async move {
                        if let Err(err) = handle_ipc_command(stream, state, shutdown_request).await {
                            warn!(error = %err, "daemon IPC command failed");
                        }
                    });
                }
            }
        }

        let _ = tokio::fs::remove_file(&socket_path).await;
    }))
}

/// Parse an IPC request as either a JSON `DaemonCmd` or a legacy plain-text
/// command. Plain-text commands (`status`, `stop`, `reload`, `shutdown`) are
/// still accepted for backward compatibility with existing `daemon_stop` and
/// `daemon_status` callers.
fn parse_ipc_request(raw: &str) -> Result<DaemonCmd, String> {
    let trimmed = raw.trim();
    // Try JSON first.
    if trimmed.starts_with('{') {
        return serde_json::from_str(trimmed).map_err(|e| format!("invalid JSON command: {e}"));
    }
    // Legacy plain-text fallback.
    match trimmed.to_ascii_lowercase().as_str() {
        "status" => Ok(DaemonCmd::Status),
        "stop" | "shutdown" => Ok(DaemonCmd::Stop),
        "reload" => Ok(DaemonCmd::Reload),
        "restart" => Ok(DaemonCmd::Restart),
        "list_subscriptions" => Ok(DaemonCmd::ListSubscriptions),
        other => Err(format!("unknown command: {other}")),
    }
}

async fn handle_ipc_command(
    mut stream: UnixStream,
    state: Arc<AppState>,
    shutdown_request: CancellationToken,
) -> Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .context("read daemon IPC request")?;
    let raw = String::from_utf8_lossy(&buf[..n]);

    let cmd = match parse_ipc_request(&raw) {
        Ok(cmd) => cmd,
        Err(err) => {
            let resp = DaemonResponse::failure("unknown", &err);
            let bytes = serde_json::to_vec(&resp).unwrap_or_default();
            stream.write_all(&bytes).await?;
            stream.write_all(b"\n").await?;
            return Ok(());
        }
    };

    let response = dispatch_daemon_cmd(cmd, &state, &shutdown_request).await;
    let bytes = serde_json::to_vec(&response).unwrap_or_default();
    stream.write_all(&bytes).await?;
    stream.write_all(b"\n").await?;
    Ok(())
}

/// Execute a parsed `DaemonCmd` and produce a response.
async fn dispatch_daemon_cmd(
    cmd: DaemonCmd,
    state: &AppState,
    shutdown_request: &CancellationToken,
) -> DaemonResponse {
    match cmd {
        DaemonCmd::Status => {
            let active_agents = state.supervisor.count().await;
            let subscriptions = state.subscriptions.all().len();
            let uptime_secs = state.started_at.elapsed().as_secs();
            DaemonResponse::success_with_data(
                "status",
                json!({
                    "pid": std::process::id(),
                    "active_agents": active_agents,
                    "subscriptions": subscriptions,
                    "uptime_secs": uptime_secs,
                }),
            )
        }
        DaemonCmd::Stop => {
            shutdown_request.cancel();
            DaemonResponse::success("stop")
        }
        DaemonCmd::Restart => {
            // Restart is handled by the caller re-invoking daemon_start after
            // daemon_stop completes. From the IPC perspective we trigger stop.
            shutdown_request.cancel();
            DaemonResponse::success("restart")
        }
        DaemonCmd::Reload => {
            let reload = reload_daemon_runtime(state).await;
            if reload.ok {
                if reload.warnings.is_empty() {
                    info!(
                        subscriptions = reload.subscriptions,
                        templates = reload.templates,
                        loaded = reload.loaded,
                        "daemon config reloaded after IPC request"
                    );
                } else {
                    warn!(
                        subscriptions = reload.subscriptions,
                        templates = reload.templates,
                        loaded = reload.loaded,
                        warnings = ?reload.warnings,
                        "daemon config reloaded after IPC request with warnings"
                    );
                }
                DaemonResponse::success_with_data(
                    "reload",
                    json!({
                        "subscriptions": reload.subscriptions,
                        "templates": reload.templates,
                        "loaded": reload.loaded,
                        "warnings": reload.warnings,
                    }),
                )
            } else {
                DaemonResponse::failure(
                    "reload",
                    reload
                        .error
                        .unwrap_or_else(|| "daemon reload failed".into()),
                )
            }
        }
        DaemonCmd::ListSubscriptions => {
            let subs: Vec<SubscriptionSummary> = state
                .subscriptions
                .all()
                .iter()
                .map(|s| SubscriptionSummary {
                    id: s.id.clone(),
                    template: s.template.clone(),
                    trigger: s.trigger.clone(),
                    enabled: s.enabled,
                    concurrency_limit: s.concurrency_limit,
                })
                .collect();
            DaemonResponse::success_with_data(
                "list_subscriptions",
                serde_json::to_value(&subs).unwrap_or_default(),
            )
        }
        DaemonCmd::PauseSubscription { id } => match state.subscriptions.get_by_id(&id) {
            Some(mut sub) => {
                sub.enabled = false;
                let _ = state.subscriptions.update_by_id(&id, sub);
                info!(subscription_id = %id, "subscription paused via IPC");
                DaemonResponse::success_with_data(
                    "pause_subscription",
                    json!({ "id": id, "enabled": false }),
                )
            }
            None => DaemonResponse::failure(
                "pause_subscription",
                format!("subscription not found: {id}"),
            ),
        },
        DaemonCmd::ResumeSubscription { id } => match state.subscriptions.get_by_id(&id) {
            Some(mut sub) => {
                sub.enabled = true;
                let _ = state.subscriptions.update_by_id(&id, sub);
                info!(subscription_id = %id, "subscription resumed via IPC");
                DaemonResponse::success_with_data(
                    "resume_subscription",
                    json!({ "id": id, "enabled": true }),
                )
            }
            None => DaemonResponse::failure(
                "resume_subscription",
                format!("subscription not found: {id}"),
            ),
        },
    }
}

async fn reload_daemon_runtime(state: &AppState) -> DaemonReloadResponse {
    let warnings = match roko_serve::reload_config_from_disk(state) {
        Ok(warnings) => warnings,
        Err(err) => {
            error!(error = %err, "daemon config reload failed; keeping previous config");
            return DaemonReloadResponse {
                ok: false,
                command: "reload".to_string(),
                subscriptions: 0,
                templates: 0,
                loaded: 0,
                warnings: Vec::new(),
                error: Some(err.to_string()),
            };
        }
    };

    let subscriptions_report = {
        let roko_config = state.load_roko_config().as_ref().clone();
        let registry = roko_serve::dispatch::SubscriptionRegistry::load_from_project(
            &state.workdir,
            &roko_config,
        );
        state.subscriptions.replace_with(registry.all())
    };

    let templates_report = {
        let mut templates = state.templates.write().await;
        templates.scan()
    };

    DaemonReloadResponse {
        ok: true,
        command: "reload".to_string(),
        subscriptions: subscriptions_report,
        templates: templates_report.loaded,
        loaded: subscriptions_report + templates_report.loaded,
        warnings,
        error: None,
    }
}

async fn wait_for_reload_signal(state: Arc<AppState>) {
    #[cfg(unix)]
    {
        let mut sighup = signal(SignalKind::hangup()).expect("install SIGHUP handler");
        while sighup.recv().await.is_some() {
            info!("SIGHUP received, reloading config");
            let response = reload_daemon_runtime(&state).await;
            if response.ok {
                if response.warnings.is_empty() {
                    info!(
                        subscriptions = response.subscriptions,
                        templates = response.templates,
                        loaded = response.loaded,
                        "daemon config reloaded after SIGHUP"
                    );
                } else {
                    warn!(
                        subscriptions = response.subscriptions,
                        templates = response.templates,
                        loaded = response.loaded,
                        warnings = ?response.warnings,
                        "daemon config reloaded after SIGHUP with warnings"
                    );
                }
            }
        }
        return;
    }

    #[cfg(not(unix))]
    {
        let _ = state;
    }
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

async fn run_daemon_http_server(
    state: Arc<AppState>,
    port: u16,
    shutdown: CancellationToken,
) -> Result<()> {
    let roko_config = state.load_roko_config().as_ref().clone();
    let router = roko_serve::routes::build_router(
        Arc::clone(&state),
        &roko_config.server.cors_origins,
        roko_config.serve.auth.clone(),
    );
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind to {addr}"))?;

    info!("roko server listening on http://{addr}");
    info!("workdir: {}", state.workdir.display());

    let shutdown = shutdown.clone();
    serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown.cancelled().await;
        })
        .await
        .context("axum server error")?;

    info!("server stopped");
    Ok(())
}

async fn graceful_shutdown_daemon(
    workdir: &Path,
    state: Arc<AppState>,
    info: &DaemonInfo,
    shutdown_request: CancellationToken,
    http_shutdown: CancellationToken,
    reload_signal_task: JoinHandle<()>,
    server: JoinHandle<Result<()>>,
    ipc_server: JoinHandle<()>,
) -> Result<()> {
    let mut stopping = info.clone();
    stopping.state = DaemonState::Stopping;
    write_daemon_info(workdir, &stopping)?;

    http_shutdown.cancel();

    match timeout(Duration::from_secs(10), server).await {
        Ok(join_result) => match join_result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                warn!(error = %err, "daemon HTTP server stopped with error");
            }
            Err(join_err) => {
                warn!(error = %join_err, "daemon HTTP server join failed");
            }
        },
        Err(_) => {
            warn!("daemon HTTP server did not stop within timeout");
        }
    }

    shutdown_request.cancel();
    state.cancel.cancel();
    reload_signal_task.abort();
    let _ = reload_signal_task.await;

    let active_agents = state.supervisor.count().await;
    if active_agents > 0 {
        info!(active_agents, "waiting for active agents to complete");
    }

    let completed = state.supervisor.wait_all(Duration::from_secs(60)).await;
    if !completed.is_empty() {
        info!(
            completed = completed.len(),
            "active agents exited during graceful shutdown"
        );
    }

    let remaining = state.supervisor.count().await;
    if remaining > 0 {
        info!(remaining, "killing remaining agents");
        let killed = state.supervisor.kill_all().await;
        if !killed.is_empty() {
            info!(killed = killed.len(), "killed remaining agents");
        }
    }

    flush_daemon_artifacts(workdir).await?;

    cleanup_shutdown_runtime_files(workdir);

    let _ = ipc_server.await;

    let mut stopped = info.clone();
    stopped.state = DaemonState::Stopped;
    write_daemon_json(workdir, &stopped)?;

    Ok(())
}

async fn flush_daemon_artifacts(workdir: &Path) -> Result<()> {
    flush_file(workdir.join(".roko").join("engrams.jsonl")).await?;
    flush_file(workdir.join(".roko").join("episodes.jsonl")).await?;
    flush_file(workdir.join(".roko").join("learn").join("heartbeat.json")).await?;
    flush_file(workdir.join(".roko").join("learn").join("heartbeat.jsonl")).await?;
    flush_file(workdir.join(".roko").join("logs").join("daemon.log")).await?;
    flush_file(workdir.join(".roko").join("logs").join("daemon.err")).await?;
    std::io::stdout().flush().context("flush stdout")?;
    std::io::stderr().flush().context("flush stderr")?;
    Ok(())
}

async fn flush_file(path: PathBuf) -> Result<()> {
    match File::open(&path).await {
        Ok(file) => file
            .sync_all()
            .await
            .with_context(|| format!("sync {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("open {}", path.display())),
    }
}

// ─── DEPLOY-11: Multi-repo daemon coordination ──────────────────────────────

/// Per-repository subscription managed by the daemon (DEPLOY-11).
///
/// Each repo has its own `.roko/` state directory, agent budget, and
/// scheduling priority. The daemon's [`SubscriptionManager`] coordinates
/// them under a shared agent concurrency limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoSubscription {
    /// Unique subscription identifier (e.g. `"repo-0"`).
    pub id: String,
    /// Absolute path to the repository root.
    pub repo_path: PathBuf,
    /// Repository-local `.roko/` directory (derived from `repo_path`).
    pub state_dir: PathBuf,
    /// Whether the subscription is currently active.
    pub enabled: bool,
    /// Per-repo USD budget limit (0 = unlimited).
    pub budget_limit_usd: f64,
    /// USD spent so far in the current billing period.
    pub budget_spent_usd: f64,
    /// Current number of active agents for this repo.
    pub active_agents: usize,
    /// Scheduling priority (higher = more urgent; recent changes bump this).
    pub priority: u32,
    /// Trigger configuration from `roko.toml`.
    pub trigger: String,
    /// Agent template name.
    pub template: String,
}

impl RepoSubscription {
    /// Create a subscription for a repository.
    #[must_use]
    pub fn new(id: String, repo_path: PathBuf, template: String, trigger: String) -> Self {
        let state_dir = repo_path.join(".roko");
        Self {
            id,
            repo_path,
            state_dir,
            enabled: true,
            budget_limit_usd: 0.0,
            budget_spent_usd: 0.0,
            active_agents: 0,
            priority: 0,
            trigger,
            template,
        }
    }

    /// Whether the subscription has remaining budget (or unlimited).
    #[must_use]
    pub fn has_budget(&self) -> bool {
        self.budget_limit_usd <= 0.0 || self.budget_spent_usd < self.budget_limit_usd
    }

    /// Record spending against this repo's budget.
    pub fn record_spend(&mut self, amount_usd: f64) {
        self.budget_spent_usd += amount_usd;
    }
}

/// Manages N repository subscriptions under a shared agent limit (DEPLOY-11).
///
/// The daemon creates one `SubscriptionManager` on startup, loads repos
/// from the `[[subscriptions]]` config, and distributes agent slots using
/// priority-based scheduling.
#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    /// All registered repository subscriptions.
    pub repos: Vec<RepoSubscription>,
    /// Maximum total agents across all repos.
    pub max_total_agents: usize,
    /// Current total active agents.
    pub total_active_agents: usize,
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self {
            repos: Vec::new(),
            max_total_agents: 8,
            total_active_agents: 0,
        }
    }
}

impl SubscriptionManager {
    /// Create a manager with the given agent concurrency limit.
    #[must_use]
    pub fn new(max_total_agents: usize) -> Self {
        Self {
            repos: Vec::new(),
            max_total_agents,
            total_active_agents: 0,
        }
    }

    /// Load subscriptions from the config's `[[subscriptions]]` entries.
    ///
    /// Each subscription that has a `path` field in its trigger_config
    /// becomes a repo subscription. Subscriptions without paths are
    /// skipped (they belong to the primary repo).
    pub fn load_from_config(
        &mut self,
        subscriptions: &[roko_core::config::schema::SubscriptionConfig],
        primary_workdir: &Path,
    ) {
        self.repos.clear();
        for (i, sub) in subscriptions.iter().enumerate() {
            let repo_path = if let Some(ref trigger_cfg) = sub.trigger_config {
                match trigger_cfg {
                    roko_core::config::SubscriptionTrigger::FileWatch { paths, .. } => {
                        // If the watch path looks absolute, derive repo from it.
                        paths
                            .first()
                            .and_then(|p| {
                                let path = PathBuf::from(p);
                                if path.is_absolute() {
                                    path.parent().map(|p| p.to_path_buf())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_else(|| primary_workdir.to_path_buf())
                    }
                    _ => primary_workdir.to_path_buf(),
                }
            } else {
                primary_workdir.to_path_buf()
            };

            let id = format!("sub-{i}");
            self.repos.push(RepoSubscription::new(
                id,
                repo_path,
                sub.template.clone(),
                sub.trigger.clone(),
            ));
        }
    }

    /// Whether we can schedule another agent (global limit not reached).
    #[must_use]
    pub fn can_schedule(&self) -> bool {
        self.total_active_agents < self.max_total_agents
    }

    /// Try to allocate an agent slot for the given repo.
    ///
    /// Returns `true` if the slot was granted.
    pub fn allocate_agent(&mut self, repo_id: &str) -> bool {
        if !self.can_schedule() {
            return false;
        }
        if let Some(repo) = self.repos.iter_mut().find(|r| r.id == repo_id) {
            if !repo.enabled || !repo.has_budget() {
                return false;
            }
            repo.active_agents += 1;
            self.total_active_agents += 1;
            true
        } else {
            false
        }
    }

    /// Release an agent slot for the given repo.
    pub fn release_agent(&mut self, repo_id: &str) {
        if let Some(repo) = self.repos.iter_mut().find(|r| r.id == repo_id) {
            repo.active_agents = repo.active_agents.saturating_sub(1);
            self.total_active_agents = self.total_active_agents.saturating_sub(1);
        }
    }

    /// Return the next repo that should receive an agent slot, based on
    /// priority. Repos with higher priority and fewer active agents are
    /// preferred.
    #[must_use]
    pub fn next_schedulable(&self) -> Option<&RepoSubscription> {
        self.repos
            .iter()
            .filter(|r| r.enabled && r.has_budget())
            .max_by_key(|r| {
                // Higher priority wins; tie-break by fewer active agents.
                (r.priority, usize::MAX - r.active_agents)
            })
    }

    /// Pause a subscription by ID.
    pub fn pause(&mut self, repo_id: &str) -> Result<()> {
        if let Some(repo) = self.repos.iter_mut().find(|r| r.id == repo_id) {
            repo.enabled = false;
            Ok(())
        } else {
            Err(anyhow!("subscription not found: {repo_id}"))
        }
    }

    /// Resume a subscription by ID.
    pub fn resume(&mut self, repo_id: &str) -> Result<()> {
        if let Some(repo) = self.repos.iter_mut().find(|r| r.id == repo_id) {
            repo.enabled = true;
            Ok(())
        } else {
            Err(anyhow!("subscription not found: {repo_id}"))
        }
    }

    /// Bump the priority of a repo (e.g. when recent changes detected).
    pub fn bump_priority(&mut self, repo_id: &str, amount: u32) {
        if let Some(repo) = self.repos.iter_mut().find(|r| r.id == repo_id) {
            repo.priority = repo.priority.saturating_add(amount);
        }
    }

    /// Return summaries of all subscriptions.
    #[must_use]
    pub fn summaries(&self) -> Vec<SubscriptionSummary> {
        self.repos
            .iter()
            .map(|r| SubscriptionSummary {
                id: r.id.clone(),
                template: r.template.clone(),
                trigger: r.trigger.clone(),
                enabled: r.enabled,
                concurrency_limit: self.max_total_agents,
            })
            .collect()
    }

    /// Number of registered repos.
    #[must_use]
    pub fn repo_count(&self) -> usize {
        self.repos.len()
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

    // ─── DaemonCmd IPC protocol tests ────────────────────────────────────

    #[test]
    fn parse_ipc_plain_text_status() {
        let cmd = parse_ipc_request("status").unwrap();
        assert_eq!(cmd, DaemonCmd::Status);
    }

    #[test]
    fn parse_ipc_plain_text_stop() {
        let cmd = parse_ipc_request("stop").unwrap();
        assert_eq!(cmd, DaemonCmd::Stop);
    }

    #[test]
    fn parse_ipc_plain_text_shutdown_maps_to_stop() {
        let cmd = parse_ipc_request("shutdown").unwrap();
        assert_eq!(cmd, DaemonCmd::Stop);
    }

    #[test]
    fn parse_ipc_plain_text_reload() {
        let cmd = parse_ipc_request("reload").unwrap();
        assert_eq!(cmd, DaemonCmd::Reload);
    }

    #[test]
    fn parse_ipc_plain_text_list_subscriptions() {
        let cmd = parse_ipc_request("list_subscriptions").unwrap();
        assert_eq!(cmd, DaemonCmd::ListSubscriptions);
    }

    #[test]
    fn parse_ipc_json_status() {
        let cmd = parse_ipc_request(r#"{"cmd":"status"}"#).unwrap();
        assert_eq!(cmd, DaemonCmd::Status);
    }

    #[test]
    fn parse_ipc_json_pause_subscription() {
        let cmd = parse_ipc_request(r#"{"cmd":"pause_subscription","id":"config-0"}"#).unwrap();
        assert_eq!(
            cmd,
            DaemonCmd::PauseSubscription {
                id: "config-0".into()
            }
        );
    }

    #[test]
    fn parse_ipc_json_resume_subscription() {
        let cmd = parse_ipc_request(r#"{"cmd":"resume_subscription","id":"config-1"}"#).unwrap();
        assert_eq!(
            cmd,
            DaemonCmd::ResumeSubscription {
                id: "config-1".into()
            }
        );
    }

    #[test]
    fn parse_ipc_json_list_subscriptions() {
        let cmd = parse_ipc_request(r#"{"cmd":"list_subscriptions"}"#).unwrap();
        assert_eq!(cmd, DaemonCmd::ListSubscriptions);
    }

    #[test]
    fn parse_ipc_unknown_command_fails() {
        assert!(parse_ipc_request("wiggle").is_err());
    }

    #[test]
    fn parse_ipc_invalid_json_fails() {
        assert!(parse_ipc_request("{bad json").is_err());
    }

    #[test]
    fn daemon_response_success_serializes() {
        let resp = DaemonResponse::success("stop");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"ok\":true"));
        assert!(json.contains("\"command\":\"stop\""));
    }

    #[test]
    fn daemon_response_failure_serializes() {
        let resp = DaemonResponse::failure("reload", "disk full");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"ok\":false"));
        assert!(json.contains("\"error\":\"disk full\""));
    }

    #[test]
    fn subscription_summary_roundtrip() {
        let summary = SubscriptionSummary {
            id: "config-0".into(),
            template: "pr-review".into(),
            trigger: "github.pull_request.*".into(),
            enabled: true,
            concurrency_limit: 3,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: SubscriptionSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(summary, parsed);
    }

    // ─── DEPLOY-11: SubscriptionManager tests ───────────────────────────

    #[test]
    fn subscription_manager_default_limit() {
        let mgr = SubscriptionManager::default();
        assert_eq!(mgr.max_total_agents, 8);
        assert!(mgr.can_schedule());
    }

    #[test]
    fn subscription_manager_allocate_and_release() {
        let mut mgr = SubscriptionManager::new(2);
        mgr.repos.push(RepoSubscription::new(
            "r1".into(),
            PathBuf::from("/repo/a"),
            "template".into(),
            "watch".into(),
        ));
        mgr.repos.push(RepoSubscription::new(
            "r2".into(),
            PathBuf::from("/repo/b"),
            "template".into(),
            "cron".into(),
        ));

        assert!(mgr.allocate_agent("r1"));
        assert!(mgr.allocate_agent("r2"));
        assert!(!mgr.allocate_agent("r1"), "limit of 2 reached");
        assert_eq!(mgr.total_active_agents, 2);

        mgr.release_agent("r1");
        assert_eq!(mgr.total_active_agents, 1);
        assert!(mgr.allocate_agent("r1"));
    }

    #[test]
    fn subscription_manager_budget_enforcement() {
        let mut mgr = SubscriptionManager::new(10);
        let mut sub = RepoSubscription::new(
            "r1".into(),
            PathBuf::from("/repo"),
            "t".into(),
            "watch".into(),
        );
        sub.budget_limit_usd = 10.0;
        sub.budget_spent_usd = 9.5;
        mgr.repos.push(sub);

        assert!(mgr.allocate_agent("r1"), "still under budget");
        mgr.release_agent("r1");

        mgr.repos[0].budget_spent_usd = 10.0;
        assert!(!mgr.allocate_agent("r1"), "budget exhausted");
    }

    #[test]
    fn subscription_manager_pause_resume() {
        let mut mgr = SubscriptionManager::new(10);
        mgr.repos.push(RepoSubscription::new(
            "r1".into(),
            PathBuf::from("/repo"),
            "t".into(),
            "watch".into(),
        ));

        mgr.pause("r1").unwrap();
        assert!(!mgr.repos[0].enabled);
        assert!(!mgr.allocate_agent("r1"), "paused repo should not allocate");

        mgr.resume("r1").unwrap();
        assert!(mgr.repos[0].enabled);
        assert!(mgr.allocate_agent("r1"));
    }

    #[test]
    fn subscription_manager_priority_scheduling() {
        let mut mgr = SubscriptionManager::new(10);
        let mut sub1 =
            RepoSubscription::new("r1".into(), PathBuf::from("/a"), "t".into(), "w".into());
        sub1.priority = 5;
        let mut sub2 =
            RepoSubscription::new("r2".into(), PathBuf::from("/b"), "t".into(), "w".into());
        sub2.priority = 10;
        mgr.repos.push(sub1);
        mgr.repos.push(sub2);

        let next = mgr.next_schedulable().unwrap();
        assert_eq!(next.id, "r2", "higher priority should be scheduled first");
    }

    #[test]
    fn subscription_manager_summaries() {
        let mut mgr = SubscriptionManager::new(4);
        mgr.repos.push(RepoSubscription::new(
            "r1".into(),
            PathBuf::from("/a"),
            "template-a".into(),
            "cron".into(),
        ));
        let summaries = mgr.summaries();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "r1");
        assert_eq!(summaries[0].template, "template-a");
    }

    #[test]
    fn repo_subscription_state_dir_derived() {
        let sub = RepoSubscription::new(
            "x".into(),
            PathBuf::from("/home/user/project"),
            "t".into(),
            "w".into(),
        );
        assert_eq!(sub.state_dir, PathBuf::from("/home/user/project/.roko"));
    }
}
