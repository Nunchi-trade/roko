//! ACP (Agent Client Protocol) server for Roko.
//!
//! Implements the ACP JSON-RPC 2.0 protocol over stdio, enabling Roko
//! to work as a coding agent from any ACP-compatible editor (JetBrains,
//! Zed, Neovim, VS Code, etc.).

pub mod bridge_events;
pub mod bridge_fs;
pub mod bridge_gates;
pub mod bridge_plan;
pub mod bridge_terminal;
pub mod bridge_usage;
pub mod commands;
pub mod config;
pub mod config_options;
pub mod elicitation;
pub mod handler;
pub mod permissions;
pub mod session;
pub mod transport;
pub mod types;
