//! Integration tests for the PRD pipeline: idea capture, draft scaffold, and
//! markdown materialization.
//!
//! These tests exercise the public `roko_cli::prd` helpers directly without
//! invoking a real LLM agent.

use std::fs;

#[test]
fn prd_idea_creates_ideas_file() {
    let tmp = tempfile::tempdir().unwrap();
    // Initialize .roko/ structure
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    // Capture idea
    roko_cli::prd::cmd_idea(tmp.path(), "test integration idea").unwrap();

    // Verify file exists and contains the idea
    let ideas = tmp.path().join(".roko/prd/ideas.md");
    assert!(ideas.exists(), "ideas.md should be created");
    let content = fs::read_to_string(&ideas).unwrap();
    assert!(
        content.contains("test integration idea"),
        "ideas.md should contain the captured idea, got:\n{content}"
    );
}

#[test]
fn prd_idea_appends_multiple_ideas() {
    let tmp = tempfile::tempdir().unwrap();
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    roko_cli::prd::cmd_idea(tmp.path(), "first idea").unwrap();
    roko_cli::prd::cmd_idea(tmp.path(), "second idea").unwrap();

    let ideas = tmp.path().join(".roko/prd/ideas.md");
    let content = fs::read_to_string(&ideas).unwrap();
    assert!(content.contains("first idea"));
    assert!(content.contains("second idea"));

    // Both ideas should be on separate lines starting with "- "
    let idea_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("- ")).collect();
    assert!(
        idea_lines.len() >= 2,
        "expected at least 2 idea lines, got {}",
        idea_lines.len()
    );
}

#[test]
fn prd_draft_scaffold_creates_markdown() {
    let tmp = tempfile::tempdir().unwrap();
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    let title = "Test Title";
    let slug = roko_cli::prd::slugify(title);
    let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, title);
    let scaffold = format!(
        "{frontmatter}# {title}\n\n\
         ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
         ## Design\n\n## References\n"
    );
    let draft_path = tmp.path().join(format!(".roko/prd/drafts/{slug}.md"));
    fs::write(&draft_path, &scaffold).unwrap();

    assert!(draft_path.exists(), "draft file should be created");
    let content = fs::read_to_string(&draft_path).unwrap();
    assert!(
        content.contains(&slug),
        "draft should contain the slug '{slug}'"
    );
    assert!(
        content.contains("status: draft"),
        "draft frontmatter should contain 'status: draft'"
    );
}

#[test]
fn prd_slugify_normalizes_title() {
    let slug = roko_cli::prd::slugify("Wire SystemPromptBuilder into orchestrate.rs");
    assert_eq!(slug, "wire-systempromptbuilder-into-orchestrate-rs");

    let slug2 = roko_cli::prd::slugify("  Multiple   Spaces  ");
    assert_eq!(slug2, "multiple-spaces");

    let slug3 = roko_cli::prd::slugify("UPPER-case_mixed");
    assert_eq!(slug3, "upper-case-mixed");
}

#[test]
fn materialize_agent_markdown_output_prepends_scaffold_when_frontmatter_absent() {
    let title = "Recovery Test";
    let slug = roko_cli::prd::slugify(title);
    let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, title);
    let scaffold = format!("{frontmatter}# {title}\n\n## Overview\n\n## Requirements\n");

    // Simulate agent output without frontmatter
    let agent_output = "This is the agent-generated content.\n\nIt has multiple paragraphs.";
    let result = roko_cli::prd::materialize_agent_markdown_output(agent_output, Some(&scaffold));

    assert!(result.is_some(), "should produce materialized output");
    let materialized = result.unwrap();
    assert!(
        materialized.starts_with("---"),
        "materialized output should start with frontmatter delimiter"
    );
    assert!(
        materialized.contains("status: draft"),
        "materialized output should contain frontmatter status"
    );
    assert!(
        materialized.contains("agent-generated content"),
        "materialized output should contain agent content"
    );
}

#[test]
fn materialize_agent_markdown_output_preserves_existing_frontmatter() {
    let agent_output =
        "---\nid: prd-custom\ntitle: Custom\nstatus: draft\n---\n\n# Custom PRD\n\nContent here.";
    let result =
        roko_cli::prd::materialize_agent_markdown_output(agent_output, Some("ignored scaffold"));

    assert!(result.is_some());
    let materialized = result.unwrap();
    assert!(
        materialized.starts_with("---"),
        "output with existing frontmatter should be preserved as-is"
    );
    assert!(
        materialized.contains("id: prd-custom"),
        "original frontmatter should be intact"
    );
    assert!(
        !materialized.contains("ignored scaffold"),
        "scaffold should NOT be prepended when frontmatter already exists"
    );
}

#[test]
fn materialize_agent_markdown_output_strips_code_fence() {
    let agent_output = "```markdown\n---\nid: prd-fenced\nstatus: draft\n---\n\n# Fenced\n```";
    let result = roko_cli::prd::materialize_agent_markdown_output(agent_output, None);

    assert!(result.is_some());
    let materialized = result.unwrap();
    assert!(
        materialized.starts_with("---"),
        "code fence should be stripped, revealing frontmatter"
    );
    assert!(
        !materialized.contains("```markdown"),
        "outer code fence markers should be removed"
    );
}

#[test]
fn materialize_agent_markdown_output_returns_none_for_empty() {
    let result = roko_cli::prd::materialize_agent_markdown_output("", None);
    assert!(result.is_none(), "empty input should return None");

    let result2 = roko_cli::prd::materialize_agent_markdown_output("   \n\n  ", None);
    assert!(
        result2.is_none(),
        "whitespace-only input should return None"
    );
}

#[test]
fn ensure_dirs_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();

    // Call twice — should not panic or fail.
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();
    roko_cli::prd::ensure_dirs(tmp.path()).unwrap();

    assert!(tmp.path().join(".roko/prd/drafts").is_dir());
    assert!(tmp.path().join(".roko/prd/published").is_dir());
    assert!(tmp.path().join(".roko/prd/ideas.md").exists());
}
