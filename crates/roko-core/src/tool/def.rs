//! Tool definition types: [`ToolDef`], [`ToolSchema`], [`ToolCategory`],
//! [`ToolPermission`], [`ToolConcurrency`].
//!
//! A [`ToolDef`] describes **what** a tool is (name, schema, category,
//! required capabilities, concurrency, timeout). Tool *execution* lives
//! elsewhere: see [`crate::tool::ToolHandler`] for the async handler
//! contract and [`crate::tool::ToolContext`] for the runtime environment
//! that is threaded through every invocation.
//!
//! Tool *registration* (mapping names to [`ToolDef`]s) lives in the
//! [`crate::tool::ToolRegistry`] trait; a concrete registry with the
//! built-in 16 tools is built in `roko-std` (see §36.9 of the parity
//! checklist).

use serde::{Deserialize, Serialize};

use crate::ToolPermissions;

// ─── ToolCategory ─────────────────────────────────────────────────────────

/// Broad category of a tool — which subsystem it touches.
///
/// Categories map loosely to the [`ToolPermission`] flags a tool needs at
/// runtime: `Read` tools require `read`, `Write`/`Notebook` tools require
/// `write`, `Exec` tools require `exec`, etc. Categories are primarily a
/// documentation/UI concern; permission enforcement uses the explicit
/// [`ToolDef::permission`] field, not the category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolCategory {
    /// Reads files, globs, or searches content (no mutation).
    Read,
    /// Writes or edits files in the worktree.
    Write,
    /// Executes shell commands or subprocesses.
    Exec,
    /// Performs git operations (branch/commit/merge).
    Git,
    /// Makes outbound network requests.
    Network,
    /// Meta tools (task/agent delegation, plan-mode control).
    Meta,
    /// Notebook (`.ipynb`) cell editing.
    Notebook,
    /// MCP-backed tools (registered dynamically from an MCP server).
    Mcp,
}

impl ToolCategory {
    /// Stable short identifier for logs / TUI columns.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Exec => "exec",
            Self::Git => "git",
            Self::Network => "network",
            Self::Meta => "meta",
            Self::Notebook => "notebook",
            Self::Mcp => "mcp",
        }
    }
}

// ─── ToolConcurrency ──────────────────────────────────────────────────────

/// Whether two invocations of this tool can run concurrently.
///
/// Used by the dispatcher (§36.41) when the LLM emits multiple tool calls
/// in one turn: `Parallel` tools join via `join_all`, `Serial` tools run
/// one at a time to avoid races (e.g. shell-state interleaving).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolConcurrency {
    /// Runs serially — avoid racing writes or shared process state.
    Serial,
    /// Safe to run concurrently with peers (read-only / per-file).
    Parallel,
}

// ─── ToolPermission ───────────────────────────────────────────────────────

/// The set of capability flags a tool **requires** from the role's
/// [`ToolPermissions`].
///
/// Each field is independently `true` when the tool needs that capability.
/// Enforcement (§36.46) calls [`ToolPermission::satisfied_by`] against the
/// role's [`ToolPermissions`]; any missing flag yields
/// [`ToolError::PermissionDenied`](crate::tool::ToolError::PermissionDenied).
///
/// This mirrors the shape of [`ToolPermissions`] (see §6.2 of the parity
/// checklist) but is semantically distinct: [`ToolPermissions`] is what a
/// role *grants*, [`ToolPermission`] is what a tool *needs*.
#[allow(clippy::struct_excessive_bools)] // mirrors ToolPermissions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ToolPermission {
    /// Requires filesystem read access.
    pub read: bool,
    /// Requires filesystem write access.
    pub write: bool,
    /// Requires shell execution.
    pub exec: bool,
    /// Requires git operations.
    pub git: bool,
    /// Requires outbound network access.
    pub network: bool,
}

