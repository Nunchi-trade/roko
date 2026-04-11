//! Backend wrappers for request orchestration policies.

/// Tail-latency hedging for latency-sensitive requests.
pub mod hedged;

pub use hedged::HedgedBackend;
