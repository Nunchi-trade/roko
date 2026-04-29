//! Shareable run transcript routes.
//!
//! `GET /api/runs/{id}` — JSON transcript.
//! `GET /runs/{id}` — Self-contained HTML page.

use std::sync::{Arc, OnceLock};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Response},
};
use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use roko_core::runtime_event::{RuntimeEvent, RuntimeEventEnvelope, WorkflowOutcome};
use roko_core::{config::schema::RokoConfig, obs::LogScrubber};
use roko_orchestrator::{ServiceConfig, ServiceFactory};
use roko_runtime::{
    JsonlLogger, WorkflowConfig, WorkflowEngine, WorkflowRunConfig, WorkflowRunReport,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::AppState;

/// A persisted run transcript for sharing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTranscript {
    /// Unique run ID.
    pub id: String,
    /// Agent name.
    pub agent: String,
    /// Agent role.
    pub role: String,
    /// The original prompt.
    pub prompt: String,
    /// Whether the run succeeded.
    pub success: bool,
    /// Gate verdicts: (name, passed).
    pub gates: Vec<(String, bool)>,
    /// Agent output text.
    pub output: Option<String>,
    /// Cost in USD.
    pub cost_usd: Option<f64>,
    /// Input tokens.
    pub input_tokens: Option<u64>,
    /// Output tokens.
    pub output_tokens: Option<u64>,
    /// Model used.
    pub model: Option<String>,
    /// Duration in seconds.
    pub duration_s: Option<f64>,
    /// Episode ID.
    pub episode_id: Option<String>,
    /// Durable runtime event transcript.
    #[serde(default)]
    pub transcript: Vec<RuntimeEventEnvelope>,
    /// Timestamp (ISO 8601).
    pub timestamp: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateShareRequest {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    workflow: Option<String>,
    #[serde(default)]
    enabled_gates: Option<Vec<String>>,
    /// Request a public base URL instead of the default local-only path.
    #[serde(default)]
    public: bool,
    /// Create a permanent share that never expires.
    #[serde(default, alias = "no-expire")]
    no_expire: bool,
}

/// Metadata stored alongside a shared transcript.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShareMetadata {
    /// Whether the transcript was scrubbed before persistence.
    pub scrubbed: bool,
    /// Whether the share should resolve through the configured public URL.
    pub public: bool,
    /// Expiration timestamp; `None` means the share is permanent.
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

/// On-disk representation for newly shared transcripts.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SharedTranscriptRecord {
    #[serde(default)]
    metadata: ShareMetadata,
    transcript: RunTranscript,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredTranscript {
    Wrapped(SharedTranscriptRecord),
    Bare(RunTranscript),
}

#[derive(Debug, Clone)]
struct LoadedTranscript {
    metadata: ShareMetadata,
    transcript: RunTranscript,
}

#[derive(Debug, Clone)]
enum TranscriptLookup {
    Missing,
    Expired { expires_at: DateTime<Utc> },
    Found(LoadedTranscript),
}

/// `GET /api/runs/{id}` — JSON transcript.
pub async fn get_run_json(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match load_transcript(&state, &id) {
        TranscriptLookup::Found(loaded) => Json(json!(loaded.transcript)).into_response(),
        TranscriptLookup::Expired { expires_at } => expired_share_response(expires_at),
        TranscriptLookup::Missing => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /api/shared/{token}` — JSON transcript for a shared token.
pub async fn get_shared_run(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    match load_transcript(&state, &token) {
        TranscriptLookup::Found(loaded) => Json(json!(loaded.transcript)).into_response(),
        TranscriptLookup::Expired { expires_at } => expired_share_response(expires_at),
        TranscriptLookup::Missing => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /api/runs/{id}/share` — create a shared run token.
pub async fn create_share(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    payload: Option<Json<CreateShareRequest>>,
) -> Response {
    let token = format!("{}-{:04x}", id, std::process::id() as u16);
    let requested_public = payload
        .as_ref()
        .map(|Json(request)| request.public)
        .unwrap_or(false);
    let no_expire = payload
        .as_ref()
        .map(|Json(request)| request.no_expire)
        .unwrap_or(false);
    let workspace_config = state.load_roko_config();
    let shared_dir = state.workdir.join(".roko").join("shared");
    if let Err(e) = std::fs::create_dir_all(&shared_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("mkdir failed: {e}")})),
        )
            .into_response();
    }
    let now = Utc::now();
    let loaded = load_transcript_record(&state, &token)
        .filter(|loaded| share_expired_at(&loaded.metadata, &now).is_none());
    let had_existing_transcript = loaded.is_some();
    let existing_public = loaded
        .as_ref()
        .map(|loaded| loaded.metadata.public)
        .unwrap_or(false);
    let existing_expires_at = loaded
        .as_ref()
        .and_then(|loaded| loaded.metadata.expires_at);
    let transcript = match loaded {
        Some(loaded) => loaded.transcript,
        None => match transcript_from_runtime_events(&state, &id, &token) {
            Some(transcript) => transcript,
            None => {
                let Some(Json(request)) = payload else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                match run_shared_workflow(&state, &token, request, Arc::clone(&workspace_config))
                    .await
                {
                    Ok(transcript) => transcript,
                    Err(e) => {
                        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e})))
                            .into_response();
                    }
                }
            }
        },
    };
    let public = requested_public || existing_public;
    let expires_at = if no_expire {
        None
    } else if had_existing_transcript {
        existing_expires_at
    } else {
        Some(default_share_expires_at(
            workspace_config.serve.share_ttl_days,
        ))
    };
    let metadata = ShareMetadata {
        scrubbed: true,
        public,
        expires_at,
    };
    let transcript = match scrub_run_transcript(transcript, state.scrubber.as_ref()) {
        Some(transcript) => transcript,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "failed to scrub share transcript"})),
            )
                .into_response();
        }
    };
    let path = shared_dir.join(format!("{token}.json"));
    let stored = SharedTranscriptRecord {
        metadata: metadata.clone(),
        transcript: transcript.clone(),
    };
    let share_url =
        match share_url_for(workspace_config.relay.public_url.as_deref(), &token, public) {
            Ok(url) => url,
            Err(error) => {
                return (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response();
            }
        };
    match serde_json::to_string_pretty(&stored) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("write failed: {e}")})),
                )
                    .into_response();
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("serialize failed: {e}")})),
            )
                .into_response();
        }
    }
    Json(json!({
        "token": token,
        "url": share_url,
        "metadata": stored.metadata,
        "transcript": transcript,
    }))
    .into_response()
}