impl ToolPermission {
    /// A tool that only needs read access (e.g. `read_file`, `grep`).
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            exec: false,
            git: false,
            network: false,
        }
    }

    /// A tool that needs read + write (e.g. `write_file`, `edit_file`).
    #[must_use]
    pub const fn writes() -> Self {
        Self {
            read: true,
            write: true,
            exec: false,
            git: false,
            network: false,
        }
    }

    /// A tool that needs read + exec (e.g. `bash`, `run_tests`).
    #[must_use]
    pub const fn executes() -> Self {
        Self {
            read: true,
            write: false,
            exec: true,
            git: false,
            network: false,
        }
    }

    /// A tool that needs read + write + exec (e.g. `multi_edit` + post-hook).
    #[must_use]
    pub const fn writes_and_executes() -> Self {
        Self {
            read: true,
            write: true,
            exec: true,
            git: false,
            network: false,
        }
    }

    /// A tool that needs git (e.g. branch protection, merge-resolver).
    #[must_use]
    pub const fn git_ops() -> Self {
        Self {
            read: true,
            write: true,
            exec: true,
            git: true,
            network: false,
        }
    }

    /// A tool that needs network access (e.g. `web_fetch`, `web_search`).
    #[must_use]
    pub const fn networked() -> Self {
        Self {
            read: true,
            write: false,
            exec: false,
            git: false,
            network: true,
        }
    }

    /// Check that every required flag on `self` is granted by `perms`.
    ///
    /// A tool is allowed iff every flag it sets is also set on the role's
    /// [`ToolPermissions`]. Flags the tool *doesn't* set are ignored —
    /// a read-only tool doesn't care whether the role has `write` or not.
    #[must_use]
    pub const fn satisfied_by(self, perms: &ToolPermissions) -> bool {
        (!self.read || perms.read)
            && (!self.write || perms.write)
            && (!self.exec || perms.exec)
            && (!self.git || perms.git)
            && (!self.network || perms.network)
    }
}

// ─── ToolSchema ───────────────────────────────────────────────────────────

/// JSON Schema describing a tool's input arguments.
///
/// The schema is the ground truth for argument validation (§36.42) and
/// is sent verbatim to raw backends (Ollama/OpenAI) as the tool's
/// `parameters` field. For hosted backends (Claude CLI) it is advisory —
/// the CLI owns the canonical schema and Roko's copy is used only for
/// client-side validation before dispatch.
#[allow(clippy::derive_partial_eq_without_eq)] // serde_json::Value isn't Eq (has f64)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolSchema(pub serde_json::Value);

impl ToolSchema {
    /// Wrap a raw `serde_json::Value` as a schema.
    #[must_use]
    pub const fn from_value(v: serde_json::Value) -> Self {
        Self(v)
    }

    /// A permissive schema accepting any JSON object with arbitrary keys.
    ///
    /// Useful for prototype tools and as a default. Real tools should
    /// provide a concrete schema so invalid calls fail fast.
    #[must_use]
    pub fn any_object() -> Self {
        Self(serde_json::json!({
            "type": "object",
            "additionalProperties": true,
        }))
    }

    /// Underlying JSON value.
    #[must_use]
    pub const fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}

impl Default for ToolSchema {
    fn default() -> Self {
        Self::any_object()
    }
}

// ─── ToolDef ──────────────────────────────────────────────────────────────

/// Schema + metadata for one callable tool.
///
/// A [`ToolDef`] is everything a dispatcher needs to validate, authorize,
/// execute, and audit a tool call — **except** the handler implementation
/// itself (see [`ToolHandler`](crate::tool::ToolHandler)).
///
/// # Wire format
///
/// [`ToolDef`] is the canonical form; per-backend translators (§36.c)
/// convert it into each backend's expected tool shape (Claude `--tools`
/// CSV, Codex MCP config, Ollama OpenAI-compatible `tools` array, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Canonical snake_case name (see [`crate::tool::aliases`]).
    pub name: String,
    /// Human-readable description — sent to the LLM as the tool's help text.
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub parameters: ToolSchema,
    /// Which subsystem the tool touches.
    pub category: ToolCategory,
    /// Capability flags the tool requires at dispatch.
    pub permission: ToolPermission,
    /// Maximum wall-clock duration before the dispatcher cancels the call.
    pub timeout_ms: u64,
    /// Whether the tool is safe to run concurrently with peers.
    pub concurrency: ToolConcurrency,
    /// Whether calling the tool twice with identical arguments has no
    /// additional side effects (e.g. `read_file` is idempotent, `bash` is not).
    pub idempotent: bool,
}

