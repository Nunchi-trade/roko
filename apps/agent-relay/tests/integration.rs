#![allow(clippy::expect_used, clippy::unwrap_used, missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use agent_relay::{
    app,
    protocol::{ConnectedAgent, RelayMessageResponse, RelayOutboundFrame},
    state::RelayState,
};
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

type AgentSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

struct TestServer {
    base_url: String,
    client: Client,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test relay listener");
        let addr = listener.local_addr().expect("read test relay address");
        let base_url = format!("http://{addr}");
        let state = Arc::new(RelayState::new());
        let task = tokio::spawn(async move {
            axum::serve(listener, app(state))
                .await
                .expect("serve test relay");
        });

        let server = Self {
            base_url,
            client: Client::new(),
            task,
        };
        server.wait_until_ready().await;
        server
    }

    fn ws_url(&self, path: &str) -> String {
        format!(
            "ws://{}{}",
            self.base_url.trim_start_matches("http://"),
            path
        )
    }

    async fn wait_until_ready(&self) {
        for _ in 0..50 {
            match self
                .client
                .get(format!("{}/relay/health", self.base_url))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => return,
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        panic!("test relay did not become ready");
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn connect_agent(server: &TestServer, hello: Value) -> AgentSocket {
    let (mut socket, _) = connect_async(server.ws_url("/relay/agents/ws"))
        .await
        .expect("connect agent websocket");
    send_json(&mut socket, hello).await;
    let ack = recv_json(&mut socket).await;
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["event"], "hello");
    socket
}

async fn send_json(socket: &mut AgentSocket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .expect("send websocket json");
}

async fn recv_json(socket: &mut AgentSocket) -> Value {
    loop {
        match socket
            .next()
            .await
            .expect("websocket message")
            .expect("ws frame")
        {
            Message::Text(text) => {
                return serde_json::from_str(text.as_str()).expect("parse websocket json");
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .expect("reply to ping");
            }
            Message::Pong(_) => {}
            Message::Close(frame) => panic!("unexpected websocket close: {frame:?}"),
            other => panic!("unexpected websocket frame: {other:?}"),
        }
    }
}

#[tokio::test]
async fn relay_health_returns_ok() {
    let server = TestServer::spawn().await;

    let response = server
        .client
        .get(format!("{}/relay/health", server.base_url))
        .send()
        .await
        .expect("GET /relay/health");

    assert!(response.status().is_success());
    assert_eq!(response.text().await.expect("health body"), "ok");
}

#[tokio::test]
async fn hello_lists_connected_agent() {
    let server = TestServer::spawn().await;
    let _socket = connect_agent(
        &server,
        json!({
            "type": "hello",
            "agent_id": "agent-123",
            "name": "Directory Agent",
            "capabilities": ["search", "delegate"],
            "rest_endpoint": "http://agent.local/invoke"
        }),
    )
    .await;

    let response = server
        .client
        .get(format!("{}/relay/agents", server.base_url))
        .send()
        .await
        .expect("GET /relay/agents");

    assert!(response.status().is_success());
    let agents: Vec<ConnectedAgent> = response.json().await.expect("agents json");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_id, "agent-123");
    assert_eq!(agents[0].name.as_deref(), Some("Directory Agent"));
    assert_eq!(agents[0].capabilities, vec!["search", "delegate"]);
    assert_eq!(
        agents[0].rest_endpoint.as_deref(),
        Some("http://agent.local/invoke")
    );
    assert_eq!(agents[0].card_uri, None);
    assert!(agents[0].relay_backed);
    assert!(agents[0].connected_at_ms > 0);
}

#[tokio::test]
async fn pushed_card_is_served_from_relay() {
    let server = TestServer::spawn().await;
    let mut socket = connect_agent(
        &server,
        json!({
            "type": "hello",
            "agent_id": "agent-card"
        }),
    )
    .await;

    let pushed_card = json!({
        "agentId": "agent-card",
        "name": "Card Agent",
        "version": "0.1.0",
        "serviceEndpoints": {
            "ws": "wss://relay.example.test/agents/agent-card"
        }
    });
    send_json(
        &mut socket,
        json!({
            "type": "card",
            "card": pushed_card,
        }),
    )
    .await;

    let ack = recv_json(&mut socket).await;
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["event"], "card");

    let response = server
        .client
        .get(format!("{}/relay/cards/agent-card", server.base_url))
        .send()
        .await
        .expect("GET /relay/cards/{id}");

    assert!(response.status().is_success());
    let card: Value = response.json().await.expect("card json");
    assert_eq!(
        card,
        json!({
            "agentId": "agent-card",
            "name": "Card Agent",
            "version": "0.1.0",
            "serviceEndpoints": {
                "ws": "wss://relay.example.test/agents/agent-card"
            }
        })
    );
}

