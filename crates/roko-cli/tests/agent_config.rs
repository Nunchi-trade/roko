use roko_core::config::{RokoConfig, RoleOverride};

#[test]
fn parse_all_keys_happy_path() {
    let cfg = RokoConfig::from_toml(
        r#"
[agent]
default_model = "claude-sonnet-4-6"

[agent.roles.implementer]
role = "code_implementer"
model = "claude-opus-4-6"
backend = "claude"
effort = "high"
context_limit_k = 300
tools = ["read_file", "edit_file", "git-*"]
budget = { max_tokens_per_turn = 12000, max_cost_usd_cents_per_turn = 550 }
thresholds = { gate_pass_rate_floor = 0.72 }
routing_overrides = { force_backend = "claude", force_tier = "focused" }
turn_budget_usd = 5.5
"#,
    )
    .expect("parse roko.toml");

    let role = cfg
        .agent
        .roles
        .get("implementer")
        .expect("implementer role");
    assert_eq!(
        role,
        &RoleOverride {
            role: Some("code_implementer".to_string()),
            model: Some("claude-opus-4-6".to_string()),
            backend: Some("claude".to_string()),
            effort: Some("high".to_string()),
            context_limit_k: Some(300),
            tools: Some(vec![
                "read_file".to_string(),
                "edit_file".to_string(),
                "git-*".to_string(),
            ]),
            budget: Some(roko_core::config::AgentBudget {
                max_tokens_per_turn: Some(12_000),
                max_cost_usd_cents_per_turn: Some(550),
            }),
            thresholds: Some(roko_core::config::AgentThresholds {
                gate_pass_rate_floor: Some(0.72),
            }),
            routing_overrides: Some(roko_core::config::RoutingOverrides {
                force_backend: Some("claude".to_string()),
                force_tier: Some("focused".to_string()),
            }),
            turn_budget_usd: Some(5.5),
            temperament: None,
        }
    );
}

#[test]
fn parse_tolerates_missing_optional_keys() {
    let cfg = RokoConfig::from_toml(
        r#"
[agent]
default_model = "claude-sonnet-4-6"

[agent.roles.scribe]
model = "gpt-5.4-mini"
"#,
    )
    .expect("parse roko.toml");

    let role = cfg.agent.roles.get("scribe").expect("scribe role");
    assert_eq!(role.model.as_deref(), Some("gpt-5.4-mini"));
    assert!(role.role.is_none());
    assert!(role.backend.is_none());
    assert!(role.effort.is_none());
    assert!(role.context_limit_k.is_none());
    assert!(role.tools.is_none());
    assert!(role.budget.is_none());
    assert!(role.thresholds.is_none());
    assert!(role.routing_overrides.is_none());
    assert!(role.turn_budget_usd.is_none());
    assert!(role.temperament.is_none());
}
