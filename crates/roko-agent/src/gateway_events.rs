//! Durable gateway event log and projections.
//!
//! The gateway log is append-only JSONL. Missing log files load as empty
//! projections so callers can query a fresh worktree without special casing
//! first-run state.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One model call observed at the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayEvent {
    /// Unique request id assigned by the gateway.
    pub request_id: String,
    /// Who initiated this call.
    pub caller: String,
    /// Model slug that served the request.
    pub model: String,
    /// Provider that served the request, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens generated.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Wall-clock latency in milliseconds.
    pub latency_ms: u64,
    /// Whether a cache hit satisfied this request.
    pub cache_hit: bool,
    /// Whether the call completed successfully.
    #[serde(default = "default_success")]
    pub success: bool,
    /// Error message if the call failed, None on success.
    pub error: Option<String>,
    /// ISO 8601 timestamp.
    pub timestamp: String,
}

/// Append-only writer for gateway events.
pub struct GatewayEventWriter {
    path: PathBuf,
}

impl GatewayEventWriter {
    /// Create a writer targeting the given JSONL file.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Create a writer at the conventional location:
    /// `{workdir}/.roko/learn/gateway.jsonl`.
    #[must_use]
    pub fn for_workdir(workdir: &Path) -> Self {
        let path = gateway_events_path(workdir);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        Self { path }
    }

    /// Append a single gateway event as a JSON line.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the parent directory or file cannot be
    /// created, the event cannot be serialized, or the line cannot be
    /// written.
    pub fn write(&self, event: &GatewayEvent) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut line = serde_json::to_vec(event)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        line.push(b'\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)
    }

    /// Load a projection from this writer's target file.
    pub fn projection(&self) -> io::Result<GatewayProjection> {
        GatewayProjection::load(&self.path)
    }
}

/// In-memory projection over durable gateway events.
pub struct GatewayProjection {
    events: Vec<GatewayEvent>,
}

impl GatewayProjection {
    /// Load all events from the given JSONL file.
    ///
    /// Missing files are treated as empty projections.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be read, or
    /// [`io::ErrorKind::InvalidData`] if any non-empty line is invalid JSON.
    pub fn load(path: &Path) -> io::Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(Self { events: Vec::new() });
            }
            Err(err) => return Err(err),
        };

        let mut events = Vec::new();
        for raw in text.lines() {
            if raw.trim().is_empty() {
                continue;
            }
            let mut event: GatewayEvent = serde_json::from_str(raw)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            if event.error.is_some() {
                event.success = false;
            }
            events.push(event);
        }

        Ok(Self { events })
    }

    /// Load from the conventional workdir location.
    ///
    /// Missing files are treated as empty projections.
    pub fn for_workdir(workdir: &Path) -> io::Result<Self> {
        Self::load(&gateway_events_path(workdir))
    }

    /// Find an event by request_id.
    #[must_use]
    pub fn by_request_id(&self, request_id: &str) -> Option<&GatewayEvent> {
        self.events
            .iter()
            .find(|event| event.request_id == request_id)
    }

    /// Aggregate stats grouped by caller.
    #[must_use]
    pub fn stats_by_caller(&self) -> HashMap<String, AggregateStats> {
        self.aggregate_by(|event| &event.caller)
    }

    /// Aggregate stats grouped by model.
    #[must_use]
    pub fn stats_by_model(&self) -> HashMap<String, AggregateStats> {
        self.aggregate_by(|event| &event.model)
    }

    /// Aggregate stats grouped by provider.
    #[must_use]
    pub fn stats_by_provider(&self) -> HashMap<String, AggregateStats> {
        let mut stats: HashMap<String, AggregateStats> = HashMap::new();
        for event in &self.events {
            let provider = event.provider.as_deref().unwrap_or("unknown");
            let aggregate = stats.entry(provider.to_string()).or_default();
            aggregate.record(event);
        }
        stats
    }

    /// Total number of events.
    #[must_use]
    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Total cost across all events.
    #[must_use]
    pub fn total_cost_usd(&self) -> f64 {
        self.events.iter().map(|event| event.cost_usd).sum()
    }

    fn aggregate_by<'a>(
        &'a self,
        key: impl Fn(&'a GatewayEvent) -> &'a str,
    ) -> HashMap<String, AggregateStats> {
        let mut stats: HashMap<String, AggregateStats> = HashMap::new();
        for event in &self.events {
            let aggregate = stats.entry(key(event).to_string()).or_default();
            aggregate.record(event);
        }
        stats
    }
}

