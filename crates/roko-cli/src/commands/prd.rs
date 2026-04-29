//! prd command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_prd(cli: &Cli, cmd: PrdCmd) -> Result<i32> {
    use roko_cli::agent_config::{command_from_config, load_gateway_env, model_from_config};
    use roko_cli::agent_exec::{AgentExecOpts, run_agent_capture_silent};

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
                let title = title.join(" ");
                let slug = roko_cli::prd::slugify(&title);
                let drafts = roko_cli::workspace_paths::drafts_dir(&workdir);
                roko_cli::prd::ensure_dirs(&workdir)?;
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
                        eprintln!("Draft already exists with content: {}", target.display());
                        eprintln!("Use: roko prd draft edit {slug}");
                        return Ok(1);
                    }
                    eprintln!("Found empty scaffold from previous run — regenerating.");
                }
                let model_key =
                    resolve_effective_model_key(&workdir, cli.model.clone(), Some("scribe"), "prd draft new")?;
                // Write scaffold first so agent can read and fill it
                let frontmatter = roko_cli::prd::new_draft_frontmatter(&slug, &title);
                let scaffold = format!(
                    "{frontmatter}# {title}\n\n\
                     ## Overview\n\n## Requirements\n\n## Acceptance criteria\n\n\
                     ## Design\n\n## References\n"
                );
                std::fs::write(&target, &scaffold)?;
                println!("📄 Creating PRD: {title}");

                let system = roko_cli::prd::prd_agent_prompt(
                    &workdir,
                    &format!(
                        "Fill in the draft PRD at {path}. \
                         If you have file tools, read the codebase to understand what exists \
                         and write the PRD directly to {path}. \
                         If you do NOT have file tools, output the complete PRD markdown \
                         (with YAML frontmatter) as your response — do not wrap in code fences. \
                         Follow the PRD quality standards in your system prompt exactly.",
                        path = target.display()
                    ),
                );
                let task_prompt = format!(
                    "Generate a complete PRD for: {title}. \
                     If you have file tools available, search the codebase to understand \
                     what exists and write the completed PRD to {path}. \
                     Otherwise, output the complete PRD markdown with YAML frontmatter. \
                     Include specific requirements, machine-verifiable acceptance criteria, \
                     and a design section.",
                    path = target.display()
                );
                // Snapshot file mtime before agent runs so we can detect
                // whether a CLI agent wrote the file directly.
                let mtime_before = std::fs::metadata(&target).and_then(|m| m.modified()).ok();

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
                })
                .await?;

                // Check if the agent already wrote the file (CLI agents with tools).
                let mtime_after = std::fs::metadata(&target).and_then(|m| m.modified()).ok();
                let file_was_modified = match (mtime_before, mtime_after) {
                    (Some(before), Some(after)) => after > before,
                    _ => false,
                };

                if file_was_modified {
                    // Agent wrote the file directly — verify it has content.
                    let content = std::fs::read_to_string(&target).unwrap_or_default();
                    let has_content = roko_cli::prd::has_substantive_markdown_content(&content);
                    if has_content {
                        println!("📄 Draft written to {}", target.display());
                    } else {
                        eprintln!(
                            "Agent modified file but left it empty at {}",
                            target.display()
                        );
                    }
                } else if exit_code == 0 && !output.trim().is_empty() {
                    // Agent returned content as text — write it to the file.
                    let content =
                        roko_cli::prd::materialize_agent_markdown_output(&output, Some(&scaffold))
                            .unwrap_or_else(|| scaffold.clone());
                    std::fs::write(&target, content)?;
                    println!("📄 Draft written to {}", target.display());
                } else if exit_code != 0 {
                    eprintln!(
                        "Agent failed (exit {exit_code}). Scaffold preserved at {}",
                        target.display()
                    );
                } else {
                    eprintln!(
                        "Agent returned empty output. Scaffold preserved at {}",
                        target.display()
                    );
                }
                let _ = crate::commands::util::persist_capture_episode(
                    &workdir,
                    &agent_command,
                    Some(model_key.as_str()),
                    "prd-draft-new",
                    &format!("prd:draft:new:{slug}"),
                    &task_prompt,
                    &output,
                    exit_code == 0,
                    started.elapsed().as_millis() as u64,
                    resume_session,
                )
                .await;
                Ok(exit_code)
            }
            PrdDraftCmd::Edit { slug } => {
                let draft = roko_cli::workspace_paths::draft_prd_path(&workdir, &slug);
                if !draft.exists() {
                    eprintln!("Draft not found: {}", draft.display());
                    return Ok(1);
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
                let _ = crate::commands::util::persist_capture_episode(
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
            let prd_path = find_prd(&workdir, &slug)?;
            let model_key =
                resolve_effective_model_key(&workdir, cli.model.clone(), Some("strategist"), "prd plan")?;
            let _generated_plans_root =
                roko_cli::prd::generate_plan_from_prd_with_model(
                    &slug,
                    &prd_path,
                    dry_run,
                    Some(model_key.as_str()),
                )
                .await?;
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
            })
            .await?;
            if !output.is_empty() {
                print!("{output}");
            }
            let _ = crate::commands::util::persist_capture_episode(
                &workdir,
                &agent_command,
                model_ref,
                "prd-consolidate",
                "prd:draft:consolidate",
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

fn resolve_effective_model_key(
    workdir: &Path,
    cli_model: Option<String>,
    role: Option<&str>,
    context: &str,
) -> Result<String> {
    let config = crate::load_roko_config(workdir)?;
    let selection = roko_cli::model_selection::resolve_effective_model(
        cli_model,
        None,
        role.map(str::to_owned),
        None,
        &config,
    )
    .map_err(|err| anyhow::anyhow!("resolve model selection for {context}: {err}"))?;
    eprintln!("[{context}] effective selection: {}", selection.reason);
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
