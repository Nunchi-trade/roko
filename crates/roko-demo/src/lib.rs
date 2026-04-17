#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::assigning_clones,
    clippy::if_not_else,
    clippy::incompatible_msrv,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::explicit_iter_loop,
    clippy::unnecessary_wraps,
    clippy::unnecessary_literal_bound,
    clippy::implicit_hasher,
    clippy::struct_field_names,
    clippy::suboptimal_flops,
    clippy::redundant_clone,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::useless_conversion,
    clippy::manual_clamp,
    clippy::used_underscore_items,
    clippy::literal_string_with_formatting_args
)]

//! Manifest-driven orchestrator for the roko demo environment.
//!
//! See `roko/demo/` for the declarative config + scenarios consumed by this
//! crate's `roko-demo` binary.

pub mod autonomous;
pub mod benchmark;
pub mod bindings;
pub mod chain_ctx;
pub mod deploy;
pub mod events;
pub mod fixtures;
pub mod manifest;
pub mod scenarios;
pub mod tournament;
pub mod tui;
pub mod verify;
pub mod ws_server;

pub use chain_ctx::ChainCtx;
pub use deploy::{ContractArtifact, DeployCtx, DeployedSuite, deploy_suite};
pub use fixtures::{FixtureRegistry, RustFixture, run_fixtures};
pub use manifest::{LoadedManifest, Manifest, Scenario};
