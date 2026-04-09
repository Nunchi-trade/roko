//! Chain witness subsystem scaffold.
//!
//! Placeholder API only; witness/triage behavior is not yet implemented.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder chain witness engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChainWitnessEngine;

impl ChainWitnessEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::ChainWitness;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Chain Witness";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: chain_witness";

    /// Construct a placeholder chain witness engine.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Summary metadata for this scaffold subsystem.
    #[must_use]
    pub const fn summary(self) -> GolemSubsystemSummary {
        GolemSubsystemSummary::new(Self::ID, Self::LABEL, Self::MARKER)
    }

    /// Returns a static marker describing scaffold behavior.
    #[must_use]
    pub const fn observe(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for ChainWitnessEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}