/// `GET /runs/{id}` — Self-contained HTML page.
pub async fn get_run_html(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match load_transcript(&state, &id) {
        TranscriptLookup::Found(loaded) => {
            let html = render_html(&loaded.transcript);
            Html(html).into_response()
        }
        TranscriptLookup::Expired { expires_at } => expired_share_response(expires_at),
        TranscriptLookup::Missing => StatusCode::NOT_FOUND.into_response(),
    }
}

fn load_transcript_record(state: &AppState, id: &str) -> Option<LoadedTranscript> {
    let path = state
        .workdir
        .join(".roko")
        .join("shared")
        .join(format!("{id}.json"));
    let data = std::fs::read_to_string(path).ok()?;
    let stored: StoredTranscript = serde_json::from_str(&data).ok()?;
    let (metadata, transcript) = match stored {
        StoredTranscript::Wrapped(record) => (
            ShareMetadata {
                scrubbed: true,
                public: record.metadata.public,
                expires_at: record.metadata.expires_at,
            },
            record.transcript,
        ),
        StoredTranscript::Bare(transcript) => (
            ShareMetadata {
                scrubbed: true,
                public: false,
                expires_at: None,
            },
            transcript,
        ),
    };
    let transcript = scrub_run_transcript(transcript, state.scrubber.as_ref())?;

    Some(LoadedTranscript {
        metadata,
        transcript,
    })
}

fn load_transcript(state: &AppState, id: &str) -> TranscriptLookup {
    match load_transcript_record(state, id) {
        Some(loaded) => match share_expired_at(&loaded.metadata, &Utc::now()) {
            Some(expires_at) => TranscriptLookup::Expired { expires_at },
            None => TranscriptLookup::Found(loaded),
        },
        None => TranscriptLookup::Missing,
    }
}

