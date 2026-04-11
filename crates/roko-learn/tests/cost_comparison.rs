use std::collections::HashMap;

use roko_agent::Usage;
use roko_learn::cost_table::{CostTable, ModelPricing};
use roko_learn::efficiency::{AgentEfficiencyEvent, compute_role_profiles};

const KIMI: &str = "kimi-k2.5";
const GLM: &str = "glm-5.1";
const CLAUDE: &str = "claude-opus-4-6";

fn pricing_table() -> CostTable {
    CostTable {
        models: HashMap::from([
            (
                KIMI.to_string(),
                ModelPricing {
                    input_per_m: 0.60,
                    output_per_m: 3.00,
                    cache_read_per_m: 0.10,
                    cache_write_per_m: 0.75,
                    tokenizer_ratio: 0.98,
                },
            ),
            (
                GLM.to_string(),
                ModelPricing {
                    input_per_m: 1.40,
                    output_per_m: 4.40,
                    cache_read_per_m: 0.26,
                    cache_write_per_m: 1.75,
                    tokenizer_ratio: 1.05,
                },
            ),
            (
                CLAUDE.to_string(),
                ModelPricing {
                    input_per_m: 15.00,
                    output_per_m: 75.00,
                    cache_read_per_m: 3.75,
                    cache_write_per_m: 18.75,
                    tokenizer_ratio: 1.0,
                },
            ),
        ]),
    }
}

fn usage() -> Usage {
    Usage {
        input_tokens: 1_000,
        output_tokens: 250,
        cache_read_tokens: 0,
        cache_create_tokens: 0,
        ..Usage::default()
    }
}

fn turn_event(model: &str, turn: usize, cost_usd: f64, gate_passed: bool) -> AgentEfficiencyEvent {
    AgentEfficiencyEvent {
        agent_id: format!("{model}-{turn}"),
        role: model.to_string(),
        backend: String::from("routing-test"),
        model: model.to_string(),
        plan_id: String::from("plan-cost-comparison"),
        task_id: format!("turn-{turn}"),
        input_tokens: usage().input_tokens as u64,
        output_tokens: usage().output_tokens as u64,
        cost_usd,
        cost_usd_without_cache: cost_usd,
        tools_available: 4,
        tools_used: 2,
        wall_time_ms: 900,
        duration_ms: 900,
        time_to_first_token_ms: 80,
        was_warm_start: turn > 0,
        iteration: (turn + 1) as u32,
        gate_passed,
        outcome: if gate_passed {
            String::from("success")
        } else {
            String::from("retry")
        },
        model_used: model.to_string(),
        timestamp: String::from("2026-04-11T00:00:00Z"),
        ..AgentEfficiencyEvent::default()
    }
}

fn profile_cost_per_successful_task(
    profiles: &[roko_learn::efficiency::RoleCostProfile],
    role: &str,
) -> f64 {
    profiles
        .iter()
        .find(|profile| profile.role == role)
        .unwrap_or_else(|| panic!("missing profile for role {role}"))
        .cost_per_successful_task()
}

#[test]
fn cost_comparison_e2e() {
    let table = pricing_table();
    let usage = usage();

    let mut events = Vec::with_capacity(100);
    let mut per_model_turns: HashMap<&str, usize> =
        HashMap::from([(KIMI, 0), (GLM, 0), (CLAUDE, 0)]);

    for turn in 0..100 {
        let model = match turn % 3 {
            0 => KIMI,
            1 => GLM,
            _ => CLAUDE,
        };
        let model_turn = per_model_turns
            .get_mut(model)
            .unwrap_or_else(|| panic!("missing turn counter for model {model}"));
        let gate_passed = *model_turn % 2 == 0;
        let cost_usd = table.calculate(model, &usage);

        events.push(turn_event(model, *model_turn, cost_usd, gate_passed));
        *model_turn += 1;
    }

    let profiles = compute_role_profiles(&events);

    let kimi_cost = profile_cost_per_successful_task(&profiles, KIMI);
    let glm_cost = profile_cost_per_successful_task(&profiles, GLM);
    let claude_cost = profile_cost_per_successful_task(&profiles, CLAUDE);

    assert!(
        kimi_cost < glm_cost,
        "expected Kimi to be cheaper than GLM: {kimi_cost} vs {glm_cost}"
    );
    assert!(
        glm_cost < claude_cost,
        "expected GLM to be cheaper than Claude: {glm_cost} vs {claude_cost}"
    );

    let kimi_blended = table.blended_cost_per_m(KIMI);
    let glm_blended = table.blended_cost_per_m(GLM);
    let claude_blended = table.blended_cost_per_m(CLAUDE);

    assert!(
        kimi_blended < glm_blended,
        "expected Kimi blended cost to be lower than GLM: {kimi_blended} vs {glm_blended}"
    );
    assert!(
        glm_blended < claude_blended,
        "expected GLM blended cost to be lower than Claude: {glm_blended} vs {claude_blended}"
    );
}
