//! MCP (Model Context Protocol) server launch discovery and configuration.
//!
//! An MCP server is always spawned as a **child of the agent** (never of Roko
//! itself). This module handles:
//! - Discovering the MCP config by walking up from the working directory.
//! - Normalizing the launch command (resolving relative paths, falling back
//!   to `cargo run`).
//! - Writing/updating MCP config files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A resolved MCP server launch specification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpLaunch {
    /// The command to execute (e.g. an absolute path to a binary, or `cargo`).
    pub command: String,
    /// Arguments to pass to the command.
    pub args: Vec<String>,
}

/// Load an MCP launch config from a JSON file with `mcpServers.roko` shape.
///
/// Expected format:
/// ```json
/// { "mcpServers": { "roko": { "command": "...", "args": [...] } } }
/// ```
fn load_json_mcp_launch(path: &Path) -> Option<McpLaunch> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let server = value.get("mcpServers")?.get("roko")?;
    let command = server.get("command")?.as_str()?.to_string();
    let args = server
        .get("args")
        .and_then(|a| a.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(McpLaunch { command, args })
}

/// Load an MCP launch config from a TOML file with `mcp_servers.roko` shape.
///
/// Expected format:
/// ```toml
/// [mcp_servers.roko]
/// command = "..."
/// args = ["..."]
/// ```
fn load_toml_mcp_launch(path: &Path) -> Option<McpLaunch> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value: toml::Value = raw.parse().ok()?;
    let server = value.get("mcp_servers")?.get("roko")?;
    let command = server.get("command")?.as_str()?.to_string();
    let args = server
        .get("args")
        .and_then(|a| a.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(McpLaunch { command, args })
}

/// Check whether the command basename is `roko-mcp`.
fn command_looks_like_roko_mcp(command: &str) -> bool {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .map_or(command == "roko-mcp", |name| name == "roko-mcp")
}

