//! End-to-end workflow: prompts → agent → gate → persistence → policies.
//!
//! This test wires together every Roko primitive in a realistic
//! coding-agent workflow:
//!
//! 1. **Substrate setup**: FileSubstrate in a tempdir (persistent state).
//! 2. **Prompt sections** seeded into the substrate (role, task, hints).
//! 3. **PromptComposer** assembles them into a final prompt under a budget.
//! 4. **Agent** (mocked) processes the prompt, emits output.
//! 5. **CompileGate** verifies a toy cargo project.
//! 6. **Policy** emits an Episode signal tying everything together.
//! 7. All signals persist and are queryable with full lineage.
//!
//! This is the proof that the 7-primitive architecture can run a
//! Mori-like workflow end-to-end.

use roko_agent::{Agent, ExecAgent, MockAgent, safety::SafetyLayer};
use roko_compose::{CacheLayer, Placement, PromptComposer, PromptSection, SectionPriority};
use roko_core::{
    Body, Budget, Compose, ContentHash, Context, Decay, Engram, Kind, Provenance, Query, React,
    Store, Verdict, Verify,
};
use roko_fs::FileSubstrate;
use roko_gate::{BuildSystem, CompileGate, GatePayload};
use roko_std::NoOpScorer;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::fs;

/// An episode policy: every gate verdict gets logged as an Episode signal
/// whose body records {task_id, verdict, timestamp}.
struct EpisodePolicy;

impl React for EpisodePolicy {
    fn decide(&self, stream: &[Engram], ctx: &Context) -> Vec<Engram> {
        stream
            .iter()
            .filter(|s| s.kind == Kind::GateVerdict)
            .map(|v| {
                Engram::builder(Kind::Episode)
                    .body(
                        Body::from_json(&serde_json::json!({
                            "verdict_id": v.id.to_hex(),
                            "passed": v.tag("passed").unwrap_or("unknown"),
                            "logged_at_ms": ctx.now_ms,
                        }))
                        .unwrap_or(Body::empty()),
                    )
                    .provenance(Provenance::trusted("episode_policy"))
                    .lineage([v.id])
                    .decay(Decay::WISDOM)
                    .build()
            })
            .collect()
    }

    fn name(&self) -> &str {
        "episode_policy"
    }
}