impl ToolDef {
    /// Construct a [`ToolDef`] with sensible defaults:
    /// - schema: [`ToolSchema::any_object`]
    /// - timeout: 60 seconds
    /// - concurrency: [`ToolConcurrency::Parallel`]
    /// - idempotent: `false`
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        category: ToolCategory,
        permission: ToolPermission,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: ToolSchema::any_object(),
            category,
            permission,
            timeout_ms: 60_000,
            concurrency: ToolConcurrency::Parallel,
            idempotent: false,
        }
    }

    /// Set the argument schema.
    #[must_use]
    pub fn with_parameters(mut self, schema: ToolSchema) -> Self {
        self.parameters = schema;
        self
    }

    /// Set the timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Set the concurrency policy.
    #[must_use]
    pub const fn with_concurrency(mut self, concurrency: ToolConcurrency) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Mark the tool idempotent (or not).
    #[must_use]
    pub const fn with_idempotent(mut self, idempotent: bool) -> Self {
        self.idempotent = idempotent;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_category_stable_strings() {
        assert_eq!(ToolCategory::Read.as_str(), "read");
        assert_eq!(ToolCategory::Notebook.as_str(), "notebook");
        assert_eq!(ToolCategory::Mcp.as_str(), "mcp");
    }

    #[test]
    fn tool_category_serde_roundtrip() {
        let json = serde_json::to_string(&ToolCategory::Exec).unwrap();
        assert_eq!(json, "\"exec\"");
        let decoded: ToolCategory = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ToolCategory::Exec);
    }

    #[test]
    fn tool_concurrency_serde_roundtrip() {
        let json = serde_json::to_string(&ToolConcurrency::Serial).unwrap();
        assert_eq!(json, "\"serial\"");
        let decoded: ToolConcurrency = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ToolConcurrency::Serial);
    }

    #[test]
    fn read_only_perm_satisfied_by_readonly_role() {
        let role = ToolPermissions::read_only();
        assert!(ToolPermission::read_only().satisfied_by(&role));
    }

    #[test]
    fn write_perm_not_satisfied_by_readonly_role() {
        let role = ToolPermissions::read_only();
        assert!(!ToolPermission::writes().satisfied_by(&role));
    }

    #[test]
    fn exec_perm_satisfied_by_read_exec_role() {
        let role = ToolPermissions::read_exec();
        assert!(ToolPermission::executes().satisfied_by(&role));
        // Read-exec does not grant write.
        assert!(!ToolPermission::writes().satisfied_by(&role));
    }

    #[test]
    fn git_perm_requires_git_flag() {
        let mut role = ToolPermissions::full();
        role.git = false;
        assert!(!ToolPermission::git_ops().satisfied_by(&role));
        role.git = true;
        assert!(ToolPermission::git_ops().satisfied_by(&role));
    }

    #[test]
    fn network_perm_requires_network_flag() {
        let role = ToolPermissions::networked();
        assert!(ToolPermission::networked().satisfied_by(&role));
        assert!(!ToolPermission::networked().satisfied_by(&ToolPermissions::read_only()));
    }

    #[test]
    fn tool_schema_default_is_any_object() {
        let s = ToolSchema::default();
        let v = s.as_value();
        assert_eq!(v["type"], "object");
        assert_eq!(v["additionalProperties"], true);
    }

    #[test]
    fn tool_def_new_has_sensible_defaults() {
        let t = ToolDef::new(
            "read_file",
            "Read a UTF-8 file",
            ToolCategory::Read,
            ToolPermission::read_only(),
        );
        assert_eq!(t.name, "read_file");
        assert_eq!(t.timeout_ms, 60_000);
        assert_eq!(t.concurrency, ToolConcurrency::Parallel);
        assert!(!t.idempotent);
    }

    #[test]
    fn tool_def_builder_chaining() {
        let schema = ToolSchema::from_value(serde_json::json!({"type": "object"}));
        let t = ToolDef::new(
            "bash",
            "run a command",
            ToolCategory::Exec,
            ToolPermission::executes(),
        )
        .with_parameters(schema.clone())
        .with_timeout_ms(120_000)
        .with_concurrency(ToolConcurrency::Serial)
        .with_idempotent(false);
        assert_eq!(t.parameters, schema);
        assert_eq!(t.timeout_ms, 120_000);
        assert_eq!(t.concurrency, ToolConcurrency::Serial);
    }

    #[test]
    fn tool_def_serde_roundtrip() {
        let t = ToolDef::new(
            "glob",
            "glob matcher",
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
        .with_timeout_ms(5_000);
        let json = serde_json::to_string(&t).unwrap();
        let decoded: ToolDef = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, t);
    }
}
