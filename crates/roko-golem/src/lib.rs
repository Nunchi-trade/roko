//! `roko-golem` scaffold crate.
//!
//! This crate is intentionally minimal and does **not** implement runtime
//! behavior yet. It exists to reserve module boundaries and a feature-gated API
//! surface for phase-2 Golem integration work.
//!
//! Enable `scaffold` to expose placeholder subsystem modules:
//! - `mortality`
//! - `daimon`
//! - `dreams`
//! - `hypnagogia`
//! - `grimoire`
//! - `chain_witness`

/// Whether the scaffold API is compiled in this build.
pub const SCAFFOLD_ENABLED: bool = cfg!(feature = "scaffold");

/// High-level status of this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// Feature-gated placeholder APIs are available.
    ScaffoldOnly,
    /// Placeholder APIs are compiled out.
    Disabled,
}

/// Returns the current integration status.
#[must_use]
pub const fn status() -> IntegrationStatus {
    if SCAFFOLD_ENABLED {
        IntegrationStatus::ScaffoldOnly
    } else {
        IntegrationStatus::Disabled
    }
}

#[cfg(feature = "scaffold")]
pub mod chain_witness;
#[cfg(feature = "scaffold")]
pub mod daimon;
#[cfg(feature = "scaffold")]
pub mod dreams;
#[cfg(feature = "scaffold")]
pub mod grimoire;
#[cfg(feature = "scaffold")]
pub mod hypnagogia;
#[cfg(feature = "scaffold")]
pub mod mortality;

#[cfg(test)]
mod tests {
    use super::{IntegrationStatus, SCAFFOLD_ENABLED, status};

    #[test]
    fn status_matches_feature_flag() {
        if SCAFFOLD_ENABLED {
            assert_eq!(status(), IntegrationStatus::ScaffoldOnly);
        } else {
            assert_eq!(status(), IntegrationStatus::Disabled);
        }
    }
}
