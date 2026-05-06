//! SSE client for streaming remote `DashboardEvent` progress from `roko serve`.
//!
//! Connects to the `/api/events` SSE endpoint and prints structured events
//! to stderr using the same formatting as [`FormattedStderrSink`].
//!
//! The SSE protocol is trivially simple: each event is one or more `data:` lines
//! separated by a blank line. We also handle `id:` for reconnection and ignore
//! `:` comment lines (keep-alives).
//!
//! [`FormattedStderrSink`]: super::output_sink::FormattedStderrSink

use std::io::Write as _;
use std::time::Duration;

use futures::StreamExt as _;
use roko_core::dashboard_snapshot::DashboardEvent;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::output_sink::format_dashboard_event;

/// SSE client that streams `DashboardEvent`s from a `roko serve` instance
/// and prints formatted lines to stderr.
pub struct SseStreamClient {
    /// Base URL of the roko serve instance (e.g. `http://localhost:6677`).
    url: String,
    /// Whether to emit ANSI color codes.
    color: bool,
    /// Maximum number of reconnection attempts before giving up.
    max_retries: u32,
}

impl SseStreamClient {
    /// Create a new SSE client pointing at the given `roko serve` base URL.
    pub fn new(url: &str, color: bool) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            color,
            max_retries: 3,
        }
    }

    /// Stream events until cancelled or disconnected (with retries exhausted).
    ///
    /// Returns `Ok(())` on clean cancellation, `Err` on connection failure.
    pub async fn stream(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        let mut attempt = 0u32;
        let mut last_event_id: Option<String> = None;

        loop {
            if cancel.is_cancelled() {
                return Ok(());
            }

            let endpoint = format!("{}/api/events", self.url);
            debug!(url = %endpoint, attempt, "connecting to SSE endpoint");

            let mut req = reqwest::Client::new().get(&endpoint);
            if let Some(ref id) = last_event_id {
                req = req.header("Last-Event-ID", id.as_str());
            }

            let response = match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    attempt = 0; // Reset on successful connect.
                    resp
                }
                Ok(resp) => {
                    let status = resp.status();
                    warn!(status = %status, "SSE endpoint returned non-success status");
                    attempt += 1;
                    if attempt > self.max_retries {
                        anyhow::bail!(
                            "SSE connection failed after {} retries (last status: {status})",
                            self.max_retries
                        );
                    }
                    let backoff = exponential_backoff(attempt);
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => continue,
                        _ = cancel.cancelled() => return Ok(()),
                    }
                }
                Err(err) => {
                    warn!(error = %err, "SSE connection error");
                    attempt += 1;
                    if attempt > self.max_retries {
                        anyhow::bail!(
                            "SSE connection failed after {} retries: {err}",
                            self.max_retries
                        );
                    }
                    let backoff = exponential_backoff(attempt);
                    tokio::select! {
                        _ = tokio::time::sleep(backoff) => continue,
                        _ = cancel.cancelled() => return Ok(()),
                    }
                }
            };

            // Stream the response body as bytes, parsing SSE frames.
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            let mut current_id: Option<String> = None;
            let mut data_lines: Vec<String> = Vec::new();

            loop {
                let chunk = tokio::select! {
                    chunk = stream.next() => chunk,
                    _ = cancel.cancelled() => return Ok(()),
                };

                let Some(chunk_result) = chunk else {
                    // Stream ended; try reconnecting.
                    debug!("SSE stream ended, will attempt reconnect");
                    break;
                };

                let bytes = match chunk_result {
                    Ok(b) => b,
                    Err(err) => {
                        warn!(error = %err, "SSE read error");
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&bytes));

                // Process complete lines from the buffer.
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        // Blank line = dispatch accumulated event.
                        if !data_lines.is_empty() {
                            let data = data_lines.join("\n");
                            data_lines.clear();

                            if let Some(ref id) = current_id {
                                last_event_id = Some(id.clone());
                            }
                            current_id = None;

                            self.handle_sse_data(&data);
                        }
                    } else if let Some(data) = line.strip_prefix("data:") {
                        data_lines.push(data.trim_start().to_string());
                    } else if let Some(id) = line.strip_prefix("id:") {
                        current_id = Some(id.trim().to_string());
                    } else if line.starts_with(':') {
                        // Comment / keep-alive — ignore.
                    }
                    // Other lines (event:, retry:) are ignored per spec
                    // requirements; we only consume data + id.
                }
            }

            // If we got here, the stream ended. Try reconnecting.
            attempt += 1;
            if attempt > self.max_retries {
                anyhow::bail!(
                    "SSE stream disconnected after {} reconnection attempts",
                    self.max_retries
                );
            }
            let backoff = exponential_backoff(attempt);
            debug!(attempt, backoff_ms = backoff.as_millis(), "reconnecting after stream end");
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {},
                _ = cancel.cancelled() => return Ok(()),
            }
        }
    }

    /// Parse a `data:` payload as a `DashboardEvent` and print it.
    fn handle_sse_data(&self, data: &str) {
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return;
        }

        match serde_json::from_str::<DashboardEvent>(trimmed) {
            Ok(event) => {
                if let Some(line) = format_dashboard_event(&event, self.color) {
                    let mut stderr = std::io::stderr().lock();
                    let _ = writeln!(stderr, "{line}");
                }
            }
            Err(err) => {
                debug!(error = %err, data = %trimmed, "failed to parse SSE DashboardEvent");
            }
        }
    }
}