#[tokio::test]
async fn post_messages_forwards_and_returns_agent_response() {
    let server = TestServer::spawn().await;
    let mut socket = connect_agent(
        &server,
        json!({
            "type": "hello",
            "agent_id": "agent-forward"
        }),
    )
    .await;

    let client = server.client.clone();
    let base_url = server.base_url.clone();
    let response_task = tokio::spawn(async move {
        let response = client
            .post(format!("{base_url}/relay/messages"))
            .json(&json!({
                "agent_id": "agent-forward",
                "message": {
                    "kind": "task",
                    "prompt": "summarize relay status"
                },
                "timeout_ms": 5_000
            }))
            .send()
            .await
            .expect("POST /relay/messages");
        let status = response.status();
        let body = response
            .json::<RelayMessageResponse>()
            .await
            .expect("message response body");
        (status, body)
    });

    let outbound = recv_json(&mut socket).await;
    let frame: RelayOutboundFrame =
        serde_json::from_value(outbound.clone()).expect("parse outbound relay frame");
    let RelayOutboundFrame::Message {
        message_id,
        message,
    } = frame
    else {
        panic!("expected outbound message frame, got {outbound:?}");
    };
    assert_eq!(
        message,
        json!({
            "kind": "task",
            "prompt": "summarize relay status"
        })
    );

    send_json(
        &mut socket,
        json!({
            "type": "response",
            "message_id": message_id,
            "response": {
                "status": "ok",
                "result": "relay healthy"
            }
        }),
    )
    .await;

    let (status, body) = response_task.await.expect("join HTTP response task");
    assert!(status.is_success());
    assert_eq!(body.agent_id, "agent-forward");
    assert_eq!(
        body.response,
        json!({
            "status": "ok",
            "result": "relay healthy"
        })
    );
    assert!(!body.message_id.is_empty());
}

// ── Pub/sub integration tests (A3) ──────────────────────────────────────────

#[tokio::test]
async fn topic_subscribe_and_receive() {
    let server = TestServer::spawn().await;

    // Connect two agents using distinct IDs.
    let mut publisher = connect_agent(
        &server,
        json!({
            "type": "hello",
            "agent_id": "publisher-1"
        }),
    )
    .await;
    let mut subscriber = connect_agent(
        &server,
        json!({
            "type": "hello",
            "agent_id": "subscriber-1"
        }),
    )
    .await;

    // Subscriber subscribes to "isfr:rates".
    send_json(
        &mut subscriber,
        json!({"type": "subscribe", "topic": "isfr:rates"}),
    )
    .await;
    let ack = recv_json(&mut subscriber).await;
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["event"], "subscribed:isfr:rates");

    // Publisher publishes a rate update.
    send_json(
        &mut publisher,
        json!({
            "type": "publish",
            "topic": "isfr:rates",
            "msg_type": "rate_update",
            "payload": { "bps": 620 }
        }),
    )
    .await;
    let pub_ack = recv_json(&mut publisher).await;
    assert_eq!(pub_ack["type"], "ack");
    assert!(
        pub_ack["event"]
            .as_str()
            .unwrap()
            .starts_with("published:isfr:rates:"),
        "unexpected ack event: {}",
        pub_ack["event"]
    );

    // Subscriber receives the message.
    let msg = recv_json(&mut subscriber).await;
    assert_eq!(msg["type"], "topic_message");
    assert_eq!(msg["topic"], "isfr:rates");
    assert_eq!(msg["msg_type"], "rate_update");
    assert_eq!(msg["payload"]["bps"], 620);
    assert_eq!(msg["publisher_id"], "publisher-1");
}

#[tokio::test]
async fn topic_unsubscribe_stops_delivery() {
    let server = TestServer::spawn().await;

    let mut publisher =
        connect_agent(&server, json!({"type": "hello", "agent_id": "pub-unsub-1"})).await;
    let mut subscriber =
        connect_agent(&server, json!({"type": "hello", "agent_id": "sub-unsub-1"})).await;

    // Subscribe.
    send_json(
        &mut subscriber,
        json!({"type": "subscribe", "topic": "test:topic"}),
    )
    .await;
    let _ = recv_json(&mut subscriber).await; // ack

    // Unsubscribe.
    send_json(
        &mut subscriber,
        json!({"type": "unsubscribe", "topic": "test:topic"}),
    )
    .await;
    let unack = recv_json(&mut subscriber).await;
    assert_eq!(unack["type"], "ack");
    assert_eq!(unack["event"], "unsubscribed:test:topic");

    // Publisher sends a message — subscriber should NOT receive it.
    send_json(
        &mut publisher,
        json!({
            "type": "publish",
            "topic": "test:topic",
            "msg_type": "evt",
            "payload": {}
        }),
    )
    .await;
    let _ = recv_json(&mut publisher).await; // publisher ack

    // Give any spurious delivery a moment to arrive, then close.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    subscriber.close(None).await.expect("close subscriber");
}

#[tokio::test]
async fn subscribe_receives_replay_messages() {
    let server = TestServer::spawn().await;

    let mut publisher = connect_agent(
        &server,
        json!({"type": "hello", "agent_id": "pub-replay-1"}),
    )
    .await;

    // Publish a message before any subscriber exists.
    send_json(
        &mut publisher,
        json!({
            "type": "publish",
            "topic": "replay:topic",
            "msg_type": "first",
            "payload": { "n": 1 }
        }),
    )
    .await;
    let _ = recv_json(&mut publisher).await; // ack

    // Now a late subscriber joins — it should receive the replayed message
    // before the subscription ACK.
    let mut late = connect_agent(
        &server,
        json!({"type": "hello", "agent_id": "late-subscriber-1"}),
    )
    .await;
    send_json(
        &mut late,
        json!({"type": "subscribe", "topic": "replay:topic"}),
    )
    .await;

    // First frame should be the replayed message.
    let replayed = recv_json(&mut late).await;
    assert_eq!(replayed["type"], "topic_message");
    assert_eq!(replayed["msg_type"], "first");
    assert_eq!(replayed["payload"]["n"], 1);

    // Second frame should be the subscription ACK.
    let ack = recv_json(&mut late).await;
    assert_eq!(ack["type"], "ack");
    assert_eq!(ack["event"], "subscribed:replay:topic");
}
