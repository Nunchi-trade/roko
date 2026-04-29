//! Research endpoints — topic research, PRD/plan/task enhancement, analysis.

use std::fmt::Write as _;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use validator::{Validate, ValidationError};

use crate::error::{ApiError, validate_path_segment};
use crate::events::ServerEvent;
use crate::extract::{RequestPayload, ValidJson, validate_with_validator};
use crate::state::{AppState, OperationHandle, OperationStatus};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/research", get(list_research))
        .route("/research/topic", post(research_topic))
        .route("/research/enhance-prd/{slug}", post(enhance_prd))
        .route("/research/enhance-plan/{plan}", post(enhance_plan))
        .route("/research/enhance-tasks/{plan}", post(enhance_tasks))
        .route("/research/analyze", post(analyze))
}

const VALID_INTENTS: &[&str] = &["position", "evaluate", "monitor", "explore", "audit"];

/// `GET /api/research` — list research artifacts from `.roko/research/`.
async fn list_research(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let dir = state.workdir.join(".roko").join("research");
    if !dir.is_dir() {
        return Ok(Json(json!([])));
    }

    let mut artifacts = Vec::new();
    let mut rd = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("read research dir: {e}")))?;

    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("read entry: {e}")))?
    {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let meta = tokio::fs::metadata(&path).await.ok();
            artifacts.push(json!({
                "name": name,
                "size": meta.as_ref().map(std::fs::Metadata::len),
                "is_file": path.is_file(),
            }));
        }
    }

    Ok(Json(Value::Array(artifacts)))
}

#[derive(Deserialize, Validate)]
struct TopicRequest {
    #[validate(
        length(min = 1),
        custom(function = "crate::extract::validate_non_blank")
    )]
    topic: String,
    #[serde(default = "default_intent")]
    #[validate(custom(function = "validate_intent"))]
    intent: String,
}

impl RequestPayload for TopicRequest {
    fn validate_payload(&self) -> Result<(), ApiError> {
        validate_with_validator(self)
    }
}

fn default_intent() -> String {
    "explore".to_string()
}

/// `POST /api/research/topic` — spawn background topic research.
async fn research_topic(
    State(state): State<Arc<AppState>>,
    ValidJson(body): ValidJson<TopicRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let topic = body.topic.trim();
    let prompt = build_topic_prompt(&state.workdir, topic, &body.intent);
    let (status, payload) = spawn_research_op(
        &state,
        ResearchMode::Topic,
        format!("{}:{topic}", body.intent),
        prompt,
    )
    .await?;
    Ok((
        status,
        Json(json!({
            "id": payload.0["id"].clone(),
            "intent": body.intent,
        })),
    ))
}

fn validate_intent(value: &str) -> Result<(), ValidationError> {
    if VALID_INTENTS.contains(&value) {
        Ok(())
    } else {
        let mut error = ValidationError::new("intent");
        error.message = Some(format!("must be one of: {}", VALID_INTENTS.join(", ")).into());
        Err(error)
    }
}

/// `POST /api/research/enhance-prd/:slug` — enhance a PRD with research.
async fn enhance_prd(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_path_segment(&slug, "slug")?;
    let (prd_path, prd_status, prd_content) = read_prd_context(&state.workdir, &slug).await?;
    let prompt =
        build_enhance_prd_prompt(&state.workdir, &slug, &prd_path, &prd_status, &prd_content);
    spawn_research_op(&state, ResearchMode::EnhancePrd, slug, prompt).await
}

/// `POST /api/research/enhance-plan/:plan` — optimize a plan with research.
async fn enhance_plan(
    State(state): State<Arc<AppState>>,
    Path(plan): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_path_segment(&plan, "plan")?;
    let context = read_plan_context(&state.workdir, &plan).await?;
    let prompt = build_enhance_plan_prompt(&state.workdir, &plan, &context);
    spawn_research_op(&state, ResearchMode::EnhancePlan, plan, prompt).await
}

/// `POST /api/research/enhance-tasks/:plan` — split/optimize tasks.
async fn enhance_tasks(
    State(state): State<Arc<AppState>>,
    Path(plan): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate_path_segment(&plan, "plan")?;
    let context = read_plan_context(&state.workdir, &plan).await?;
    let prompt = build_enhance_tasks_prompt(&state.workdir, &plan, &context);
    spawn_research_op(&state, ResearchMode::EnhanceTasks, plan, prompt).await
}

/// `POST /api/research/analyze` — analyze execution data.
async fn analyze(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    let context = read_analysis_context(&state).await?;
    let prompt = build_analyze_prompt(&state.workdir, &context);
    spawn_research_op(
        &state,
        ResearchMode::Analyze,
        "execution_data".to_string(),
        prompt,
    )
    .await
}