fn transcript_from_runtime_events(
    state: &AppState,
    run_id: &str,
    token: &str,
) -> Option<RunTranscript> {
    let path = state.layout.root().join("runtime-events.jsonl");
    let data = std::fs::read_to_string(path).ok()?;

    let mut agent = None;
    let mut role = None;
    let mut prompt = None;
    let mut output = None;
    let mut model = None;
    let mut success = None;
    let mut gates = Vec::new();
    let mut cost_usd = 0.0;
    let mut saw_cost = false;
    let mut total_tokens: u64 = 0;
    let mut saw_tokens = false;
    let mut first_ts = None;
    let mut last_ts = None;
    let mut transcript = Vec::new();

    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let envelope: RuntimeEventEnvelope = match serde_json::from_str(line) {
            Ok(envelope) => envelope,
            Err(_) => continue,
        };
        if envelope.run_id != run_id {
            continue;
        }

        first_ts.get_or_insert(envelope.ts);
        last_ts = Some(envelope.ts);
        transcript.push(envelope.clone());

        match envelope.payload {
            RuntimeEvent::WorkflowStarted {
                prompt: event_prompt,
                ..
            } => {
                if let Some(value) = non_empty_owned(event_prompt) {
                    prompt = Some(value);
                }
            }
            RuntimeEvent::AgentSpawned {
                agent_id,
                role: event_role,
                model: event_model,
                ..
            } => {
                if agent.is_none() {
                    agent = non_empty_owned(agent_id);
                }
                if role.is_none() {
                    role = non_empty_owned(event_role);
                }
                if model.is_none() {
                    model = non_empty_owned(event_model);
                }
            }
            RuntimeEvent::AgentCompleted {
                agent_id,
                output: event_output,
                cost_usd: event_cost,
                tokens_used: event_tokens,
                ..
            } => {
                if agent.is_none() {
                    agent = non_empty_owned(agent_id);
                }
                if let Some(value) = non_empty_owned(event_output) {
                    output = Some(value);
                }
                cost_usd += event_cost;
                saw_cost = true;
                total_tokens += event_tokens;
                saw_tokens = true;
            }
            RuntimeEvent::AgentFailed {
                agent_id, error, ..
            } => {
                if agent.is_none() {
                    agent = non_empty_owned(agent_id);
                }
                if let Some(value) = non_empty_owned(error) {
                    output = Some(value);
                }
            }
            RuntimeEvent::GatePassed { gate_name, .. } => {
                if let Some(name) = non_empty_owned(gate_name) {
                    gates.push((name, true));
                }
            }
            RuntimeEvent::GateFailed { gate_name, .. } => {
                if let Some(name) = non_empty_owned(gate_name) {
                    gates.push((name, false));
                }
            }
            RuntimeEvent::WorkflowCompleted {
                outcome: event_outcome,
                ..
            } => {
                success = Some(matches!(event_outcome, WorkflowOutcome::Success { .. }));
            }
            _ => {}
        }
    }

    let started_at = first_ts?;
    let finished_at = last_ts.unwrap_or(started_at);
    let duration_s = finished_at
        .signed_duration_since(started_at)
        .num_milliseconds()
        .max(0) as f64
        / 1000.0;

    Some(RunTranscript {
        id: token.to_string(),
        agent: agent?,
        role: role?,
        prompt: prompt?,
        success: success.unwrap_or(false),
        gates,
        output,
        cost_usd: saw_cost.then_some(cost_usd),
        // TODO: RuntimeEvent::AgentCompleted only carries `tokens_used` (total);
        // split into input/output tokens when the event gains that breakdown.
        input_tokens: saw_tokens.then_some(total_tokens),
        output_tokens: None,
        model,
        duration_s: Some(duration_s),
        episode_id: Some(run_id.to_string()),
        transcript,
        timestamp: started_at.to_rfc3339(),
    })
}

async fn run_shared_workflow(
    state: &AppState,
    token: &str,
    request: CreateShareRequest,
    workspace_config: Arc<RokoConfig>,
) -> Result<RunTranscript, String> {
    let prompt = request
        .prompt
        .as_deref()
        .and_then(non_empty)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "share request requires a non-empty prompt".to_string())?;
    let service_bundle = ServiceFactory::build(ServiceConfig::production(
        state.workdir.clone(),
        workspace_config.as_ref().clone(),
    ))
    .map_err(|error| format!("build workflow services: {error}"))?;

    let workflow = workflow_config_for_name(request.workflow.as_deref().unwrap_or("express"));
    let config = WorkflowRunConfig {
        prompt,
        workdir: state.workdir.clone(),
        workflow,
        enabled_gates: request.enabled_gates.unwrap_or_default(),
        shell_gates: Vec::new(),
        commit_prefix: Some("feat".to_string()),
    };

    let mut engine = WorkflowEngine::new(service_bundle.effect_services());
    engine.add_consumer(Arc::new(JsonlLogger::from_roko_dir(state.layout.root())));
    let report = engine
        .run(config)
        .await
        .map_err(|error| format!("workflow engine failed: {error}"))?;

    Ok(transcript_from_report(token.to_string(), &report))
}

