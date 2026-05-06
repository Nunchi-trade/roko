//! Non-blocking HTTP forwarding sink for `RuntimeEvent`s.

use std::time::Duration;

use roko_core::RuntimeEvent;
use roko_core::foundation::EventConsumer;
use tokio::sync::mpsc;
use tracing::{debug, warn};

const CHANNEL_CAPACITY: usize = 256;
const BATCH_MAX_EVENTS: usize = 32;
const BATCH_WINDOW: Duration = Duration::from_millis(50);

/// Fire-and-forget sink that forwards runtime events to a running `roko serve`.
#[derive(Clone)]
pub struct HttpEventSink {
    tx: mpsc::Sender<RuntimeEvent>,
}

impl HttpEventSink {
    /// Creates a sink from `ROKO_SERVE_URL`, returning `None` when it is unset.
    pub fn from_env() -> Option<Self> {
        let serve_url = std::env::var("ROKO_SERVE_URL").ok()?;
        let serve_url = serve_url.trim();
        if serve_url.is_empty() {
            return None;
        }

        let auth_token = std::env::var("ROKO_SERVER_AUTH_TOKEN")
            .ok()
            .map(|token| token.trim().to_string())
            .filter(|token| !token.is_empty());

        Some(Self::new(serve_url, auth_token))
    }

    /// Creates a sink for an explicit server base URL.
    pub fn new(serve_url: impl AsRef<str>, auth_token: Option<String>) -> Self {
        let endpoint = batch_endpoint(serve_url.as_ref());
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let client = reqwest::Client::new();

        tokio::spawn(async move {
            batch_and_post(rx, client, endpoint, auth_token).await;
        });

        Self { tx }
    }

    /// Attempts to enqueue an event without blocking the caller.
    pub fn emit(&self, event: RuntimeEvent) {
        let _ = self.tx.try_send(event);
    }
}

impl EventConsumer for HttpEventSink {
    fn consume(&self, event: &RuntimeEvent) {
        self.emit(event.clone());
    }
}

