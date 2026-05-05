//! STATUS: NOT WIRED -- built but no non-test runtime caller.
//!
//! Domain-specific Oracle implementations.
//!
//! Each oracle implements the [`roko_core::Oracle`] trait for a specific
//! prediction domain, providing `predict()` and `evaluate()` methods that
//! produce calibrated, falsifiable predictions.

pub mod chain;
pub mod coding;
pub mod research;
pub mod selector;
pub mod witness;