fn default_success() -> bool {
    true
}

/// Aggregated gateway statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregateStats {
    pub count: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
    pub cache_hits: u64,
    pub errors: u64,
}

impl AggregateStats {
    fn record(&mut self, event: &GatewayEvent) {
        self.count += 1;
        self.total_input_tokens += event.input_tokens;
        self.total_output_tokens += event.output_tokens;
        self.total_cost_usd += event.cost_usd;
        if event.cache_hit {
            self.cache_hits += 1;
        }
        if event.error.is_some() {
            self.errors += 1;
        }
    }
}

fn gateway_events_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("learn").join("gateway.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn event(request_id: &str, caller: &str, model: &str) -> GatewayEvent {
        GatewayEvent {
            request_id: request_id.to_string(),
            caller: caller.to_string(),
            model: model.to_string(),
            provider: Some("test-provider".to_string()),
            input_tokens: 10,
            output_tokens: 20,
            cost_usd: 0.03,
            latency_ms: 250,
            cache_hit: false,
            success: true,
            error: None,
            timestamp: "2026-04-28T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gateway.jsonl");
        let writer = GatewayEventWriter::new(&path);

        writer
            .write(&event("req-1", "cli", "model-a"))
            .expect("write req-1");
        writer
            .write(&event("req-2", "serve", "model-b"))
            .expect("write req-2");
        writer
            .write(&event("req-3", "cli", "model-a"))
            .expect("write req-3");

        let projection = GatewayProjection::load(&path).expect("load");
        assert_eq!(projection.total_events(), 3);
        assert_eq!(
            projection.by_request_id("req-1").expect("req-1").caller,
            "cli"
        );
        assert_eq!(
            projection.by_request_id("req-2").expect("req-2").model,
            "model-b"
        );
        assert!((projection.total_cost_usd() - 0.09).abs() < f64::EPSILON);
    }

    #[test]
    fn by_request_id_finds_correct_event() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gateway.jsonl");
        let writer = GatewayEventWriter::new(&path);

        writer
            .write(&event("req-a", "cli", "model-a"))
            .expect("write req-a");
        writer
            .write(&event("req-b", "serve", "model-b"))
            .expect("write req-b");

        let projection = GatewayProjection::load(&path).expect("load");
        assert_eq!(
            projection.by_request_id("req-a").expect("req-a").caller,
            "cli"
        );
        assert_eq!(
            projection.by_request_id("req-b").expect("req-b").caller,
            "serve"
        );
        assert!(projection.by_request_id("missing").is_none());
    }

    #[test]
    fn stats_by_model_aggregates_correctly() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gateway.jsonl");
        let writer = GatewayEventWriter::new(&path);

        let mut first = event("req-1", "cli", "model-a");
        first.input_tokens = 15;
        first.output_tokens = 25;
        first.cost_usd = 0.04;
        first.cache_hit = true;

        let mut second = event("req-2", "serve", "model-a");
        second.input_tokens = 5;
        second.output_tokens = 10;
        second.cost_usd = 0.02;
        second.error = Some("rate limited".to_string());

        let mut third = event("req-3", "serve", "model-b");
        third.input_tokens = 7;
        third.output_tokens = 8;
        third.cost_usd = 0.01;

        writer.write(&first).expect("write first");
        writer.write(&second).expect("write second");
        writer.write(&third).expect("write third");

        let stats = GatewayProjection::load(&path)
            .expect("load")
            .stats_by_model();

        let model_a = stats.get("model-a").expect("model-a");
        assert_eq!(model_a.count, 2);
        assert_eq!(model_a.total_input_tokens, 20);
        assert_eq!(model_a.total_output_tokens, 35);
        assert!((model_a.total_cost_usd - 0.06).abs() < f64::EPSILON);
        assert_eq!(model_a.cache_hits, 1);
        assert_eq!(model_a.errors, 1);

        let model_b = stats.get("model-b").expect("model-b");
        assert_eq!(model_b.count, 1);
        assert_eq!(model_b.total_input_tokens, 7);
        assert_eq!(model_b.total_output_tokens, 8);
        assert!((model_b.total_cost_usd - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_file_loads_cleanly() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("gateway.jsonl");
        fs::write(&path, "").expect("create empty file");

        let projection = GatewayProjection::load(&path).expect("load");
        assert_eq!(projection.total_events(), 0);
        assert_eq!(projection.total_cost_usd(), 0.0);
    }
}