fn batch_endpoint(serve_url: &str) -> String {
    format!(
        "{}/api/events/ingest/batch",
        serve_url.trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn batch_endpoint_trims_trailing_slash() {
        assert_eq!(
            batch_endpoint("http://127.0.0.1:6677/"),
            "http://127.0.0.1:6677/api/events/ingest/batch"
        );
        assert_eq!(
            batch_endpoint("http://127.0.0.1:6677"),
            "http://127.0.0.1:6677/api/events/ingest/batch"
        );
        assert_eq!(
            batch_endpoint("http://host:1234///"),
            "http://host:1234/api/events/ingest/batch"
        );
    }

    #[test]
    fn emit_does_not_block_when_channel_saturated() {
        // Use a minimal channel capacity and prove emit returns immediately
        // even when the background task hasn't drained the channel.
        let (tx, _rx) = mpsc::channel(1);
        let sink = HttpEventSink { tx };

        // Fill the channel beyond capacity; try_send will silently drop.
        for _ in 0..10 {
            sink.emit(RuntimeEvent::AgentOutput {
                run_id: "drop-test".into(),
                agent_id: "a1".into(),
                chunk: "x".into(),
            });
        }
        // If we reached here, emit didn't block or panic.
    }

    #[test]
    fn from_env_returns_none_when_unset() {
        // Ensure the env var is not set in the test environment.
        if std::env::var("ROKO_SERVE_URL").is_ok() {
            // Skip: the env var is set in this test run.
            return;
        }
        assert!(HttpEventSink::from_env().is_none());
    }

    #[tokio::test]
    async fn sink_posts_to_batch_endpoint() {
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::{Arc, Mutex};

        // Capture posted batches in a shared vec.
        let captured: Arc<Mutex<Vec<Vec<RuntimeEvent>>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured);

        let app = Router::new().route(
            "/api/events/ingest/batch",
            post(move |Json(events): Json<Vec<RuntimeEvent>>| {
                let captured = Arc::clone(&captured_clone);
                async move {
                    captured.lock().unwrap().push(events);
                    StatusCode::ACCEPTED
                }
            }),
        );

        // Bind to a random port.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let serve_url = format!("http://127.0.0.1:{}", addr.port());
        let sink = HttpEventSink::new(&serve_url, None);

        sink.emit(RuntimeEvent::AgentOutput {
            run_id: "sink-test".into(),
            agent_id: "a1".into(),
            chunk: "hello-sink".into(),
        });

        // Wait for the batch window to flush.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let batches = captured.lock().unwrap();
        assert!(
            !batches.is_empty(),
            "sink should have posted at least one batch"
        );
        let all_events: Vec<&RuntimeEvent> = batches.iter().flat_map(|b| b.iter()).collect();
        assert!(
            all_events.iter().any(|e| matches!(
                e,
                RuntimeEvent::AgentOutput { chunk, .. } if chunk == "hello-sink"
            )),
            "posted batch should contain the emitted event"
        );
    }

    #[tokio::test]
    async fn sink_includes_bearer_token() {
        use axum::http::HeaderMap as AxumHeaderMap;
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::{Arc, Mutex};

        let captured_auth: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured_auth);

        let app = Router::new().route(
            "/api/events/ingest/batch",
            post(
                move |headers: AxumHeaderMap, Json(_events): Json<Vec<RuntimeEvent>>| {
                    let captured = Arc::clone(&captured_clone);
                    async move {
                        let auth = headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(|s| s.to_string());
                        captured.lock().unwrap().push(auth);
                        StatusCode::ACCEPTED
                    }
                },
            ),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let serve_url = format!("http://127.0.0.1:{}", addr.port());
        let sink = HttpEventSink::new(&serve_url, Some("test-secret".to_owned()));

        sink.emit(RuntimeEvent::AgentOutput {
            run_id: "auth-test".into(),
            agent_id: "a1".into(),
            chunk: "x".into(),
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        let auths = captured_auth.lock().unwrap();
        assert!(!auths.is_empty(), "should have received at least one post");
        assert_eq!(
            auths[0].as_deref(),
            Some("Bearer test-secret"),
            "bearer token should be included in Authorization header"
        );
    }

    #[tokio::test]
    async fn sink_batches_up_to_max() {
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::{Arc, Mutex};

        let batch_sizes: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
        let sizes_clone = Arc::clone(&batch_sizes);

        let app = Router::new().route(
            "/api/events/ingest/batch",
            post(move |Json(events): Json<Vec<RuntimeEvent>>| {
                let sizes = Arc::clone(&sizes_clone);
                async move {
                    sizes.lock().unwrap().push(events.len());
                    StatusCode::ACCEPTED
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });

        let serve_url = format!("http://127.0.0.1:{}", addr.port());
        let sink = HttpEventSink::new(&serve_url, None);

        // Emit more than BATCH_MAX_EVENTS (32) rapidly.
        for i in 0..64 {
            sink.emit(RuntimeEvent::AgentOutput {
                run_id: format!("batch-{i}"),
                agent_id: "a1".into(),
                chunk: "x".into(),
            });
        }

        // Wait for all batches to flush.
        tokio::time::sleep(Duration::from_millis(300)).await;

        let sizes = batch_sizes.lock().unwrap();
        assert!(!sizes.is_empty(), "should have received at least one batch");
        assert!(
            sizes.iter().all(|&s| s <= BATCH_MAX_EVENTS),
            "no batch should exceed {BATCH_MAX_EVENTS} events: {sizes:?}"
        );
        let total: usize = sizes.iter().sum();
        assert!(
            total >= 32,
            "should have received most of the 64 emitted events: got {total}"
        );
    }
}

async fn batch_and_post(
    mut rx: mpsc::Receiver<RuntimeEvent>,
    client: reqwest::Client,
    endpoint: String,
    auth_token: Option<String>,
) {
    while let Some(first) = rx.recv().await {
        let mut batch = Vec::with_capacity(BATCH_MAX_EVENTS);
        batch.push(first);

        let deadline = tokio::time::sleep(BATCH_WINDOW);
        tokio::pin!(deadline);

        while batch.len() < BATCH_MAX_EVENTS {
            tokio::select! {
                () = &mut deadline => {
                    break;
                }
                next = rx.recv() => {
                    match next {
                        Some(event) => batch.push(event),
                        None => break,
                    }
                }
            }
        }

        post_batch(&client, &endpoint, auth_token.as_deref(), &batch).await;
    }
}

async fn post_batch(
    client: &reqwest::Client,
    endpoint: &str,
    auth_token: Option<&str>,
    events: &[RuntimeEvent],
) {
    let mut request = client.post(endpoint).json(events);
    if let Some(token) = auth_token {
        request = request.bearer_auth(token);
    }

    match request.send().await {
        Ok(response) if response.status().is_success() => {
            debug!(
                endpoint,
                count = events.len(),
                "forwarded runtime event batch"
            );
        }
        Ok(response) => {
            warn!(
                endpoint,
                status = %response.status(),
                count = events.len(),
                "runtime event batch forward failed"
            );
        }
        Err(err) => {
            warn!(
                endpoint,
                error = %err,
                count = events.len(),
                "runtime event batch forward failed"
            );
        }
    }
}