fn scrub_run_transcript(
    transcript: RunTranscript,
    scrubber: &LogScrubber,
) -> Option<RunTranscript> {
    let mut value = serde_json::to_value(transcript).ok()?;
    scrub_json_value(&mut value, scrubber);
    serde_json::from_value(value).ok()
}

fn scrub_json_value(value: &mut Value, scrubber: &LogScrubber) {
    match value {
        Value::String(text) => {
            *text = scrub_share_text(text.as_str(), scrubber);
        }
        Value::Array(items) => {
            for item in items {
                scrub_json_value(item, scrubber);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                scrub_json_value(item, scrubber);
            }
        }
        _ => {}
    }
}

fn share_expired_at(metadata: &ShareMetadata, now: &DateTime<Utc>) -> Option<DateTime<Utc>> {
    metadata.expires_at.as_ref().and_then(|expires_at| {
        if now >= expires_at {
            Some(*expires_at)
        } else {
            None
        }
    })
}

fn default_share_expires_at(ttl_days: u64) -> DateTime<Utc> {
    let ttl_days = ttl_days.min(i64::MAX as u64) as i64;
    Utc::now() + Duration::days(ttl_days)
}

fn expired_share_response(expires_at: DateTime<Utc>) -> Response {
    (
        StatusCode::GONE,
        Json(json!({
            "error": "share expired",
            "expires_at": expires_at,
        })),
    )
        .into_response()
}

fn scrub_share_text(text: &str, scrubber: &LogScrubber) -> String {
    let redacted = scrubber.scrub(text);
    scrub_long_secret_like_strings(&redacted)
}

fn scrub_long_secret_like_strings(text: &str) -> String {
    let redacted = long_hex_secret_regex().replace_all(text, "$1[REDACTED]$3");
    long_base64_secret_regex()
        .replace_all(redacted.as_ref(), "$1[REDACTED]$3")
        .into_owned()
}

fn long_hex_secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(^|[^0-9A-Fa-f])([0-9A-Fa-f]{32,})([^0-9A-Fa-f]|$)")
            .expect("valid hex secret regex")
    })
}

fn long_base64_secret_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(^|[^A-Za-z0-9+/=])([A-Za-z0-9+/=]{32,})([^A-Za-z0-9+/=]|$)")
            .expect("valid base64 secret regex")
    })
}

fn share_url_for(
    public_base_url: Option<&str>,
    token: &str,
    public: bool,
) -> Result<String, String> {
    if !public {
        return Ok(format!("/runs/{token}"));
    }

    let base_url = public_base_url
        .and_then(non_empty)
        .ok_or_else(|| "public sharing requires [relay].public_url in roko.toml".to_string())?;
    Ok(format!("{}/runs/{token}", base_url.trim_end_matches('/')))
}

fn workflow_config_for_name(name: &str) -> WorkflowConfig {
    match name {
        "standard" => WorkflowConfig::standard(),
        "full" => WorkflowConfig::full(),
        _ => WorkflowConfig::express(),
    }
}

fn transcript_from_report(token: String, report: &WorkflowRunReport) -> RunTranscript {
    let (agent, role) = report_agent_role(report);

    // Aggregate tokens_used from all AgentCompleted events. Fall back to the
    // report-level token_usage total if no per-agent events are present.
    // TODO: RuntimeEvent::AgentCompleted only carries `tokens_used` (total);
    // split into input/output tokens when the event gains that breakdown.
    let aggregated_tokens: u64 = report
        .events
        .iter()
        .filter_map(|envelope| {
            if let RuntimeEvent::AgentCompleted { tokens_used, .. } = &envelope.payload {
                Some(*tokens_used)
            } else {
                None
            }
        })
        .sum();
    let input_tokens = if aggregated_tokens > 0 {
        Some(aggregated_tokens)
    } else if report.token_usage > 0 {
        Some(report.token_usage)
    } else {
        None
    };

    RunTranscript {
        id: token,
        agent,
        role,
        prompt: report.prompt_summary.clone(),
        success: report.success,
        gates: report
            .gates
            .iter()
            .map(|gate| (gate.name.clone(), gate.passed))
            .collect(),
        output: non_empty(&report.output).map(ToOwned::to_owned),
        cost_usd: report.cost,
        input_tokens,
        output_tokens: None,
        model: Some(report.model.clone()),
        duration_s: Some(report.duration_secs),
        episode_id: Some(report.run_id.clone()),
        transcript: report.events.clone(),
        timestamp: report
            .events
            .first()
            .map(|event| event.ts.to_rfc3339())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
    }
}

