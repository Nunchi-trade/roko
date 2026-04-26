//! Extracted submodules for the cascade router.
//!
//! This module contains types, helpers, and persistence code used by
//! [`crate::cascade_router::CascadeRouter`]. The router struct itself and its
//! core routing methods remain in `cascade_router.rs`.

pub mod helpers;
pub mod persistence;
pub mod types;

#[cfg(test)]
mod tests;

pub use helpers::slug_family;
pub use types::*;
