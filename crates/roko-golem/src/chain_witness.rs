//! Chain witness subsystem scaffold.
//!
//! Placeholder API only; witness/triage behavior is not yet implemented.

/// Placeholder chain witness engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChainWitnessEngine;

impl ChainWitnessEngine {
    /// Construct a placeholder chain witness engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn observe(self) -> &'static str {
        "roko-golem scaffold: chain_witness"
    }
}