fn report_agent_role(report: &WorkflowRunReport) -> (String, String) {
    let mut first = None;
    for envelope in &report.events {
        if let RuntimeEvent::AgentSpawned { agent_id, role, .. } = &envelope.payload {
            if role == "implementer" {
                return (agent_id.clone(), role.clone());
            }
            first.get_or_insert_with(|| (agent_id.clone(), role.clone()));
        }
    }
    first.unwrap_or_else(|| ("workflow".to_string(), "workflow".to_string()))
}

fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn render_html(t: &RunTranscript) -> String {
    use std::fmt::Write as _;
    let gates_html: String = t
        .gates
        .iter()
        .fold(String::new(), |mut acc, (name, passed)| {
            let icon = if *passed { "✔" } else { "✖" };
            let color = if *passed { "#7d9e8c" } else { "#c36e55" };
            let _ = write!(acc, "<span style=\"color:{color}\">{icon} {name}</span>  ");
            acc
        });

    let output_html = t
        .output
        .as_deref()
        .unwrap_or("")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    let cost_str = t
        .cost_usd
        .map(|c| format!("${c:.4}"))
        .unwrap_or_else(|| "—".into());
    let model_str = t.model.as_deref().unwrap_or("—");
    let duration_str = t
        .duration_s
        .map(|d| format!("{d:.1}s"))
        .unwrap_or_else(|| "—".into());
    let result_icon = if t.success { "✔" } else { "✖" };
    let result_color = if t.success { "#7d9e8c" } else { "#c36e55" };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>roko run · {id}</title>
<style>
  :root {{
    --bg: #16121a;
    --fg: #a58e9e;
    --dim: #916e8a;
    --rose: #b97894;
    --sage: #7d9e8c;
    --ember: #c36e55;
    --bone: #d7c69e;
    --dream: #7873a5;
    --ghost: #372a37;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{
    background: var(--bg);
    color: var(--fg);
    font-family: 'Geist Mono', 'SF Mono', 'Menlo', monospace;
    font-size: 14px;
    line-height: 1.6;
    padding: 2rem;
    max-width: 800px;
    margin: 0 auto;
  }}
  .header {{
    border-bottom: 1px solid var(--ghost);
    padding-bottom: 1rem;
    margin-bottom: 1.5rem;
  }}
  .header h1 {{
    color: var(--rose);
    font-size: 1.1rem;
    font-weight: 600;
  }}
  .header .meta {{
    color: var(--dim);
    font-size: 0.85rem;
    margin-top: 0.3rem;
  }}
  .section {{
    margin-bottom: 1.5rem;
  }}
  .section-title {{
    color: var(--dim);
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 0.5rem;
  }}
  .row {{
    display: flex;
    gap: 2rem;
    margin-bottom: 0.3rem;
  }}
  .label {{
    color: var(--dim);
    min-width: 80px;
  }}
  .value {{
    color: var(--fg);
  }}
  .gates {{
    font-size: 0.9rem;
  }}
  .output {{
    background: #0e0c10;
    border: 1px solid var(--ghost);
    border-radius: 6px;
    padding: 1rem;
    overflow-x: auto;
    white-space: pre-wrap;
    font-size: 0.85rem;
    color: var(--bone);
    max-height: 500px;
    overflow-y: auto;
  }}
  .result {{
    font-size: 1rem;
    font-weight: 600;
    padding: 0.5rem 0;
  }}
  .footer {{
    border-top: 1px solid var(--ghost);
    padding-top: 1rem;
    margin-top: 2rem;
    color: var(--ghost);
    font-size: 0.75rem;
  }}
</style>
</head>
<body>
  <div class="header">
    <h1>◆ roko run</h1>
    <div class="meta">{agent} · {role} · {timestamp}</div>
  </div>

  <div class="section">
    <div class="section-title">prompt</div>
    <div style="color:var(--bone)">{prompt}</div>
  </div>

  <div class="section">
    <div class="row"><span class="label">cost</span><span class="value" style="color:var(--sage)">{cost}</span></div>
    <div class="row"><span class="label">model</span><span class="value" style="color:var(--dream)">{model}</span></div>
    <div class="row"><span class="label">duration</span><span class="value">{duration}</span></div>
  </div>

  <div class="section">
    <div class="section-title">gates</div>
    <div class="gates">{gates}</div>
  </div>

  <div class="section">
    <div class="section-title">output</div>
    <div class="output">{output}</div>
  </div>

  <div class="result" style="color:{result_color}">{result_icon} {result_text}</div>

  <div class="footer">
    roko · episode {episode} · generated by the roko agent runtime
  </div>
</body>
</html>"#,
        id = t.id,
        agent = t.agent,
        role = t.role,
        timestamp = t.timestamp,
        prompt = t.prompt.replace('<', "&lt;").replace('>', "&gt;"),
        cost = cost_str,
        model = model_str,
        duration = duration_str,
        gates = gates_html,
        output = output_html,
        result_color = result_color,
        result_icon = result_icon,
        result_text = if t.success { "completed" } else { "failed" },
        episode = t.episode_id.as_deref().unwrap_or("—"),
    )
}

