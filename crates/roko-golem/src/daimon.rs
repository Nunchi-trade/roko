//! Daimon subsystem scaffold.
//!
//! Placeholder API only; affect or motivation modeling is not yet implemented.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder daimon engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DaimonEngine;

impl DaimonEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Daimon;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Daimon";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: daimon";

    /// Construct a placeholder daimon engine.
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
    pub const fn evaluate(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for DaimonEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}
