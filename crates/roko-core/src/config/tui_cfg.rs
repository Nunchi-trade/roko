//! Terminal UI configuration.

use serde::{Deserialize, Serialize};

/// Terminal UI preferences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuiConfig {
    /// Refresh interval in milliseconds.
    #[serde(default = "default_refresh_rate")]
    pub refresh_rate_ms: u64,
}

const fn default_refresh_rate() -> u64 {
    250
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            refresh_rate_ms: default_refresh_rate(),
        }
    }
}
