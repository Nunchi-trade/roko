//! Integration tests for subprocess safety parity on public agent surfaces.

use roko_agent::provider::{AgentOptions, create_agent_for_model};
use roko_agent::{Agent, ExecAgent, SafetyLayer};
use roko_core::config::schema::RokoConfig;
use roko_core::{Body, Context, Engram, Kind};

fn prompt(text: &str) -> Engram {
    Engram::builder(Kind::Prompt).body(Body::text(text)).build()
}

#[tokio::test]
async fn exec_agent_blocks_direct_git_force_push_before_spawn() {
    let agent = ExecAgent::new(
        "git",
        vec![
            "push".to_string(),
            "--force".to_string(),
            "origin".to_string(),
            "main".to_string(),
        ],
    )
    .with_safety_layer(Some(SafetyLayer::with_defaults()));

    let result = agent.run(&prompt(""), &Context::now()).await;
    assert!(
        !result.success,
        "dangerous direct git subprocess should be blocked"
    );

    let output = result.output.body.as_text().expect("failure output text");
    assert!(
        output.contains("blocked by safety layer"),
        "expected safety-layer failure, got: {output}"
    );
    assert!(
        output.contains("block_force_push"),
        "expected git policy rule in output, got: {output}"
    );
}

#[tokio::test]
async fn create_agent_for_model_exec_fallback_keeps_default_safety() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sentinel = temp.path().join("sentinel");
    let mut config = RokoConfig::default();
    config.agent.command = Some("sh".to_string());

    let agent = create_agent_for_model(
        &config,
        "mystery-model",
        AgentOptions {
            timeout_ms: Some(250),
            name: "fallback-agent".to_string(),
            extra_args: vec![
                "-c".to_string(),
                format!("touch {}; rm -rf /", sentinel.display()),
            ],
            ..Default::default()
        },
    )
    .expect("fallback exec agent");

    let result = agent.run(&prompt(""), &Context::now()).await;
    assert!(
        !result.success,
        "fallback subprocess should inherit the default safety layer"
    );

    let output = result.output.body.as_text().expect("failure output text");
    assert!(
        output.contains("blocked by safety layer"),
        "expected safety-layer failure, got: {output}"
    );
    assert!(
        !sentinel.exists(),
        "blocked fallback command should not spawn the shell"
    );
}
