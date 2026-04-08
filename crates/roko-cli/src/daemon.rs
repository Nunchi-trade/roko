//! Headless daemon mode (`--headless`).
//!
//! Runs roko as a background service that listens on a Unix-domain socket
//! for commands (status queries, signal injection, prompt dispatch). This
//! mode is used by IDE integrations and CI pipelines that want to keep a
//! long-lived agent session warm.

use std::path::{Path, PathBuf};

/// State of the headless daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
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