// ── helpers ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum ResearchMode {
    Topic,
    EnhancePrd,
    EnhancePlan,
    EnhanceTasks,
    Analyze,
}

impl ResearchMode {
    fn operation_kind(self) -> &'static str {
        match self {
            Self::Topic => "research_topic",
            Self::EnhancePrd => "research_enhance_prd",
            Self::EnhancePlan => "research_enhance_plan",
            Self::EnhanceTasks => "research_enhance_tasks",
            Self::Analyze => "research_analyze",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Topic => "topic research",
            Self::EnhancePrd => "PRD enhancement",
            Self::EnhancePlan => "plan enhancement",
            Self::EnhanceTasks => "task enhancement",
            Self::Analyze => "execution analysis",
        }
    }
}

/// Spawn a generic background research operation.
async fn spawn_research_op(
    state: &AppState,
    mode: ResearchMode,
    target: String,
    prompt: String,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let op_id = uuid::Uuid::new_v4().to_string();
    let bus = state.event_bus.clone();
    let runtime = state.runtime.clone();
    let workdir = state.workdir.clone();
    let kind = mode.operation_kind().to_string();
    let target_for_kind = target.clone();
    let target_for_log = target_for_kind.clone();

    let handle = tokio::spawn({
        let op_id = op_id.clone();
        let kind = kind.clone();
        async move {
            bus.publish(ServerEvent::OperationStarted {
                op_id: op_id.clone(),
                kind: kind.clone(),
            });

            match runtime.run_once(&workdir, &prompt).await {
                Ok(result) => {
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind,
                        success: result.success,
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        mode = %mode.label(),
                        target = %target_for_log,
                        error = %err,
                        "research operation failed"
                    );
                    bus.publish(ServerEvent::Error {
                        message: format!("{} failed for {target_for_log}: {err}", mode.label()),
                    });
                    bus.publish(ServerEvent::OperationCompleted {
                        op_id,
                        kind,
                        success: false,
                    });
                }
            }
        }
    });

    let op = OperationHandle {
        id: op_id.clone(),
        kind: format!("{kind}:{target_for_kind}"),
        status: OperationStatus::Running,
        handle,
    };

    state.operations.write().await.insert(op_id.clone(), op);

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "id": op_id })),
    ))
}

fn build_topic_prompt(workdir: &FsPath, topic: &str, intent: &str) -> String {
    let research_path = research_artifact_path(workdir, topic);
    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "You are a technical research analyst working inside a roko workspace."
    );
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(prompt, "Research topic: {topic}\n");
    let _ = writeln!(prompt, "Research intent: {intent}\n");
    let _ = writeln!(
        prompt,
        "Find real sources, synthesize them into actionable guidance, and write the report to {}.",
        research_path.display()
    );
    let _ = writeln!(
        prompt,
        "Use a findings / relevance / recommendation structure and keep the result specific to the workspace."
    );
    let _ = writeln!(prompt, "Finish with this intent-specific ending:");
    let _ = writeln!(prompt, "{}", intent_instructions(intent));
    prompt
}

fn intent_instructions(intent: &str) -> &'static str {
    match intent {
        "position" => "Directional recommendation, confidence level, and the single biggest risk.",
        "evaluate" => "Risk scores, red flags, and comparison to the most relevant alternatives.",
        "monitor" => "Timeline of changes, impact assessment, and concrete alerts to set.",
        "audit" => "Checklist of verified claims, unverified gaps, and severity for each gap.",
        _ => "Landscape map, key players, and the most important knowledge gaps.",
    }
}

fn build_enhance_prd_prompt(
    workdir: &FsPath,
    slug: &str,
    prd_path: &FsPath,
    prd_status: &str,
    prd_content: &str,
) -> String {
    let research_path = research_artifact_path(workdir, slug);
    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "You are enhancing a PRD with research-backed guidance."
    );
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(prompt, "PRD status: {prd_status}");
    let _ = writeln!(prompt, "PRD path: {}\n", prd_path.display());
    let _ = writeln!(
        prompt,
        "Read the PRD fully, add missing citations, flag unsupported claims, add mermaid diagrams where helpful, and update the file in place."
    );
    let _ = writeln!(
        prompt,
        "Also save a short research summary to {}.\n",
        research_path.display()
    );
    let _ = writeln!(prompt, "## PRD content\n```md\n{prd_content}\n```");
    prompt
}

