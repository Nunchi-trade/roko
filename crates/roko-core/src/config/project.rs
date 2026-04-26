//! Project and PRD configuration sections.

use serde::{Deserialize, Serialize};

use crate::task::TaskDomain;

// ---- [project] -----------------------------------------------------------

/// Project-level metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Human-readable project name.
    #[serde(default = "default_project_name")]
    pub name: String,
    /// Project root directory (relative or absolute).
    #[serde(default = "default_dot")]
    pub root: String,
    /// Git branch used as the base for fresh batch/worktree creation.
    #[serde(default = "default_fresh_base_branch")]
    pub fresh_base_branch: String,
    /// Default work domain for tasks that don't declare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_domain: Option<TaskDomain>,
}

fn default_project_name() -> String {
    "roko-project".into()
}

fn default_dot() -> String {
    ".".into()
}

fn default_fresh_base_branch() -> String {
    "main".into()
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: default_project_name(),
            root: default_dot(),
            fresh_base_branch: default_fresh_base_branch(),
            default_domain: None,
        }
    }
}

// ---- [prd] ---------------------------------------------------------------

/// PRD lifecycle settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrdConfig {
    /// Automatically generate a plan when a PRD is promoted.
    #[serde(default)]
    pub auto_plan: bool,
}

impl Default for PrdConfig {
    fn default() -> Self {
        Self { auto_plan: false }
    }
}
