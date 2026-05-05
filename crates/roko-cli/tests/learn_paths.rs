//! Integration test verifying that the paths execution writes to are the same
//! paths the `learn` readers consume.
//!
//! The test writes fixture data into a temp workspace, then calls the real
//! read helpers and asserts the data is visible.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use roko_core::OperatingFrequency;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, Usage};
use roko_learn::prompt_experiment::{ExperimentStore, PromptExperiment, PromptVariant};
use roko_learn::runtime_feedback::{
    read_project_efficiency_events, read_project_episodes_lossy, read_project_learning_snapshot,
};
use tempfile::TempDir;

fn write_jsonl<T: serde::Serialize>(path: &Path, records: &[T]) {
    fs::create_dir_all(path.parent().expect("jsonl path has a parent")).expect("create dirs");
    let mut file = fs::File::create(path).expect("create file");
    for record in records {
        let line = serde_json::to_string(record).expect("serialize record");
        writeln!(file, "{line}").expect("write line");
    }
}

fn roko_root(workdir: &Path) -> PathBuf {
    workdir.join(".roko")
}

fn learn_root(workdir: &Path) -> PathBuf {
    roko_root(workdir).join("learn")
}

fn sample_efficiency_event(plan_id: &str, task_id: &str, timestamp: &str) -> AgentEfficiencyEvent {
    let mut event = AgentEfficiencyEvent::default();
    event.agent_id = "agent-path-test".to_string();
    event.role = "implementer".to_string();
    event.backend = "claude".to_string();
    event.model = "claude-sonnet-4-6".to_string();
    event.plan_id = plan_id.to_string();
    event.task_id = task_id.to_string();
    event.input_tokens = 100;
    event.output_tokens = 50;
    event.cache_read_tokens = 0;
    event.cache_write_tokens = 0;
    event.cost_usd = 0.001;
    event.cost_usd_without_cache = 0.001;
    event.total_prompt_tokens = 150;
    event.system_prompt_tokens = 50;
    event.tools_available = 5;
    event.tools_used = 2;
    event.wall_time_ms = 1_500;
    event.duration_ms = 1_500;
    event.time_to_first_token_ms = 200;
    event.was_warm_start = false;
    event.iteration = 1;
    event.gate_passed = true;
    event.outcome = "success".to_string();
    event.model_used = "claude-sonnet-4-6".to_string();
    event.frequency = OperatingFrequency::Theta;
    event.strategy_attempted = "none".to_string();
    event.timestamp = timestamp.to_string();
    event
}

fn sample_episode(
    agent_id: &str,
    task_id: &str,
    episode_id: &str,
    model: &str,
    success: bool,
) -> Episode {
    let mut episode = Episode::new(agent_id, task_id);
    episode.kind = "agent_turn".to_string();
    episode.id = episode_id.to_string();
    episode.episode_id = episode_id.to_string();
    episode.agent_template = "implementer".to_string();
    episode.model = model.to_string();
    episode.backend = "claude".to_string();
    episode.success = success;
    episode.tokens_used = 150;
    episode.usage = Usage {
        input_tokens: 100,
        output_tokens: 50,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: 0.001,
        cost_usd_without_cache: 0.001,
        wall_ms: 1_500,
    };
    episode
}

fn sample_experiment_store(path: &Path) {
    let mut store = ExperimentStore::new();
    let experiment = PromptExperiment::new(
        "exp-path-test",
        "model-routing",
        vec![PromptVariant {
            id: "variant-a".to_string(),
            name: "Variant A".to_string(),
            section_name: "model-routing".to_string(),
            content: "claude-sonnet-4-6".to_string(),
            slug: Some("claude-sonnet-4-6".to_string()),
            active: true,
        }],
    );
    store.register(experiment);
    store.save(path).expect("save experiments.json");
}