/// Register the shared-run routes on a router.
pub fn routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/api/runs/{id}", axum::routing::get(get_run_json))
        .route("/api/runs/{id}/share", axum::routing::post(create_share))
        .route("/api/shared/{token}", axum::routing::get(get_shared_run))
        .route("/runs/{id}", axum::routing::get(get_run_html))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_retrieve_share() {
        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().join(".roko").join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();

        let transcript = RunTranscript {
            id: "test_run_abc123".to_string(),
            agent: "implementer".to_string(),
            role: "implementer".to_string(),
            prompt: "fix the bug".to_string(),
            success: true,
            gates: vec![("compile".to_string(), true), ("test".to_string(), true)],
            output: Some("Fixed the null pointer dereference.".to_string()),
            cost_usd: Some(0.0042),
            input_tokens: Some(1500),
            output_tokens: Some(350),
            model: Some("claude-sonnet-4-20250514".to_string()),
            duration_s: Some(3.2),
            episode_id: Some("ep_001".to_string()),
            transcript: Vec::new(),
            timestamp: "2026-04-28T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&transcript).unwrap();
        let path = shared_dir.join("test_run_abc123.json");
        std::fs::write(&path, &json).unwrap();

        let loaded: RunTranscript =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(loaded.id, "test_run_abc123");
        assert_eq!(loaded.agent, "implementer");
        assert!(loaded.success);
        assert_eq!(loaded.gates.len(), 2);
        assert_eq!(
            loaded.output.as_deref(),
            Some("Fixed the null pointer dereference.")
        );
        assert!((loaded.cost_usd.unwrap() - 0.0042).abs() < f64::EPSILON);
    }

    #[test]
    fn retrieve_share_renders_html() {
        let transcript = RunTranscript {
            id: "run_html_test".to_string(),
            agent: "reviewer".to_string(),
            role: "reviewer".to_string(),
            prompt: "review the changes".to_string(),
            success: false,
            gates: vec![("clippy".to_string(), false)],
            output: Some("Found 3 lint warnings.".to_string()),
            cost_usd: Some(0.0018),
            input_tokens: Some(800),
            output_tokens: Some(200),
            model: Some("claude-sonnet-4-20250514".to_string()),
            duration_s: Some(1.5),
            episode_id: Some("ep_002".to_string()),
            transcript: Vec::new(),
            timestamp: "2026-04-28T13:00:00Z".to_string(),
        };

        let html = render_html(&transcript);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("run_html_test"));
        assert!(html.contains("reviewer"));
        assert!(html.contains("review the changes"));
        assert!(html.contains("clippy"));
        assert!(html.contains("Found 3 lint warnings."));
        assert!(html.contains("failed"));
    }

    fn sample_share_transcript() -> RunTranscript {
        RunTranscript {
            id: "shared-1".into(),
            agent: "researcher".into(),
            role: "analyst".into(),
            prompt: "ANTHROPIC_API_KEY=sk-ant-abc123".into(),
            success: true,
            gates: vec![("compile".into(), true)],
            output: Some(
                "Bearer abcdefghijklmnopqrstuvwxyz1234 0123456789abcdef0123456789ABCDEF QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo="
                    .into(),
            ),
            cost_usd: Some(0.0),
            input_tokens: Some(1),
            output_tokens: None,
            model: Some("claude-sonnet-4-20250514".into()),
            duration_s: Some(1.0),
            episode_id: Some("ep".into()),
            transcript: vec![RuntimeEventEnvelope::new(
                "ep",
                1,
                "workflow",
                RuntimeEvent::WorkflowStarted {
                    run_id: "ep".into(),
                    template: "express".into(),
                    prompt: "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz".into(),
                },
            )],
            timestamp: "2026-04-28T12:00:00Z".into(),
        }
    }

    #[test]
    fn scrub_run_transcript_redacts_secret_patterns() {
        let transcript = sample_share_transcript();
        let scrubbed = scrub_run_transcript(transcript, &LogScrubber::new())
            .expect("transcript scrubs successfully");
        let json = serde_json::to_string(&scrubbed).unwrap();
        assert!(!json.contains("sk-ant-abc123"));
        assert!(!json.contains("sk-proj-abcdefghijklmnopqrstuvwxyz"));
        assert!(!json.contains("abcdefghijklmnopqrstuvwxyz1234"));
        assert!(!json.contains("0123456789abcdef0123456789ABCDEF"));
        assert!(!json.contains("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo="));
        assert!(json.contains("[REDACTED]"));
    }

    #[test]
    fn stored_transcript_supports_wrapped_and_bare_formats() {
        let transcript = sample_share_transcript();
        let expires_at = chrono::DateTime::parse_from_rfc3339("2026-05-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let wrapped_json = serde_json::to_string(&SharedTranscriptRecord {
            metadata: ShareMetadata {
                scrubbed: true,
                public: true,
                expires_at: Some(expires_at.clone()),
            },
            transcript: transcript.clone(),
        })
        .unwrap();
        match serde_json::from_str::<StoredTranscript>(&wrapped_json).unwrap() {
            StoredTranscript::Wrapped(record) => {
                assert!(record.metadata.scrubbed);
                assert!(record.metadata.public);
                assert_eq!(record.metadata.expires_at, Some(expires_at.clone()));
                assert_eq!(record.transcript.id, transcript.id);
            }
            StoredTranscript::Bare(_) => panic!("wrapped share should deserialize as wrapped"),
        }

        let bare_json = serde_json::to_string(&transcript).unwrap();
        assert!(matches!(
            serde_json::from_str::<StoredTranscript>(&bare_json).unwrap(),
            StoredTranscript::Bare(_)
        ));
    }

    #[test]
    fn share_expired_at_distinguishes_permanent_and_expired_shares() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-06T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let expired = ShareMetadata {
            scrubbed: true,
            public: false,
            expires_at: Some(
                chrono::DateTime::parse_from_rfc3339("2026-05-05T12:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        };
        let permanent = ShareMetadata {
            scrubbed: true,
            public: false,
            expires_at: None,
        };

        assert!(share_expired_at(&expired, &now).is_some());
        assert!(share_expired_at(&permanent, &now).is_none());
    }

    #[test]
    fn expired_share_response_uses_gone_status() {
        let expires_at = chrono::DateTime::parse_from_rfc3339("2026-05-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let response = expired_share_response(expires_at);
        assert_eq!(response.status(), StatusCode::GONE);
    }

    #[test]
    fn share_url_supports_local_and_public_scopes() {
        assert_eq!(
            share_url_for(None, "abc123", false).expect("local url"),
            "/runs/abc123"
        );
        assert_eq!(
            share_url_for(Some("https://share.example.com/"), "abc123", true).expect("public url"),
            "https://share.example.com/runs/abc123"
        );
        assert!(share_url_for(None, "abc123", true).is_err());
    }

    #[test]
    fn unknown_share_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let shared_dir = dir.path().join(".roko").join("shared");
        std::fs::create_dir_all(&shared_dir).unwrap();

        let path = shared_dir.join("nonexistent_run.json");
        let data = std::fs::read_to_string(path);
        assert!(data.is_err());

        let bad_path = shared_dir.join("bad_run.json");
        std::fs::write(&bad_path, "not valid json").unwrap();
        let result: Result<RunTranscript, _> =
            serde_json::from_str(&std::fs::read_to_string(&bad_path).unwrap());
        assert!(result.is_err());
    }
}
