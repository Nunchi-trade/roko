//! The `roko` binary's library surface.
//!
//! This crate wires Roko's primitives (Substrate, Composer, Agent, Gate,
//! Policy) into a one-shot CLI loop. It does **not** implement a plan runner
//! or DAG executor — it drives a single prompt through the universal loop
//! and writes the resulting signals to disk.
//!
//! See [`run_once`] for the core loop and [`Config`] for the `roko.toml`
//! schema.

#![allow(clippy::module_name_repetitions)]

extern crate self as roko_cli;

pub mod agent_exec;
pub mod clean;
pub mod config;
pub mod config_cmd;
pub mod daemon;
pub mod episode;
pub mod index;
pub mod inject;
pub mod oneshot;
pub mod orchestrate;
pub mod pipe;
pub mod plan;
pub mod plan_generate;
pub mod prd;
pub mod prd_prompt;
pub mod repl;
pub mod research;
pub mod run;
pub mod secrets;
pub mod status;
pub mod task_parser;
pub mod tui;
pub mod worker;

#[path = "../../roko-serve/src/deploy/mod.rs"]
pub mod deploy;
#[path = "../../roko-serve/src/error.rs"]
pub mod error;
#[path = "../../roko-serve/src/events.rs"]
pub mod events;
#[path = "../../roko-serve/src/routes/mod.rs"]
pub mod routes;
#[path = "../../roko-serve/src/state.rs"]
pub mod state;
#[path = "../../roko-serve/src/templates.rs"]
pub mod templates;

/// Backwards-compatible namespace for server-related modules.
pub mod serve {
    pub use crate::deploy;
    pub use crate::error;
    pub use crate::events;
    pub use crate::routes;
    pub use crate::state;
    pub use crate::templates;
}

pub use config::{
    AgentConfig, Config, ConfigLayer, ConfigPaths, ConfigSources, GateConfig, PromptConfig,
    PromptFile, ResolvedConfig, ServeAuthLayer, ServeLayer, Source, ToolsConfig, load_layered,
};
pub use config_cmd::{EditTarget, WizardInputs, run_init_wizard};
pub use daemon::{DaemonConfig, DaemonMode, DaemonState, DaemonStatus};
pub use episode::EpisodePolicy;
pub use inject::{InjectKind, InjectRequest};
pub use oneshot::{OneshotMode, OneshotResult};
pub use orchestrate::{OrchestrationReport, PlanRunReport, PlanRunner};
pub use pipe::{PipeInput, PipeMode, stdin_is_tty};
pub use plan::{Plan, PlanSummary, PlanTask};
pub use repl::{ReplCommand, ReplMode};
pub use run::{RunReport, run_once};
pub use secrets::SecretsCmd;
pub use status::SessionStatus;
pub use tui::{
    DashboardData, DashboardScaffold, DashboardSummary, PageId, PageScaffold, Theme,
    WidgetScaffold,
};
