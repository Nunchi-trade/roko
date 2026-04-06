//! Resumable loop state (§36.57).
//!
//! A [`Checkpoint`] captures the tool loop's mutable state (iteration
//! count, accumulated tool calls, and conversation messages) so the
//! loop can be serialized to disk and resumed later — e.g. after a
//! crash, manual pause, or context-window rotation.

use roko_core::tool::ToolCall;
use serde::{Deserialize, Serialize};

/// Serializable snapshot of a [`ToolLoop`](super::ToolLoop) mid-execution.
///
/// Created by the loop when it stops for any reason other than
/// [`StopReason::Stop`](super::StopReason::Stop) (i.e. the normal
/// "final answer" path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Number of tool-call iterations completed before this snapshot.
    pub iterations: usize,
    /// All tool calls dispatched so far (across all iterations).
    pub tool_calls: Vec<ToolCall>,
    /// The full conversation message history at snapshot time.
    pub messages: Vec<serde_json::Value>,
}

impl Checkpoint {
    /// Create a new checkpoint from the loop's current state.
    #[must_use]
    pub const fn new(
        iterations: usize,
        tool_calls: Vec<ToolCall>,
        messages: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            iterations,
            tool_calls,
            messages,
        }
    }

    /// Serialize to JSON bytes for persistence.
    ///
    /// # Errors
    ///
    /// Returns a serialization error if any field fails to serialize.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Deserialize from JSON bytes.
    ///
    /// # Errors
    ///
    /// Returns a deserialization error if the bytes are not a valid
    /// `Checkpoint`.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let call = ToolCall::new("c1", "echo", serde_json::json!({"x": 1}));
        let cp = Checkpoint::new(
            3,
            vec![call],
            vec![
                serde_json::json!({"role": "system", "content": "sys"}),
                serde_json::json!({"role": "user", "content": "usr"}),
            ],
        );
        let bytes = cp.to_bytes().expect("serialize");
        let recovered = Checkpoint::from_bytes(&bytes).expect("deserialize");
        assert_eq!(recovered.iterations, 3);
        assert_eq!(recovered.tool_calls.len(), 1);
        assert_eq!(recovered.tool_calls[0].name, "echo");
        assert_eq!(recovered.messages.len(), 2);
    }

    #[test]
    fn empty_checkpoint_round_trips() {
        let cp = Checkpoint::new(0, vec![], vec![]);
        let bytes = cp.to_bytes().expect("serialize");
        let recovered = Checkpoint::from_bytes(&bytes).expect("deserialize");
        assert_eq!(recovered.iterations, 0);
        assert!(recovered.tool_calls.is_empty());
        assert!(recovered.messages.is_empty());
    }
}
