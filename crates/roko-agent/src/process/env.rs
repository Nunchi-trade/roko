//! Agent environment configuration.
//!
//! Provides [`AgentEnv`] for declaring environment variables that should be set
//! (or cleared) on an agent subprocess, and [`apply_agent_env`] to stamp them
//! onto a [`tokio::process::Command`] before spawn.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// Environment configuration for an agent subprocess.
///
/// Stores key-value pairs to set, plus keys to explicitly unset (`env_remove`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEnv {
    /// Variables to set in the child environment.
    pub vars: HashMap<String, String>,
    /// Variables to remove from the inherited environment.
    pub remove: Vec<String>,
    /// Working directory for the child process.
    pub working_dir: Option<PathBuf>,
}

impl AgentEnv {
    /// Create an empty environment config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a variable to be set in the child.
    #[must_use]
    pub fn set(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// Mark a variable for removal from the child environment.
    #[must_use]
    pub fn unset(mut self, key: impl Into<String>) -> Self {
        self.remove.push(key.into());
        self
    }

    /// Set the working directory for the child.
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}

/// Apply an [`AgentEnv`] to a [`Command`] before spawning.
///
/// Sets each variable from `env.vars`, removes each key in `env.remove`,
/// and sets the working directory if specified.
pub fn apply_agent_env(cmd: &mut Command, env: &AgentEnv) {
    for (key, value) in &env.vars {
        cmd.env(key, value);
    }
    for key in &env.remove {
        cmd.env_remove(key);
    }
    if let Some(dir) = &env.working_dir {
        cmd.current_dir(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_env_builder_pattern() {
        let env = AgentEnv::new()
            .set("FOO", "bar")
            .set("BAZ", "qux")
            .unset("SECRET")
            .with_working_dir("/tmp");

        assert_eq!(env.vars.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(env.vars.get("BAZ"), Some(&"qux".to_string()));
        assert!(env.remove.contains(&"SECRET".to_string()));
        assert_eq!(env.working_dir, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn apply_agent_env_does_not_panic() {
        let env = AgentEnv::new().set("TEST_VAR", "hello").unset("PATH_EXTRA");
        let mut cmd = Command::new("echo");
        apply_agent_env(&mut cmd, &env);
        // Cannot inspect the command's env directly, but ensure no panic.
    }

    #[test]
    fn default_env_is_empty() {
        let env = AgentEnv::default();
        assert!(env.vars.is_empty());
        assert!(env.remove.is_empty());
        assert!(env.working_dir.is_none());
    }
}
