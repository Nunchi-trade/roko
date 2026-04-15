//! Minimal WebSocket broadcast server for demo events.

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Start a background WebSocket server and return its broadcast sender.
pub async fn start_ws_server(port: u16) -> anyhow::Result<broadcast::Sender<String>> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let local_addr = listener.local_addr()?;
    let (sender, _) = broadcast::channel::<String>(256);
    let broadcast_sender = sender.clone();

    tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                break;
            };
            let sender = broadcast_sender.clone();
            tokio::spawn(async move {
                let Ok(websocket) = accept_async(stream).await else {
                    return;
                };
                let (mut writer, mut reader) = websocket.split();
                let mut receiver = sender.subscribe();
                tracing::info!(peer = %peer, "demo ws client connected");

                let read_task = tokio::spawn(async move {
                    while let Some(message) = reader.next().await {
                        if message.is_err() {
                            break;
                        }
                    }
                });

                while let Ok(payload) = receiver.recv().await {
                    if writer.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                read_task.abort();
            });
        }
    });

    tracing::info!(
        ws_port = local_addr.port(),
        "demo event websocket server listening"
    );
    Ok(sender)
}