fn build_enhance_plan_prompt(workdir: &FsPath, plan: &str, context: &str) -> String {
    let research_path = research_path_for_plan(workdir, plan, "plan");
    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "You are optimizing an implementation plan with research-backed techniques."
    );
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(prompt, "Plan: {plan}\n");
    let _ = writeln!(
        prompt,
        "Improve task decomposition, verification quality, context injection, and parallelism. Update the plan files in place."
    );
    let _ = writeln!(
        prompt,
        "Also save a research summary to {}.\n",
        research_path.display()
    );
    let _ = writeln!(prompt, "## Plan context\n{context}");
    prompt
}

fn build_enhance_tasks_prompt(workdir: &FsPath, plan: &str, context: &str) -> String {
    let research_path = research_path_for_plan(workdir, plan, "tasks");
    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "You are optimizing tasks for a plan using research-backed execution practices."
    );
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(prompt, "Plan: {plan}\n");
    let _ = writeln!(
        prompt,
        "For each task, tighten context.read_files ranges, add tier and model_hint, keep verify commands runnable, and remove unnecessary dependency edges."
    );
    let _ = writeln!(
        prompt,
        "Update tasks.toml in place and save a research summary to {}.\n",
        research_path.display()
    );
    let _ = writeln!(prompt, "## Task context\n{context}");
    prompt
}

fn build_analyze_prompt(workdir: &FsPath, context: &str) -> String {
    let analysis_path = workdir.join(".roko").join("research").join(format!(
        "execution-analysis-{}.md",
        chrono::Local::now().format("%Y%m%d")
    ));
    let mut prompt = String::new();
    let _ = writeln!(
        prompt,
        "You are analyzing execution data for self-learning and routing improvements."
    );
    let _ = writeln!(prompt, "Workspace: {}", workdir.display());
    let _ = writeln!(
        prompt,
        "Analyze .roko/memory/episodes.jsonl and .roko/engrams.jsonl, then write the results to {}.",
        analysis_path.display()
    );
    let _ = writeln!(
        prompt,
        "Report first-attempt pass rate, cost by tier, retry patterns, context-size correlation, and concrete recommendations.\n"
    );
    let _ = writeln!(prompt, "## Execution context\n{context}");
    prompt
}

fn research_artifact_path(workdir: &FsPath, topic: &str) -> PathBuf {
    workdir
        .join(".roko")
        .join("research")
        .join(format!("{}.md", slug(topic)))
}

fn research_path_for_plan(workdir: &FsPath, plan: &str, suffix: &str) -> PathBuf {
    workdir
        .join(".roko")
        .join("research")
        .join(format!("{}-{suffix}.md", slug(plan)))
}

