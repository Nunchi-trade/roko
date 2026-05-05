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
