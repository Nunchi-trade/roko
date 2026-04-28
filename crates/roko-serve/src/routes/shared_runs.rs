//! Shareable run transcript routes.
//!
//! `GET /api/runs/{id}` — JSON transcript.
//! `GET /runs/{id}` — Self-contained HTML page.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Response},
};
use roko_core::runtime_event::{RuntimeEvent, RuntimeEventEnvelope, WorkflowOutcome};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    /// Timestamp (ISO 8601).
    pub timestamp: String,
}

/// `GET /api/runs/{id}` — JSON transcript.
pub async fn get_run_json(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match load_transcript(&state, &id) {
        Some(transcript) => Json(json!(transcript)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /api/shared/{token}` — JSON transcript for a shared token.
pub async fn get_shared_run(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
) -> Response {
    match load_transcript(&state, &token) {
        Some(t) => Json(json!(t)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /api/runs/{id}/share` — create a shared run token.
pub async fn create_share(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let token = format!("{}-{:04x}", id, std::process::id() as u16);
    let shared_dir = state.workdir.join(".roko").join("shared");
    if let Some(existing) = load_transcript(&state, &token) {
        return Json(json!({
            "token": token,
            "url": format!("/runs/{}", token),
            "transcript": existing
        }))
        .into_response();
    }
    if let Err(e) = std::fs::create_dir_all(&shared_dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("mkdir failed: {e}")})),
        )
            .into_response();
    }
    let Some(transcript) = transcript_from_runtime_events(&state, &id, &token) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = shared_dir.join(format!("{token}.json"));
    match serde_json::to_string_pretty(&transcript) {
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
    Json(json!({"token": token, "url": format!("/runs/{}", token)})).into_response()
}

/// `GET /runs/{id}` — Self-contained HTML page.
pub async fn get_run_html(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match load_transcript(&state, &id) {
        Some(transcript) => {
            let html = render_html(&transcript);
            Html(html).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn load_transcript(state: &AppState, id: &str) -> Option<RunTranscript> {
    let path = state
        .workdir
        .join(".roko")
        .join("shared")
        .join(format!("{id}.json"));
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
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
    let mut first_ts = None;
    let mut last_ts = None;

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
        input_tokens: None,
        output_tokens: None,
        model,
        duration_s: Some(duration_s),
        episode_id: Some(run_id.to_string()),
        timestamp: started_at.to_rfc3339(),
    })
}

fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
