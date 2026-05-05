//! Shared WebSocket bridge for reverse-proxying upstream WebSocket services.

use axum::extract::ws::{Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// Bidirectional WebSocket bridge: shuttles frames between the client-facing
/// axum `WebSocket` and an upstream `tokio-tungstenite` connection.
///
/// Both sides are closed when either disconnects.
pub(crate) async fn bridge_ws(server_socket: WebSocket, upstream_url: String) {
    let Ok((upstream, _)) = connect_async(&upstream_url).await else {
        tracing::warn!(url = %upstream_url, "proxy_ws: failed to connect upstream");
        return;
    };

    let (mut server_tx, mut server_rx) = server_socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    loop {
        tokio::select! {
            // Client → upstream
            msg = server_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if upstream_tx.send(WsMessage::Text(text.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        if upstream_tx.send(WsMessage::Binary(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if upstream_tx.send(WsMessage::Ping(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(data))) => {
                        if upstream_tx.send(WsMessage::Pong(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                }
            }
            // Upstream → client
            msg = upstream_rx.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        if server_tx.send(Message::Text(text.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Binary(data))) => {
                        if server_tx.send(Message::Binary(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        if server_tx.send(Message::Ping(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Pong(data))) => {
                        if server_tx.send(Message::Pong(data.to_vec().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(WsMessage::Frame(_))) => {}
                }
            }
        }
    }

    let _ = server_tx.close().await;
    let _ = upstream_tx.close().await;
}
