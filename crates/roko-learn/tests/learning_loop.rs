//! Integration tests for the learning loop wiring (checklist §I.3).
//!
//! These tests verify that the three subsystems added in this batch —
//! episode logging, bandit persistence, and provider health — work
//! end-to-end across crate boundaries.

use std::time::Duration;
use tempfile::TempDir;

use roko_core::agent::AgentRole;
use roko_core::task::{TaskCategory, TaskComplexityBand};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::model_router::{COLD_START_THRESHOLD, LinUCBRouter, RoutingContext};
use roko_learn::provider_health::ProviderHealthTracker;

// ─── Helpers ───────────────────────────────────────────────────────────────

fn default_ctx() -> RoutingContext {
    RoutingContext {
        task_category: TaskCategory::Implementation,
        complexity: TaskComplexityBand::Standard,
        iteration: 0,
        role: AgentRole::Implementer,
        crate_familiarity: 0.5,
        has_prior_failure: false,
    }
}

fn test_slugs() -> Vec<String> {
    vec![
        "claude-haiku-3-5".to_string(),
        "claude-sonnet-4-5".to_string(),
        "claude-opus-4".to_string(),
    ]
}

// ─── Test 1: Episode logging — append and read back ────────────────────────

#[tokio::test]
async fn episode_logger_roundtrip_in_temp_dir() {
    let tmp = TempDir::new().unwrap();
    let ep_path = tmp.path().join("episodes.jsonl");
    let logger = EpisodeLogger::new(&ep_path);

    let mut ep = Episode::new("test-agent", "task-42");
    ep.success = true;
    logger.append(&ep).await.unwrap();

    let read_back = EpisodeLogger::read_all(&ep_path).await.unwrap();
    assert_eq!(read_back.len(), 1);
    assert_eq!(read_back[0].agent_id, "test-agent");
    assert_eq!(read_back[0].task_id, "task-42");
    assert!(read_back[0].success);
}

// ─── Test 2: Episode logging — multiple appends ────────────────────────────

#[tokio::test]
async fn episode_logger_multiple_appends() {
    let tmp = TempDir::new().unwrap();
    let ep_path = tmp.path().join("episodes.jsonl");
    let logger = EpisodeLogger::new(&ep_path);

    for i in 0..5 {
        let ep = Episode::new("agent", format!("task-{i}"));
        logger.append(&ep).await.unwrap();
    }

    let read_back = EpisodeLogger::read_all(&ep_path).await.unwrap();
    assert_eq!(read_back.len(), 5);
}

// ─── Test 3: Bandit persistence — update, drop, reload ────────────────────

#[test]
fn bandit_persistence_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("router.json");

    // Create, train 3 times, and save.
    let router = LinUCBRouter::new(test_slugs()).with_persist_path(&path);
    let ctx = default_ctx();
    router.update(&ctx, "claude-sonnet-4-5", 0.9, true);
    router.update(&ctx, "claude-haiku-3-5", 0.3, true);
    router.update(&ctx, "claude-opus-4", 0.7, true);
    router.save().unwrap();

    // Drop the router — state is only on disk now.
    drop(router);

    // Reload from the same path.
    let reloaded = LinUCBRouter::load(&path, test_slugs()).unwrap();
    assert_eq!(reloaded.total_observations(), 3);

    let stats = reloaded.arm_stats();
    let sonnet = stats
        .iter()
        .find(|a| a.slug == "claude-sonnet-4-5")
        .unwrap();
    assert_eq!(sonnet.observations, 1);
    let haiku = stats.iter().find(|a| a.slug == "claude-haiku-3-5").unwrap();
    assert_eq!(haiku.observations, 1);
    let opus = stats.iter().find(|a| a.slug == "claude-opus-4").unwrap();
    assert_eq!(opus.observations, 1);
}

// ─── Test 4: Auto-persist on update ────────────────────────────────────────

#[test]
fn auto_persist_on_update() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("auto.json");

    // Router with persist_path — update() should auto-save.
    let router = LinUCBRouter::new(test_slugs()).with_persist_path(&path);
    let ctx = default_ctx();
    router.update(&ctx, "claude-sonnet-4-5", 0.8, true);
    router.update(&ctx, "claude-sonnet-4-5", 0.6, true);

    // Verify the file was created by the auto-persist.
    assert!(path.exists(), "router file should exist after update()");

    // Reload and verify.
    let reloaded = LinUCBRouter::load(&path, test_slugs()).unwrap();
    assert_eq!(reloaded.total_observations(), 2);
}

