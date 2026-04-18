//! `.mcp.json` walk-up config reader (SS36.61).
//!
//! Searches upward from a starting directory to find the nearest
//! `.mcp.json` config file, then parses it into [`McpConfig`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerConfig {
    /// Logical name for the server (used as prefix in tool names).
    pub name: String,
    /// The command to spawn the server process.
    pub command: String,
    /// Arguments passed to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables set for the server process.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Top-level MCP configuration parsed from `.mcp.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    /// List of MCP server configurations.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpConfig {
    /// Load and parse an MCP config from an explicit file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or is not valid JSON.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        load_config(path).map(|(_path, cfg)| cfg)
    }
}

/// Walk up from `start_dir` looking for a `.mcp.json` file.
///
/// Checks `start_dir`, then its parent, then grandparent, etc. until
/// the filesystem root. Returns the parsed config and its path on
/// success.
///
/// # Errors
///
/// Returns `None` if no `.mcp.json` is found. Returns `Some(Err(...))`
/// if a file is found but cannot be read or parsed.
pub fn find_mcp_config(start_dir: &Path) -> Option<Result<(PathBuf, McpConfig), ConfigError>> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let candidate = dir.join(".mcp.json");
        if candidate.is_file() {
            return Some(load_config(&candidate));
        }
        if !dir.pop() {
            break;
        }
    }

    let home_candidate = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".mcp.json"));
    if let Some(candidate) = home_candidate.filter(|candidate| candidate.is_file()) {
        return Some(load_config(&candidate));
    }

    None
}

/// Load and parse a `.mcp.json` file.
fn load_config(path: &Path) -> Result<(PathBuf, McpConfig), ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|e| ConfigError::Io {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    let config: McpConfig = serde_json::from_str(&content).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    Ok((path.to_path_buf(), config))
}

/// Errors from MCP config loading.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// Could not read the config file.
    #[error("failed to read {path}: {detail}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error description.
        detail: String,
    },

    /// Could not parse the config file as JSON.
    #[error("failed to parse {path}: {detail}")]
    Parse {
        /// Path that failed.
        path: PathBuf,
        /// Underlying error description.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mcp_config_parse_full() {
        let json = r#"{
            "servers": [
                {
                    "name": "filesystem",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "env": {"HOME": "/tmp"}
                },
                {
                    "name": "git",
                    "command": "mcp-git",
                    "args": []
                }
            ]
        }"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.servers.len(), 2);
        assert_eq!(config.servers[0].name, "filesystem");
        assert_eq!(config.servers[0].command, "npx");
        assert_eq!(
            config.servers[0].args,
            vec!["-y", "@modelcontextprotocol/server-filesystem"]
        );
        assert_eq!(config.servers[0].env.get("HOME").unwrap(), "/tmp");
        assert_eq!(config.servers[1].name, "git");
        assert!(config.servers[1].env.is_empty());
    }

    #[test]
    fn mcp_config_parse_empty_servers() {
        let json = r#"{"servers": []}"#;
        let config: McpConfig = serde_json::from_str(json).unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn mcp_find_config_in_start_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "test".to_string(),
                command: "echo".to_string(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        let config_path = tmp.path().join(".mcp.json");
        fs::write(&config_path, serde_json::to_string(&config).unwrap()).unwrap();

        let result = find_mcp_config(tmp.path());
        assert!(result.is_some());
        let (path, parsed) = result.unwrap().unwrap();
        assert_eq!(path, config_path);
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "test");
    }

    #[test]
    fn mcp_find_config_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        let child = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&child).unwrap();

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "parent".to_string(),
                command: "cat".to_string(),
                args: vec![],
                env: HashMap::new(),
            }],
        };
        let config_path = tmp.path().join(".mcp.json");
        fs::write(&config_path, serde_json::to_string(&config).unwrap()).unwrap();

        let result = find_mcp_config(&child);
        assert!(result.is_some());
        let (path, parsed) = result.unwrap().unwrap();
        assert_eq!(path, config_path);
        assert_eq!(parsed.servers[0].name, "parent");
    }

    #[test]
    fn mcp_find_config_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let child = tmp.path().join("no_config_here");
        fs::create_dir_all(&child).unwrap();
        let result = find_mcp_config(&child);
        assert!(result.is_none());
    }

    #[test]
    fn mcp_find_config_invalid_json() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join(".mcp.json");
        fs::write(&config_path, "not valid json!!!").unwrap();

        let result = find_mcp_config(tmp.path());
        assert!(result.is_some());
        let err = result.unwrap().unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
    }
}
