//! prd command handlers.
#![allow(unused_imports)]

use crate::*;
use serde::{Deserialize, Serialize};

/// Extract feature keywords from a PRD slug and description for context lookup.
/// Splits on hyphens, underscores, and spaces. Filters short words (<3 chars).
/// Returns up to 10 lowercase unique keywords.
fn extract_keywords_from_slug_and_description(slug: &str, description: &str) -> Vec<String> {
    let mut words: Vec<String> = slug
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .chain(description.split(|c: char| c == '-' || c == '_' || c.is_whitespace()))
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect();
    words.sort();
    words.dedup();
    words.truncate(10);
    words
}

/// Check that a generated PRD contains the required Repository Grounding section.
/// Warns to stderr if missing. Returns true if found, false if missing.
fn check_grounding_section(prd_content: &str, slug: &str) -> bool {
    let has_section = prd_content.lines().any(|line| {
        let trimmed = line.trim().to_lowercase();
        trimmed.starts_with("## repository grounding")
    });

    if !has_section {
        eprintln!(
            "WARNING: PRD '{}' is missing '## Repository Grounding' section. \
             The PRD may not be grounded in the actual repository.",
            slug
        );
    }

    has_section
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ArtifactKind {
    Prd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ValidationIssue {
    pub(crate) severity: Severity,
    pub(crate) category: String,
    pub(crate) message: String,
    pub(crate) location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ArtifactValidationReport {
    pub(crate) slug: String,
    pub(crate) artifact_path: String,
    pub(crate) artifact_kind: ArtifactKind,
    pub(crate) process_success: bool,
    pub(crate) artifact_valid: bool,
    pub(crate) issues: Vec<ValidationIssue>,
    pub(crate) timestamp: String,
}

fn severity_label(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "ERROR",
        Severity::Warning => "WARNING",
    }
}

fn normalize_crate_name(name: &str) -> String {
    name.trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .to_ascii_lowercase()
        .replace('_', "-")
}

/// Extract a markdown section by heading name (case-insensitive `## heading`).
/// Returns the body text from after the heading until the next `##` heading or end of file.
fn extract_markdown_section(content: &str, heading: &str) -> Option<String> {
    let heading_lower = heading.trim().to_ascii_lowercase();
    let mut in_section = false;
    let mut lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let trimmed_lower = trimmed.to_ascii_lowercase();

        if let Some(rest) = trimmed_lower.strip_prefix("## ") {
            if in_section {
                break;
            }

            if rest.trim().starts_with(heading_lower.as_str()) {
                in_section = true;
            }

            continue;
        }

        if in_section {
            lines.push(line);
        }
    }

    let body = lines.join("\n");
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

/// Extract a proposed new crate name from a line.
/// Matches patterns like:
/// - "**New crates**: roko-foo"
/// - "- roko-foo (new crate)"
/// - "create crate `roko-bar`"
/// - "new crate: roko-baz"
fn extract_new_crate_proposal(line: &str) -> Option<String> {
    let line_lower = line.to_ascii_lowercase();
    let patterns = ["new crate", "new crates", "create crate", "add crate"];
    if !patterns.iter().any(|pattern| line_lower.contains(pattern)) {
        return None;
    }

    for token in line.split_whitespace() {
        let token =
            token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_');
        let normalized = token.to_ascii_lowercase();
        if normalized.starts_with("roko-") || normalized.starts_with("roko_") {
            return Some(token.to_string());
        }
    }

    None
}

/// Validate the Repository Grounding section of a generated PRD against workspace members.
pub(crate) fn validate_prd_grounding(
    prd_content: &str,
    slug: &str,
    workspace_members: &[String],
) -> ArtifactValidationReport {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let grounding_text = extract_markdown_section(prd_content, "repository grounding");

    if grounding_text.is_none() {
        issues.push(ValidationIssue {
            severity: Severity::Warning,
            category: "missing_section".to_string(),
            message: "PRD has no '## Repository Grounding' section".to_string(),
            location: None,
        });
    }

    if let Some(text) = grounding_text.as_deref() {
        let text_lower = text.to_ascii_lowercase();

        if (text_lower.contains("no existing crates") || text_lower.contains("no relevant crates"))
            && !workspace_members.is_empty()
        {
            let mut preview: Vec<String> = workspace_members.iter().take(5).cloned().collect();
            let more = workspace_members.len().saturating_sub(preview.len());
            if more > 0 {
                preview.push(format!("and {more} more"));
            }

            issues.push(ValidationIssue {
                severity: Severity::Warning,
                category: "false_negative".to_string(),
                message: format!(
                    "PRD claims no existing crates but workspace has {} crate(s): {}",
                    workspace_members.len(),
                    preview.join(", ")
                ),
                location: Some("Repository Grounding".to_string()),
            });
        }

        let workspace_members_normalized: Vec<String> = workspace_members
            .iter()
            .map(|member| normalize_crate_name(member))
            .collect();

        for line in text.lines() {
            if let Some(proposed_name) = extract_new_crate_proposal(line) {
                let proposed_normalized = normalize_crate_name(&proposed_name);
                if workspace_members_normalized
                    .iter()
                    .any(|member| member == &proposed_normalized)
                {
                    issues.push(ValidationIssue {
                        severity: Severity::Error,
                        category: "duplicate_crate".to_string(),
                        message: format!(
                            "PRD proposes creating crate '{}' which already exists in the workspace",
                            proposed_name
                        ),
                        location: Some("Repository Grounding".to_string()),
                    });
                }
            }
        }
    }

    let artifact_valid = !issues.iter().any(|issue| issue.severity == Severity::Error);

    ArtifactValidationReport {
        slug: slug.to_string(),
        artifact_path: String::new(),
        artifact_kind: ArtifactKind::Prd,
        process_success: true,
        artifact_valid,
        issues,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Sidecar record for the repository context used during generation.
#[derive(Serialize)]
struct ContextSidecar<'a> {
    slug: &'a str,
    timestamp: String,
    /// Full repository context pack — all fields the agent saw at generation time.
    context_pack: &'a roko_cli::repo_context::RepoContextPack,
}

/// Persist the generation context as a JSON sidecar file alongside the PRD.
/// Non-blocking: write failures produce warnings only.
fn persist_context_sidecar(
    prd_drafts_dir: &std::path::Path,
    slug: &str,
    repo_context: &roko_cli::repo_context::RepoContextPack,
) {
    let sidecar_path = prd_drafts_dir.join(format!("{slug}.context.json"));
    let sidecar = ContextSidecar {
        slug,
        timestamp: chrono::Utc::now().to_rfc3339(),
        context_pack: repo_context,
    };

    match serde_json::to_string_pretty(&sidecar) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&sidecar_path, json) {
                eprintln!(
                    "WARNING: Failed to write context sidecar {}: {}",
                    sidecar_path.display(),
                    e
                );
            }
        }
        Err(e) => {
            eprintln!("WARNING: Failed to serialize context sidecar: {}", e);
        }
    }
}

