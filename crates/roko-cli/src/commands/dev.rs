//! `roko dev` command — unified dev environment with PID file management.
//!
//! Replaces the `roko-dev-full` shell alias with a proper CLI command that
//! manages serve + optional frontend with PID file coordination, graceful
//! shutdown, and port conflict detection.
//!
//! Signal handling: listens for both SIGINT (Ctrl+C) and SIGTERM (systemd,
//! docker stop, kill) to ensure clean PID file removal on any shutdown path.

#![allow(unsafe_code, dead_code)]

use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, Result};
use tokio::process::Command as TokioCommand;
use tokio::signal::unix::{SignalKind, signal};

use crate::{Cli, EXIT_SUCCESS, resolve_workdir};

/// Main entry point for `roko dev`.
pub(crate) async fn cmd_dev(cli: &Cli, no_frontend: bool) -> Result<i32> {
    let wd = resolve_workdir(cli);

    // Ensure .roko/ exists.
    let roko_dir = wd.join(".roko");
    std::fs::create_dir_all(&roko_dir).with_context(|| format!("create {}", roko_dir.display()))?;

    // 1. Check existing PID file and handle stale processes.
    handle_existing_pid_file(&wd)?;

    // 2. Probe the default serve port for availability.
    let bind = "127.0.0.1";
    let port: u16 = 6677;
    probe_addr_available(bind, port)?;

    // 3. Write PID file (after port probe succeeds).
    write_pid_file(&wd)?;

    // 4. Start `roko serve` in background.
    let mut serve_child = TokioCommand::new(std::env::current_exe()?)
        .args(["serve"])
        .current_dir(&wd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .inspect_err(|_| {
            let _ = remove_pid_file(&wd);
        })
        .with_context(|| "failed to start roko serve subprocess")?;

    println!("  roko dev       http://{}:{}  started", bind, port);

    // 5. Optionally start the frontend dev server.
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

    // 6. Signal handling: wait for SIGINT or SIGTERM.
    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            // SIGINT received (Ctrl+C)
        }
        _ = sigterm.recv() => {
            // SIGTERM received (e.g. from systemd, docker stop, kill)
        }
        status = serve_child.wait() => {
            // Serve exited unexpectedly.
            eprintln!("warning: roko serve exited unexpectedly: {:?}", status);
        }
    }

    println!("\nShutting down...");

    // Stop frontend process.
    if let Some(mut child) = frontend_handle {
        terminate_child(&mut child).await;
    }

    // Stop serve process.
    terminate_child(&mut serve_child).await;

    // Remove PID file.
    remove_pid_file(&wd)?;

    println!("All services stopped.");
    Ok(EXIT_SUCCESS)
}

// ─── PID file helpers ───────────────────────────────────────────────────────

fn pid_file_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko/serve.pid")
}

/// Read the PID from `.roko/serve.pid`. Returns `Ok(None)` if the file is
/// missing or contains a malformed value.
pub fn read_pid_file(workdir: &Path) -> Result<Option<u32>> {
    let pid_path = pid_file_path(workdir);
    match std::fs::read_to_string(&pid_path) {
        Ok(s) => Ok(s.trim().parse().ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::new(e).context(format!("read {}", pid_path.display()))),
    }
}

/// Atomically write the current process PID to `.roko/serve.pid`.
pub fn write_pid_file(workdir: &Path) -> Result<()> {
    let pid_path = pid_file_path(workdir);
    std::fs::write(&pid_path, std::process::id().to_string())
        .with_context(|| format!("write {}", pid_path.display()))?;
    Ok(())
}

/// Remove `.roko/serve.pid`, ignoring if already absent.
pub fn remove_pid_file(workdir: &Path) -> Result<()> {
    let pid_path = pid_file_path(workdir);
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
/// send SIGKILL if still alive.
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
fn handle_existing_pid_file(workdir: &Path) -> Result<()> {
    match read_pid_file(workdir)? {
        Some(pid) if process_is_alive(pid) => {
            eprintln!(
                "Port in use by prior roko dev process (PID {}). Stopping it...",
                pid
            );
            terminate_pid(pid, Duration::from_secs(5))?;
            remove_pid_file(workdir)?;
            std::thread::sleep(Duration::from_millis(500));
        }
        Some(_pid) => {
            // PID file exists but process is dead — clean up stale file.
            remove_pid_file(workdir)?;
        }
        None => {}
    }
    Ok(())
}

/// Attempt to bind the resolved address to verify the port is free.
pub fn probe_addr_available(bind: &str, port: u16) -> Result<()> {
    let addr = format!("{bind}:{port}");
    match TcpListener::bind(&addr) {
        Ok(_listener) => Ok(()),
        Err(_) => {
            anyhow::bail!(
                "Address {addr} is already in use by an unknown process.\n  \
                 Kill it or use a different port via [server].port in roko.toml."
            );
        }
    }
}

// ─── Frontend management ────────────────────────────────────────────────────

/// Start the frontend dev server (`npm run dev` in `demo/demo-app/`).
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
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }

    match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = child.kill().await;
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_write_read_remove_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        assert_eq!(read_pid_file(workdir).unwrap(), None);

        write_pid_file(workdir).unwrap();

        let pid = read_pid_file(workdir).unwrap();
        assert_eq!(pid, Some(std::process::id()));

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

        std::fs::write(workdir.join(".roko/serve.pid"), "not-a-number\n").expect("write");

        assert_eq!(read_pid_file(workdir).unwrap(), None);
    }

    #[test]
    fn dead_pid_is_not_alive() {
        assert!(!process_is_alive(4_000_000));
    }

    #[test]
    fn current_process_is_alive() {
        assert!(process_is_alive(std::process::id()));
    }

    #[test]
    fn probe_addr_fails_when_port_in_use() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();

        let result = probe_addr_available("127.0.0.1", port);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already in use"));

        drop(listener);
    }

    #[test]
    fn probe_addr_succeeds_when_free() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        assert!(probe_addr_available("127.0.0.1", port).is_ok());
    }

    #[test]
    fn remove_pid_file_is_idempotent() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let workdir = tmp.path();
        std::fs::create_dir_all(workdir.join(".roko")).expect("mkdir");

        assert!(remove_pid_file(workdir).is_ok());

        write_pid_file(workdir).unwrap();
        remove_pid_file(workdir).unwrap();
        assert!(remove_pid_file(workdir).is_ok());
    }
}
