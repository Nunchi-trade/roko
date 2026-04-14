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
pub mod agent_config;
pub mod clean;
pub mod config;
pub mod config_cmd;
pub mod daemon;
pub mod episode;
pub mod event_sources;
pub mod index;
pub mod inject;
pub mod oneshot;
pub mod orchestrate;
pub mod pipe;
pub mod plan;
pub mod plan_generate;
pub mod prd;
pub mod prd_prompt;
pub mod prompting;
pub mod agent_spawn;
pub mod repl;
pub mod research;
pub mod run;
pub mod secrets;
pub mod status;
pub mod subscriptions;
pub mod task_parser;
pub mod tui;
pub mod worker;

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
    DashboardData, DashboardScaffold, DashboardSummary, PageId, PageScaffold, Theme, WidgetScaffold,
};