async fn scaffold_cargo_project(root: &std::path::Path) {
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "roko_e2e_fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .await
    .unwrap();
    fs::create_dir_all(root.join("src")).await.unwrap();
    fs::write(
        root.join("src/lib.rs"),
        "pub fn greet() -> &'static str { \"hi\" }\n",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn coding_agent_full_loop() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("project");
    fs::create_dir_all(&project).await.unwrap();
    scaffold_cargo_project(&project).await;

    // 1. FileSubstrate: durable storage for all signals.
    let substrate: Arc<dyn Store> =
        Arc::new(FileSubstrate::open(tmp.path().join(".roko")).await.unwrap());

    // 2. Seed prompt sections. Each section is its own signal.
    let role = PromptSection::new("role", "You are a Rust implementer agent.")
        .with_priority(SectionPriority::Critical)
        .with_cache_layer(CacheLayer::Role)
        .with_placement(Placement::Start)
        .into_signal()
        .unwrap();
    let task = PromptSection::new(
        "task",
        "Implement a function `greet()` that returns \"hi\".",
    )
    .with_priority(SectionPriority::Critical)
    .with_cache_layer(CacheLayer::Plan)
    .with_placement(Placement::End)
    .into_signal()
    .unwrap();
    let hint = PromptSection::new("hint", "Prefer minimal diffs, use &'static str.")
        .with_priority(SectionPriority::Normal)
        .with_cache_layer(CacheLayer::Workspace)
        .with_placement(Placement::Middle)
        .into_signal()
        .unwrap();

    substrate.put(role.clone()).await.unwrap();
    substrate.put(task.clone()).await.unwrap();
    substrate.put(hint.clone()).await.unwrap();

    // 3. Compose a prompt from the sections (via PromptComposer under budget).
    let composer = PromptComposer::new();
    let sections = substrate
        .query(&Query::of_kind(Kind::PromptSection), &Context::now())
        .await
        .unwrap();
    let prompt_signal = composer
        .compose(
            &sections,
            &Budget::tokens(10_000),
            &NoOpScorer,
            &Context::now(),
        )
        .unwrap();
    substrate.put(prompt_signal.clone()).await.unwrap();

    // Verify prompt assembled correctly.
    let prompt_text = prompt_signal.body.as_text().unwrap();
    assert!(prompt_text.contains("Rust implementer"));
    assert!(prompt_text.contains("greet()"));
    // Prompt lineage = all three section signal ids.
    assert_eq!(prompt_signal.lineage.len(), 3);

    // 4. Agent (mocked) processes the prompt.
    // In a real run this would be ClaudeAgent; here we simulate an agent
    // that "already wrote the code" — the test fixture already has a valid
    // `greet()` function in the scaffold.
    let agent = MockAgent::reply("// Implemented greet() as a constant string literal.");
    let agent_result = agent.run(&prompt_signal, &Context::now()).await;
    assert!(agent_result.success);
    substrate.put(agent_result.output.clone()).await.unwrap();
    for trace_sig in &agent_result.trace {
        substrate.put(trace_sig.clone()).await.unwrap();
    }

    // 5. Gate verifies the project actually compiles.
    let compile_gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(120_000);
    let gate_input_sig = Engram::builder(Kind::Task)
        .body(Body::from_json(&GatePayload::in_dir(&project).with_label("e2e-test")).unwrap())
        .lineage([agent_result.output.id]) // chain back to the agent run
        .build();
    substrate.put(gate_input_sig.clone()).await.unwrap();

    let verdict = compile_gate.verify(&gate_input_sig, &Context::now()).await;
    assert!(
        verdict.passed,
        "compile failed: {} ({:?})",
        verdict.reason, verdict.detail
    );

    // Wrap the verdict as a signal and store it.
    let verdict_sig = gate_input_sig
        .derive(Kind::GateVerdict, Body::from_json(&verdict).unwrap())
        .tag("passed", &verdict.passed.to_string())
        .tag("gate", &verdict.gate)
        .build();
    substrate.put(verdict_sig.clone()).await.unwrap();

    // 6. Policy emits an episode signal in response to the verdict.
    let policy = EpisodePolicy;
    let episodes = policy.decide(&[verdict_sig.clone()], &Context::now());
    assert_eq!(episodes.len(), 1);
    let episode = &episodes[0];
    assert_eq!(episode.kind, Kind::Episode);
    assert_eq!(episode.lineage, vec![verdict_sig.id]);
    substrate.put(episode.clone()).await.unwrap();

    // 7. Verify the substrate is populated with the expected signals.
    let all_kinds = [
        (Kind::PromptSection, 3),
        (Kind::Prompt, 1),
        (Kind::AgentOutput, 1),
        (Kind::Task, 1),
        (Kind::GateVerdict, 1),
        (Kind::Episode, 1),
    ];
    for (kind, expected) in all_kinds {
        let count = substrate
            .query(&Query::of_kind(kind.clone()), &Context::now())
            .await
            .unwrap()
            .len();
        assert_eq!(
            count, expected,
            "kind {kind:?} had {count} signals, expected {expected}"
        );
    }

    // 8. Trace the lineage chain: Episode → GateVerdict → Task → AgentOutput → Prompt → PromptSection(x3)
    let chain: Vec<Engram> = trace_lineage(&*substrate, episode.id).await;
    assert!(chain.len() >= 6, "lineage chain too short: {chain:?}");
    // The chain should contain at least one of each traced kind.
    let kinds_in_chain: Vec<_> = chain.iter().map(|s| &s.kind).collect();
    assert!(kinds_in_chain.contains(&&Kind::Episode));
    assert!(kinds_in_chain.contains(&&Kind::GateVerdict));
    assert!(kinds_in_chain.contains(&&Kind::AgentOutput));
    assert!(kinds_in_chain.contains(&&Kind::Prompt));
    assert!(kinds_in_chain.contains(&&Kind::PromptSection));

    // 9. Restart: new FileSubstrate instance, same dir → same state.
    drop(substrate);
    let sub2 = FileSubstrate::open(tmp.path().join(".roko")).await.unwrap();
    let episodes_after_restart = sub2
        .query(&Query::of_kind(Kind::Episode), &Context::now())
        .await
        .unwrap();
    assert_eq!(episodes_after_restart.len(), 1);
}

/// Walk the lineage DAG breadth-first from a starting signal, collecting
/// every ancestor.
async fn trace_lineage(substrate: &dyn Store, start: ContentHash) -> Vec<Engram> {
    let mut visited = std::collections::HashSet::new();
    let mut queue = vec![start];
    let mut out = Vec::new();
    while let Some(id) = queue.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Ok(Some(sig)) = substrate.get(&id).await {
            queue.extend(sig.lineage.iter().copied());
            out.push(sig);
        }
    }
    out
}