/// Walk ancestor directories looking for `target/{release,debug}/roko-mcp`.
fn find_roko_mcp_binary_from_ancestors(start: &Path) -> Option<String> {
    for dir in start.ancestors() {
        for rel in ["target/release/roko-mcp", "target/debug/roko-mcp"] {
            let candidate = dir.join(rel);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Check whether the command exists at the given path (absolute or relative to `probe_root`).
fn mcp_command_exists(command: &str, probe_root: &Path) -> bool {
    let command_path = Path::new(command);
    if command_path.is_absolute() {
        return command_path.exists();
    }
    probe_root.join(command_path).exists()
}

/// Build a `cargo run -p roko-mcp --release --` launch.
fn cargo_run_roko_mcp_launch(args: Vec<String>) -> McpLaunch {
    let mut launch_args = vec![
        "run".to_string(),
        "-p".to_string(),
        "roko-mcp".to_string(),
        "--release".to_string(),
        "--".to_string(),
    ];
    launch_args.extend(args);
    McpLaunch {
        command: "cargo".to_string(),
        args: launch_args,
    }
}

/// Normalize a discovered MCP launch.
///
/// If the command looks like `roko-mcp` but the binary doesn't exist at the
/// configured path, try to resolve it from ancestor `target/` directories or
/// fall back to `cargo run`.
pub fn normalize_mcp_launch(launch: McpLaunch, probe_root: &Path) -> McpLaunch {
    if command_looks_like_roko_mcp(&launch.command)
        && (!mcp_command_exists(&launch.command, probe_root) || launch.command == "roko-mcp")
    {
        if let Some(command) = find_roko_mcp_binary_from_ancestors(probe_root) {
            return McpLaunch {
                command,
                args: launch.args,
            };
        }
        return cargo_run_roko_mcp_launch(launch.args);
    }
    launch
}

/// Search for an MCP config starting from `working_dir` and walking up.
///
/// Checks (in order at each directory level):
/// 1. `$ROKO_MCP_CONFIG` env var (explicit override)
/// 2. `.roko/mcp-config.local.json`
/// 3. `.roko/mcp-config.json`
/// 4. `.codex/config.toml`
///
/// Returns the first valid config found, normalized via [`normalize_mcp_launch`].
pub fn find_mcp_launch(working_dir: &Path) -> Option<McpLaunch> {
    // Check explicit env override first.
    if let Ok(config_path) = std::env::var("ROKO_MCP_CONFIG") {
        let explicit = Path::new(&config_path);
        if explicit.exists() {
            return load_json_mcp_launch(explicit).map(|launch| {
                normalize_mcp_launch(launch, explicit.parent().unwrap_or(working_dir))
            });
        }
    }

    let mut search: Option<&Path> = Some(working_dir);
    while let Some(dir) = search {
        for rel in [".roko/mcp-config.local.json", ".roko/mcp-config.json"] {
            let candidate = dir.join(rel);
            if candidate.exists() {
                if let Some(config) = load_json_mcp_launch(&candidate) {
                    return Some(normalize_mcp_launch(config, dir));
                }
            }
        }
        // Also check codex-style TOML config.
        let codex_candidate = dir.join(".codex/config.toml");
        if codex_candidate.exists() {
            if let Some(config) = load_toml_mcp_launch(&codex_candidate) {
                return Some(normalize_mcp_launch(config, dir));
            }
        }
        search = dir.parent();
    }
    None
}

/// Write an MCP config JSON file for an agent to discover.
///
/// Creates the file at `<base>/.roko/mcp-config.json` with the standard
/// `mcpServers.roko` shape.
pub fn write_mcp_config(base: &Path, launch: &McpLaunch) -> Result<PathBuf, std::io::Error> {
    let dir = base.join(".roko");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("mcp-config.json");
    let value = serde_json::json!({
        "mcpServers": {
            "roko": {
                "command": launch.command,
                "args": launch.args,
            }
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&value).unwrap_or_default())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_looks_like_roko_mcp_checks_basename() {
        assert!(command_looks_like_roko_mcp("roko-mcp"));
        assert!(command_looks_like_roko_mcp("/usr/local/bin/roko-mcp"));
        assert!(command_looks_like_roko_mcp("target/release/roko-mcp"));
        assert!(!command_looks_like_roko_mcp("cargo"));
        assert!(!command_looks_like_roko_mcp("node"));
    }

    #[test]
    fn normalize_falls_back_to_cargo_run() {
        let launch = McpLaunch {
            command: "roko-mcp".to_string(),
            args: vec!["--port".to_string(), "9999".to_string()],
        };
        // probe_root is /tmp which won't have target/release/roko-mcp
        let normalized = normalize_mcp_launch(launch, Path::new("/tmp"));
        assert_eq!(normalized.command, "cargo");
        assert!(normalized.args.contains(&"roko-mcp".to_string()));
        assert!(normalized.args.contains(&"--port".to_string()));
    }

    #[test]
    fn write_and_read_mcp_config_roundtrips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let launch = McpLaunch {
            command: "/usr/bin/roko-mcp".to_string(),
            args: vec!["--verbose".to_string()],
        };
        let path = write_mcp_config(tmp.path(), &launch).expect("write");
        assert!(path.exists());

        let loaded = load_json_mcp_launch(&path).expect("load");
        assert_eq!(loaded.command, launch.command);
        assert_eq!(loaded.args, launch.args);
    }

    #[test]
    fn find_mcp_launch_returns_none_in_empty_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = find_mcp_launch(tmp.path());
        assert!(result.is_none());
    }

    #[test]
    fn find_mcp_launch_discovers_written_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Use a non-roko-mcp command name so normalization does not rewrite it.
        let launch = McpLaunch {
            command: "/usr/bin/custom-mcp-server".to_string(),
            args: vec!["--port".to_string(), "8080".to_string()],
        };
        write_mcp_config(tmp.path(), &launch).expect("write");
        let found = find_mcp_launch(tmp.path());
        assert!(found.is_some());
        let found = found.expect("should find config");
        assert_eq!(found.command, "/usr/bin/custom-mcp-server");
        assert_eq!(found.args, vec!["--port", "8080"]);
    }

    #[test]
    fn cargo_run_launch_includes_release_flag() {
        let launch = cargo_run_roko_mcp_launch(vec!["--flag".to_string()]);
        assert_eq!(launch.command, "cargo");
        assert!(launch.args.contains(&"--release".to_string()));
        assert!(launch.args.contains(&"--flag".to_string()));
    }
}