// ─── Test 5: Provider health — 3 failures makes unhealthy ─────────────────

#[test]
fn provider_health_three_failures_trips_breaker() {
    let tracker = ProviderHealthTracker::new(); // default: 3 failures, 120s recovery
    tracker.record_failure("ol");
    tracker.record_failure("ol");
    assert!(tracker.is_healthy("ol"), "still healthy after 2 failures");

    tracker.record_failure("ol");
    assert!(!tracker.is_healthy("ol"), "unhealthy after 3 failures");
}

// ─── Test 6: Provider health — success resets counter ──────────────────────

#[test]
fn provider_health_success_resets() {
    let tracker = ProviderHealthTracker::new();
    tracker.record_failure("ol");
    tracker.record_failure("ol");
    tracker.record_success("ol");
    tracker.record_failure("ol");
    tracker.record_failure("ol");
    assert!(tracker.is_healthy("ol"), "counter was reset by success");
}

// ─── Test 7: Provider health — unhealthy provider excluded from select ─────

#[test]
fn unhealthy_provider_excluded_from_select() {
    // Use a short recovery window so the test doesn't hang.
    let tracker = ProviderHealthTracker::with_config(1, Duration::from_secs(600));
    // Trip "ol" (Ollama) immediately.
    tracker.record_failure("ol");

    let router = LinUCBRouter::new(vec![
        "claude-sonnet-4-5".to_string(),
        "ollama/llama3".to_string(),
    ])
    .with_health_tracker(tracker);

    let ctx = default_ctx();

    // Train heavily past cold start to exercise LinUCB selection.
    // Only update the healthy arm; the ollama arm stays at zero observations
    // but we give it a high reward the few times we do update it (while
    // keeping gate_passed=false so health stays tripped).
    for _ in 0..COLD_START_THRESHOLD {
        router.update(&ctx, "claude-sonnet-4-5", 0.5, true);
        // Record an update for ollama but with gate_passed=false so the
        // health tracker stays tripped.
        router.update(&ctx, "ollama/llama3", 0.9, false);
    }

    // Since "ol" is unhealthy, the router should not select ollama.
    let model = router.select_model(&ctx);
    assert_eq!(
        model.slug, "claude-sonnet-4-5",
        "should skip unhealthy ollama provider"
    );
}

// ─── Test 8: Health tracking integrated into update() ──────────────────────

#[test]
fn update_records_health() {
    let router = LinUCBRouter::new(test_slugs());
    let ctx = default_ctx();

    // Record 3 failures for claude (all slugs map to "cl" provider).
    router.update(&ctx, "claude-haiku-3-5", 0.0, false);
    router.update(&ctx, "claude-haiku-3-5", 0.0, false);
    router.update(&ctx, "claude-haiku-3-5", 0.0, false);

    // The provider "cl" should now be unhealthy via the embedded tracker.
    let snap = router.health_tracker().snapshot();
    let cl = snap.iter().find(|s| s.provider == "cl").unwrap();
    assert_eq!(cl.consecutive_failures, 3);
}

// ─── Test 9: filter_arms falls back when all unhealthy ─────────────────────

#[test]
fn all_unhealthy_fallback_still_selects() {
    // Trip every provider immediately.
    let tracker = ProviderHealthTracker::with_config(1, Duration::from_secs(600));
    tracker.record_failure("cl");

    let router =
        LinUCBRouter::new(vec!["claude-sonnet-4-5".to_string()]).with_health_tracker(tracker);

    let ctx = default_ctx();

    // Train past cold start.
    for _ in 0..COLD_START_THRESHOLD {
        router.update(&ctx, "claude-sonnet-4-5", 0.8, true);
    }

    // Even though provider is unhealthy, select_model should return
    // *something* rather than panicking.
    let model = router.select_model(&ctx);
    assert!(
        !model.slug.is_empty(),
        "should return a model even when all unhealthy"
    );
}