fn slug(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

async fn read_prd_context(
    workdir: &FsPath,
    slug: &str,
) -> Result<(PathBuf, String, String), ApiError> {
    let prd_dir = workdir.join(".roko").join("prd");
    let published = prd_dir.join("published").join(format!("{slug}.md"));
    if published.is_file() {
        let content = tokio::fs::read_to_string(&published)
            .await
            .map_err(|e| ApiError::internal(format!("read published prd: {e}")))?;
        return Ok((published, "published".into(), content));
    }

    let draft = prd_dir.join("drafts").join(format!("{slug}.md"));
    if draft.is_file() {
        let content = tokio::fs::read_to_string(&draft)
            .await
            .map_err(|e| ApiError::internal(format!("read draft prd: {e}")))?;
        return Ok((draft, "draft".into(), content));
    }

    Err(ApiError::not_found(format!("PRD '{slug}' not found")))
}

async fn read_plan_context(workdir: &FsPath, plan: &str) -> Result<String, ApiError> {
    let mut sections = Vec::new();
    let mut found = false;

    for (label, fence, path) in plan_context_candidates(workdir, plan) {
        if !path.is_file() {
            continue;
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ApiError::internal(format!("read {label}: {e}")))?;
        sections.push(file_context_block(label, &path, fence, &content));
        found = true;
    }

    if !found {
        return Err(ApiError::not_found(format!("plan '{plan}' not found")));
    }

    Ok(sections.join("\n\n"))
}

fn plan_context_candidates(
    workdir: &FsPath,
    plan: &str,
) -> Vec<(&'static str, &'static str, PathBuf)> {
    let plan_dir = workdir.join("plans").join(plan);
    let roko_plan_dir = workdir.join(".roko").join("plans").join(plan);
    let roko_flat_plan = workdir.join(".roko").join("plans");

    vec![
        ("plan.md", "md", plan_dir.join("plan.md")),
        ("tasks.toml", "toml", plan_dir.join("tasks.toml")),
        ("plan.md", "md", roko_plan_dir.join("plan.md")),
        ("tasks.toml", "toml", roko_plan_dir.join("tasks.toml")),
        (
            "plan.json",
            "json",
            roko_flat_plan.join(format!("{plan}.json")),
        ),
        (
            "plan.toml",
            "toml",
            roko_flat_plan.join(format!("{plan}.toml")),
        ),
    ]
}

async fn read_analysis_context(state: &AppState) -> Result<String, ApiError> {
    let mut sections = Vec::new();

    let episodes_path = state.layout.episodes_path();
    if episodes_path.is_file() {
        let content = tokio::fs::read_to_string(&episodes_path)
            .await
            .map_err(|e| ApiError::internal(format!("read episodes: {e}")))?;
        sections.push(file_context_block(
            "Episodes",
            &episodes_path,
            "jsonl",
            &tail_lines(&content, 120),
        ));
    }

    let signals_path = state.workdir.join(".roko").join("engrams.jsonl");
    if signals_path.is_file() {
        let content = tokio::fs::read_to_string(&signals_path)
            .await
            .map_err(|e| ApiError::internal(format!("read signals: {e}")))?;
        sections.push(file_context_block(
            "Signals",
            &signals_path,
            "jsonl",
            &tail_lines(&content, 120),
        ));
    }

    if sections.is_empty() {
        sections.push(String::from("No execution data files were found."));
    }

    Ok(sections.join("\n\n"))
}

fn truncate_for_prompt(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let mut out = String::with_capacity(content.len().min(max_chars + 32));
    for _ in 0..max_chars {
        if let Some(ch) = chars.next() {
            out.push(ch);
        } else {
            return content.to_string();
        }
    }
    if chars.next().is_some() {
        out.push_str("\n\n[... truncated ...]\n");
    }
    out
}

fn tail_lines(content: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= max_lines {
        return content.to_string();
    }

    let start = lines.len() - max_lines;
    let mut out = String::from("[... truncated ...]\n");
    out.push_str(&lines[start..].join("\n"));
    out
}

fn file_context_block(label: &str, path: &FsPath, fence: &str, content: &str) -> String {
    format!(
        "## {label}\nPath: {}\n\n```{fence}\n{}\n```",
        path.display(),
        truncate_for_prompt(content, 24_000)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;
    use std::time::Duration;

    use anyhow::Result;
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use tempfile::tempdir;
    use tokio::time::timeout;

    use crate::deploy::create_backend;
    use crate::runtime::{CliRuntime, DashboardInfo, RepoInfo, RunResult, SessionStatusInfo};

    #[derive(Default)]
    struct CapturingRuntime {
        runs: Arc<Mutex<Vec<(PathBuf, String)>>>,
    }

    #[async_trait]
    impl CliRuntime for CapturingRuntime {
        async fn run_once(&self, workdir: &FsPath, prompt: &str) -> Result<RunResult> {
            self.runs
                .lock()
                .expect("lock runs")
                .push((workdir.to_path_buf(), prompt.to_string()));
            Ok(RunResult {
                success: true,
                output_text: None,
                usage: None,
            })
        }

        fn session_status(&self, workdir: PathBuf) -> SessionStatusInfo {
            SessionStatusInfo {
                session_id: None,
                workdir,
                daemon_running: false,
                signal_count: None,
                episode_count: None,
                last_episode_passed: None,
            }
        }

        fn dashboard_scaffold(&self, _workdir: &FsPath) -> DashboardInfo {
            DashboardInfo {
                rendered: String::new(),
            }
        }

        fn resolve_repo_workdir(&self, _repo_full_name: &str) -> Option<PathBuf> {
            None
        }

        fn repo_roko_config(
            &self,
            _repo_name: &str,
        ) -> Option<roko_core::config::schema::RokoConfig> {
            None
        }

        fn list_repos(&self) -> Vec<RepoInfo> {
            Vec::new()
        }
    }

    fn test_state_with_runtime() -> (tempfile::TempDir, Arc<AppState>, Arc<CapturingRuntime>) {
        let dir = tempdir().expect("tempdir");
        let workdir = dir.path().to_path_buf();
        let runtime = Arc::new(CapturingRuntime::default());
        let deploy_backend =
            Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
        let runtime_trait: Arc<dyn CliRuntime> = runtime.clone();
        let state = Arc::new(
            AppState::new(
                workdir,
                runtime_trait,
                roko_core::config::schema::RokoConfig::default(),
                deploy_backend,
            )
            .expect("AppState::new"),
        );
        (dir, state, runtime)
    }

    async fn wait_for_runs(runtime: &CapturingRuntime, expected: usize) {
        timeout(Duration::from_secs(2), async {
            loop {
                if runtime.runs.lock().expect("lock runs").len() >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for background research job");
    }

    #[tokio::test]
    async fn topic_research_spawns_a_background_run_with_report_path() {
        let (_dir, state, runtime) = test_state_with_runtime();

        let response = research_topic(
            State(Arc::clone(&state)),
            ValidJson(TopicRequest {
                topic: "Model Routing".into(),
                intent: "position".into(),
            }),
        )
        .await
        .expect("topic research");
        let response = response.into_response();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&body).expect("parse json body");
        assert!(body["id"].is_string());
        assert_eq!(body["intent"], "position");

        wait_for_runs(&runtime, 1).await;
        let runs = runtime.runs.lock().expect("lock runs");
        let (workdir, prompt) = &runs[0];
        assert_eq!(workdir, &state.workdir);
        assert!(prompt.contains("Research topic: Model Routing"));
        assert!(prompt.contains("Research intent: position"));
        assert!(prompt.contains(".roko/research/model-routing.md"));

        let ops = state.operations.read().await;
        let op = ops.values().next().expect("operation stored");
        assert!(op.handle.is_finished());
        assert_eq!(op.kind, "research_topic:position:Model Routing");
    }

    #[tokio::test]
    async fn topic_research_rejects_invalid_intent() {
        assert!(
            TopicRequest {
                topic: "Model Routing".into(),
                intent: "bogus".into(),
            }
            .validate()
            .is_err()
        );
    }

    #[tokio::test]
    async fn enhancement_research_prompts_include_workspace_documents() {
        let (dir, state, runtime) = test_state_with_runtime();
        let prd_dir = dir.path().join(".roko").join("prd").join("published");
        tokio::fs::create_dir_all(&prd_dir).await.expect("prd dir");
        tokio::fs::write(
            prd_dir.join("alpha.md"),
            "# Alpha\n\nFocus on operator ergonomics.\n",
        )
        .await
        .expect("write prd");

        let plan_dir = dir.path().join("plans").join("alpha");
        tokio::fs::create_dir_all(&plan_dir)
            .await
            .expect("plan dir");
        tokio::fs::write(
            plan_dir.join("plan.md"),
            "# Plan Alpha\n\nImprove the plan structure.\n",
        )
        .await
        .expect("write plan");
        tokio::fs::write(
            plan_dir.join("tasks.toml"),
            "[[tasks]]\nid = \"task-1\"\ndescription = \"do something\"\n",
        )
        .await
        .expect("write tasks");

        enhance_prd(State(Arc::clone(&state)), Path("alpha".into()))
            .await
            .expect("enhance prd");
        enhance_plan(State(Arc::clone(&state)), Path("alpha".into()))
            .await
            .expect("enhance plan");
        enhance_tasks(State(Arc::clone(&state)), Path("alpha".into()))
            .await
            .expect("enhance tasks");

        wait_for_runs(&runtime, 3).await;
        let runs = runtime.runs.lock().expect("lock runs");
        assert!(runs[0].1.contains("PRD status: published"));
        assert!(runs[0].1.contains("alpha.md"));
        assert!(runs[0].1.contains("Focus on operator ergonomics."));
        assert!(runs[1].1.contains("## Plan context"));
        assert!(runs[1].1.contains("Plan Alpha"));
        assert!(runs[2].1.contains("## Task context"));
        assert!(runs[2].1.contains("tasks.toml"));
        assert!(runs[2].1.contains("tier and model_hint"));
    }

    #[tokio::test]
    async fn analyze_research_uses_recent_execution_files() {
        let (dir, state, runtime) = test_state_with_runtime();
        let episodes_path = state.layout.episodes_path();
        if let Some(parent) = episodes_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .expect("episodes dir");
        }
        tokio::fs::write(&episodes_path, "episode-1\nepisode-2\n")
            .await
            .expect("write episodes");

        let signals_path = dir.path().join(".roko").join("engrams.jsonl");
        tokio::fs::create_dir_all(signals_path.parent().expect("signals parent"))
            .await
            .expect("signals dir");
        tokio::fs::write(&signals_path, "signal-1\nsignal-2\n")
            .await
            .expect("write signals");

        analyze(State(Arc::clone(&state))).await.expect("analyze");

        wait_for_runs(&runtime, 1).await;
        let runs = runtime.runs.lock().expect("lock runs");
        assert!(runs[0].1.contains("execution-analysis"));
        assert!(runs[0].1.contains("episode-1"));
        assert!(runs[0].1.contains("signal-1"));
        assert!(runs[0].1.contains("execution data"));
    }
}
