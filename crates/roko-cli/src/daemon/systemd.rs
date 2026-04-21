use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, anyhow};

/// systemd user-service name for the Roko daemon.
pub const SERVICE_NAME: &str = "roko.service";

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("resolve home directory"))
}

fn exe_path() -> PathBuf {
    env::current_exe().unwrap_or_else(|_| PathBuf::from("roko"))
}

/// Return the user-level systemd unit path for the daemon.
pub fn unit_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir().unwrap_or(home_dir()?.join(".config"));
    Ok(config_dir.join("systemd").join("user").join(SERVICE_NAME))
}

/// Generate a systemd user unit for the daemon at the given HTTP port.
///
/// The unit mirrors the deployment docs: user-scoped service management,
/// journald logging, bounded restart backoff, watchdog metadata, and a
/// hardened writable surface for Roko config and state.
pub fn generate_unit(port: u16) -> Result<String> {
    let binary = exe_path();
    let home = home_dir()?;
    let log_level = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());

    Ok(format!(
        r#"[Unit]
Description=Roko cognitive agent daemon
Documentation=https://github.com/nunchi/roko
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={binary} daemon start --foreground --port {port}

Restart=on-failure
RestartSec=10
RestartMaxDelaySec=300
RestartSteps=5

Environment=RUST_LOG={log_level}
Environment=HOME={home}
EnvironmentFile=-{home}/.config/roko/daemon.env

LimitNOFILE=4096

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths={home}/.roko {home}/.local/state/roko {home}/.config/roko
PrivateTmp=true

WorkingDirectory={home}

WatchdogSec=60

[Install]
WantedBy=default.target
"#,
        binary = binary.display(),
        home = home.display(),
        log_level = escape_systemd_env_value(&log_level),
    ))
}

/// Install and start the systemd user service for the Roko daemon.
pub fn install_systemd(port: u16) -> Result<()> {
    let unit_path = unit_path()?;
    let unit_dir = unit_path.parent().context("resolve systemd user dir")?;
    std::fs::create_dir_all(unit_dir).with_context(|| format!("create {}", unit_dir.display()))?;

    let unit = generate_unit(port)?;
    std::fs::write(&unit_path, unit).with_context(|| format!("write {}", unit_path.display()))?;

    run_systemctl(&["daemon-reload"])?;
    run_systemctl(&["enable", SERVICE_NAME])?;
    run_systemctl(&["start", SERVICE_NAME])?;

    Ok(())
}

/// Stop, disable, and remove the systemd user service for the Roko daemon.
pub fn uninstall_systemd() -> Result<()> {
    let unit_path = unit_path()?;

    let _ = run_systemctl(&["stop", SERVICE_NAME]);
    let _ = run_systemctl(&["disable", SERVICE_NAME]);

    if unit_path.exists() {
        std::fs::remove_file(&unit_path)
            .with_context(|| format!("remove {}", unit_path.display()))?;
    }

    run_systemctl(&["daemon-reload"])?;
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .with_context(|| format!("run systemctl --user {}", args.join(" ")))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "systemctl --user {} failed with {status}",
            args.join(" ")
        ))
    }
}

fn escape_systemd_env_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_unit_matches_deployment_contract() {
        let unit = generate_unit(9090).unwrap();
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("Description=Roko cognitive agent daemon"));
        assert!(unit.contains("ExecStart="));
        assert!(unit.contains("daemon start --foreground --port 9090"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("EnvironmentFile=-"));
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("WatchdogSec=60"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn unit_path_uses_systemd_user_directory() {
        let path = unit_path().unwrap();
        assert!(path.ends_with("systemd/user/roko.service"));
    }

    #[test]
    fn systemd_env_escape_handles_quotes_and_slashes() {
        assert_eq!(
            escape_systemd_env_value(r#"roko="info""#),
            r#"roko=\"info\""#
        );
        assert_eq!(escape_systemd_env_value(r#"a\b"#), r#"a\\b"#);
    }
}
