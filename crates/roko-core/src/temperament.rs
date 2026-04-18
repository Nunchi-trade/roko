//! Typed agent temperament labels shared across config, routing, and runtime metadata.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// High-level execution temperament for an agent or route.
///
/// This is intentionally narrow: it provides a stable typed contract for the
/// small set of temperament labels already described in the docs and carried as
/// free-form metadata elsewhere in the runtime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Temperament {
    /// Favor stronger models and safer routing choices.
    Conservative,
    /// Keep the existing runtime heuristics unchanged.
    #[default]
    Balanced,
    /// Favor faster / cheaper execution when the router can do so safely.
    Aggressive,
    /// Explore more alternatives before converging on one path.
    Exploratory,
}

impl Temperament {
    /// Canonical snake_case label for config and logs.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Conservative => "conservative",
            Self::Balanced => "balanced",
            Self::Aggressive => "aggressive",
            Self::Exploratory => "exploratory",
        }
    }

    /// Parse a temperament from a human-readable label.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        let normalized = label.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "conservative" | "cautious" | "careful" | "safe" => Some(Self::Conservative),
            "balanced" | "default" | "neutral" => Some(Self::Balanced),
            "aggressive" | "assertive" | "decisive" | "fast" => Some(Self::Aggressive),
            "exploratory" | "creative" | "curious" | "research" => Some(Self::Exploratory),
            _ => None,
        }
    }

    /// Bias applied to tier retargeting in the learning router.
    ///
    /// Positive values move toward stronger tiers; negative values move toward
    /// cheaper tiers.
    #[must_use]
    pub const fn routing_tier_shift(self) -> i8 {
        match self {
            Self::Conservative => 1,
            Self::Balanced | Self::Exploratory => 0,
            Self::Aggressive => -1,
        }
    }

    /// Multiplier applied to the router's exploration alpha when present.
    #[must_use]
    pub const fn exploration_multiplier(self) -> f64 {
        match self {
            Self::Conservative => 0.8,
            Self::Balanced => 1.0,
            Self::Aggressive => 0.9,
            Self::Exploratory => 1.25,
        }
    }
}

impl fmt::Display for Temperament {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Temperament {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_label(s).ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::Temperament;

    #[test]
    fn parses_known_labels_and_aliases() {
        assert_eq!(
            Temperament::from_label("conservative"),
            Some(Temperament::Conservative)
        );
        assert_eq!(
            Temperament::from_label("neutral"),
            Some(Temperament::Balanced)
        );
        assert_eq!(
            Temperament::from_label("decisive"),
            Some(Temperament::Aggressive)
        );
        assert_eq!(
            Temperament::from_label("creative"),
            Some(Temperament::Exploratory)
        );
        assert_eq!(Temperament::from_label("unknown"), None);
    }
}
