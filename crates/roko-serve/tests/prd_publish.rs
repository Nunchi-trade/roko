//! Integration coverage for PRD publish auto-orchestration.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use roko_core::config::schema::RokoConfig;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_runtime::event_bus::{PublishOrigin, RokoEvent, global_event_bus};
use roko_serve::deploy::create_backend;
use roko_serve::runtime::{CliRuntime, DashboardInfo, RunResult, SessionStatusInfo};
use roko_serve::start_prd_publish_orchestrator;
use roko_serve::state::AppState;
use tempfile::tempdir;
use tokio::sync::{Mutex, Notify};

static TEST_GUARD: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

fn test_guard() -> &'static Mutex<()> {
    TEST_GUARD.get_or_init(|| Mutex::new(()))
}

#[derive(Clone)]
struct RecordingRuntime {
    call_count: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

#[async_trait::async_trait]
impl CliRuntime for RecordingRuntime {
    async fn run_once(&self, workdir: &Path, prompt: &str) -> anyhow::Result<RunResult> {
        self.call_count.fetch_add(1, Ordering::SeqCst);

        if prompt.contains(".roko/plans") {
            let plan_dir = workdir.join(".roko").join("plans").join("demo");
            tokio::fs::create_dir_all(&plan_dir).await?;
            tokio::fs::write(plan_dir.join("plan.md"), "# Demo plan\n").await?;
            tokio::fs::write(
                plan_dir.join("tasks.toml"),
                "[[tasks]]\nid = \"demo\"\ntitle = \"Demo\"\nstatus = \"todo\"\n",
            )
            .await?;
        }

        self.notify.notify_waiters();
        Ok(RunResult {
            success: true,
            output_text: None,
            usage: None,
            gate_results: Vec::new(),
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

    fn dashboard_scaffold(&self, _workdir: &Path) -> DashboardInfo {
        DashboardInfo {
            rendered: String::new(),
        }
    }
}

fn test_state() -> (
    tempfile::TempDir,
    Arc<AppState>,
    Arc<AtomicUsize>,
    Arc<Notify>,
) {
    let dir = tempdir().expect("tempdir");
    let mut config = RokoConfig::default();
    config.prd.auto_plan = true;
    let call_count = Arc::new(AtomicUsize::new(0));
    let notify = Arc::new(Notify::new());
    let runtime = Arc::new(RecordingRuntime {
        call_count: Arc::clone(&call_count),
        notify: Arc::clone(&notify),
    });
    let deploy_backend =
        Arc::from(create_backend("manual", None, None, None).expect("manual backend"));
    let state = Arc::new(
        AppState::new(dir.path().to_path_buf(), runtime, config, deploy_backend)
            .expect("AppState::new"),
    );
    (dir, state, call_count, notify)
}

async fn write_published_prd(workdir: &Path, slug: &str) -> PathBuf {
    let path = workdir
        .join(".roko")
        .join("prd")
        .join("published")
        .join(format!("{slug}.md"));
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create published dir");
    }
    tokio::fs::write(&path, format!("# {slug}\n"))
        .await
        .expect("write published prd");
    path
}

async fn append_publish_episode(workdir: &Path, slug: &str, path: &Path, origin: PublishOrigin) {
    let published_at = Utc::now();
    let mut episode = Episode::new("test", slug);
    episode.kind = "prd_published".to_string();
    episode.timestamp = published_at;
    episode.started_at = published_at;
    episode.completed_at = published_at;
    episode.success = true;
    episode
        .extra
        .insert("slug".to_string(), serde_json::json!(slug));
    episode.extra.insert(
        "path".to_string(),
        serde_json::json!(path.display().to_string()),
    );
    episode.extra.insert(
        "origin".to_string(),
        serde_json::to_value(origin).expect("origin json"),
    );
    episode.extra.insert(
        "published_at".to_string(),
        serde_json::json!(published_at.to_rfc3339()),
    );

    EpisodeLogger::new(workdir.join(".roko").join("episodes.jsonl"))
        .append(&episode)
        .await
        .expect("append publish episode");
}

#[tokio::test]
async fn audit_publish_triggers_plan_generation() {
    let _guard = test_guard().lock().await;
    let (_dir, state, call_count, notify) = test_state();
    let published_path = write_published_prd(&state.workdir, "demo").await;
    let _subscriber = start_prd_publish_orchestrator(Arc::clone(&state));
    // Allow the subscriber task to start and register its watcher.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    append_publish_episode(&state.workdir, "demo", &published_path, PublishOrigin::Cli).await;

    tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
        .await
        .expect("publish audit should trigger plan generation");

    let tasks_path = state
        .workdir
        .join(".roko")
        .join("plans")
        .join("demo")
        .join("tasks.toml");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if tasks_path.is_file() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("tasks.toml should be created");

    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    state.shutdown().await;
}

#[tokio::test]
async fn repeated_publish_events_are_deduped() {
    let _guard = test_guard().lock().await;
    let (_dir, state, call_count, notify) = test_state();
    let published_path = write_published_prd(&state.workdir, "dedupe-demo").await;
    let _subscriber = start_prd_publish_orchestrator(Arc::clone(&state));

    let event = RokoEvent::PrdPublished {
        slug: "dedupe-demo".to_string(),
        path: published_path,
        published_at: Utc::now(),
        origin: PublishOrigin::Cli,
    };
    global_event_bus().emit(event.clone());
    global_event_bus().emit(event.clone());
    global_event_bus().emit(event);

    tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
        .await
        .expect("first publish should trigger plan generation");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    state.shutdown().await;
}
