//! Resumable loop state (§36.57).
//!
//! A [`Checkpoint`] captures the tool loop's mutable state (iteration
//! count, accumulated tool calls, and conversation messages) so the
//! loop can be serialized to disk and resumed later — e.g. after a
//! crash, manual pause, or context-window rotation.

use roko_core::Result;
use roko_core::tool::ToolCall;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::translate::SessionState;

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
    /// Provider-issued session identifiers required to resume the conversation.
    #[serde(default)]
    pub session: SessionState,
}

impl Checkpoint {
    /// Create a new checkpoint from the loop's current state.
    #[must_use]
    pub fn new(
        iterations: usize,
        tool_calls: Vec<ToolCall>,
        messages: Vec<serde_json::Value>,
    ) -> Self {
        Self {
            iterations,
            tool_calls,
            messages,
            session: SessionState::default(),
        }
    }

    /// Attach provider session continuity state to the checkpoint.
    #[must_use]
    pub fn with_session(mut self, session: SessionState) -> Self {
        self.session = session;
        self
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

    /// Persist the checkpoint to disk as formatted JSON.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails, the parent directory
    /// cannot be created, or the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load a checkpoint from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the JSON is invalid.
    pub fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let cp: Self = serde_json::from_str(&json)?;
        Ok(cp)
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
        assert_eq!(recovered.session, SessionState::default());
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

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("state").join("checkpoint.json");
        let cp = Checkpoint::new(
            2,
            vec![ToolCall::new("c1", "echo", serde_json::json!({"value": 7}))],
            vec![serde_json::json!({"role": "user", "content": "resume me"})],
        );

        cp.save(&path).expect("save checkpoint");
        let loaded = Checkpoint::load(&path).expect("load checkpoint");

        assert_eq!(loaded.iterations, cp.iterations);
        assert_eq!(loaded.tool_calls.len(), 1);
        assert_eq!(loaded.tool_calls[0].name, "echo");
        assert_eq!(loaded.messages, cp.messages);
    }
}