/// Persist the validation report as a JSON sidecar file alongside the PRD.
/// Non-blocking: write failures produce warnings only.
fn persist_validation_sidecar(
    prd_drafts_dir: &std::path::Path,
    slug: &str,
    report: &ArtifactValidationReport,
) {
    let sidecar_path = prd_drafts_dir.join(format!("{slug}.validation.json"));

    match serde_json::to_string_pretty(report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&sidecar_path, json) {
                eprintln!(
                    "WARNING: Failed to write validation sidecar {}: {}",
                    sidecar_path.display(),
                    e
                );
            }
        }
        Err(e) => {
            eprintln!("WARNING: Failed to serialize validation sidecar: {}", e);
        }
    }
}

pub(crate) async fn cmd_prd(cli: &Cli, cmd: PrdCmd) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env, model_from_config};
    use roko_cli::agent_exec::{AgentExecOpts, persist_capture_episode, run_agent_capture_silent};

    let workdir = resolve_workdir(cli);
    let gw = load_gateway_env(&workdir);
    let model = cli.model.clone().or_else(|| model_from_config(&workdir));
    let model_ref = model.as_deref();
    let effort = cli.effort.map(|effort| effort.to_string());
    let effort_ref = effort.as_deref();
    let resume_session = cli.resume.as_deref();
    let agent_command = command_from_config(&workdir).unwrap_or_else(|| "claude".to_string());

    match cmd {
        PrdCmd::Idea { text } => {
            let joined = text.join(" ");
            roko_cli::prd::cmd_idea(&workdir, &joined)?;
            Ok(0)
        }
        PrdCmd::List => {
            roko_cli::prd::cmd_list(&workdir)?;
            Ok(0)
        }
        PrdCmd::Status => {
            roko_cli::prd::cmd_status(&workdir, None)?;
            Ok(0)
        }
        PrdCmd::Draft { cmd: draft_cmd } => match draft_cmd {
            PrdDraftCmd::New { title } => {
                let t_total = Instant::now();
                let t_phase = Instant::now();
                let title = title.join(" ");
                let slug = roko_cli::prd::slugify(&title);
                let feature_keywords = extract_keywords_from_slug_and_description(&slug, &title);
                let drafts = roko_cli::workspace_paths::drafts_dir(&workdir);
                roko_cli::prd::ensure_dirs(&workdir)?;
                let _lock =
                    roko_cli::workspace_lock::acquire_workspace_lock(&workdir.join(".roko"))?;
                let target = drafts.join(format!("{slug}.md"));
                // If the draft exists and has real content (not just scaffold),
                // point the user to `edit` instead. But if it's only the
                // skeleton left by a failed `new` run, overwrite it.
                if target.exists() {
                    let existing = std::fs::read_to_string(&target).unwrap_or_default();
                    let is_skeleton = existing
                        .lines()
                        .filter(|l| {
                            !l.starts_with("---")
                                && !l.starts_with('#')
                                && !l.starts_with("##")
                                && !l.trim().is_empty()
                        })
                        .count()
                        == 0;
                    if !is_skeleton {
                        anyhow::bail!(
                            "draft already exists with content: {}; use: roko prd draft edit {slug}",
                            target.display()
                        );
                    }
                    eprintln!("Found empty scaffold from previous run — regenerating.");
                }
                let model_key = roko_cli::model_selection::resolve_effective_model_key(
                    &workdir,
                    cli.model.clone(),
                    Some("scribe"),
                    "prd draft new",
                )?;
                // Pre-flight: check only the provider for the resolved model.
                {
                    let prd_config: roko_core::config::schema::RokoConfig =
                        std::fs::read_to_string(workdir.join("roko.toml"))
                            .ok()
                            .and_then(|s| roko_core::config::schema::RokoConfig::from_toml(&s).ok())
                            .unwrap_or_default();
                    crate::commands::util::preflight_provider_for_model(&prd_config, &model_key)?;
                    // Aggregate provider readiness: warn/abort if no providers are usable.
                    crate::commands::util::preflight_providers_aggregate(&prd_config)?;
                }
                // Write scaffold first so agent can read and fill it
                let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, &title);
                let scaffold = format!(
                    "{frontmatter}# {title}\n\n\
                     ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
                     ## Design\n\n## References\n"
                );
                std::fs::write(&target, &scaffold)?;
                println!("📄 Creating PRD: {title}");

                let init_ms = t_phase.elapsed().as_millis();
                let t_phase = Instant::now();
                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Fill in the draft PRD at {path}. \
                         Output the complete PRD markdown (with YAML frontmatter) as your response. \
                         Do NOT use file tools — they are not available. \
                         Do NOT wrap in code fences. \
                         Follow the PRD quality standards in your system prompt exactly.",
                        path = target.display()
                    ),
                );
                let prompt_ms = t_phase.elapsed().as_millis();
                let feature_keyword_refs: Vec<&str> =
                    feature_keywords.iter().map(String::as_str).collect();
                // Skip repo context scanning for workspaces without source code
                // (e.g. freshly-initialized workspaces from `roko init`).
                let t_phase = Instant::now();
                let has_source_code = workdir.join("src").is_dir()
                    || workdir.join("crates").is_dir()
                    || workdir.join("lib").is_dir()
                    || workdir.join("Cargo.toml").is_file()
                    || workdir.join("package.json").is_file();
                // Keep the full pack so it can be persisted to the context sidecar.
                let repo_context_pack: Option<roko_cli::repo_context::RepoContextPack> =
                    if has_source_code {
                        match roko_cli::repo_context::build_repo_context(
                            &workdir,
                            &feature_keyword_refs,
                        )
                        .await
                        {
                            Ok(pack) => {
                                if !pack.context_root_verified {
                                    eprintln!(
                                        "WARNING: Repository context not verified for keywords {:?}. \
                                         Generated PRD may reference nonexistent code.",
                                        feature_keywords
                                    );
                                }
                                Some(pack)
                            }
                            Err(err) => {
                                eprintln!(
                                    "WARNING: Repository context unavailable for keywords {:?}: {err}",
                                    feature_keywords
                                );
                                None
                            }
                        }
                    } else {
                        None // Empty workspace — skip context scanning
                    };
                let repo_context_section: Option<String> =
                    repo_context_pack.as_ref().map(|p| p.to_prompt_section());
                let context_suffix = repo_context_section
                    .as_deref()
                    .map(|ctx| format!("\n\n---\n\n{ctx}"))
                    .unwrap_or_default();
                let task_prompt = format!(
                    "Generate a complete PRD for: {title}. \
                     Output the complete PRD markdown with YAML frontmatter. \
                     Include specific requirements, machine-verifiable acceptance criteria, \
                     and a design section.{context_suffix}",
                    context_suffix = context_suffix
                );
                let context_ms = t_phase.elapsed().as_millis();
                // Snapshot file bytes before the agent runs so direct writes
                // are detected even on coarse-mtime filesystems.
                let content_before = std::fs::read(&target).ok();

                eprintln!("  Generating PRD draft: {slug}");
                let t_phase = Instant::now();
                let started = Instant::now();
                let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: Some(model_key.as_str()),
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                    role: Some("scribe"),
                    allowed_tools: Some("none"),
                })
                .await?;
                if exit_code == 0 {
                    eprintln!("  ✓ Draft generated: {slug}");
                } else {
                    eprintln!("  ✗ PRD draft generation failed");
                }

                let agent_ms = t_phase.elapsed().as_millis();
                let t_phase = Instant::now();
                // Check if the agent already wrote the file (CLI agents with tools).
                let content_after = std::fs::read(&target).ok();
                let file_was_modified = match (&content_before, &content_after) {
                    (Some(before), Some(after)) => before != after,
                    (None, Some(_)) => true,
                    _ => false,
                };

                let mut draft_written = false;
                if file_was_modified {
                    // Agent wrote the file directly — verify it has content.
                    let content = std::fs::read_to_string(&target).unwrap_or_default();
                    let has_content = roko_cli::prd::has_substantive_markdown_content(&content);
                    if has_content {
                        draft_written = true;
                        println!("📄 Draft written to {}", target.display());
                    } else {
                        eprintln!(
                            "Agent modified file but left it empty at {}",
                            target.display()
                        );
                    }
                } else if !output.trim().is_empty() {
                    // Agent returned content as text — write it to the file.
                    let content =
                        roko_cli::prd::materialize_agent_markdown_output(&output, Some(&scaffold))
                            .unwrap_or_else(|| scaffold.clone());
                    if roko_cli::prd::has_substantive_markdown_content(&content) {
                        std::fs::write(&target, content)?;
                        draft_written = true;
                        println!("📄 Draft written to {}", target.display());
                    } else {
                        let _ = std::fs::remove_file(&target);
                        eprintln!(
                            "Agent output did not contain a substantive PRD — no draft created."
                        );
                    }
                } else if exit_code != 0 {
                    let _ = std::fs::remove_file(&target);
                    eprintln!("Agent failed (exit {exit_code}) — no draft created.");
                } else {
                    let _ = std::fs::remove_file(&target);
                    eprintln!("Agent returned empty output — no draft created.");
                }

                let artifact_success = draft_written && target.is_file();
                if artifact_success && exit_code != 0 {
                    eprintln!(
                        "Agent exited with {exit_code}, but the draft artifact was written; treating draft creation as successful."
                    );
                }

                let workspace_members: Vec<String> = if artifact_success {
                    let crates_dir = workdir.join("crates");
                    let mut workspace_members: Vec<String> = Vec::new();
                    if let Ok(entries) = std::fs::read_dir(&crates_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                workspace_members
                                    .push(entry.file_name().to_string_lossy().into_owned());
                            }
                        }
                    }
                    workspace_members.sort_unstable();
                    workspace_members.dedup();
                    workspace_members
                } else {
                    Vec::new()
                };
                // Post-generation grounding check and semantic validation.
                let validation_report: Option<ArtifactValidationReport> = if artifact_success {
                    if let Ok(written_content) = std::fs::read_to_string(&target) {
                        check_grounding_section(&written_content, &slug);

                        let mut report =
                            validate_prd_grounding(&written_content, &slug, &workspace_members);
                        report.artifact_path = target.display().to_string();

                        for issue in &report.issues {
                            eprintln!(
                                "[{}] {}: {}",
                                severity_label(&issue.severity),
                                issue.category,
                                issue.message
                            );
                        }

                        Some(report)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if artifact_success {
                    if let Some(ref pack) = repo_context_pack {
                        persist_context_sidecar(&drafts, &slug, pack);
                    }
                    if let Some(ref report) = validation_report {
                        persist_validation_sidecar(&drafts, &slug, report);
                        println!("Context sidecar: {}.context.json", slug);
                        println!("Validation sidecar: {}.validation.json", slug);
                        let error_count = report
                            .issues
                            .iter()
                            .filter(|issue| issue.severity == Severity::Error)
                            .count();
                        let warning_count = report
                            .issues
                            .iter()
                            .filter(|issue| issue.severity == Severity::Warning)
                            .count();

                        if report.artifact_valid {
                            println!("Artifact validation: PASSED ({warning_count} warnings)");
                        } else {
                            println!(
                                "Artifact validation: FAILED ({error_count} errors, {warning_count} warnings)"
                            );
                        }
                    }
                }
                let post_ms = t_phase.elapsed().as_millis();
                let t_phase = Instant::now();
                let _ = persist_capture_episode(
                    &workdir,
                    &agent_command,
                    Some(model_key.as_str()),
                    "prd-draft-new",
                    &format!("prd:draft:{slug}"),
                    &task_prompt,
                    &output,
                    artifact_success,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                let learn_ms = t_phase.elapsed().as_millis();
                let total_ms = t_total.elapsed().as_millis();
                tracing::info!(
                    init_ms,
                    prompt_ms,
                    context_ms,
                    agent_ms,
                    post_ms,
                    learn_ms,
                    total_ms,
                    "prd draft new: phase timing"
                );
                eprintln!(
                    "  Timing: init={init_ms}ms prompt={prompt_ms}ms context={context_ms}ms agent={agent_ms}ms post={post_ms}ms learn={learn_ms}ms total={total_ms}ms"
                );
                Ok(if artifact_success {
                    0
                } else if exit_code == 0 {
                    1
                } else {
                    exit_code
                })
            }
            PrdDraftCmd::Edit { slug } => {
                let draft = roko_cli::workspace_paths::draft_prd_path(&workdir, &slug);
                if !draft.exists() {
                    anyhow::bail!("draft not found: {}", draft.display());
                }
                println!("📝 Refining draft: {slug}");
                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Read and improve the draft PRD at {path}. \
                         If you have file tools, update that file directly. \
                         If you do NOT have file tools, output the complete improved PRD markdown \
                         with YAML frontmatter and no code fences. \
                         Follow the PRD quality standards in your system prompt.",
                        path = draft.display()
                    ),
                );
                let task_prompt = format!(
                    "Read {path} and improve it: \
                     (1) Are requirements specific and testable? \
                     (2) Are acceptance criteria machine-verifiable shell commands? \
                     (3) Are there 10+ citations with [AUTHOR-YEAR] format? \
                     (4) Are there 2+ mermaid diagrams with color styling? \
                     Search the codebase to verify claims. \
                     If you have file tools, update the file in place. \
                     Otherwise, output the complete improved PRD markdown with YAML frontmatter.",
                    path = draft.display()
                );
                let mtime_before = std::fs::metadata(&draft).and_then(|m| m.modified()).ok();
                let started = Instant::now();
                let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                    prompt: &task_prompt,
                    workdir: &workdir,
                    model: model_ref,
                    effort: effort_ref,
                    system_prompt: Some(&system),
                    resume_session,
                    env_vars: &gw.vars,
                    role: Some("scribe"),
                    allowed_tools: None,
                })
                .await?;
                let mtime_after = std::fs::metadata(&draft).and_then(|m| m.modified()).ok();
                let file_was_modified = match (mtime_before, mtime_after) {
                    (Some(before), Some(after)) => after > before,
                    _ => false,
                };
                if file_was_modified {
                    let content = std::fs::read_to_string(&draft).unwrap_or_default();
                    if roko_cli::prd::has_substantive_markdown_content(&content) {
                        println!("📄 Draft updated at {}", draft.display());
                    } else {
                        eprintln!(
                            "Agent modified file but left it empty at {}",
                            draft.display()
                        );
                    }
                } else if exit_code == 0 {
                    if let Some(content) =
                        roko_cli::prd::materialize_agent_markdown_output(&output, None)
                    {
                        std::fs::write(&draft, content)?;
                        println!("📄 Draft updated at {}", draft.display());
                    } else {
                        eprintln!(
                            "Agent returned empty output. Existing draft preserved at {}",
                            draft.display()
                        );
                    }
                } else if !output.is_empty() {
                    print!("{output}");
                }
                let _ = persist_capture_episode(
                    &workdir,
                    &agent_command,
                    model_ref,
                    "prd-draft-edit",
                    &format!("prd:draft:edit:{slug}"),
                    &task_prompt,
                    &output,
                    exit_code == 0,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                Ok(exit_code)
            }
            PrdDraftCmd::Promote { slug, auto_execute } => {
                roko_cli::prd::cmd_promote(&workdir, &slug, auto_execute).await?;
                Ok(0)
            }
            PrdDraftCmd::List => {
                let drafts = roko_cli::workspace_paths::drafts_dir(&workdir);
                roko_cli::prd::ensure_dirs(&workdir)?;
                let files = roko_cli::prd::list_md_files(&drafts);
                if files.is_empty() {
                    println!("No drafts. Create one: roko prd draft new \"title\"");
                } else {
                    for f in &files {
                        println!("  {}", f.file_stem().unwrap_or_default().to_string_lossy());
                    }
                }
                Ok(0)
            }
        },
        PrdCmd::Plan { slug, dry_run } => {
            let t_total = Instant::now();
            let t_phase = Instant::now();
            let _lock = roko_cli::workspace_lock::acquire_workspace_lock(&workdir.join(".roko"))?;
            let prd_path = find_prd(&workdir, &slug)?;
            let model_key = roko_cli::model_selection::resolve_effective_model_key(
                &workdir,
                cli.model.clone(),
                Some("strategist"),
                "prd plan",
            )?;
            // Pre-flight: check only the provider for the resolved model.
            {
                let plan_config: roko_core::config::schema::RokoConfig =
                    std::fs::read_to_string(workdir.join("roko.toml"))
                        .ok()
                        .and_then(|s| roko_core::config::schema::RokoConfig::from_toml(&s).ok())
                        .unwrap_or_default();
                crate::commands::util::preflight_provider_for_model(&plan_config, &model_key)?;
                // Aggregate provider readiness: warn/abort if no providers are usable.
                crate::commands::util::preflight_providers_aggregate(&plan_config)?;
            }
            let init_ms = t_phase.elapsed().as_millis();
            let t_phase = Instant::now();
            let _generated_plans_root = roko_cli::prd::generate_plan_from_prd_with_model(
                &slug,
                &prd_path,
                dry_run,
                Some(model_key.as_str()),
            )
            .await?;
            let generate_ms = t_phase.elapsed().as_millis();
            let total_ms = t_total.elapsed().as_millis();
            tracing::info!(init_ms, generate_ms, total_ms, "prd plan: phase timing");
            eprintln!("  Timing: init={init_ms}ms generate={generate_ms}ms total={total_ms}ms");
            Ok(0)
        }
        PrdCmd::Consolidate => {
            println!("🔄 Scanning all PRDs for duplicates, gaps, and inconsistencies...");
            let mut all_context = String::new();
            for dir_name in ["published", "drafts"] {
                let dir = roko_cli::workspace_paths::prd_dir(&workdir).join(dir_name);
                for path in roko_cli::prd::list_md_files(&dir) {
                    if let Ok(c) = std::fs::read_to_string(&path) {
                        let truncated: String = c.lines().take(50).collect::<Vec<_>>().join("\n");
                        let _ = write!(all_context, "### {}\n{truncated}\n---\n\n", path.display());
                    }
                }
            }
            let ideas = std::fs::read_to_string(roko_cli::workspace_paths::ideas_path(&workdir))
                .unwrap_or_default();
            let task_prompt = format!(
                "Review ALL existing PRDs and ideas. Report: \
                 (1) DUPLICATES: PRDs covering the same thing (propose merge). \
                 (2) GAPS: Areas with no PRD coverage. \
                 (3) INCONSISTENCIES: Conflicting requirements. \
                 (4) STALE: Requirements already implemented (check the code). \
                 (5) IDEAS TO PROMOTE: Ideas that should become draft PRDs. \
                 After analysis, create new drafts for gaps and update existing PRDs.\n\n\
                 PRDs:\n{all_context}\n\nIdeas:\n{ideas}"
            );
            let system = roko_cli::prd::prd_agent_prompt(&workdir, "Consolidate all PRDs");
            let started = Instant::now();
            let (exit_code, output) = run_agent_capture_silent(AgentExecOpts {
                prompt: &task_prompt,
                workdir: &workdir,
                model: model_ref,
                effort: effort_ref,
                system_prompt: Some(&system),
                resume_session,
                env_vars: &gw.vars,
                role: Some("strategist"),
                allowed_tools: None,
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "prd-consolidate",
                "prd:consolidate",
                &task_prompt,
                &output,
                exit_code == 0,
                started.elapsed().as_millis() as u64,
                resume_session,
            )
            .await;
            Ok(exit_code)
        }
    }
}

