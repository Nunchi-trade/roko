//! Process supervision primitives for agent subprocesses.
//!
//! This module provides cross-platform (with Unix-optimized) utilities for
//! managing the lifecycle of agent child processes:
//!
//! - **Process groups** ([`group`]): place agents in dedicated groups so their
//!   entire tree can be signaled atomically.
//! - **Kill escalation** ([`kill`]): stdin-close → SIGTERM → SIGKILL with
//!   configurable grace periods.
//! - **PID registry** ([`registry`]): global in-memory + disk-persisted set of
//!   spawned PIDs for orphan cleanup across restarts.
//! - **MCP discovery** ([`mcp`]): walk-up config search for MCP server launch
//!   specifications.
//! - **Agent environment** ([`env`]): structured env-var configuration for
//!   child processes.
//! - **Stderr suppression** ([`stderr`]): classify and deduplicate benign
//!   agent stderr noise.
//!
//! # Platform support
//!
//! All Unix-specific code (`setpgid`, `libc::kill`, `pgrep -P`) is gated
//! behind `#[cfg(unix)]`. Non-Unix targets get no-op stubs that compile
//! cleanly but perform no process-management.

pub mod env;
pub mod group;
pub mod kill;
pub mod mcp;
pub mod registry;
pub mod stderr;

// Re-export the primary public API surface for convenience.
pub use env::{apply_agent_env, AgentEnv};
pub use group::{collect_descendants, kill_process_group, set_process_group};
pub use kill::kill_tree;
pub use mcp::{find_mcp_launch, normalize_mcp_launch, write_mcp_config, McpLaunch};
pub use registry::{
    cleanup_orphaned_agents, reap_orphaned_children, register_spawned_pid, registered_pids,
    unregister_pid,
};
pub use stderr::{benign_stderr_warn_once, classify_benign_stderr, BenignStderr};
