//! ACP (Agent Client Protocol) server for Roko.
//!
//! Implements the ACP JSON-RPC 2.0 protocol over stdio, enabling Roko
//! to work as a coding agent from any ACP-compatible editor (JetBrains,
//! Zed, Neovim, VS Code, etc.).

pub mod acp_adapter;
pub mod builtin_tools;
pub mod bridge_events;
pub mod config;
pub mod config_watch;
pub mod event_forward;
pub mod handler;
pub(crate) mod knowledge;
pub mod pipeline;
pub mod runner;
pub mod session;
pub mod transport;
pub mod types;
pub mod workflow;

pub use config::AcpConfig;
pub use handler::run_acp_server;