// ───────────────────────────────────────────────────────────────────────────
// A second scenario: a FAILURE case where the gate rejects bad code.
// Proves the system correctly propagates failure signals end-to-end.
// ───────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn failure_flows_through_pipeline() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path().join("bad_project");
    fs::create_dir_all(&project).await.unwrap();

    // Scaffold a BROKEN cargo project.
    fs::write(
        project.join("Cargo.toml"),
        r#"[package]
name = "roko_broken"
version = "0.1.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
    )
    .await
    .unwrap();
    fs::create_dir_all(project.join("src")).await.unwrap();
    fs::write(
        project.join("src/lib.rs"),
        "fn broken() { this doesn't parse }",
    )
    .await
    .unwrap();

    let substrate = FileSubstrate::open(tmp.path().join(".roko")).await.unwrap();
    let gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(60_000);
    let input = Engram::builder(Kind::Task)
        .body(Body::from_json(&GatePayload::in_dir(&project)).unwrap())
        .build();
    substrate.put(input.clone()).await.unwrap();

    let verdict = gate.verify(&input, &Context::now()).await;
    assert!(!verdict.passed);

    let verdict_sig = input
        .derive(Kind::GateVerdict, Body::from_json(&verdict).unwrap())
        .tag("passed", "false")
        .build();
    substrate.put(verdict_sig.clone()).await.unwrap();

    // Verify the failure verdict is queryable and its lineage correct.
    let failures = substrate
        .query(
            &Query::of_kind(Kind::GateVerdict).with_tag("passed", "false"),
            &Context::now(),
        )
        .await
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].lineage, vec![input.id]);
}

// ───────────────────────────────────────────────────────────────────────────
// Verify that ExecAgent (real subprocess) also integrates cleanly.
// Uses `cat` so no external LLM is required.
// ───────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn exec_agent_integrates_with_composer() {
    let composer = PromptComposer::new().without_headers();
    let sec = PromptSection::new("x", "echo this back")
        .with_priority(SectionPriority::Critical)
        .into_signal()
        .unwrap();

    let prompt = composer
        .compose(&[sec], &Budget::unlimited(), &NoOpScorer, &Context::at(0))
        .unwrap();

    // ExecAgent with `cat` echoes the prompt back.
    let agent = ExecAgent::new("cat", vec![], SafetyLayer::with_defaults());
    let result = agent.run(&prompt, &Context::now()).await;
    assert!(result.success);
    let echoed = result.output.body.as_text().unwrap();
    assert!(echoed.contains("echo this back"));
    // Output lineage points back to the prompt.
    assert_eq!(result.output.lineage, vec![prompt.id]);
    // Prompt lineage points back to the section.
    assert_eq!(prompt.lineage.len(), 1);
}

// ───────────────────────────────────────────────────────────────────────────
// A minimal Router + feedback demonstration: pick among agent candidates,
// observe outcomes, and confirm feedback is received.
// ───────────────────────────────────────────────────────────────────────────

struct FeedbackCounter {
    count: parking_lot_fork::Mutex<u32>,
}

// Inline a simple parking_lot-like Mutex using std::sync for tests.
mod parking_lot_fork {
    pub struct Mutex<T> {
        inner: std::sync::Mutex<T>,
    }
    impl<T> Mutex<T> {
        pub const fn new(v: T) -> Self {
            Self {
                inner: std::sync::Mutex::new(v),
            }
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.inner.lock().unwrap()
        }
    }
}

impl roko_core::Route for FeedbackCounter {
    fn select(&self, candidates: &[Engram], _ctx: &Context) -> Option<roko_core::Selection> {
        candidates
            .first()
            .map(|s| roko_core::Selection::new(s.id, "feedback_counter"))
    }

    fn feedback(&self, _outcome: &roko_core::Outcome) {
        *self.count.lock() += 1;
    }

    fn name(&self) -> &str {
        "feedback_counter"
    }
}

#[tokio::test]
async fn router_receives_feedback() {
    let router = FeedbackCounter {
        count: parking_lot_fork::Mutex::new(0),
    };
    let candidates = [
        Engram::builder(Kind::Task).body(Body::text("a")).build(),
        Engram::builder(Kind::Task).body(Body::text("b")).build(),
    ];
    let sel = roko_core::Route::select(&router, &candidates, &Context::at(0)).unwrap();

    // Simulate an outcome.
    let outcome = roko_core::Outcome::success(sel.clone()).with_reward(0.9);
    roko_core::Route::feedback(&router, &outcome);
    roko_core::Route::feedback(&router, &outcome);

    assert_eq!(*router.count.lock(), 2);
}

// Ensures the ephemeral Policy trait (episode_policy) lives in
// roko-core and can be implemented by downstream crates.
#[tokio::test]
async fn episode_policy_emits_signals_for_verdicts() {
    let policy = EpisodePolicy;
    let verdict_sig = Engram::builder(Kind::GateVerdict)
        .body(Body::from_json(&Verdict::pass("x")).unwrap())
        .tag("passed", "true")
        .tag("gate", "x")
        .build();

    let out = policy.decide(&[verdict_sig.clone()], &Context::at(12345));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].kind, Kind::Episode);
    assert_eq!(out[0].lineage, vec![verdict_sig.id]);

    // Non-verdict signals are ignored.
    let task = Engram::builder(Kind::Task).body(Body::text("x")).build();
    assert!(policy.decide(&[task], &Context::at(0)).is_empty());
}
