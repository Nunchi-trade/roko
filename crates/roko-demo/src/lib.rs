#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(
    clippy::doc_markdown,
    clippy::missing_const_for_fn,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::cast_sign_loss,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::explicit_iter_loop,
    clippy::unnecessary_wraps,
    clippy::implicit_hasher,
    clippy::useless_conversion,
    clippy::manual_clamp,
    clippy::used_underscore_items,
    clippy::literal_string_with_formatting_args
)]

//! Manifest-driven orchestrator for the roko demo environment.
//!
//! See `roko/demo/` for the declarative config + scenarios consumed by this
//! crate's `roko-demo` binary.

pub mod bindings;
pub mod chain_ctx;
pub mod deploy;
pub mod fixtures;
pub mod manifest;
pub mod scenarios;
pub mod verify;

pub use chain_ctx::ChainCtx;
pub use deploy::{ContractArtifact, DeployCtx, DeployedSuite, deploy_suite};
pub use fixtures::{FixtureRegistry, RustFixture, run_fixtures};
pub use manifest::{LoadedManifest, Manifest, Scenario};