/// Exponential backoff: 1s, 2s, 4s, capped at 8s.
fn exponential_backoff(attempt: u32) -> Duration {
    let secs = (1u64 << attempt.min(3)).min(8);
    Duration::from_secs(secs)
}

// ─── SSE Parsing helpers (standalone, for testing) ──────────────────────────

/// Parse a single SSE frame from raw text. Returns `(id, data)` pairs.
///
/// This is exposed for unit testing the SSE parser logic.
pub(crate) fn parse_sse_frames(text: &str) -> Vec<(Option<String>, String)> {
    let mut frames = Vec::new();
    let mut current_id: Option<String> = None;
    let mut data_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        if line.is_empty() {
            if !data_lines.is_empty() {
                frames.push((current_id.take(), data_lines.join("\n")));
                data_lines.clear();
            }
        } else if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.trim_start().to_string());
        } else if let Some(id) = line.strip_prefix("id:") {
            current_id = Some(id.trim().to_string());
        } else if line.starts_with(':') {
            // Comment — ignore.
        }
    }
    // Trailing data without final blank line.
    if !data_lines.is_empty() {
        frames.push((current_id.take(), data_lines.join("\n")));
    }

    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_frame() {
        let input = "id: 1\ndata: {\"type\":\"plan_started\",\"plan_id\":\"p1\"}\n\n";
        let frames = parse_sse_frames(input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0.as_deref(), Some("1"));
        assert!(frames[0].1.contains("plan_started"));
    }

    #[test]
    fn parse_multiple_frames() {
        let input = concat!(
            "id: 1\n",
            "data: {\"type\":\"plan_started\",\"plan_id\":\"p1\"}\n",
            "\n",
            "id: 2\n",
            "data: {\"type\":\"plan_completed\",\"plan_id\":\"p1\",\"success\":true}\n",
            "\n",
        );
        let frames = parse_sse_frames(input);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0.as_deref(), Some("1"));
        assert_eq!(frames[1].0.as_deref(), Some("2"));
    }

    #[test]
    fn parse_multiline_data() {
        let input = "data: line1\ndata: line2\n\n";
        let frames = parse_sse_frames(input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].1, "line1\nline2");
    }

    #[test]
    fn parse_ignores_comments() {
        let input = ": keepalive\ndata: {\"type\":\"error\",\"message\":\"oops\"}\n\n";
        let frames = parse_sse_frames(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].1.contains("error"));
    }

    #[test]
    fn parse_no_id() {
        let input = "data: hello\n\n";
        let frames = parse_sse_frames(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].0.is_none());
        assert_eq!(frames[0].1, "hello");
    }

    #[test]
    fn parse_invalid_json_does_not_panic() {
        let client = SseStreamClient::new("http://localhost:6677", false);
        // This should not panic, just log a debug warning.
        client.handle_sse_data("not valid json");
    }

    #[test]
    fn parse_valid_event_formats() {
        let client = SseStreamClient::new("http://localhost:6677", false);
        // Should not panic and should produce output on stderr.
        client.handle_sse_data(r#"{"type":"plan_started","plan_id":"test-plan"}"#);
    }

    #[test]
    fn exponential_backoff_values() {
        assert_eq!(exponential_backoff(0), Duration::from_secs(1));
        assert_eq!(exponential_backoff(1), Duration::from_secs(2));
        assert_eq!(exponential_backoff(2), Duration::from_secs(4));
        assert_eq!(exponential_backoff(3), Duration::from_secs(8));
        assert_eq!(exponential_backoff(4), Duration::from_secs(8)); // capped
        assert_eq!(exponential_backoff(100), Duration::from_secs(8)); // capped
    }

    #[test]
    fn parse_bulk_event_filtered() {
        let client = SseStreamClient::new("http://localhost:6677", false);
        // Bulk data events should be silently filtered (no output).
        client.handle_sse_data(r#"{"type":"cascade_router_updated","snapshot_json":"{}"}"#);
    }
}
