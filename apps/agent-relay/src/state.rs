use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};
use uuid::Uuid;

use crate::protocol::{
    AgentHello, ConnectedAgent, ConnectedWorkspace, RelayEvent, RelayMessageRequest,
    RelayMessageResponse, RelayOutboundFrame, WorkspaceHello,
};

struct ConnectedAgentHandle {
    session_id: Uuid,
    agent: ConnectedAgent,
    tx: mpsc::UnboundedSender<RelayOutboundFrame>,
}

struct PendingResponse {
    agent_id: String,
    tx: oneshot::Sender<Result<Value, String>>,
}

#[derive(Default)]
struct RelayStateInner {
    agents: HashMap<String, ConnectedAgentHandle>,
    cards: HashMap<String, Value>,
    pending: HashMap<String, PendingResponse>,
    workspaces: HashMap<String, ConnectedWorkspace>,
}

/// Shared in-memory relay state for directory, cards, and pending replies.
pub struct RelayState {
    inner: RwLock<RelayStateInner>,
    events_tx: broadcast::Sender<RelayEvent>,
}

impl Default for RelayState {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayState {
    #[must_use]
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(256);
        Self {
            inner: RwLock::new(RelayStateInner::default()),
            events_tx,
        }
    }

    #[must_use]
    pub fn list_agents(&self) -> Vec<ConnectedAgent> {
        let mut agents = self
            .inner
            .read()
            .agents
            .values()
            .map(|entry| entry.agent.clone())
            .collect::<Vec<_>>();
        agents.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));
        agents
    }

    #[must_use]
    pub fn card(&self, agent_id: &str) -> Option<Value> {
        self.inner.read().cards.get(agent_id).cloned()
    }

    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<RelayEvent> {
        self.events_tx.subscribe()
    }

    pub fn register_agent(
        &self,
        hello: AgentHello,
        tx: mpsc::UnboundedSender<RelayOutboundFrame>,
    ) -> RegisteredAgent {
        let session_id = Uuid::new_v4();
        let card_uri = hello
            .card
            .as_ref()
            .map(|_| relay_card_uri(&hello.agent_id))
            .or_else(|| hello.card_uri.clone());
        let agent = ConnectedAgent {
            agent_id: hello.agent_id.clone(),
            name: hello.name,
            capabilities: hello.capabilities,
            rest_endpoint: hello.rest_endpoint,
            card_uri: card_uri.clone(),
            connected_at_ms: now_ms(),
            relay_backed: true,
        };

        {
            let mut inner = self.inner.write();
            if let Some(card) = hello.card {
                inner.cards.insert(agent.agent_id.clone(), card);
            }
            inner.agents.insert(
                agent.agent_id.clone(),
                ConnectedAgentHandle {
                    session_id,
                    agent: agent.clone(),
                    tx,
                },
            );
        }

        let _ = self.events_tx.send(RelayEvent::AgentConnected {
            agent: agent.clone(),
        });
        if let Some(card_uri) = card_uri {
            let _ = self.events_tx.send(RelayEvent::CardUpdated {
                agent_id: agent.agent_id.clone(),
                card_uri,
            });
        }

        RegisteredAgent {
            session_id,
            agent_id: agent.agent_id,
        }
    }

    pub fn update_card(&self, agent_id: &str, card: Value, card_uri: Option<String>) {
        let resolved_card_uri = card_uri.unwrap_or_else(|| relay_card_uri(agent_id));
        {
            let mut inner = self.inner.write();
            inner.cards.insert(agent_id.to_string(), card);
            if let Some(agent) = inner.agents.get_mut(agent_id) {
                agent.agent.card_uri = Some(resolved_card_uri.clone());
            }
        }
        let _ = self.events_tx.send(RelayEvent::CardUpdated {
            agent_id: agent_id.to_string(),
            card_uri: resolved_card_uri,
        });
    }

    pub fn unregister_agent(&self, agent_id: &str, session_id: Uuid) {
        let pending = {
            let mut inner = self.inner.write();
            let Some(current) = inner.agents.get(agent_id) else {
                return;
            };
            if current.session_id != session_id {
                return;
            }
            inner.agents.remove(agent_id);
            let mut pending = Vec::new();
            for (message_id, pending_response) in std::mem::take(&mut inner.pending) {
                if pending_response.agent_id == agent_id {
                    pending.push(pending_response);
                } else {
                    inner.pending.insert(message_id, pending_response);
                }
            }
            drop(inner);
            pending
        };

        for pending in pending {
            let _ = pending.tx.send(Err("agent disconnected".to_string()));
        }
        let _ = self.events_tx.send(RelayEvent::AgentDisconnected {
            agent_id: agent_id.to_string(),
        });
    }

    pub fn begin_message(
        &self,
        request: RelayMessageRequest,
    ) -> Result<PendingMessage, BeginMessageError> {
        let message_id = Uuid::new_v4().to_string();
        let timeout_ms = request.timeout_ms();
        let (response_tx, response_rx) = oneshot::channel();

        let agent_tx = {
            let mut inner = self.inner.write();
            let Some(agent) = inner.agents.get(&request.agent_id) else {
                return Err(BeginMessageError::UnknownAgent);
            };
            let agent_tx = agent.tx.clone();
            inner.pending.insert(
                message_id.clone(),
                PendingResponse {
                    agent_id: request.agent_id.clone(),
                    tx: response_tx,
                },
            );
            agent_tx
        };

        if agent_tx
            .send(RelayOutboundFrame::Message {
                message_id: message_id.clone(),
                message: request.message,
            })
            .is_err()
        {
            self.inner.write().pending.remove(&message_id);
            return Err(BeginMessageError::NotConnected);
        }

        let _ = self.events_tx.send(RelayEvent::MessageDelivered {
            agent_id: request.agent_id.clone(),
            message_id: message_id.clone(),
        });

        Ok(PendingMessage {
            agent_id: request.agent_id,
            message_id,
            timeout_ms,
            response_rx,
        })
    }

    pub fn resolve_response(&self, message_id: &str, result: Result<Value, String>) -> bool {
        let Some(pending) = self.inner.write().pending.remove(message_id) else {
            return false;
        };

        let event = match &result {
            Ok(_) => RelayEvent::MessageResponded {
                agent_id: pending.agent_id.clone(),
                message_id: message_id.to_string(),
            },
            Err(error) => RelayEvent::AgentError {
                agent_id: pending.agent_id.clone(),
                message_id: Some(message_id.to_string()),
                error: error.clone(),
            },
        };
        let _ = pending.tx.send(result);
        let _ = self.events_tx.send(event);
        true
    }

    pub fn agent_error(&self, agent_id: &str, message_id: Option<String>, error: String) {
        if let Some(message_id) = message_id {
            let _ = self.resolve_response(&message_id, Err(error.clone()));
        }
        let _ = self.events_tx.send(RelayEvent::AgentError {
            agent_id: agent_id.to_string(),
            message_id: None,
            error,
        });
    }

    // ── Workspace directory ──────────────────────────────────────────

    #[must_use]
    pub fn list_workspaces(&self) -> Vec<ConnectedWorkspace> {
        let mut workspaces = self
            .inner
            .read()
            .workspaces
            .values()
            .cloned()
            .collect::<Vec<_>>();
        workspaces.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
        workspaces
    }

    pub fn register_workspace(&self, hello: WorkspaceHello) {
        let now = now_ms();
        let workspace = ConnectedWorkspace {
            workspace_id: hello.workspace_id.clone(),
            name: hello.name,
            url: hello.url,
            version: hello.version,
            owner_wallet: hello.owner_wallet,
            agents_count: hello.agents_count,
            connected_at_ms: now,
            last_heartbeat_ms: now,
        };
        self.inner
            .write()
            .workspaces
            .insert(hello.workspace_id, workspace.clone());
        let _ = self
            .events_tx
            .send(RelayEvent::WorkspaceConnected { workspace });
    }

    pub fn workspace_heartbeat(&self, workspace_id: &str, agents_count: u32) {
        let mut inner = self.inner.write();
        if let Some(ws) = inner.workspaces.get_mut(workspace_id) {
            ws.last_heartbeat_ms = now_ms();
            ws.agents_count = agents_count;
        }
        drop(inner);
        let _ = self.events_tx.send(RelayEvent::WorkspaceHeartbeat {
            workspace_id: workspace_id.to_string(),
            agents_count,
        });
    }

    pub fn unregister_workspace(&self, workspace_id: &str) {
        self.inner.write().workspaces.remove(workspace_id);
        let _ = self.events_tx.send(RelayEvent::WorkspaceDisconnected {
            workspace_id: workspace_id.to_string(),
        });
    }

    /// Remove workspaces that haven't sent a heartbeat in `stale_ms`.
    pub fn expire_stale_workspaces(&self, stale_ms: u64) -> Vec<String> {
        let now = now_ms();
        let mut expired = Vec::new();
        let mut inner = self.inner.write();
        inner.workspaces.retain(|id, ws| {
            if now.saturating_sub(ws.last_heartbeat_ms) > stale_ms {
                expired.push(id.clone());
                false
            } else {
                true
            }
        });
        drop(inner);
        for id in &expired {
            let _ = self.events_tx.send(RelayEvent::WorkspaceDisconnected {
                workspace_id: id.clone(),
            });
        }
        expired
    }
}

