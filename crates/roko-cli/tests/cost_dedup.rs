//! Regression test: verify one cost event per dispatch attempt.
//!
//! Tests the invariant that:
//! - A successful dispatch produces exactly one efficiency event with real `cost_usd`
//! - A gate failure produces one failure event with `cost_usd = 0.0`
//! - Both events share the same `attempt_id`
//!
//! No real LLM calls. The events are constructed directly.

use roko_core::OperatingFrequency;
use roko_learn::efficiency::AgentEfficiencyEvent;

fn base_event(plan_id: &str, task_id: &str, attempt_id: &str) -> AgentEfficiencyEvent {
    let mut event = AgentEfficiencyEvent::default();
    event.agent_id = format!("{plan_id}:{task_id}:agent");
    event.role = "Implementer".to_string();
    event.backend = "claude".to_string();
    event.model = "claude-sonnet-4-6".to_string();
    event.plan_id = plan_id.to_string();
    event.task_id = task_id.to_string();
    event.attempt_id = attempt_id.to_string();
    event.iteration = 1;
    event.model_used = "claude-sonnet-4-6".to_string();
    event.frequency = OperatingFrequency::Theta;
    event
}

/// Build a minimal `AgentEfficiencyEvent` as if emitted by the success path.
fn make_success_event(
    plan_id: &str,
    task_id: &str,
    cost_usd: f64,
    attempt_id: &str,
) -> AgentEfficiencyEvent {
    let mut event = base_event(plan_id, task_id, attempt_id);
    event.input_tokens = 1000;
    event.output_tokens = 200;
    event.reasoning_tokens = 0;
    event.cache_read_tokens = 0;
    event.cache_write_tokens = 0;
    event.cost_usd = cost_usd;
    event.cost_usd_without_cache = cost_usd;
    event.prompt_sections = Vec::new();
    event.total_prompt_tokens = 1000;
    event.system_prompt_tokens = 0;
    event.tools_available = 0;
    event.tools_used = 0;
    event.tool_calls = Vec::new();
    event.wall_time_ms = 5000;
    event.duration_ms = 5000;
    event.time_to_first_token_ms = 0;
    event.was_warm_start = false;
    event.gate_passed = true;
    event.outcome = "success".to_string();
    event.gate_errors = Vec::new();
    event.strategy_attempted = "none".to_string();
    event.timestamp = "2026-04-29T00:00:00Z".to_string();
    event
}

/// Build a minimal `AgentEfficiencyEvent` as if emitted by the failure path.
fn make_failure_event(
    plan_id: &str,
    task_id: &str,
    attempt_id: &str,
    gate_errors: Vec<String>,
) -> AgentEfficiencyEvent {
    let mut event = base_event(plan_id, task_id, attempt_id);
    event.cost_usd = 0.0;
    event.cost_usd_without_cache = 0.0;
    event.input_tokens = 0;
    event.output_tokens = 0;
    event.reasoning_tokens = 0;
    event.cache_read_tokens = 0;
    event.cache_write_tokens = 0;
    event.prompt_sections = Vec::new();
    event.total_prompt_tokens = 0;
    event.system_prompt_tokens = 0;
    event.tools_available = 0;
    event.tools_used = 0;
    event.tool_calls = Vec::new();
    event.wall_time_ms = 0;
    event.duration_ms = 0;
    event.time_to_first_token_ms = 0;
    event.was_warm_start = false;
    event.gate_passed = false;
    event.outcome = "failure".to_string();
    event.gate_errors = gate_errors;
    event.strategy_attempted = "retry_same".to_string();
    event.timestamp = "2026-04-29T00:00:01Z".to_string();
    event
}

/// Returns all events where `cost_usd > 0.0`.
fn cost_events(events: &[AgentEfficiencyEvent]) -> Vec<&AgentEfficiencyEvent> {
    events.iter().filter(|event| event.cost_usd > 0.0).collect()
}

/// Returns all failure events.
fn failure_events(events: &[AgentEfficiencyEvent]) -> Vec<&AgentEfficiencyEvent> {
    events.iter().filter(|event| !event.gate_passed).collect()
}

