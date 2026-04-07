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

pub mod clean;
pub mod config;
pub mod config_cmd;
pub mod daemon;
pub mod episode;
pub mod inject;
pub mod oneshot;
pub mod orchestrate;
pub mod pipe;
pub mod plan;
pub mod repl;
pub mod run;
pub mod secrets;
pub mod status;

pub use config::{
    load_layered, AgentConfig, Config, ConfigLayer, ConfigPaths, ConfigSources, GateConfig,
    PromptConfig, PromptFile, ResolvedConfig, Source,
};
pub use config_cmd::{run_init_wizard, EditTarget, WizardInputs};
pub use daemon::{DaemonConfig, DaemonMode, DaemonState, DaemonStatus};
pub use episode::EpisodePolicy;
pub use inject::{InjectKind, InjectRequest};
pub use oneshot::{OneshotMode, OneshotResult};
pub use orchestrate::{OrchestrationReport, PlanRunReport, PlanRunner};
pub use pipe::{stdin_is_tty, PipeInput, PipeMode};
pub use plan::{Plan, PlanSummary, PlanTask};
pub use repl::{ReplCommand, ReplMode};
pub use run::{run_once, RunReport};
pub use secrets::SecretsCmd;
pub use status::SessionStatus;