/// Live registration metadata returned after an agent hello succeeds.
pub struct RegisteredAgent {
    pub session_id: Uuid,
    pub agent_id: String,
}

/// Pending HTTP request waiting on an agent response.
pub struct PendingMessage {
    pub agent_id: String,
    pub message_id: String,
    pub timeout_ms: u64,
    response_rx: oneshot::Receiver<Result<Value, String>>,
}

impl PendingMessage {
    pub async fn await_response(self) -> Result<RelayMessageResponse, AwaitMessageError> {
        match tokio::time::timeout(
            std::time::Duration::from_millis(self.timeout_ms),
            self.response_rx,
        )
        .await
        {
            Ok(Ok(Ok(response))) => Ok(RelayMessageResponse {
                message_id: self.message_id,
                agent_id: self.agent_id,
                response,
            }),
            Ok(Ok(Err(error))) => Err(AwaitMessageError::Agent(error)),
            Ok(Err(_)) => Err(AwaitMessageError::Agent(
                "response channel closed".to_string(),
            )),
            Err(_) => Err(AwaitMessageError::Timeout),
        }
    }
}

/// Errors returned while queuing a relay message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginMessageError {
    UnknownAgent,
    NotConnected,
}

/// Errors returned while waiting for an agent response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwaitMessageError {
    Timeout,
    Agent(String),
}

#[must_use]
pub fn relay_card_uri(agent_id: &str) -> String {
    format!("/relay/cards/{agent_id}")
}

fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}
