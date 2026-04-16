//! Approval IPC between the orchestrator and the TUI.
//!
//! The orchestrator sends an [`ApprovalRequest`] when an agent command needs
//! human approval. The TUI receives the request, shows the existing approval
//! modal, and answers through the embedded oneshot sender.

use tokio::sync::{mpsc, oneshot};

/// Request sent from the orchestrator to the TUI for approval.
pub struct ApprovalRequest {
    /// Agent role or label associated with the request.
    pub role: String,
    /// Command or tool invocation that needs approval.
    pub command: String,
    /// Opaque approval identifier carried through the flow.
    pub approval_id: String,
    /// Reply channel: `true` approves, `false` denies.
    pub response_tx: oneshot::Sender<bool>,
}

/// Channel pair used to move approval requests into the TUI.
pub struct ApprovalChannel {
    /// Sender owned by the orchestrator side.
    pub tx: mpsc::Sender<ApprovalRequest>,
    /// Receiver owned by the TUI side.
    pub rx: mpsc::Receiver<ApprovalRequest>,
}

impl ApprovalChannel {
    /// Create a bounded approval channel with the requested buffer size.
    #[must_use]
    pub fn new(buffer: usize) -> Self {
        let (tx, rx) = mpsc::channel(buffer);
        Self { tx, rx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_channel_creates_working_pair() {
        let mut channel = ApprovalChannel::new(1);
        let (response_tx, _response_rx) = oneshot::channel();

        channel
            .tx
            .try_send(ApprovalRequest {
                role: "reviewer".to_string(),
                command: "echo hello".to_string(),
                approval_id: "approval-1".to_string(),
                response_tx,
            })
            .expect("send approval request");

        let request = channel.rx.try_recv().expect("receive approval request");
        assert_eq!(request.role, "reviewer");
        assert_eq!(request.command, "echo hello");
        assert_eq!(request.approval_id, "approval-1");
    }
}