#[allow(dead_code)]
fn resolve_effective_model_key(
    workdir: &Path,
    cli_model: Option<String>,
    role: Option<&str>,
    context: &str,
) -> Result<String> {
    let config = roko_core::config::loader::load_config_unified(workdir)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let selection = roko_cli::model_selection::resolve_effective_model(
        cli_model,
        None,
        role.map(str::to_string),
        None,
        &config,
        None,
    )
    .map_err(|err| anyhow::anyhow!("resolve model selection for {context}: {err}"))?;
    selection.print_stderr();
    Ok(selection.effective_model_key)
}

/// Find a PRD by slug in either published or drafts.
pub(crate) fn find_prd(workdir: &Path, slug: &str) -> Result<PathBuf> {
    if let Some(path) = roko_cli::workspace_paths::find_prd_path(workdir, slug) {
        return Ok(path);
    }
    anyhow::bail!("PRD not found: {slug} (checked published/ and drafts/)");
}

/// Auto-detect the project domain from file patterns in the target directory.
pub(crate) fn detect_project_domain(target: &Path) -> &'static str {
    if target.join("Cargo.toml").exists() {
        "rust"
    } else if target.join("package.json").exists() {
        "typescript"
    } else if target.join("go.mod").exists() {
        "go"
    } else if target.join("requirements.txt").exists()
        || target.join("pyproject.toml").exists()
        || target.join("setup.py").exists()
    {
        "python"
    } else if target.join("Gemfile").exists() {
        "ruby"
    } else if target.join("pom.xml").exists() || target.join("build.gradle").exists() {
        "java"
    } else {
        "general"
    }
}

