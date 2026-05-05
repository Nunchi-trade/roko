//! Crate-level error types for roko-learn.

use crate::episode_logger::LoggerError;

/// Error from a learning subsystem operation.
#[derive(Debug, thiserror::Error)]
pub enum LearnError {
    /// Persistent state file could not be read or written.
    #[error("learn state I/O error at {path}: {source}")]
    Io {
        /// Path to the file that caused the error.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// JSON parsing or serialization failed for persisted learning data.
    #[error("learn state parse error: {0}")]
    Parse(#[from] serde_json::Error),

    /// A router or bandit state file was corrupt. The subsystem should
    /// reset to defaults rather than propagating this error upward.
    #[error("learn state corrupt at {path}: {reason}")]
    Corrupt {
        /// Path to the corrupt file.
        path: String,
        /// Reason the file is considered corrupt.
        reason: String,
    },

    /// Episode logger error.
    #[error("episode logger error: {0}")]
    Logger(#[from] LoggerError),

    /// Generic catch-all. Prefer adding typed variants above.
    #[error("{0}")]
    Other(String),
}