#[test]
fn one_cost_event_per_successful_dispatch() {
    let events = vec![make_success_event("plan-a", "T1", 0.05, "plan-a:T1:hash123")];

    let cost_evts = cost_events(&events);
    assert_eq!(
        cost_evts.len(),
        1,
        "expected exactly 1 cost event for 1 dispatch, got {}",
        cost_evts.len()
    );
    assert!(
        (cost_evts[0].cost_usd - 0.05).abs() < 1e-9,
        "cost should be 0.05, got {}",
        cost_evts[0].cost_usd
    );
}

#[test]
fn gate_failure_produces_zero_cost_event_not_additional_cost() {
    let attempt_id = "plan-a:T1:hash456";
    let events = vec![
        make_success_event("plan-a", "T1", 0.05, attempt_id),
        make_failure_event(
            "plan-a",
            "T1",
            attempt_id,
            vec!["compile: error[E0308]".to_string()],
        ),
    ];

    let cost_evts = cost_events(&events);
    let fail_evts = failure_events(&events);

    assert_eq!(
        cost_evts.len(),
        1,
        "gate failure should not add another cost event, got {} cost events",
        cost_evts.len()
    );
    assert_eq!(
        fail_evts.len(),
        1,
        "expected 1 failure event, got {}",
        fail_evts.len()
    );
    assert_eq!(
        fail_evts[0].cost_usd, 0.0,
        "failure event should have 0.0 cost, got {}",
        fail_evts[0].cost_usd
    );
    assert!(
        !fail_evts[0].gate_errors.is_empty(),
        "failure event should have gate errors populated"
    );
}

#[test]
fn attempt_id_links_cost_and_failure_events() {
    let attempt_id = "plan-a:T1:hash789";
    let events = vec![
        make_success_event("plan-a", "T1", 0.07, attempt_id),
        make_failure_event(
            "plan-a",
            "T1",
            attempt_id,
            vec!["test: 3 failures".to_string()],
        ),
    ];

    assert_eq!(
        events[0].attempt_id, events[1].attempt_id,
        "cost event and failure event must share attempt_id"
    );
}

#[test]
fn two_retries_produce_two_cost_events() {
    let attempt_1 = "plan-a:T1:hash001";
    let attempt_2 = "plan-a:T1:hash002";
    let events = vec![
        make_success_event("plan-a", "T1", 0.04, attempt_1),
        make_failure_event("plan-a", "T1", attempt_1, vec!["compile failed".to_string()]),
        make_success_event("plan-a", "T1", 0.05, attempt_2),
    ];

    let cost_evts = cost_events(&events);

    assert_eq!(
        cost_evts.len(),
        2,
        "two dispatch attempts should produce 2 cost events, got {}",
        cost_evts.len()
    );

    let total: f64 = cost_evts.iter().map(|event| event.cost_usd).sum();
    assert!(
        (total - 0.09).abs() < 1e-9,
        "total cost should be 0.09, got {total}"
    );

    assert_ne!(
        cost_evts[0].attempt_id, cost_evts[1].attempt_id,
        "different dispatches must have different attempt_ids"
    );
}

#[test]
fn total_cost_from_cost_events_only_not_doubled() {
    let attempt_id = "plan-a:T2:hash999";
    let events = vec![
        make_success_event("plan-a", "T2", 0.10, attempt_id),
        make_failure_event(
            "plan-a",
            "T2",
            attempt_id,
            vec!["clippy: 5 warnings".to_string()],
        ),
    ];

    let naive_sum: f64 = events.iter().map(|event| event.cost_usd).sum();
    let cost_only_sum: f64 = cost_events(&events).iter().map(|event| event.cost_usd).sum();

    assert!(
        (naive_sum - 0.10).abs() < 1e-9,
        "naive sum should be 0.10 (failure event must have 0 cost)"
    );
    assert!(
        (cost_only_sum - 0.10).abs() < 1e-9,
        "cost-only sum should be 0.10"
    );
    assert_eq!(
        naive_sum, cost_only_sum,
        "naive sum and cost-only sum must match (failure events must have 0 cost)"
    );
}
