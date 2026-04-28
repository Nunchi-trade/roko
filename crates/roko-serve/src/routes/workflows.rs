//! Workflow artifact and execution projections.
//!
//! These routes expose the end-to-end PRD -> plan -> tasks -> execution
//! workflow as a first-class read model. The read model is built from real
//! `.roko` artifacts and merged with the live StateHub snapshot, so web
//! clients do not need to scrape terminals or understand on-disk details.

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, Query, State};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures::stream::{self, Stream, StreamExt};
use futures::{SinkExt, StreamExt as FuturesStreamExt};
use roko_core::dashboard_snapshot::{DashboardEvent, DashboardSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tokio::time::{self, MissedTickBehavior};
use toml::Value as TomlValue;
use tracing::{debug, warn};

use crate::error::ApiError;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/workflows", get(list_workflows))
        .route("/workflows/latest", get(get_latest_workflow))
        .route("/workflows/latest/stream", get(stream_latest_workflow))
        .route("/workflows/{id}", get(get_workflow))
        .route("/workflows/{id}/tasks", get(get_workflow_tasks))
        .route("/workflows/{id}/stream", get(stream_workflow))
        .route("/workflow/ws", get(workflow_ws_upgrade))
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WorkflowQuery {
    /// Workspace root. Defaults to the server's configured workdir.
    root: Option<String>,
    /// Last StateHub sequence observed by the client.
    cursor: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowListResponse {
    channel: &'static str,
    workdir: String,
    cursor: u64,
    latest: Option<String>,
    workflows: Vec<WorkflowSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowSummary {
    id: String,
    title: String,
    phase: String,
    updated_at_millis: u64,
    prd_status: Option<String>,
    plan_count: usize,
    task_count: usize,
    active_tasks: usize,
    done_tasks: usize,
    failed_tasks: usize,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowSnapshot {
    id: String,
    title: String,
    phase: String,
    workdir: String,
    updated_at_millis: u64,
    summary: WorkflowSummary,
    prd: Option<WorkflowPrd>,
    plans: Vec<WorkflowPlan>,
    live: WorkflowLive,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowPrd {
    slug: String,
    title: String,
    path: String,
    status: String,
    excerpt: String,
    requirements: Vec<String>,
    acceptance: Vec<String>,
    body_markdown: String,
    updated_at_millis: u64,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowPlan {
    id: String,
    title: String,
    path: String,
    status: String,
    excerpt: String,
    estimated_minutes: Option<u64>,
    plan_markdown: String,
    updated_at_millis: u64,
    tasks: Vec<WorkflowTask>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowTask {
    id: String,
    title: String,
    description: Option<String>,
    status: String,
    raw_status: Option<String>,
    route_tier: Option<String>,
    tier: Option<String>,
    role: Option<String>,
    model_hint: Option<String>,
    selected_model: Option<String>,
    max_loc: Option<u64>,
    files: Vec<String>,
    depends_on: Vec<String>,
    depends_on_plan: Vec<String>,
    verify: Vec<WorkflowVerifyStep>,
    acceptance: Vec<String>,
    domain: Option<String>,
    phase: Option<String>,
    agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkflowVerifyStep {
    phase: String,
    command: String,
    fail_msg: Option<String>,
    timeout_ms: Option<u64>,
    status: String,
}

#[derive(Debug, Clone, Default, Serialize)]
struct WorkflowLive {
    plans: Vec<LivePlan>,
    tasks: Vec<LiveTask>,
    gates: Vec<LiveGate>,
    agents: Vec<LiveAgent>,
    events: Vec<LiveEvent>,
    stats: WorkflowLiveStats,
}

#[derive(Debug, Clone, Default, Serialize)]
struct WorkflowLiveStats {
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
    gates_passed: usize,
    gates_failed: usize,
}

#[derive(Debug, Clone, Serialize)]
struct LivePlan {
    plan_id: String,
    phase: String,
    active: bool,
    tasks_total: usize,
    tasks_done: usize,
    tasks_failed: usize,
}

#[derive(Debug, Clone, Serialize)]
struct LiveTask {
    plan_id: String,
    task_id: String,
    title: String,
    phase: String,
    outcome: Option<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct LiveGate {
    plan_id: String,
    task_id: String,
    gate: String,
    passed: bool,
    ts_millis: u64,
}

#[derive(Debug, Clone, Serialize)]
struct LiveAgent {
    agent_id: String,
    role: String,
    model: String,
    active: bool,
    current_plan: String,
    current_task: String,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
    output_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
struct LiveEvent {
    timestamp_ms: u64,
    event_type: String,
    plan_id: String,
    task_id: String,
    message: String,
}

async fn list_workflows(
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<WorkflowListResponse>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    let snapshots = read_workflow_snapshots(&root, &state);
    Ok(Json(list_response(&root, &state, &snapshots)))
}

async fn get_latest_workflow(
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    latest_snapshot(&root, &state)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no workflow artifacts found"))
}

async fn get_workflow(
    AxumPath(id): AxumPath<String>,
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<WorkflowSnapshot>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    snapshot_by_id(&root, &state, &id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("workflow '{id}' not found")))
}

async fn get_workflow_tasks(
    AxumPath(id): AxumPath<String>,
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Value>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    let snapshot = snapshot_by_id(&root, &state, &id)
        .ok_or_else(|| ApiError::not_found(format!("workflow '{id}' not found")))?;
    let tasks: Vec<&WorkflowTask> = snapshot.plans.iter().flat_map(|plan| &plan.tasks).collect();
    Ok(Json(json!({
        "channel": "workflow.tasks",
        "workdir": root.display().to_string(),
        "workflow_id": snapshot.id,
        "cursor": state.state_hub.total_published(),
        "tasks": tasks,
    })))
}

async fn stream_latest_workflow(
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    Ok(workflow_sse(state, root, None, query.cursor.unwrap_or(0)))
}

async fn stream_workflow(
    AxumPath(id): AxumPath<String>,
    Query(query): Query<WorkflowQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let root = resolve_workdir(&state, &query)?;
    Ok(workflow_sse(
        state,
        root,
        Some(id),
        query.cursor.unwrap_or(0),
    ))
}

async fn workflow_ws_upgrade(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_workflow_ws(state, socket))
}

fn workflow_sse(
    state: Arc<AppState>,
    root: PathBuf,
    workflow_id: Option<String>,
    cursor: u64,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let initial_frame = workflow_state_frame(&root, &state, workflow_id.as_deref(), cursor);
    let initial_payload = initial_frame.to_string();
    let initial_event = Event::default()
        .event("state")
        .id(initial_frame["cursor"]
            .as_u64()
            .unwrap_or_default()
            .to_string())
        .data(initial_payload.clone());

    let mut interval = time::interval(Duration::from_millis(1250));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let stream_state = WorkflowSseState {
        state,
        root,
        workflow_id,
        rx: None,
        interval,
        last_payload: initial_payload,
    };

    let live_stream = stream::unfold(stream_state, |mut st| async move {
        if st.rx.is_none() {
            st.rx = Some(st.state.state_hub.subscribe_events());
        }

        loop {
            tokio::select! {
                _ = st.interval.tick() => {
                    let frame = workflow_state_frame(
                        &st.root,
                        &st.state,
                        st.workflow_id.as_deref(),
                        st.state.state_hub.total_published(),
                    );
                    let payload = frame.to_string();
                    if payload == st.last_payload {
                        continue;
                    }
                    st.last_payload = payload.clone();
                    let event = Event::default()
                        .event("state")
                        .id(frame["cursor"].as_u64().unwrap_or_default().to_string())
                        .data(payload);
                    return Some((Ok(event), st));
                }
                received = async {
                    let rx = st.rx.as_mut().expect("workflow sse receiver initialized");
                    rx.recv().await
                } => {
                    match received {
                        Ok(envelope) => {
                            if !dashboard_event_matches_workflow(&envelope.payload, st.workflow_id.as_deref()) {
                                continue;
                            }
                            let frame = workflow_delta_frame(&st.root, &st.state, st.workflow_id.as_deref(), envelope.seq, &envelope.payload);
                            let payload = frame.to_string();
                            st.last_payload = workflow_state_frame(
                                &st.root,
                                &st.state,
                                st.workflow_id.as_deref(),
                                envelope.seq,
                            ).to_string();
                            let event = Event::default()
                                .event("delta")
                                .id(envelope.seq.to_string())
                                .data(payload);
                            return Some((Ok(event), st));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            warn!(skipped, "workflow sse client lagged");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        }
    });

    Sse::new(stream::once(async move { Ok(initial_event) }).chain(live_stream))
        .keep_alive(KeepAlive::default())
}

struct WorkflowSseState {
    state: Arc<AppState>,
    root: PathBuf,
    workflow_id: Option<String>,
    rx: Option<broadcast::Receiver<roko_runtime::event_bus::Envelope<DashboardEvent>>>,
    interval: time::Interval,
    last_payload: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WorkflowWsClientMessage {
    Subscribe {
        request_id: Option<String>,
        workflow_id: Option<String>,
        root: Option<String>,
        cursor: Option<u64>,
        projections: Option<Vec<String>>,
    },
    Ping {
        request_id: Option<String>,
    },
}

#[derive(Debug)]
struct WorkflowWsSubscription {
    root: PathBuf,
    workflow_id: Option<String>,
    last_payload: String,
}

async fn handle_workflow_ws(state: Arc<AppState>, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = state.state_hub.subscribe_events();
    let mut interval = time::interval(Duration::from_millis(1250));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut subscription: Option<WorkflowWsSubscription> = None;

    loop {
        tokio::select! {
            msg = FuturesStreamExt::next(&mut stream) => {
                let Some(msg) = msg else {
                    debug!("workflow ws client disconnected");
                    break;
                };
                match msg {
                    Ok(Message::Text(text)) => {
                        let Ok(command) = serde_json::from_str::<WorkflowWsClientMessage>(&text) else {
                            if send_ws_json(&mut sink, json!({
                                "type": "error",
                                "code": "invalid_json",
                                "message": "workflow websocket messages must be JSON"
                            })).await.is_err() {
                                break;
                            }
                            continue;
                        };

                        match command {
                            WorkflowWsClientMessage::Subscribe {
                                request_id,
                                workflow_id,
                                root,
                                cursor,
                                projections,
                            } => {
                                let query = WorkflowQuery { root, cursor };
                                let resolved = match resolve_workdir(&state, &query) {
                                    Ok(root) => root,
                                    Err(error) => {
                                        if send_ws_json(&mut sink, json!({
                                            "type": "error",
                                            "request_id": request_id,
                                            "code": error.code,
                                            "message": error.message,
                                        })).await.is_err() {
                                            break;
                                        }
                                        continue;
                                    }
                                };

                                let frame = workflow_state_frame(
                                    &resolved,
                                    &state,
                                    workflow_id.as_deref(),
                                    query.cursor.unwrap_or_else(|| state.state_hub.total_published()),
                                );
                                let payload = frame.to_string();
                                subscription = Some(WorkflowWsSubscription {
                                    root: resolved,
                                    workflow_id,
                                    last_payload: payload,
                                });

                                if send_ws_json(&mut sink, json!({
                                    "type": "ack",
                                    "request_id": request_id,
                                    "channel": "workflow",
                                    "projections": projections.unwrap_or_else(|| vec![
                                        "workflow.artifacts".to_string(),
                                        "workflow.execution".to_string(),
                                        "workflow.gates".to_string(),
                                        "workflow.agents".to_string(),
                                    ]),
                                    "cursor": frame["cursor"],
                                })).await.is_err() {
                                    break;
                                }
                                if send_ws_json(&mut sink, frame).await.is_err() {
                                    break;
                                }
                            }
                            WorkflowWsClientMessage::Ping { request_id } => {
                                if send_ws_json(&mut sink, json!({
                                    "type": "pong",
                                    "request_id": request_id,
                                    "cursor": state.state_hub.total_published(),
                                })).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(payload)) => {
                        if sink.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!("workflow ws receive error: {err}");
                        break;
                    }
                }
            }
            _ = interval.tick() => {
                let Some(sub) = subscription.as_mut() else {
                    continue;
                };
                let frame = workflow_state_frame(
                    &sub.root,
                    &state,
                    sub.workflow_id.as_deref(),
                    state.state_hub.total_published(),
                );
                let payload = frame.to_string();
                if payload == sub.last_payload {
                    continue;
                }
                sub.last_payload = payload;
                if send_ws_json(&mut sink, frame).await.is_err() {
                    break;
                }
            }
            received = rx.recv() => {
                let Some(sub) = subscription.as_mut() else {
                    continue;
                };
                match received {
                    Ok(envelope) => {
                        if !dashboard_event_matches_workflow(&envelope.payload, sub.workflow_id.as_deref()) {
                            continue;
                        }
                        let frame = workflow_delta_frame(
                            &sub.root,
                            &state,
                            sub.workflow_id.as_deref(),
                            envelope.seq,
                            &envelope.payload,
                        );
                        sub.last_payload = workflow_state_frame(
                            &sub.root,
                            &state,
                            sub.workflow_id.as_deref(),
                            envelope.seq,
                        ).to_string();
                        if send_ws_json(&mut sink, frame).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "workflow ws client lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    let _ = sink.close().await;
}

async fn send_ws_json(
    sink: &mut futures::stream::SplitSink<WebSocket, Message>,
    value: Value,
) -> Result<(), axum::Error> {
    sink.send(Message::Text(value.to_string().into())).await
}

fn resolve_workdir(state: &AppState, query: &WorkflowQuery) -> Result<PathBuf, ApiError> {
    let Some(root) = query
        .root
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return Ok(state.workdir.clone());
    };
    let path = PathBuf::from(root);
    if !path.is_dir() {
        return Err(ApiError::bad_request(format!(
            "workflow root does not exist or is not a directory: {root}"
        )));
    }
    path.canonicalize()
        .map_err(|err| ApiError::bad_request(format!("invalid workflow root '{root}': {err}")))
}

fn list_response(
    root: &Path,
    state: &AppState,
    snapshots: &[WorkflowSnapshot],
) -> WorkflowListResponse {
    let mut summaries: Vec<WorkflowSummary> = snapshots.iter().map(summarize_workflow).collect();
    summaries.sort_by(|a, b| {
        b.updated_at_millis
            .cmp(&a.updated_at_millis)
            .then_with(|| a.id.cmp(&b.id))
    });
    let latest = summaries.first().map(|workflow| workflow.id.clone());
    WorkflowListResponse {
        channel: "workflow.list",
        workdir: root.display().to_string(),
        cursor: state.state_hub.total_published(),
        latest,
        workflows: summaries,
    }
}

fn latest_snapshot(root: &Path, state: &AppState) -> Option<WorkflowSnapshot> {
    read_workflow_snapshots(root, state)
        .into_iter()
        .max_by(|a, b| {
            a.updated_at_millis
                .cmp(&b.updated_at_millis)
                .then_with(|| b.id.cmp(&a.id))
        })
}

fn snapshot_by_id(root: &Path, state: &AppState, id: &str) -> Option<WorkflowSnapshot> {
    read_workflow_snapshots(root, state)
        .into_iter()
        .find(|workflow| workflow.id == id)
}

fn workflow_state_frame(
    root: &Path,
    state: &AppState,
    workflow_id: Option<&str>,
    cursor: u64,
) -> Value {
    let snapshots = read_workflow_snapshots(root, state);
    let list = list_response(root, state, &snapshots);
    let workflow = match workflow_id {
        Some(id) => snapshots.into_iter().find(|snapshot| snapshot.id == id),
        None => snapshots.into_iter().max_by(|a, b| {
            a.updated_at_millis
                .cmp(&b.updated_at_millis)
                .then_with(|| b.id.cmp(&a.id))
        }),
    };
    let workflow_id = workflow.as_ref().map(|workflow| workflow.id.clone());
    json!({
        "type": "state",
        "channel": "workflow",
        "cursor": cursor.max(state.state_hub.total_published()),
        "workdir": root.display().to_string(),
        "workflow_id": workflow_id,
        "workflows": list.workflows,
        "data": workflow,
    })
}

fn workflow_delta_frame(
    root: &Path,
    state: &AppState,
    workflow_id: Option<&str>,
    cursor: u64,
    event: &DashboardEvent,
) -> Value {
    let state_frame = workflow_state_frame(root, state, workflow_id, cursor);
    json!({
        "type": "delta",
        "channel": "workflow",
        "cursor": cursor,
        "workdir": root.display().to_string(),
        "workflow_id": state_frame["workflow_id"].clone(),
        "event": event,
        "data": state_frame["data"].clone(),
        "workflows": state_frame["workflows"].clone(),
    })
}

fn read_workflow_snapshots(root: &Path, state: &AppState) -> Vec<WorkflowSnapshot> {
    let prds = discover_prds(root);
    let plans = discover_plans(root);
    let dashboard = dashboard_snapshot_for_workdir(root, state);
    let mut workflows: BTreeMap<String, WorkflowSnapshot> = BTreeMap::new();

    for plan in plans {
        let id = plan.id.clone();
        let prd = find_prd_for_plan(&prds, &id);
        let title = if !plan.title.trim().is_empty() {
            plan.title.clone()
        } else {
            prd.as_ref()
                .map(|prd| prd.title.clone())
                .unwrap_or_else(|| title_from_slug(&id))
        };
        let updated_at_millis = prd
            .as_ref()
            .map(|prd| prd.updated_at_millis)
            .unwrap_or_default()
            .max(plan.updated_at_millis);
        let mut workflow = WorkflowSnapshot {
            id: id.clone(),
            title,
            phase: "tasks".to_string(),
            workdir: root.display().to_string(),
            updated_at_millis,
            summary: empty_summary(&id),
            prd,
            plans: vec![plan],
            live: WorkflowLive::default(),
        };
        merge_live_state(&mut workflow, &dashboard);
        workflow.phase = workflow_phase(&workflow);
        workflow.summary = summarize_workflow(&workflow);
        workflows.insert(id, workflow);
    }

    for prd in prds {
        if workflows.contains_key(&prd.slug) {
            continue;
        }
        let id = prd.slug.clone();
        let mut workflow = WorkflowSnapshot {
            id: id.clone(),
            title: prd.title.clone(),
            phase: prd.status.clone(),
            workdir: root.display().to_string(),
            updated_at_millis: prd.updated_at_millis,
            summary: empty_summary(&id),
            prd: Some(prd),
            plans: Vec::new(),
            live: WorkflowLive::default(),
        };
        merge_live_state(&mut workflow, &dashboard);
        workflow.phase = workflow_phase(&workflow);
        workflow.summary = summarize_workflow(&workflow);
        workflows.insert(id, workflow);
    }

    let mut out: Vec<WorkflowSnapshot> = workflows.into_values().collect();
    out.sort_by(|a, b| {
        b.updated_at_millis
            .cmp(&a.updated_at_millis)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

fn discover_prds(root: &Path) -> Vec<WorkflowPrd> {
    let prd_root = root.join(".roko").join("prd");
    let mut prds = Vec::new();

    for (status, subdir) in [
        ("idea", "ideas"),
        ("draft", "drafts"),
        ("published", "published"),
    ] {
        let dir = prd_root.join(subdir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            if let Some(prd) = parse_prd_file(&path, status) {
                prds.push(prd);
            }
        }
    }

    if prds.is_empty() {
        let ideas_path = prd_root.join("ideas.md");
        if let Some(prd) = parse_legacy_ideas_file(&ideas_path) {
            prds.push(prd);
        }
    }

    dedupe_prds_by_slug(prds)
}

fn parse_prd_file(path: &Path, status: &str) -> Option<WorkflowPrd> {
    let body = std::fs::read_to_string(path).ok()?;
    let slug = path.file_stem()?.to_string_lossy().to_string();
    Some(WorkflowPrd {
        slug: slug.clone(),
        title: extract_title(&body).unwrap_or_else(|| title_from_slug(&slug)),
        path: path.display().to_string(),
        status: status.to_string(),
        excerpt: markdown_excerpt(&body, 620),
        requirements: section_items(&body, &["requirement", "requirements"]),
        acceptance: section_items(&body, &["acceptance", "criteria"]),
        body_markdown: body,
        updated_at_millis: modified_millis(path),
    })
}

fn parse_legacy_ideas_file(path: &Path) -> Option<WorkflowPrd> {
    let body = std::fs::read_to_string(path).ok()?;
    let latest = body
        .lines()
        .rev()
        .find_map(|line| line.trim().strip_prefix("- ").map(str::trim))
        .unwrap_or("Captured ideas");
    let slug = slugify_title(latest);
    Some(WorkflowPrd {
        slug: slug.clone(),
        title: latest.to_string(),
        path: path.display().to_string(),
        status: "idea".to_string(),
        excerpt: latest.to_string(),
        requirements: Vec::new(),
        acceptance: Vec::new(),
        body_markdown: body,
        updated_at_millis: modified_millis(path),
    })
}

fn dedupe_prds_by_slug(prds: Vec<WorkflowPrd>) -> Vec<WorkflowPrd> {
    let mut by_slug: BTreeMap<String, WorkflowPrd> = BTreeMap::new();
    for prd in prds {
        match by_slug.get(&prd.slug) {
            Some(existing) if prd_rank(&existing.status) >= prd_rank(&prd.status) => {}
            _ => {
                by_slug.insert(prd.slug.clone(), prd);
            }
        }
    }
    by_slug.into_values().collect()
}

fn prd_rank(status: &str) -> u8 {
    match status {
        "published" => 3,
        "draft" => 2,
        "idea" => 1,
        _ => 0,
    }
}

fn find_prd_for_plan(prds: &[WorkflowPrd], plan_id: &str) -> Option<WorkflowPrd> {
    if let Some(prd) = prds.iter().find(|prd| prd.slug == plan_id) {
        return Some(prd.clone());
    }
    if prds.len() == 1 {
        return prds.first().cloned();
    }
    prds.iter()
        .filter(|prd| plan_id.contains(&prd.slug) || prd.slug.contains(plan_id))
        .max_by_key(|prd| prd.updated_at_millis)
        .cloned()
}

fn discover_plans(root: &Path) -> Vec<WorkflowPlan> {
    let mut plans = Vec::new();
    for plans_root in [root.join(".roko").join("plans"), root.join("plans")] {
        let Ok(entries) = std::fs::read_dir(&plans_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let plan_dir = entry.path();
            if !plan_dir.is_dir() || !plan_dir.join("tasks.toml").is_file() {
                continue;
            }
            plans.push(parse_plan_dir(&plan_dir));
        }
    }
    dedupe_plans_by_id(plans)
}

fn dedupe_plans_by_id(plans: Vec<WorkflowPlan>) -> Vec<WorkflowPlan> {
    let mut by_id: BTreeMap<String, WorkflowPlan> = BTreeMap::new();
    for plan in plans {
        match by_id.get(&plan.id) {
            Some(existing) if existing.updated_at_millis >= plan.updated_at_millis => {}
            _ => {
                by_id.insert(plan.id.clone(), plan);
            }
        }
    }
    by_id.into_values().collect()
}

fn parse_plan_dir(plan_dir: &Path) -> WorkflowPlan {
    let id = plan_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "plan".to_string());
    let plan_path = plan_dir.join("plan.md");
    let tasks_path = plan_dir.join("tasks.toml");
    let plan_markdown = std::fs::read_to_string(&plan_path).unwrap_or_default();
    let task_text = std::fs::read_to_string(&tasks_path).unwrap_or_default();
    let (tasks, estimated_minutes, meta_status, meta_title) =
        parse_tasks_toml(&task_text, &tasks_path);
    let status = derive_plan_status(meta_status.as_deref(), &tasks);
    let title = meta_title
        .or_else(|| extract_title(&plan_markdown))
        .unwrap_or_else(|| title_from_slug(&id));

    WorkflowPlan {
        id,
        title,
        path: plan_dir.display().to_string(),
        status,
        excerpt: markdown_excerpt(&plan_markdown, 520),
        estimated_minutes,
        plan_markdown,
        updated_at_millis: modified_millis(&plan_path).max(modified_millis(&tasks_path)),
        tasks,
    }
}

fn parse_tasks_toml(
    content: &str,
    source_path: &Path,
) -> (
    Vec<WorkflowTask>,
    Option<u64>,
    Option<String>,
    Option<String>,
) {
    let parsed: TomlValue = match toml::from_str(content) {
        Ok(value) => value,
        Err(err) => {
            return (
                vec![WorkflowTask {
                    id: "parse".to_string(),
                    title: "tasks.toml parse failed".to_string(),
                    description: Some(err.to_string()),
                    status: "failed".to_string(),
                    raw_status: Some("failed".to_string()),
                    route_tier: None,
                    tier: None,
                    role: None,
                    model_hint: None,
                    selected_model: None,
                    max_loc: None,
                    files: vec![source_path.display().to_string()],
                    depends_on: Vec::new(),
                    depends_on_plan: Vec::new(),
                    verify: Vec::new(),
                    acceptance: Vec::new(),
                    domain: None,
                    phase: None,
                    agent_id: None,
                }],
                None,
                Some("failed".to_string()),
                None,
            );
        }
    };

    let meta = parsed.get("meta");
    let estimated = meta
        .and_then(|value| value.get("estimated_total_minutes"))
        .and_then(toml_u64);
    let meta_status = meta
        .and_then(|value| string_value(value, "status"))
        .or_else(|| meta.and_then(|value| string_value(value, "state")));
    let meta_title = meta
        .and_then(|value| string_value(value, "title"))
        .or_else(|| {
            meta.and_then(|value| string_value(value, "plan"))
                .map(|plan| title_from_slug(&plan))
        });

    let tasks = parsed
        .get("task")
        .and_then(TomlValue::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(idx, task)| parse_task(task, idx))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    (tasks, estimated, meta_status, meta_title)
}

fn parse_task(task: &TomlValue, idx: usize) -> WorkflowTask {
    let id = string_value(task, "id").unwrap_or_else(|| format!("T{}", idx + 1));
    let title = string_value(task, "title").unwrap_or_else(|| title_from_slug(&id));
    let raw_status = string_value(task, "status")
        .or_else(|| string_value(task, "state"))
        .unwrap_or_else(|| "pending".to_string());
    let tier = string_value(task, "tier");
    let route_tier = string_value(task, "route_tier")
        .or_else(|| string_value(task, "routing_tier"))
        .or_else(|| tier.as_deref().map(route_tier_for_task_tier));
    let model_hint = string_value(task, "model_hint").or_else(|| string_value(task, "model"));
    let files = string_array(task, "files").or_else(|| context_read_files(task));
    let verify = verify_steps(task);
    WorkflowTask {
        id,
        title,
        description: string_value(task, "description"),
        status: normalize_status(&raw_status).to_string(),
        raw_status: Some(raw_status),
        route_tier,
        tier,
        role: string_value(task, "role"),
        model_hint,
        selected_model: None,
        max_loc: number_value(task, "max_loc"),
        files: files.unwrap_or_default(),
        depends_on: string_array(task, "depends_on").unwrap_or_default(),
        depends_on_plan: string_array(task, "depends_on_plan").unwrap_or_default(),
        verify,
        acceptance: string_array(task, "acceptance").unwrap_or_default(),
        domain: string_value(task, "domain"),
        phase: None,
        agent_id: None,
    }
}

fn verify_steps(task: &TomlValue) -> Vec<WorkflowVerifyStep> {
    task.get("verify")
        .and_then(TomlValue::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| {
                    let command = string_value(step, "command")?;
                    Some(WorkflowVerifyStep {
                        phase: string_value(step, "phase").unwrap_or_else(|| "verify".to_string()),
                        command,
                        fail_msg: string_value(step, "fail_msg"),
                        timeout_ms: number_value(step, "timeout_ms"),
                        status: "pending".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn context_read_files(task: &TomlValue) -> Option<Vec<String>> {
    let context = task.get("context")?;
    let read_files = context.get("read_files")?.as_array()?;
    let files: Vec<String> = read_files
        .iter()
        .filter_map(|entry| {
            entry
                .as_str()
                .map(str::to_string)
                .or_else(|| string_value(entry, "path"))
        })
        .collect();
    (!files.is_empty()).then_some(files)
}

fn string_value(value: &TomlValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: &TomlValue, key: &str) -> Option<Vec<String>> {
    value.get(key).and_then(|value| match value {
        TomlValue::Array(items) => {
            let out: Vec<String> = items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect();
            (!out.is_empty()).then_some(out)
        }
        TomlValue::String(item) if !item.trim().is_empty() => Some(vec![item.trim().to_string()]),
        _ => None,
    })
}

fn number_value(value: &TomlValue, key: &str) -> Option<u64> {
    value.get(key).and_then(toml_u64)
}

fn toml_u64(value: &TomlValue) -> Option<u64> {
    value
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .or_else(|| value.as_float().map(|n| n.max(0.0).round() as u64))
}

fn route_tier_for_task_tier(tier: &str) -> String {
    let tier = tier.to_ascii_lowercase();
    if tier.contains("architectural") || tier.contains("risk") || tier.contains("security") {
        "T3".to_string()
    } else if tier.contains("integrative") || tier.contains("focused") {
        "T2".to_string()
    } else {
        "T1".to_string()
    }
}

fn derive_plan_status(meta_status: Option<&str>, tasks: &[WorkflowTask]) -> String {
    if tasks.iter().any(|task| task.status == "failed") {
        return "failed".to_string();
    }
    if tasks.iter().any(|task| task.status == "active") {
        return "active".to_string();
    }
    if !tasks.is_empty() && tasks.iter().all(|task| task.status == "done") {
        return "complete".to_string();
    }
    normalize_plan_status(meta_status.unwrap_or("pending")).to_string()
}

fn normalize_plan_status(status: &str) -> &str {
    match status.trim().to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" | "passed" | "success" => "complete",
        "active" | "running" | "in_progress" | "implementing" | "working" => "active",
        "failed" | "fail" | "error" => "failed",
        "ready" | "pending" | "planned" | "new" => "pending",
        _ => "pending",
    }
}

fn normalize_status(status: &str) -> &str {
    match status.trim().to_ascii_lowercase().as_str() {
        "done" | "complete" | "completed" | "passed" | "success" => "done",
        "active" | "running" | "in_progress" | "implementing" | "working" | "dispatching" => {
            "active"
        }
        "failed" | "fail" | "error" => "failed",
        "blocked" | "waiting" => "blocked",
        _ => "pending",
    }
}

fn merge_live_state(workflow: &mut WorkflowSnapshot, dashboard: &DashboardSnapshot) {
    let workflow_id = workflow.id.clone();
    let plan_ids: Vec<String> = workflow.plans.iter().map(|plan| plan.id.clone()).collect();
    let mut agent_by_task: HashMap<String, LiveAgent> = HashMap::new();
    let mut live = WorkflowLive::default();

    for plan in dashboard.plans.values() {
        if plan_ids.contains(&plan.plan_id) || plan.plan_id == workflow_id {
            live.plans.push(LivePlan {
                plan_id: plan.plan_id.clone(),
                phase: plan.phase.clone(),
                active: plan.active,
                tasks_total: plan.tasks_total,
                tasks_done: plan.tasks_done,
                tasks_failed: plan.tasks_failed,
            });
        }
    }

    for task in dashboard.tasks.values() {
        if plan_ids.contains(&task.plan_id) || task.plan_id == workflow_id {
            live.tasks.push(LiveTask {
                plan_id: task.plan_id.clone(),
                task_id: task.task_id.clone(),
                title: task.title.clone(),
                phase: task.phase.clone(),
                outcome: task.outcome.clone(),
                status: live_task_status(task.phase.as_str(), task.outcome.as_deref()).to_string(),
            });
        }
    }

    for agent in dashboard.agents.values() {
        let matches_plan = !agent.current_plan.is_empty()
            && (plan_ids.contains(&agent.current_plan) || agent.current_plan == workflow_id);
        let matches_agent_id = agent.agent_id.starts_with(&format!("{workflow_id}:"));
        if matches_plan || matches_agent_id {
            let live_agent = LiveAgent {
                agent_id: agent.agent_id.clone(),
                role: agent.role.clone(),
                model: agent.model.clone(),
                active: agent.active,
                current_plan: agent.current_plan.clone(),
                current_task: agent.current_task.clone(),
                input_tokens: agent.input_tokens,
                output_tokens: agent.output_tokens,
                cost_usd: agent.cost_usd,
                output_bytes: agent.output_bytes,
            };
            if !agent.current_task.is_empty() {
                agent_by_task.insert(agent.current_task.clone(), live_agent.clone());
            }
            live.stats.cost_usd += agent.cost_usd;
            live.stats.input_tokens += agent.input_tokens;
            live.stats.output_tokens += agent.output_tokens;
            live.agents.push(live_agent);
        }
    }

    for gate in &dashboard.gates {
        if plan_ids.contains(&gate.plan_id) || gate.plan_id == workflow_id {
            if gate.passed {
                live.stats.gates_passed += 1;
            } else {
                live.stats.gates_failed += 1;
            }
            live.gates.push(LiveGate {
                plan_id: gate.plan_id.clone(),
                task_id: gate.task_id.clone(),
                gate: gate.gate.clone(),
                passed: gate.passed,
                ts_millis: gate.ts_millis,
            });
        }
    }

    for entry in dashboard.event_log.iter() {
        if plan_ids.contains(&entry.plan_id) || entry.plan_id == workflow_id {
            live.events.push(LiveEvent {
                timestamp_ms: entry.timestamp_ms,
                event_type: entry.event_type.clone(),
                plan_id: entry.plan_id.clone(),
                task_id: entry.task_id.clone(),
                message: entry.message.clone(),
            });
        }
    }

    let live_by_key: HashMap<(String, String), LiveTask> = live
        .tasks
        .iter()
        .map(|task| ((task.plan_id.clone(), task.task_id.clone()), task.clone()))
        .collect();
    let gates = live.gates.clone();

    for plan in &mut workflow.plans {
        for task in &mut plan.tasks {
            if let Some(live_task) = live_by_key.get(&(plan.id.clone(), task.id.clone())) {
                task.status = live_task.status.clone();
                task.raw_status = live_task
                    .outcome
                    .clone()
                    .or_else(|| Some(live_task.phase.clone()));
                task.phase = Some(live_task.phase.clone());
            }
            if let Some(agent) = agent_by_task.get(&task.id) {
                task.agent_id = Some(agent.agent_id.clone());
                if !agent.model.is_empty() {
                    task.selected_model = Some(agent.model.clone());
                }
            }
            for verify in &mut task.verify {
                if let Some(gate) = gates.iter().rev().find(|gate| {
                    gate.plan_id == plan.id && gate.task_id == task.id && gate.gate == verify.phase
                }) {
                    verify.status = if gate.passed { "passed" } else { "failed" }.to_string();
                }
            }
        }
        plan.status = derive_plan_status(Some(&plan.status), &plan.tasks);
    }

    workflow.live = live;
}

fn live_task_status(phase: &str, outcome: Option<&str>) -> &'static str {
    if let Some(outcome) = outcome {
        let outcome = outcome.to_ascii_lowercase();
        if outcome.contains("fail") || outcome.contains("error") {
            return "failed";
        }
        return "done";
    }
    match phase.to_ascii_lowercase().as_str() {
        "completed" | "done" => "done",
        "failed" | "error" => "failed",
        "blocked" => "blocked",
        _ => "active",
    }
}

fn summarize_workflow(workflow: &WorkflowSnapshot) -> WorkflowSummary {
    let tasks: Vec<&WorkflowTask> = workflow.plans.iter().flat_map(|plan| &plan.tasks).collect();
    WorkflowSummary {
        id: workflow.id.clone(),
        title: workflow.title.clone(),
        phase: workflow.phase.clone(),
        updated_at_millis: workflow.updated_at_millis,
        prd_status: workflow.prd.as_ref().map(|prd| prd.status.clone()),
        plan_count: workflow.plans.len(),
        task_count: tasks.len(),
        active_tasks: tasks.iter().filter(|task| task.status == "active").count(),
        done_tasks: tasks.iter().filter(|task| task.status == "done").count(),
        failed_tasks: tasks.iter().filter(|task| task.status == "failed").count(),
    }
}

fn empty_summary(id: &str) -> WorkflowSummary {
    WorkflowSummary {
        id: id.to_string(),
        title: title_from_slug(id),
        phase: "idle".to_string(),
        updated_at_millis: 0,
        prd_status: None,
        plan_count: 0,
        task_count: 0,
        active_tasks: 0,
        done_tasks: 0,
        failed_tasks: 0,
    }
}

fn workflow_phase(workflow: &WorkflowSnapshot) -> String {
    if workflow.live.plans.iter().any(|plan| plan.active)
        || workflow
            .plans
            .iter()
            .flat_map(|plan| &plan.tasks)
            .any(|task| task.status == "active")
    {
        return "implementing".to_string();
    }
    if workflow
        .plans
        .iter()
        .flat_map(|plan| &plan.tasks)
        .any(|task| task.status == "failed")
    {
        return "failed".to_string();
    }
    let task_count = workflow
        .plans
        .iter()
        .map(|plan| plan.tasks.len())
        .sum::<usize>();
    if task_count > 0
        && workflow
            .plans
            .iter()
            .flat_map(|plan| &plan.tasks)
            .all(|task| task.status == "done")
    {
        return "complete".to_string();
    }
    if task_count > 0 {
        return "tasks".to_string();
    }
    if !workflow.plans.is_empty() {
        return "planning".to_string();
    }
    workflow
        .prd
        .as_ref()
        .map(|prd| prd.status.clone())
        .unwrap_or_else(|| "idle".to_string())
}

fn dashboard_event_matches_workflow(event: &DashboardEvent, workflow_id: Option<&str>) -> bool {
    let Some(workflow_id) = workflow_id else {
        return true;
    };
    match event {
        DashboardEvent::PlanStarted { plan_id }
        | DashboardEvent::PlanCompleted { plan_id, .. }
        | DashboardEvent::TaskStarted { plan_id, .. }
        | DashboardEvent::TaskCompleted { plan_id, .. }
        | DashboardEvent::TaskPhaseChanged { plan_id, .. }
        | DashboardEvent::GateResult { plan_id, .. }
        | DashboardEvent::PhaseTransition { plan_id, .. }
        | DashboardEvent::EfficiencyEvent { plan_id, .. }
        | DashboardEvent::EventLogEntry { plan_id, .. } => plan_id == workflow_id,
        DashboardEvent::AgentSpawned { agent_id, .. }
        | DashboardEvent::AgentOutput { agent_id, .. }
        | DashboardEvent::AgentCompleted { agent_id } => {
            agent_id == workflow_id || agent_id.starts_with(&format!("{workflow_id}:"))
        }
        DashboardEvent::AtelierPrdsUpdated { prds, tasks } => {
            prds.iter().any(|prd| prd.slug == workflow_id) || tasks.contains_key(workflow_id)
        }
        _ => false,
    }
}

fn dashboard_snapshot_for_workdir(root: &Path, state: &AppState) -> DashboardSnapshot {
    let mut snapshot = state.state_hub.current_snapshot();
    let event_log = root.join(".roko").join("events.jsonl");
    let Ok(content) = std::fs::read_to_string(event_log) else {
        return snapshot;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<DashboardEvent>(line) {
            snapshot.apply(&event);
        }
    }
    snapshot
}

fn extract_title(markdown: &str) -> Option<String> {
    markdown
        .lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .or_else(|| frontmatter_string(markdown, "title"))
}

fn frontmatter_string(markdown: &str, key: &str) -> Option<String> {
    let mut lines = markdown.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        let Some((candidate, value)) = line.split_once(':') else {
            continue;
        };
        if candidate.trim() == key {
            return Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            );
        }
    }
    None
}

fn section_items(markdown: &str, names: &[&str]) -> Vec<String> {
    let mut active = false;
    let mut out = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading = heading.to_ascii_lowercase();
            active = names.iter().any(|name| heading.contains(name));
            continue;
        }
        if active {
            if trimmed.starts_with('#') {
                active = false;
                continue;
            }
            if let Some(item) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
            {
                let item = item.trim();
                if !item.is_empty() && out.len() < 8 {
                    out.push(item.to_string());
                }
            }
        }
    }
    out
}

fn markdown_excerpt(markdown: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence
            || trimmed.is_empty()
            || trimmed.starts_with("---")
            || trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
        {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(trimmed);
        if out.len() >= limit {
            out.truncate(limit);
            break;
        }
    }
    out
}

fn title_from_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slugify_title(title: &str) -> String {
    let slug = title
        .split_whitespace()
        .take(7)
        .map(|word| {
            word.chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "idea".to_string()
    } else {
        slug
    }
}

fn modified_millis(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or_else(|_| SystemTime::now())
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