/// Verify configuration hint based on domain profile.
pub(crate) fn domain_gate_hint(domain: &str) -> &'static str {
    match domain {
        "rust" => "compile (cargo check), test (cargo test), clippy (cargo clippy)",
        "typescript" => "compile (tsc --noEmit), test (npm test), lint (eslint)",
        "go" => "compile (go build), test (go test), lint (golangci-lint)",
        "python" => "test (pytest), lint (ruff), typecheck (mypy)",
        "ruby" => "test (rspec), lint (rubocop)",
        "java" => "compile (mvn compile), test (mvn test)",
        _ => "compile, test, lint (configure in roko.toml)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_markdown_section_is_case_insensitive() {
        let content = "# PRD\n\n## Repository Grounding\nGrounded text.\n\n## Next\nMore text.\n";
        let section =
            extract_markdown_section(content, "repository grounding").expect("grounding section");
        assert_eq!(section.trim(), "Grounded text.");
    }

    #[test]
    fn extract_new_crate_proposal_handles_common_patterns() {
        assert_eq!(
            extract_new_crate_proposal("**New crates**: roko-foo"),
            Some("roko-foo".to_string())
        );
        assert_eq!(
            extract_new_crate_proposal("- roko-bar (new crate)"),
            Some("roko-bar".to_string())
        );
        assert_eq!(
            extract_new_crate_proposal("create crate `roko-baz`"),
            Some("roko-baz".to_string())
        );
    }

    #[test]
    fn validate_prd_grounding_warns_and_errors_as_expected() {
        let workspace_members = vec!["roko-compose".to_string(), "roko-agent".to_string()];
        let content = "\
## Repository Grounding
No existing crates are relevant here.
Create crate `roko-compose`.

## Requirements
Do work.
";

        let report = validate_prd_grounding(content, "demo", &workspace_members);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.category == "false_negative")
        );
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.category == "duplicate_crate")
        );
        assert!(!report.artifact_valid);
    }
}
