//! Hypnagogia subsystem scaffold.
//!
//! Placeholder API only; liminal-interrupt behavior is not yet implemented.

use crate::{GolemSubsystemId, GolemSubsystemSummary, ScaffoldEngine};

/// Placeholder hypnagogia engine.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HypnagogiaEngine;

impl HypnagogiaEngine {
    /// Stable subsystem id.
    pub const ID: GolemSubsystemId = GolemSubsystemId::Hypnagogia;
    /// Human-readable subsystem label.
    pub const LABEL: &'static str = "Hypnagogia";
    /// Static scaffold marker string.
    pub const MARKER: &'static str = "roko-golem scaffold: hypnagogia";

    /// Construct a placeholder hypnagogia engine.
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
    pub const fn interrupt(self) -> &'static str {
        Self::MARKER
    }
}

impl ScaffoldEngine for HypnagogiaEngine {
    fn summary(self) -> GolemSubsystemSummary {
        self.summary()
    }
}
