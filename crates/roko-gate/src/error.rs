//! Crate-level error types for roko-gate.

/// Error from a gate or gate pipeline.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// The gate subprocess exited with a non-zero status.
    #[error("gate command failed (exit {code}): {stderr}")]
    CommandFailed {
        /// Process exit code.
        code: i32,
        /// Captured stderr output.
        stderr: String,
    },

    /// Gate could not spawn the required subprocess.
    #[error("gate spawn failed: {0}")]
    SpawnFailed(String),

    /// Gate input payload was missing a required field.
    #[error("invalid gate payload: {0}")]
    InvalidPayload(String),

    /// Gate threshold was exceeded; verdict is Fail.
    #[error("gate threshold exceeded: {metric} = {value:.3} (threshold: {threshold:.3})")]
    ThresholdExceeded {
        /// Name of the metric that exceeded its threshold.
        metric: String,
        /// Observed value.
        value: f64,
        /// Configured threshold.
        threshold: f64,
    },

    /// I/O error while reading/writing gate artifacts.
    #[error("gate I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization/deserialization error for gate verdicts.
    #[error("gate serialize error: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Generic catch-all. Prefer adding typed variants above.
    #[error("{0}")]
    Other(String),
}
