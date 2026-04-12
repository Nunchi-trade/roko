//! Regression test for cache-stable prompt prefixes across same-plan tasks.

use roko_compose::templates::{PlanSlice, TaskImplInput, TaskImplTemplate};
use roko_compose::{ContextStrategy, PromptAssembler};

fn repeated_block(heading: &str, line: &str, count: usize) -> String {
    let mut out = String::from(heading);
    out.push('\n');
    for _ in 0..count {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn build_prompt(
    task_id: &str,
    task_title: &str,
    task_files: &[&str],
    acceptance_criteria: &[&str],
) -> String {
    let template = TaskImplTemplate;
    let input = TaskImplInput {
        agents_md: repeated_block(
            "# AGENTS.md",
            "- Prefer wiring existing modules over reimplementation.",
            10,
        ),
        plan: PlanSlice {
            num: "013".into(),
            base: "model-routing".into(),
            title: "Architectural gaps".into(),
            content: repeated_block(
                "## Plan Context",
                "- Keep role and workspace layers byte-stable across tasks in the same plan.",
                24,
            ),
        },
        task_id: task_id.into(),
        task_title: task_title.into(),
        task_files: task_files.iter().map(|path| (*path).to_string()).collect(),
        acceptance_criteria: acceptance_criteria
            .iter()
            .map(|criterion| (*criterion).to_string())
            .collect(),
        brief: repeated_block(
            "## Brief",
            "- Preserve prompt-cache reuse by keeping shared sections identical.",
            16,
        ),
        workspace_map: repeated_block("## Workspace Map", "- crates/roko-compose/src/*", 22),
        prd2_extract: repeated_block(
            "## PRD2 Extract",
            "- Providers with automatic prefix caching depend on byte-identical shared prefixes.",
            14,
        ),
        cross_plan_context: repeated_block(
            "## Cross-Plan Context",
            "- Task sequencing should not perturb stable prompt layers.",
            12,
        ),
        ..TaskImplInput::default()
    };

    PromptAssembler::new()
        .assemble_from(&template, &input, None, ContextStrategy::Full)
        .prompt
}

#[test]
fn cache_prefix_stable_across_tasks() {
    let prompt_1 = build_prompt(
        "2K.13",
        "Insert Anthropic cache markers",
        &["crates/roko-agent/src/translate/claude.rs"],
        &["Anthropic requests include cache_control boundaries."],
    );
    let prompt_2 = build_prompt(
        "2K.14",
        "Normalize prompt content",
        &["crates/roko-compose/src/system_prompt_builder.rs"],
        &["Equivalent prompts normalize to identical bytes."],
    );
    let prompt_3 = build_prompt(
        "2K.15",
        "Verify cache prefix stability",
        &["crates/roko-compose/tests/cache_stability.rs"],
        &["Shared prompt prefix remains identical across plan tasks."],
    );

    let role_len_1 = prompt_1.find("## Plan Context").unwrap();
    let role_len_2 = prompt_2.find("## Plan Context").unwrap();
    let role_len_3 = prompt_3.find("## Plan Context").unwrap();
    assert_eq!(role_len_1, role_len_2);
    assert_eq!(role_len_1, role_len_3);
    assert_eq!(&prompt_1[..role_len_1], &prompt_2[..role_len_1]);
    assert_eq!(&prompt_1[..role_len_1], &prompt_3[..role_len_1]);

    let ws_len_1 = prompt_1.find("## Your Assignment").unwrap();
    let ws_len_2 = prompt_2.find("## Your Assignment").unwrap();
    let ws_len_3 = prompt_3.find("## Your Assignment").unwrap();
    assert_eq!(ws_len_1, ws_len_2);
    assert_eq!(ws_len_1, ws_len_3);
    assert_eq!(&prompt_1[..ws_len_1], &prompt_2[..ws_len_1]);
    assert_eq!(&prompt_1[..ws_len_1], &prompt_3[..ws_len_1]);

    assert!(ws_len_1 * 100 / prompt_1.len() >= 60);
    assert!(ws_len_2 * 100 / prompt_2.len() >= 60);
    assert!(ws_len_3 * 100 / prompt_3.len() >= 60);
}
