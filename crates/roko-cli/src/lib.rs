//! The `roko` binary's library surface.
//!
//! This crate wires Roko's primitives (Store, Compose, Agent, Verify,
//! React) into a one-shot CLI loop. It does **not** implement a plan runner
//! or DAG executor — it drives a single prompt through the universal loop
//! and writes the resulting signals to disk.
//!
//! See [`run_once`] for the core loop and [`Config`] for the `roko.toml`
//! schema.

#![allow(clippy::module_name_repetitions)]
#![allow(missing_docs)]
#![cfg_attr(
    clippy,
    allow(
        clippy::all,
        clippy::pedantic,
        clippy::nursery,
        clippy::restriction,
        missing_docs
    )
)]

extern crate self as roko_cli;

/// Canonical default port for the shipping `roko-serve` control plane.
pub const DEFAULT_SERVE_PORT: u16 = 6677;
/// Canonical default base URL for CLI and TUI calls into `roko-serve`.
pub const DEFAULT_SERVE_URL: &str = "http://localhost:6677";

pub mod agent_config;
pub mod agent_episode;
pub mod agent_exec;
pub mod agent_spawn;
pub mod auth;
pub mod bench;
pub mod chain_handler;
pub mod chain_registry;
pub mod chat;
pub mod clean;
pub mod config;
pub mod config_cmd;
pub mod config_helpers;
pub mod credentials;
pub mod custody;
pub mod daemon;
pub mod deployment;
pub(crate) mod dispatch_helpers;
pub mod dispatch_v2;
pub mod doctor;
pub mod episode;
pub mod event_sources;
pub mod explain;
pub(crate) mod gate_runner;
mod heartbeat;
pub mod index;
pub mod inject;
pub(crate) mod knowledge_helpers;
pub(crate) mod learning_helpers;
pub mod oneshot;
pub mod orchestrate;
pub mod pipe;
pub mod plan;
pub mod plan_generate;
pub mod prd;
pub mod prd_prompt;
pub mod prompt_helpers;
pub mod prompting;
pub mod repl;
pub mod research;
pub mod run;
pub mod runner;
pub mod scaffold;
pub mod secrets;
pub mod snapshot_migrate;
pub mod snapshot_reconcile;
pub mod status;
pub mod subscriptions;
pub mod surface_inventory;
pub mod task_helpers;
pub mod task_parser;
pub mod tui;
pub mod vision_loop;
pub mod worker;
pub mod workspace_paths;

pub mod serve_runtime;

/// Server modules re-exported from the `roko-serve` crate.
pub use roko_serve as serve;

pub use config::{
    AgentConfig, Config, ConfigLayer, ConfigPaths, ConfigSources, DreamsConfig, GateConfig,
    PromptConfig, PromptFile, RepoEntry, RepoRegistry, ResolvedConfig, ServeAuthLayer, ServeLayer,
    Source, ToolsConfig, load_layered,
};
pub use config_cmd::{EditTarget, WizardInputs, run_init_wizard};
pub use daemon::{DaemonConfig, DaemonMode, DaemonState, DaemonStatus};
pub use deployment::SigstoreVerifier;
pub use episode::EpisodePolicy;
pub use inject::{InjectKind, InjectRequest};
pub use oneshot::{OneshotMode, OneshotResult};
pub use orchestrate::{OrchestrationReport, PlanRunReport, PlanRunner};
pub use pipe::{PipeInput, PipeMode, stdin_is_tty};
pub use plan::{Plan, PlanSummary, PlanTask};
pub use repl::{ReplCommand, ReplMode, WorkspaceContext};
pub use run::{RunReport, run_once};
pub use secrets::SecretsCmd;
pub use status::SessionStatus;
pub use tui::{
    DashboardData, DashboardScaffold, DashboardSummary, PageId, PageScaffold, Theme, WidgetScaffold,
};
