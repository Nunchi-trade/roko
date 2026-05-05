//! `roko dev` command — unified dev environment with PID file management.
//!
//! Replaces the `roko-dev-full` shell alias with a proper CLI command that
//! manages serve + optional frontend with PID file coordination, graceful
//! shutdown, and port conflict detection.

#![allow(unused_imports)]

use std::io;
use std::net::TcpListener;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use roko_core::Workspace;
use tokio::process::Command as TokioCommand;

use crate::*;

/// Main entry point for `roko dev`.
pub(crate) async fn cmd_dev(cli: &Cli, no_frontend: bool) -> Result<i32> {
    let wd = resolve_workdir(cli);

    prepare_runtime_hooks(&wd, cli.quiet);

    // Ensure .roko/ exists.
    let _ = bootstrap_observability_dirs(&wd);

    // 1. Resolve workdir/config exactly like Command::Serve.
    let _lock = roko_cli::workspace_lock::acquire_workspace_lock(&wd.join(".roko"))?;
    let config = resolve_config_for_workdir(cli, &wd)?;
    let repo_registry = RepoRegistry::load(&config, &wd).unwrap_or_default();
    let state_hub = roko_serve::state::AppState::state_hub_for_workdir(&wd);
    let runtime =
        RokoCliRuntime::new_with_state_hub(config, repo_registry, state_hub.clone()).into_arc();

    let boot = roko_cli::bootstrap::RokoBootstrap::new(
        &wd,
        roko_cli::bootstrap::BootOpts {
            require_workspace: false,
            require_provider: false,
            acquire_lock: false,
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    let roko_config = boot.config;

    let server_config =
        roko_serve::ServerBuildConfig::new(wd.clone(), runtime, roko_config, None, None)
            .with_state_hub(state_hub);

    let bind = server_config.effective_bind().to_string();
    let port = server_config.effective_port();

    // 2. Check existing PID file and handle stale processes.
    handle_existing_pid_file(&wd)?;

    // 3. Probe the resolved listen address for availability.
    probe_addr_available(&bind, port)?;

    // 4. Write PID file (after port probe succeeds).
    write_pid_file(&wd)?;

    // 5. Start `roko serve` in background.
    let (serve_state, serve_handle) = roko_serve::ServerBuilder::new(server_config)
        .start_background()
        .await
        .inspect_err(|_| {
            // Clean up PID file on serve start failure.
            let _ = remove_pid_file(&wd);
        })?;

    println!("  roko dev       http://{}:{}  started", bind, port);

    // 6. Optionally start the frontend dev server.
    let frontend_handle = if !no_frontend {
        match start_frontend(&wd).await {
            Ok(child) => {
                println!("  frontend       npm run dev  started");
                Some(child)
            }
            Err(e) => {
                eprintln!("  frontend       skipped ({})", e);
                None
            }
        }
    } else {
        None
    };

    println!();
    println!("  Press Ctrl+C to stop all.");
    println!();

    // 7. Signal handling: wait for SIGINT/SIGTERM.
    tokio::signal::ctrl_c()
        .await
        .context("listen for ctrl+c")?;

    println!("\nShutting down...");

    // Stop frontend process.
    if let Some(mut child) = frontend_handle {
        terminate_child(&mut child).await;
    }

    // Cancel serve.
    serve_state.cancel.cancel();
    match tokio::time::timeout(Duration::from_secs(5), serve_handle).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => eprintln!("warning: roko-serve shutdown error: {e}"),
        Ok(Err(e)) => eprintln!("warning: roko-serve task panicked: {e}"),
        Err(_) => eprintln!("warning: roko-serve did not shut down within 5 seconds"),
    }

    // Remove PID file.
    remove_pid_file(&wd)?;

    println!("All services stopped.");
    Ok(EXIT_SUCCESS)
}

// ─── PID file helpers ──────────────��────────────────────────────────────────

/// Read the PID from `.roko/serve.pid`. Returns `Ok(None)` if the file is
/// missing or contains a malformed value.
pub fn read_pid_file(workdir: &Path) -> Result<Option<u32>> {
    let ws = Workspace::open_or_create(workdir)?;
    let pid_path = ws.serve_pid_file();
    match std::fs::read_to_string(&pid_path) {
        Ok(s) => Ok(s.trim().parse().ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::new(e).context(format!("read {}", pid_path.display()))),
    }
}

/// Atomically write the current process PID to `.roko/serve.pid`.
pub fn write_pid_file(workdir: &Path) -> Result<()> {
    let ws = Workspace::open_or_create(workdir)?;
    let pid_path = ws.serve_pid_file();
    roko_core::io::atomic_write(&pid_path, std::process::id().to_string().as_bytes())
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

/// Remove `.roko/serve.pid`, ignoring if already absent.
pub fn remove_pid_file(workdir: &Path) -> Result<()> {
    let ws = Workspace::open_or_create(workdir)?;
    let pid_path = ws.serve_pid_file();
    match std::fs::remove_file(&pid_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("remove {}", pid_path.display()))),
    }
}

