//! Snapshot coverage for canonical role system prompts.

use std::collections::BTreeSet;

use roko_compose::system_prompt_builder::normalize_for_caching;
use roko_compose::{
    ContextChunk, ContextSource, PadState, PromptSection, RoleSystemPromptSpec, TaskContext,
};
use roko_core::AgentRole;
use roko_learn::playbook::{Playbook, PlaybookStep};
use roko_learn::skill_library::Skill;

fn canonical_roles() -> Vec<AgentRole> {
    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS.iter().copied())
        .collect()
}

fn fixture_spec(role: AgentRole) -> RoleSystemPromptSpec {
    let task_context =
        TaskContext::new("Capture a deterministic golden prompt for this role.")
            .with_plan_id("UX37")
            .with_goal("Keep system prompt composition deterministic and auditable.")
            .with_workspace("crates/roko-compose")
            .with_context(
                "### Workspace Notes\n- Snapshot output groups live prompt sections into six audit buckets.\n- Keep fixtures stable across runs.",
            )
            .with_domain_notes(
                "Use the real RoleSystemPromptSpec path with fixed skills, playbooks, signals, and policy guidance.",
            );

    RoleSystemPromptSpec::new(role, task_context, "Read,Edit,Bash")
        .with_extra_conventions("Prefer deterministic fixtures and readable snapshot diffs.")
        .add_anti_pattern("Do not depend on wall-clock time or RNG in snapshot fixtures.")
        .with_relevant_skills(&[fixture_skill()])
        .with_relevant_playbooks(&[fixture_playbook()])
        .with_pheromones(&[fixture_pheromone()])
        .with_affect_state(Some(
            PadState::new(-0.4, 0.7, 0.5).with_somatic_hint(-0.6, 0.8),
        ))
}

fn fixture_skill() -> Skill {
    let mut skill = Skill::new(
        "snapshot_guardrails",
        "Keep snapshot fixtures deterministic and reviewable.",
        "Render prompts from fixed inputs and prefer readable deltas.",
    );
    skill.precondition = "Apply for snapshot tests and prompt-audit tasks.".to_string();
    skill.procedure =
        "Populate every prompt layer with fixed literals, then diff the rendered output."
            .to_string();
    skill.success_rate = 0.95;
    skill.created_at = "2026-04-17T00:00:00Z".to_string();
    skill
}

fn fixture_playbook() -> Playbook {
    let mut playbook = Playbook::new("pb-snapshot-prompts", "Audit system prompt rendering");
    playbook.name = "snapshot-prompts".to_string();
    playbook.created_at_ms = 1_776_384_000_000;
    playbook.success_count = 9;
    playbook.failure_count = 1;
    playbook.steps.push(PlaybookStep::new(
        0,
        "Build the prompt from a fully-populated deterministic spec.",
        "compose_prompt",
        vec!["prompt_rendered".to_string()],
    ));
    playbook.steps.push(PlaybookStep::new(
        1,
        "Review the diff and accept the updated snapshot.",
        "review_snapshot",
        vec!["snapshot_accepted".to_string()],
    ));
    playbook
}

fn fixture_pheromone() -> ContextChunk {
    ContextChunk {
        content: "- [Threat] Prompt layer drift can hide regressions between roles.".to_string(),
        source: ContextSource::RecentSignal {
            signal_id: "sig-ux37".to_string(),
            plan_id: "UX37".to_string(),
            kind: "pheromone".to_string(),
        },
        relevance: 0.97,
        track_record: Some(0.91),
        confidence: Some(0.88),
        recency: Some(0.94),
        emotional_tag: None,
    }
}

fn assert_prompt_contains_all_section_content(prompt: &str, sections: &[PromptSection]) {
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized_prompt = normalize(prompt);
    for section in sections {
        let normalized_section = normalize(&section.content);
        assert!(
            normalized_prompt.contains(&normalized_section),
            "section '{}' content missing from prompt build",
            section.name
        );
    }
}

fn snapshot_bucket(section_name: &str) -> &'static str {
    match section_name {
        "role_identity" => "<!-- ROLE -->",
        "relevant_techniques" => "<!-- SKILLS -->",
        "tool_instructions" => "<!-- TOOLS -->",
        "domain_context" | "context_layer" | "task_context" => "<!-- CONTEXT -->",
        "pheromone_signals" => "<!-- MEMORY -->",
        "conventions" | "anti_patterns" | "affect_guidance" => "<!-- POLICY -->",
        other => panic!("unexpected section name in snapshot renderer: {other}"),
    }
}

fn render_section(section: &PromptSection) -> String {
    match section.name.as_str() {
        "role_identity" => section.content.clone(),
        "conventions" => format!("## Project Conventions\n\n{}", section.content),
        "tool_instructions" => format!("## Tool Instructions\n\n{}", section.content),
        "domain_context" => format!("## Domain Context\n\n{}", section.content),
        "context_layer" => section.content.clone(),
        "pheromone_signals" => format!("## Active Signals\n\n{}", section.content),
        "task_context" => format!("## Current Task\n\n{}", section.content),
        "relevant_techniques" => section.content.clone(),
        "anti_patterns" => format!("## Anti-Patterns\n\n{}", section.content),
        "affect_guidance" => format!("## Affect Guidance\n\n{}", section.content),
        other => panic!("unexpected section name in snapshot renderer: {other}"),
    }
}

fn render_snapshot(role: AgentRole, prompt: &str, sections: &[PromptSection]) -> String {
    let mut parts = vec![
        format!("role = {}", role.label()),
        String::new(),
        String::from("<!-- LIVE PROMPT -->"),
        prompt.to_string(),
        String::new(),
        String::from("<!-- LAYERED VIEW -->"),
    ];

    for section in sections {
        parts.push(String::new());
        parts.push(snapshot_bucket(&section.name).to_string());
        parts.push(render_section(section));
    }

    normalize_for_caching(&parts.join("\n"))
}

fn assert_expected_snapshot_layers_present(snapshot: &str, sections: &[PromptSection]) {
    assert!(
        snapshot.contains("<!-- LIVE PROMPT -->"),
        "snapshot output missing live prompt header"
    );
    assert!(
        snapshot.contains("<!-- LAYERED VIEW -->"),
        "snapshot output missing layered view header"
    );

    let expected_headers = sections
        .iter()
        .map(|section| snapshot_bucket(&section.name))
        .collect::<BTreeSet<_>>();

    for header in expected_headers {
        assert!(
            snapshot.contains(header),
            "snapshot output missing emitted layer header {header}"
        );
    }
}

#[test]
fn canonical_system_prompts_match_snapshots() {
    for role in canonical_roles() {
        let spec = fixture_spec(role);
        let prompt = spec.build();
        let sections = spec
            .build_sections()
            .into_iter()
            .map(PromptSection::enforce_hard_cap)
            .collect::<Vec<_>>();

        assert_prompt_contains_all_section_content(&prompt, &sections);

        let snapshot = render_snapshot(role, &prompt, &sections);
        assert_expected_snapshot_layers_present(&snapshot, &sections);

        insta::assert_snapshot!(format!("role__{}", role.label()), snapshot);
    }
}
