//! Roko runtime configuration.
//!
//! # Modules
//!
//! - [`schema`] -- The unified `RokoConfig` type with hierarchical sections.
//! - [`compat`] -- Reader for legacy Mori `config.toml` format.
//! - [`presets`] -- Named presets (minimal / balanced / thorough).

pub mod compat;
pub mod presets;
pub mod schema;

// Re-exports for ergonomic use.
pub use compat::from_mori_toml;
pub use presets::Preset;
pub use schema::{
    AgentConfig, AgentRoleToggles, BudgetConfig, CURRENT_SCHEMA_VERSION, ConductorConfig,
    GatesConfig, LearningConfig, PrdConfig, ProjectConfig, RokoConfig, RoleOverride, RoutingConfig,
    SchedulerConfig, SchedulerCronConfig, ServeAuthConfig, ServeConfig, ServerConfig, TuiConfig,
    WatcherConfig, WatcherPathConfig,
};