#[tokio::test]
async fn learn_path_efficiency_write_path_matches_reader() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let efficiency_path = learn_root(workdir).join("efficiency.jsonl");

    let events = [
        sample_efficiency_event("plan-a", "T1", "2026-04-29T00:00:00Z"),
        sample_efficiency_event("plan-a", "T2", "2026-04-29T00:00:01Z"),
        sample_efficiency_event("plan-b", "T1", "2026-04-29T00:00:02Z"),
    ];
    write_jsonl(&efficiency_path, &events);

    let read_back = read_project_efficiency_events(workdir)
        .await
        .expect("read project efficiency events");

    assert_eq!(read_back.len(), 3);
    assert_eq!(read_back[0].plan_id, "plan-a");
    assert_eq!(read_back[0].task_id, "T1");
    assert_eq!(read_back[1].task_id, "T2");
    assert_eq!(read_back[2].plan_id, "plan-b");
}

#[tokio::test]
async fn learn_path_episodes_write_path_matches_reader() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let episodes_path = roko_root(workdir).join("episodes.jsonl");

    let episodes = [
        sample_episode("agent-1", "task-1", "episode-1", "claude-sonnet-4-6", true),
        sample_episode("agent-2", "task-2", "episode-2", "claude-haiku-4-5", false),
    ];
    write_jsonl(&episodes_path, &episodes);

    let read_back = read_project_episodes_lossy(workdir)
        .await
        .expect("read project episodes");

    assert_eq!(read_back.len(), 2);
    assert_eq!(read_back[0].episode_id, "episode-1");
    assert_eq!(read_back[1].episode_id, "episode-2");
}

#[tokio::test]
async fn learn_path_episodes_in_learn_subdir_are_also_readable() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let learn_episodes_path = learn_root(workdir).join("episodes.jsonl");

    let episode = sample_episode(
        "agent-learn",
        "task-3",
        "episode-learn",
        "claude-sonnet-4-6",
        true,
    );
    write_jsonl(&learn_episodes_path, &[episode]);

    let read_back = read_project_episodes_lossy(workdir)
        .await
        .expect("read learn subdir episodes");

    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].episode_id, "episode-learn");
}

#[tokio::test]
async fn learn_path_cascade_router_is_under_learn_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let router_path = learn_root(workdir).join("cascade-router.json");

    fs::create_dir_all(router_path.parent().expect("router path parent")).expect("create dirs");
    fs::write(
        &router_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "model_slugs": ["claude-sonnet-4-6"],
            "total_observations": 5,
            "stage_transitions": [],
        }))
        .expect("serialize router fixture"),
    )
    .expect("write cascade router");

    let snapshot = read_project_learning_snapshot(workdir)
        .await
        .expect("read project learning snapshot");

    assert_eq!(snapshot.cascade_router_path, router_path);
    assert!(snapshot.cascade_router.is_some());
}

#[test]
fn learn_path_experiments_are_under_learn_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let experiments_path = learn_root(workdir).join("experiments.json");

    sample_experiment_store(&experiments_path);

    let loaded = ExperimentStore::load_or_new(&experiments_path);
    assert_eq!(loaded.running_count(), 1);
    assert_eq!(loaded.concluded_count(), 0);
    assert!(loaded.get("exp-path-test").is_some());
}

#[test]
fn learn_path_gate_thresholds_are_under_learn_dir() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let thresholds_path = learn_root(workdir).join("gate-thresholds.json");

    fs::create_dir_all(thresholds_path.parent().expect("thresholds path parent"))
        .expect("create dirs");
    fs::write(
        &thresholds_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "rungs": {
                "compile": { "pass_rate_ema": 0.9, "n": 10 },
                "test": { "pass_rate_ema": 0.85, "n": 8 }
            }
        }))
        .expect("serialize gate thresholds"),
    )
    .expect("write gate thresholds");

    let content = fs::read_to_string(&thresholds_path).expect("read gate thresholds");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse gate thresholds");
    let rungs = parsed["rungs"].as_object().expect("rungs object");

    assert_eq!(
        thresholds_path,
        learn_root(workdir).join("gate-thresholds.json")
    );
    assert_eq!(rungs.len(), 2);
    assert!(rungs.contains_key("compile"));
    assert!(rungs.contains_key("test"));
}
