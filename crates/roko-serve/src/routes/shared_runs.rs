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

fn render_html(t: &RunTranscript) -> String {
    let gates_html: String = t
        .gates
        .iter()
        .map(|(name, passed)| {
            let icon = if *passed { "✔" } else { "✖" };
            let color = if *passed { "#7d9e8c" } else { "#c36e55" };
            format!("<span style=\"color:{color}\">{icon} {name}</span>  ")
        })
        .collect();

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
        r##"<!DOCTYPE html>
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
</html>"##,
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
        .route("/runs/{id}", axum::routing::get(get_run_html))
}