/// Check if a process with the given PID is alive.
///
/// Uses `kill(pid, 0)` on Unix which checks existence without sending a signal.
pub fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill with signal 0 checks process existence without side effects.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Send SIGTERM to a process, wait up to `timeout` for it to exit, then
/// send SIGKILL if still alive. Only acts on a PID that was read from the
/// PID file (never kills unknown processes).
pub fn terminate_pid(pid: u32, timeout: Duration) -> Result<()> {
    #[cfg(unix)]
    {
        // Send SIGTERM.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }

        // Poll for process exit.
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if !process_is_alive(pid) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Still alive after timeout — send SIGKILL.
        if process_is_alive(pid) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            // Brief wait for SIGKILL to take effect.
            std::thread::sleep(Duration::from_millis(200));
        }

        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, timeout);
        Ok(())
    }
}

/// Check the PID file and handle any stale or live prior process.
///
/// - If PID file points to a live process, send SIGTERM and wait.
/// - If PID file is stale (dead process or malformed), remove it.
fn handle_existing_pid_file(workdir: &Path) -> Result<()> {
    match read_pid_file(workdir)? {
        Some(pid) if process_is_alive(pid) => {
            eprintln!(
                "Port in use by prior roko dev process (PID {}). Stopping it...",
                pid
            );
            terminate_pid(pid, Duration::from_secs(5))?;
            remove_pid_file(workdir)?;
            // Brief pause for port to be released.
            std::thread::sleep(Duration::from_millis(500));
        }
        Some(_pid) => {
            // PID file exists but process is dead — clean up stale file.
            remove_pid_file(workdir)?;
        }
        None => {
            // No PID file — nothing to do.
        }
    }
    Ok(())
}

/// Attempt to bind the resolved address to verify the port is free.
///
/// If the port is in use and there was no live PID file owner, returns
/// a clear error naming the address and indicating it is owned by an
/// unknown process.
pub fn probe_addr_available(bind: &str, port: u16) -> Result<()> {
    let addr = format!("{bind}:{port}");
    match TcpListener::bind(&addr) {
        Ok(_listener) => {
            // Port is free; the listener is dropped immediately.
            Ok(())
        }
        Err(_) => {
            anyhow::bail!(
                "Address {addr} is already in use by an unknown process.\n  \
                 Kill it or use a different port via [server].port in roko.toml."
            );
        }
    }
}

// ─── Frontend management ─────────────────��───────────────────���──────────────

/// Start the frontend dev server (`npm run dev` in `demo/demo-app/`).
///
/// Returns the child process handle if the directory and `package.json` exist.
async fn start_frontend(workdir: &Path) -> Result<tokio::process::Child> {
    let demo_dir = workdir.join("demo/demo-app");
    let package_json = demo_dir.join("package.json");

    if !package_json.exists() {
        anyhow::bail!("demo/demo-app/package.json not found");
    }

    let child = TokioCommand::new("npm")
        .args(["run", "dev"])
        .current_dir(&demo_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| "failed to start `npm run dev` in demo/demo-app/")?;

    Ok(child)
}

/// Gracefully terminate a child process (SIGTERM then wait, SIGKILL on timeout).
async fn terminate_child(child: &mut tokio::process::Child) {
    // Try to get the PID for a graceful SIGTERM on Unix.
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }

    // Wait up to 5 seconds for graceful exit.
    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            // Force kill if still running.
            let _ = child.kill().await;
        }
    }
}

// ─── Tests ────────────────────────────────────────────��─────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_write_read_remove_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        // Create .roko/ so Workspace::open_or_create succeeds.
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        // Initially no PID file.
        assert_eq!(read_pid_file(workdir).unwrap(), None);

        // Write PID file.
        write_pid_file(workdir).unwrap();

        // Read it back — should be our PID.
        let pid = read_pid_file(workdir).unwrap();
        assert_eq!(pid, Some(std::process::id()));

        // Remove it.
        remove_pid_file(workdir).unwrap();
        assert_eq!(read_pid_file(workdir).unwrap(), None);
    }

    #[test]
    fn missing_pid_file_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        assert_eq!(read_pid_file(workdir).unwrap(), None);
    }

    #[test]
    fn malformed_pid_file_returns_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        // Write garbage to the PID file.
        std::fs::write(workdir.join(".roko/serve.pid"), "not-a-number\n").expect("write");

        // Should return None (malformed).
        assert_eq!(read_pid_file(workdir).unwrap(), None);
    }

    #[test]
    fn dead_pid_is_not_alive() {
        // PID 0 is never a user process on Unix; use a very large PID that
        // almost certainly does not exist.
        assert!(!process_is_alive(4_000_000));
    }

    #[test]
    fn current_process_is_alive() {
        assert!(process_is_alive(std::process::id()));
    }

    #[test]
    fn probe_addr_fails_when_port_in_use() {
        // Bind a port to hold it.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();

        // Probing the same port should fail.
        let result = probe_addr_available("127.0.0.1", port);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("already in use"));

        drop(listener);
    }

    #[test]
    fn probe_addr_succeeds_when_free() {
        // Find a free port by binding and then immediately dropping.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        // Probing should succeed now.
        assert!(probe_addr_available("127.0.0.1", port).is_ok());
    }

    #[test]
    fn remove_pid_file_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        // Remove when no file exists — should not error.
        assert!(remove_pid_file(workdir).is_ok());

        // Write and remove twice.
        write_pid_file(workdir).unwrap();
        remove_pid_file(workdir).unwrap();
        assert!(remove_pid_file(workdir).is_ok());
    }
}
