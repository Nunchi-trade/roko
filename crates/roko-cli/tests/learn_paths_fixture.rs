//! Regression coverage for learn-path alignment.
//!
//! These tests write fixture data to the exact `.roko/learn/` and
//! `.roko/neuro/` locations that the `learn` command reads, then assert
//! the CLI summary output reflects the stored counts and latest entries.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use chrono::{DateTime, Utc};
use roko_learn::cascade::CascadeStage;
use roko_learn::efficiency::AgentEfficiencyEvent;
use roko_learn::episode_logger::{Episode, Usage};
use tempfile::TempDir;

fn utc(timestamp: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(timestamp)
        .expect("valid RFC3339 timestamp")
        .with_timezone(&Utc)
}

fn write_jsonl(path: &Path, lines: &[String]) {
    fs::write(path, format!("{}\n", lines.join("\n"))).expect("write jsonl");
}

fn run_roko(workdir: &Path, args: &[&str]) -> String {
    let assert = Command::cargo_bin("roko")
        .expect("roko binary")
        .args(args)
        .arg("--workdir")
        .arg(workdir)
        .assert()
        .success();

    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

fn write_learn_fixture(workdir: &Path) {
    let learn_dir = workdir.join(".roko").join("learn");
    let neuro_dir = workdir.join(".roko").join("neuro");
    fs::create_dir_all(&learn_dir).expect("create learn dir");
    fs::create_dir_all(&neuro_dir).expect("create neuro dir");

    let router_first = utc("2026-04-10T08:00:00Z");
    let router_last = utc("2026-04-10T09:30:00Z");
    let router_path = learn_dir.join("cascade-router.json");
    let router_json = serde_json::json!({
        "model_slugs": ["claude-sonnet-4-6", "claude-haiku-4-5"],
        "total_observations": 75,
        "stage_transitions": [
            {
                "from": CascadeStage::Static,
                "to": CascadeStage::Confidence,
                "observations": 50,
                "timestamp": router_first.clone(),
            },
            {
                "from": CascadeStage::Confidence,
                "to": CascadeStage::Ucb,
                "observations": 75,
                "timestamp": router_last.clone(),
            }
        ]
    });
    fs::write(
        &router_path,
        serde_json::to_string(&router_json).expect("serialize router fixture"),
    )
    .expect("write router fixture");

    let efficiency_first = utc("2026-04-10T10:00:00Z");
    let efficiency_last = utc("2026-04-10T11:15:00Z");
    let mut efficiency_a = AgentEfficiencyEvent::default();
    efficiency_a.agent_id = "agent-a".into();
    efficiency_a.role = "Implementer".into();
    efficiency_a.backend = "claude_cli".into();
    efficiency_a.model = "claude-sonnet-4-6".into();
    efficiency_a.plan_id = "plan-a".into();
    efficiency_a.task_id = "task-a".into();
    efficiency_a.input_tokens = 100;
    efficiency_a.output_tokens = 25;
    efficiency_a.cache_read_tokens = 10;
    efficiency_a.cache_write_tokens = 5;
    efficiency_a.cost_usd = 0.0012;
    efficiency_a.cost_usd_without_cache = 0.0015;
    efficiency_a.total_prompt_tokens = 300;
    efficiency_a.system_prompt_tokens = 120;
    efficiency_a.tools_available = 4;
    efficiency_a.tools_used = 1;
    efficiency_a.wall_time_ms = 850;
    efficiency_a.time_to_first_token_ms = 175;
    efficiency_a.was_warm_start = true;
    efficiency_a.iteration = 1;
    efficiency_a.gate_passed = true;
    efficiency_a.model_used = "claude-sonnet-4-6".into();
    efficiency_a.timestamp = efficiency_first.to_rfc3339();

    let mut efficiency_b = AgentEfficiencyEvent::default();
    efficiency_b.agent_id = "agent-b".into();
    efficiency_b.role = "Reviewer".into();
    efficiency_b.backend = "claude_cli".into();
    efficiency_b.model = "claude-haiku-4-5".into();
    efficiency_b.plan_id = "plan-b".into();
    efficiency_b.task_id = "task-b".into();
    efficiency_b.input_tokens = 80;
    efficiency_b.output_tokens = 20;
    efficiency_b.cache_read_tokens = 0;
    efficiency_b.cache_write_tokens = 0;
    efficiency_b.cost_usd = 0.0034;
    efficiency_b.cost_usd_without_cache = 0.0038;
    efficiency_b.total_prompt_tokens = 220;
    efficiency_b.system_prompt_tokens = 90;
    efficiency_b.tools_available = 2;
    efficiency_b.tools_used = 1;
    efficiency_b.wall_time_ms = 910;
    efficiency_b.time_to_first_token_ms = 200;
    efficiency_b.was_warm_start = false;
    efficiency_b.iteration = 2;
    efficiency_b.gate_passed = false;
    efficiency_b.model_used = String::new();
    efficiency_b.timestamp = efficiency_last.to_rfc3339();

    let efficiency_path = learn_dir.join("efficiency.jsonl");
    write_jsonl(
        &efficiency_path,
        &[
            serde_json::to_string(&efficiency_a).expect("serialize efficiency fixture a"),
            serde_json::to_string(&efficiency_b).expect("serialize efficiency fixture b"),
        ],
    );

    let episodes_first = utc("2026-04-10T12:00:00Z");
    let episodes_last = utc("2026-04-10T12:45:00Z");
    let mut episode_a = Episode::new("agent-a", "task-a");
    episode_a.kind = "agent_turn".into();
    episode_a.id = "episode-a".into();
    episode_a.episode_id = "episode-a".into();
    episode_a.agent_id = "agent-a".into();
    episode_a.agent_template = "Implementer".into();
    episode_a.model = "claude-sonnet-4-6".into();
    episode_a.backend = "claude_cli".into();
    episode_a.turns = 1;
    episode_a.tokens_used = 125;
    episode_a.success = true;
    episode_a.usage = Usage {
        input_tokens: 100,
        output_tokens: 25,
        cache_read_tokens: 10,
        cache_write_tokens: 5,
        cost_usd: 0.0012,
        cost_usd_without_cache: 0.0015,
        wall_ms: 850,
    };
    episode_a.timestamp = episodes_first.clone();
    episode_a.started_at = episodes_first.clone();
    episode_a.completed_at = episodes_first.clone();

    let mut episode_b = Episode::new("agent-b", "task-b");
    episode_b.kind = "agent_turn".into();
    episode_b.id = "episode-b".into();
    episode_b.episode_id = "episode-b".into();
    episode_b.agent_id = "agent-b".into();
    episode_b.agent_template = "Reviewer".into();
    episode_b.model = "claude-haiku-4-5".into();
    episode_b.backend = "claude_cli".into();
    episode_b.turns = 1;
    episode_b.tokens_used = 100;
    episode_b.success = false;
    episode_b.failure_reason = Some("gate failed".into());
    episode_b.usage = Usage {
        input_tokens: 80,
        output_tokens: 20,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: 0.0022,
        cost_usd_without_cache: 0.0026,
        wall_ms: 910,
    };
    episode_b.timestamp = episodes_last.clone();
    episode_b.started_at = episodes_last.clone();
    episode_b.completed_at = episodes_last.clone();

    let episodes_path = learn_dir.join("episodes.jsonl");
    write_jsonl(
        &episodes_path,
        &[
            serde_json::to_string(&episode_a).expect("serialize episode fixture a"),
            serde_json::to_string(&episode_b).expect("serialize episode fixture b"),
        ],
    );

    let knowledge_path = neuro_dir.join("knowledge.jsonl");
    write_jsonl(
        &knowledge_path,
        &[serde_json::to_string(&serde_json::json!({
            "id": "knowledge-1",
            "topic": "fixture"
        }))
        .expect("serialize knowledge fixture")],
    );
}

#[test]
fn learn_all_reads_the_expected_fixtures() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    write_learn_fixture(workdir);

    let stdout = run_roko(workdir, &["learn", "all"]);

    let router_path = workdir
        .join(".roko")
        .join("learn")
        .join("cascade-router.json");
    let efficiency_path = workdir.join(".roko").join("learn").join("efficiency.jsonl");
    let episodes_path = workdir.join(".roko").join("learn").join("episodes.jsonl");
    let knowledge_path = workdir.join(".roko").join("neuro").join("knowledge.jsonl");

    let router_first = utc("2026-04-10T08:00:00Z");
    let router_last = utc("2026-04-10T09:30:00Z");
    let efficiency_first = utc("2026-04-10T10:00:00Z");
    let efficiency_last = utc("2026-04-10T11:15:00Z");
    let episodes_first = utc("2026-04-10T12:00:00Z");
    let episodes_last = utc("2026-04-10T12:45:00Z");

    assert!(
        stdout.contains(&format!(
            "Cascade router: 75 observations, 2 models at {}",
            router_path.display()
        )),
        "router summary missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Range: {} .. {}",
            router_first.to_rfc3339(),
            router_last.to_rfc3339()
        )),
        "router range missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Latest: {} confidence -> ucb after 75 observations",
            router_last.to_rfc3339()
        )),
        "router latest line missing or incorrect: {stdout}"
    );

    assert!(
        stdout.contains(&format!(
            "Efficiency: 2 events at {}",
            efficiency_path.display()
        )),
        "efficiency summary missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Range: {} .. {}",
            efficiency_first.to_rfc3339(),
            efficiency_last.to_rfc3339()
        )),
        "efficiency range missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Latest: {} model=claude-haiku-4-5 task=task-b plan=plan-b fail cost=$0.0034",
            efficiency_last.to_rfc3339()
        )),
        "efficiency latest line missing or incorrect: {stdout}"
    );

    assert!(
        stdout.contains(&format!(
            "Episodes: 2 entries at {}",
            episodes_path.display()
        )),
        "episode summary missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Range: {} .. {}",
            episodes_first.to_rfc3339(),
            episodes_last.to_rfc3339()
        )),
        "episode range missing or incorrect: {stdout}"
    );
    assert!(
        stdout.contains(&format!(
            "  Latest: {} model=claude-haiku-4-5 task=task-b fail cost=$0.0022",
            episodes_last.to_rfc3339()
        )),
        "episode latest line missing or incorrect: {stdout}"
    );

    assert!(
        stdout.contains(&format!(
            "Knowledge: 1 durable entries at {}",
            knowledge_path.display()
        )),
        "knowledge summary missing or incorrect: {stdout}"
    );
    assert!(
        !stdout.contains("No data at"),
        "fixture should have satisfied every learn path: {stdout}"
    );
}

#[test]
fn learn_missing_efficiency_reports_no_data_instead_of_fake_counts() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();
    let expected_path = workdir.join(".roko").join("learn").join("efficiency.jsonl");

    let stdout = run_roko(workdir, &["learn", "efficiency"]);

    assert!(
        stdout.contains(&format!("No data at {}", expected_path.display())),
        "missing efficiency file should report the read path: {stdout}"
    );
    assert!(
        !stdout.contains("Efficiency:"),
        "missing efficiency file must not produce a fake summary: {stdout}"
    );
}
