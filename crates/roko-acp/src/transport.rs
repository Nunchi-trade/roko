//! Stdio transport layer for JSON-RPC messages.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Stdin, Stdout},
    sync::{oneshot, Mutex as AsyncMutex},
};
use tracing::warn;

use crate::types::{
    JsonRpcError, JsonRpcId, JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse,
};

/// Errors returned by the ACP stdio transport.
#[derive(Debug, Error)]
pub enum TransportError {
    /// Underlying stdio I/O failed.
    #[error("stdio transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization or deserialization failed.
    #[error("stdio transport JSON codec failed: {0}")]
    Json(#[from] serde_json::Error),
    /// The pending request registry could not be accessed.
    #[error("pending request registry is poisoned")]
    PendingRequestsPoisoned,
    /// A client response never arrived for an outbound request.
    #[error("client response channel closed for request id {request_id}")]
    ResponseChannelClosed {
        /// The numeric JSON-RPC request identifier.
        request_id: u64,
    },
}

/// Result alias for ACP stdio transport operations.
pub type TransportResult<T> = Result<T, TransportError>;

/// A stdio JSON-RPC transport for ACP messages.
#[derive(Debug)]
pub struct StdioTransport<R = Stdin, W = Stdout> {
    reader: Arc<AsyncMutex<BufReader<R>>>,
    writer: Arc<AsyncMutex<W>>,
    next_id: Arc<AtomicU64>,
    pending_requests: Arc<Mutex<HashMap<u64, oneshot::Sender<JsonRpcResponse>>>>,
}

impl<R, W> Clone for StdioTransport<R, W> {
    fn clone(&self) -> Self {
        Self {
            reader: Arc::clone(&self.reader),
            writer: Arc::clone(&self.writer),
            next_id: Arc::clone(&self.next_id),
            pending_requests: Arc::clone(&self.pending_requests),
        }
    }
}

impl StdioTransport<Stdin, Stdout> {
    /// Creates a stdio transport bound to the process stdin/stdout streams.
    pub fn new() -> Self {
        Self::from_io(tokio::io::stdin(), tokio::io::stdout())
    }
}

impl Default for StdioTransport<Stdin, Stdout> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R, W> StdioTransport<R, W>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    /// Creates a transport from arbitrary async reader and writer handles.
    pub fn from_io(reader: R, writer: W) -> Self {
        Self {
            reader: Arc::new(AsyncMutex::new(BufReader::new(reader))),
            writer: Arc::new(AsyncMutex::new(writer)),
            next_id: Arc::new(AtomicU64::new(1)),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Reads one newline-delimited JSON-RPC message from stdin.
    ///
    /// Returns `Ok(None)` when EOF is reached before any bytes are read.
    pub async fn read_message(&mut self) -> TransportResult<Option<JsonRpcMessage>> {
        let mut line = String::new();
        let bytes_read = {
            let mut reader = self.reader.lock().await;
            reader.read_line(&mut line).await?
        };

        if bytes_read == 0 {
            return Ok(None);
        }

        let message = serde_json::from_str::<JsonRpcMessage>(&line)?;
        Ok(Some(message))
    }

    /// Sends a successful JSON-RPC response to the client.
    pub async fn send_response(
        &mut self,
        id: JsonRpcId,
        result: serde_json::Value,
    ) -> TransportResult<()> {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        };

        self.write_message(&response).await
    }

    /// Sends a JSON-RPC error response to the client.
    pub async fn send_error(
        &mut self,
        id: JsonRpcId,
        code: i32,
        message: String,
    ) -> TransportResult<()> {
        let response = JsonRpcResponse {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        };

        self.write_message(&response).await
    }

    /// Sends a JSON-RPC notification to the client.
    pub async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> TransportResult<()> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_owned(),
            method: method.to_owned(),
            params: Some(params),
        };

        self.write_message(&notification).await
    }

    /// Sends a JSON-RPC request to the client and waits for the matching response.
    pub async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> TransportResult<JsonRpcResponse> {
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();

        self.pending_requests
            .lock()
            .map_err(|_| TransportError::PendingRequestsPoisoned)?
            .insert(request_id, sender);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_owned(),
            id: JsonRpcId::Number(request_id),
            method: method.to_owned(),
            params: Some(params),
        };

        if let Err(error) = self.write_message(&request).await {
            self.pending_requests
                .lock()
                .map_err(|_| TransportError::PendingRequestsPoisoned)?
                .remove(&request_id);
            return Err(error);
        }

        receiver.await.map_err(|_| TransportError::ResponseChannelClosed {
            request_id,
        })
    }

    /// Routes an incoming JSON-RPC response to the matching pending outbound request.
    pub fn handle_incoming_response(&mut self, response: JsonRpcResponse) {
        let JsonRpcId::Number(request_id) = response.id.clone() else {
            warn!(
                response_id = ?response.id,
                "received outbound response with non-numeric id"
            );
            return;
        };

        let pending = match self.pending_requests.lock() {
            Ok(mut pending_requests) => pending_requests.remove(&request_id),
            Err(_) => {
                warn!("pending request registry is poisoned");
                None
            }
        };

        match pending {
            Some(sender) => {
                if sender.send(response).is_err() {
                    warn!(request_id, "response receiver dropped before delivery");
                }
            }
            None => {
                warn!(request_id, "received response for unknown outbound request");
            }
        }
    }

    async fn write_message<T>(&self, message: &T) -> TransportResult<()>
    where
        T: serde::Serialize,
    {
        let bytes = serde_json::to_vec(message)?;
        let mut writer = self.writer.lock().await;
        writer.write_all(&bytes).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, empty, sink};

    use super::*;
    use crate::types::JsonRpcNotification;

    #[tokio::test]
    async fn reads_valid_json_rpc_request() {
        let (client, server) = duplex(1024);
        let writer_task = tokio::spawn(async move {
            let mut client = client;
            client
                .write_all(br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#)
                .await
                .expect("write request bytes");
            client.write_all(b"\n").await.expect("write newline");
        });

        let mut transport = StdioTransport::from_io(server, sink());
        let message = transport
            .read_message()
            .await
            .expect("read message")
            .expect("message present");

        writer_task.await.expect("writer task");

        let JsonRpcMessage::Request(request) = message else {
            panic!("expected request message");
        };
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.id, JsonRpcId::Number(1));
        assert_eq!(request.method, "initialize");
        assert_eq!(request.params, Some(json!({ "protocolVersion": 1 })));
    }

    #[tokio::test]
    async fn returns_none_on_eof() {
        let mut transport = StdioTransport::from_io(empty(), sink());

        let message = transport.read_message().await.expect("read EOF");

        assert!(message.is_none());
    }

    #[tokio::test]
    async fn writes_json_rpc_notification() {
        let (mut client_reader, server_writer) = duplex(1024);
        let mut transport = StdioTransport::from_io(empty(), server_writer);

        transport
            .send_notification("session/update", json!({ "sessionUpdate": "plan" }))
            .await
            .expect("write notification");

        let mut line = String::new();
        BufReader::new(&mut client_reader)
            .read_line(&mut line)
            .await
            .expect("read notification line");

        let notification: JsonRpcNotification =
            serde_json::from_str(&line).expect("parse notification payload");
        assert_eq!(notification.method, "session/update");
    }
}
