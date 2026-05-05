//! Crate-level error types for roko-compose.

/// Error from prompt assembly or composition operations.
#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    /// Template rendering failed.
    #[error("template render failed: {0}")]
    Template(String),

    /// Enrichment client returned an error.
    #[error("enrichment error: {0}")]
    Enrichment(String),

    /// Token counter exceeded a budget constraint.
    #[error("token budget exceeded: used {used}, budget {budget}")]
    TokenBudgetExceeded {
        /// Tokens consumed.
        used: usize,
        /// Configured budget.
        budget: usize,
    },

    /// Generic catch-all.
    #[error("{0}")]
    Other(String),
}
