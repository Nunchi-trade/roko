#![allow(clippy::expect_used, clippy::unwrap_used, missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use agent_relay::{
    app,
    protocol::{ConnectedAgent, RelayMessageResponse},
    state::RelayState,
};
use alloy_primitives::{U256, keccak256};
use async_trait::async_trait;
use reqwest::Client;
use roko_agent::chat_types::{ChatRequest, ChatResponse, FinishReason};
use roko_agent_server::{
    AgentRegistration, AgentServer, DispatchError, DispatchLike, RelayClientConfig,
};
use roko_chain::{ChainWallet, MockChainWallet};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct TestRelay {
    base_url: String,
    client: Client,
    task: JoinHandle<()>,
}

impl TestRelay {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind relay");
        let addr = listener.local_addr().expect("relay addr");
        let base_url = format!("http://{addr}");
        let state = Arc::new(RelayState::new());
        let task = tokio::spawn(async move {
            axum::serve(listener, app(state))
                .await
                .expect("serve relay");
        });

        let relay = Self {
            base_url,
            client: Client::new(),
            task,
        };
        relay.wait_until_ready().await;
        relay
    }

    async fn wait_until_ready(&self) {
        for _ in 0..120 {
            let response = self
                .client
                .get(format!("{}/relay/health", self.base_url))
                .send()
                .await;
            if matches!(response, Ok(response) if response.status().is_success()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("relay did not become ready");
    }

    async fn wait_for_agent(&self, agent_id: &str) -> ConnectedAgent {
        for _ in 0..120 {
            let response = self
                .client
                .get(format!("{}/relay/agents", self.base_url))
                .send()
                .await
                .expect("GET /relay/agents");
            let agents: Vec<ConnectedAgent> = response.json().await.expect("relay agents");
            if let Some(agent) = agents.into_iter().find(|agent| agent.agent_id == agent_id) {
                return agent;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("agent {agent_id} did not appear in relay");
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct MockDispatcher {
    response: String,
}

#[async_trait]
impl DispatchLike for MockDispatcher {
    async fn dispatch(&self, _request: ChatRequest) -> Result<ChatResponse, DispatchError> {
        Ok(ChatResponse {
            content: self.response.clone(),
            finish_reason: FinishReason::Stop,
            ..Default::default()
        })
    }
}

struct RunningAgent {
    base_url: String,
    task: JoinHandle<()>,
}

impl RunningAgent {
    async fn spawn(agent_id: &str, registration: AgentRegistration, response: &str) -> Self {
        let (addr_tx, addr_rx) = oneshot::channel();
        let addr_tx = Arc::new(std::sync::Mutex::new(Some(addr_tx)));
        let server = AgentServer::builder()
            .agent_id(agent_id)
            .bind("127.0.0.1:0")
            .messaging()
            .with_message_dispatcher(Arc::new(MockDispatcher {
                response: response.to_string(),
            }))
            .registration(registration)
            .on_start(move |addr, _card| {
                if let Some(tx) = addr_tx.lock().expect("addr mutex").take() {
                    let _ = tx.send(addr);
                }
                async { Ok(()) }
            })
            .build()
            .expect("build agent");
        let task = tokio::spawn(async move { server.serve().await.expect("serve agent") });
        let addr = addr_rx.await.expect("agent start addr");
        let base_url = format!("http://{addr}");
        wait_for_http_ready(&base_url).await;
        Self { base_url, task }
    }
}

impl Drop for RunningAgent {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn wait_for_http_ready(base_url: &str) {
    let client = Client::new();
    for _ in 0..120 {
        let response = client.get(format!("{base_url}/health")).send().await;
        if matches!(response, Ok(response) if response.status().is_success()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("agent http server did not become ready");
}

#[tokio::test]
async fn wallet_free_relay_registration_hosts_card_and_keeps_direct_routes_working() {
    let relay = TestRelay::spawn().await;
    let expected_card_uri = format!("{}/relay/cards/agent-relay", relay.base_url);
    let registration = AgentRegistration {
        relay: Some(RelayClientConfig::new(relay.base_url.clone())),
        ..AgentRegistration::default()
    };
    let agent = RunningAgent::spawn("agent-relay", registration, "relay says hello").await;

    let connected = relay.wait_for_agent("agent-relay").await;
    assert!(connected.relay_backed);
    assert_eq!(connected.rest_endpoint, None);
    assert_eq!(
        connected.card_uri.as_deref(),
        Some(expected_card_uri.as_str())
    );

    let card: Value = relay
        .client
        .get(expected_card_uri)
        .send()
        .await
        .expect("GET relay card")
        .json()
        .await
        .expect("card json");
    assert_eq!(card["name"], json!("agent-relay"));
    assert_eq!(card["capabilities"], json!(["messaging"]));

    let relayed = relay
        .client
        .post(format!("{}/relay/messages", relay.base_url))
        .json(&json!({
            "agent_id": "agent-relay",
            "message": { "prompt": "hello over relay" },
            "timeout_ms": 5_000
        }))
        .send()
        .await
        .expect("POST /relay/messages");
    assert!(relayed.status().is_success());
    let relayed: RelayMessageResponse = relayed.json().await.expect("relay message response");
    assert_eq!(relayed.agent_id, "agent-relay");
    assert_eq!(relayed.response["response"], json!("relay says hello"));

    let direct_health = relay
        .client
        .get(format!("{}/health", agent.base_url))
        .send()
        .await
        .expect("GET /health");
    assert!(direct_health.status().is_success());

    let direct_message: Value = relay
        .client
        .post(format!("{}/message", agent.base_url))
        .json(&json!({ "prompt": "hello direct" }))
        .send()
        .await
        .expect("POST /message")
        .json()
        .await
        .expect("direct message json");
    assert_eq!(direct_message["response"], json!("relay says hello"));
}

#[tokio::test]
async fn wallet_backed_relay_registration_submits_target_abi_with_relay_card_uri() {
    let relay = TestRelay::spawn().await;
    let wallet = MockChainWallet::funded(1_000_000);
    let expected_card_uri = format!("{}/relay/cards/wallet-agent", relay.base_url);
    let registration = AgentRegistration {
        relay: Some(RelayClientConfig::new(relay.base_url.clone())),
        wallet: Some(Arc::new(wallet.clone()) as Arc<dyn ChainWallet>),
        identity_registry_address: Some("0x000000000000000000000000000000000000c0de".to_string()),
        passport_id: Some("7".to_string()),
        ..AgentRegistration::default()
    };
    let _agent = RunningAgent::spawn("wallet-agent", registration, "wallet-backed").await;

    let connected = relay.wait_for_agent("wallet-agent").await;
    assert_eq!(
        connected.card_uri.as_deref(),
        Some(expected_card_uri.as_str())
    );

    let submitted = wallet.submitted();
    assert_eq!(submitted.len(), 1);
    assert_eq!(
        submitted[0].1.to.as_deref(),
        Some("0x000000000000000000000000000000000000c0de")
    );
    assert_update_agent_card_uri_calldata(&submitted[0].1.data, 7, &expected_card_uri);
}

fn assert_update_agent_card_uri_calldata(data: &[u8], passport_id: u64, card_uri: &str) {
    assert_eq!(
        &data[..4],
        &keccak256("updateAgentCardUri(uint256,string)".as_bytes())[..4]
    );
    assert_eq!(&data[4..36], &U256::from(passport_id).to_be_bytes::<32>());
    assert_eq!(&data[36..68], &encode_word(64));

    let encoded_length = U256::from_be_slice(&data[68..100]);
    assert_eq!(encoded_length, U256::from(card_uri.len()));

    let string_end = 100 + card_uri.len();
    assert_eq!(&data[100..string_end], card_uri.as_bytes());
}

fn encode_word(value: u64) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out[24..].copy_from_slice(&value.to_be_bytes());
    out
}
