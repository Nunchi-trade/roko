//! `roko prd` subcommand — PRD lifecycle management.
//!
//! Manages product requirements documents through their lifecycle:
//! idea → draft → published → plans → implemented.
//!
//! PRDs live in `.roko/prd/` with this layout:
//! ```text
//! .roko/prd/
//! ├── ideas.md              # quick captures
//! ├── drafts/               # work-in-progress PRDs
//! │   └── <slug>.md
//! └── published/            # finalized PRDs
//!     └── <slug>.md
//! ```

mod dry_run_fs;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agent_exec::{AgentExecEpisode, AgentExecOpts, run_agent_logged};
use crate::task_parser::TasksFile;
use crate::workspace_paths::{
    drafts_dir, ideas_path, plans_dir as workspace_plans_dir, prd_dir, published_dir,
};
use anyhow::{Context as _, Result, anyhow};
use roko_core::config::schema::RokoConfig;
use roko_core::obs::MetricRegistry;
use roko_core::{Body, Engram, Kind, Provenance, Substrate};
use roko_fs::FileSubstrate;

fn tier_rank(tier: &str) -> u8 {
    match tier {
        "mechanical" => 0,
        "focused" => 1,
        "integrative" => 2,
        "architectural" => 3,
        _ => 1,
    }
}

fn rank_to_complexity(rank: u8) -> &'static str {
    match rank {
        0 => "mechanical",
        1 => "focused",
        2 => "integrative",
        3 => "architectural",
        _ => "focused",
    }
}

fn generated_plan_stats(paths: &[PathBuf]) -> Result<(usize, String)> {
    if paths.is_empty() {
        return Ok((0, "unknown".to_string()));
    }

    let mut task_count = 0usize;
    let mut max_rank = 0u8;

    for path in paths {
        let tasks_file =
            TasksFile::parse(path).with_context(|| format!("parse {}", path.display()))?;
        task_count = task_count.saturating_add(tasks_file.tasks.len());
        for task in &tasks_file.tasks {
            max_rank = max_rank.max(tier_rank(task.tier.as_str()));
        }
    }

    let estimated_complexity = if task_count == 0 {
        "unknown".to_string()
    } else {
        rank_to_complexity(max_rank).to_string()
    };

    Ok((task_count, estimated_complexity))
}

fn normalize_task_title(title: &str) -> String {
    title
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn preserve_completed_task_status(
    old_tasks: Option<&TasksFile>,
    mut regenerated: TasksFile,
    plan_dir: &Path,
) -> TasksFile {
    if let Some(old_tasks) = old_tasks {
        let completed: Vec<&crate::task_parser::TaskDef> = old_tasks
            .tasks
            .iter()
            .filter(|task| task.status.eq_ignore_ascii_case("done"))
            .collect();

        for task in &mut regenerated.tasks {
            let normalized = normalize_task_title(&task.title);
            if completed.iter().any(|old| {
                let old_title = normalize_task_title(&old.title);
                old.id == task.id
                    || old_title == normalized
                    || old_title.contains(&normalized)
                    || normalized.contains(&old_title)
            }) {
                task.status = "done".to_string();
            }
        }

        regenerated.meta.iteration = old_tasks.meta.iteration.saturating_add(1);
        if regenerated.meta.plan.trim().is_empty() {
            regenerated.meta.plan = old_tasks.meta.plan.clone();
        }
    }

    if regenerated.meta.plan.trim().is_empty() {
        regenerated.meta.plan = plan_dir
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown-plan".to_string());
    }

    regenerated.meta.total = regenerated.tasks.len() as u32;
    regenerated.meta.done = regenerated
        .tasks
        .iter()
        .filter(|task| task.status.eq_ignore_ascii_case("done"))
        .count() as u32;
    regenerated.meta.status =
        if regenerated.meta.total > 0 && regenerated.meta.done == regenerated.meta.total {
            "complete".to_string()
        } else {
            "ready".to_string()
        };

    regenerated
}

fn find_plan_source_document(plan_dir: &Path) -> Result<PathBuf> {
    for candidate in ["source-prd.md", "prd-extract.md", "plan.md"] {
        let path = plan_dir.join(candidate);
        if path.exists() {
            return Ok(path);
        }
    }

    Err(anyhow!(
        "no source PRD found in {} (looked for source-prd.md, prd-extract.md, and plan.md)",
        plan_dir.display()
    ))
}

fn old_format_plan_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let tasks_path = path.join("tasks.toml");
            if !tasks_path.is_file() {
                continue;
            }
            if matches!(
                TasksFile::validate_modern_fields(&tasks_path),
                Ok(issues) if !issues.is_empty()
            ) {
                dirs.push(path);
            }
        }
    }
    dirs.sort();
    dirs
}

async fn regenerate_old_format_plan(
    workdir: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    env_vars: &[(String, String)],
    plan_dir: &Path,
) -> Result<bool> {
    let tasks_path = plan_dir.join("tasks.toml");
    if !tasks_path.is_file() {
        return Ok(false);
    }

    let modern_issues = TasksFile::validate_modern_fields(&tasks_path)
        .with_context(|| format!("validate modern fields at {}", tasks_path.display()))?;
    if modern_issues.is_empty() {
        return Ok(false);
    }

    let existing = std::fs::read_to_string(&tasks_path)
        .with_context(|| format!("read {}", tasks_path.display()))?;
    let existing_tasks = TasksFile::parse(&tasks_path).ok();
    let source_path = find_plan_source_document(plan_dir)?;
    let source_content = std::fs::read_to_string(&source_path)
        .with_context(|| format!("read {}", source_path.display()))?;
    let system = crate::plan_generate::build_generation_prompt(workdir, &source_content, "plan");
    let task_prompt = format!(
        "Regenerate the plan at {path} from the source plan document above. \
         Rewrite tasks.toml in place with full modern metadata: tier, model_hint, \
         max_loc, files, allowed_tools, denied_tools, mcp_servers, depends_on, \
         [task.context], and [[task.verify]]. Preserve the status of any task \
         that is already marked done in the existing file. Do not create new plan \
         directories.\n\n## Existing tasks.toml\n\n```toml\n{existing}\n```",
        path = tasks_path.display(),
        existing = existing,
    );

    let plan_name = plan_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let task_id = format!("plan:regenerate:{plan_name}");
    let exit_code = match run_agent_logged(
        AgentExecOpts {
            prompt: &task_prompt,
            workdir,
            model,
            effort,
            system_prompt: Some(&system),
            resume_session: None,
            env_vars,
        },
        AgentExecEpisode {
            task_kind: "plan-regenerate",
            task_id: &task_id,
        },
    )
    .await
    {
        Ok(code) => code,
        Err(err) => {
            std::fs::write(&tasks_path, &existing)
                .with_context(|| format!("restore {}", tasks_path.display()))?;
            return Err(err);
        }
    };

    if exit_code != 0 {
        std::fs::write(&tasks_path, &existing)
            .with_context(|| format!("restore {}", tasks_path.display()))?;
        anyhow::bail!("plan regeneration agent failed with exit code {exit_code}");
    }

    let regenerated = match TasksFile::parse(&tasks_path) {
        Ok(tasks) => tasks,
        Err(err) => {
            std::fs::write(&tasks_path, &existing)
                .with_context(|| format!("restore {}", tasks_path.display()))?;
            return Err(err);
        }
    };

    let merged = preserve_completed_task_status(existing_tasks.as_ref(), regenerated, plan_dir);
    let rendered = toml::to_string_pretty(&merged).context("serialize regenerated tasks.toml")?;
    if let Err(err) = std::fs::write(&tasks_path, rendered) {
        std::fs::write(&tasks_path, &existing)
            .with_context(|| format!("restore {}", tasks_path.display()))?;
        return Err(err.into());
    }

    match TasksFile::validate_modern_fields(&tasks_path) {
        Ok(issues) if !issues.is_empty() => {
            std::fs::write(&tasks_path, &existing)
                .with_context(|| format!("restore {}", tasks_path.display()))?;
            anyhow::bail!(
                "regenerated tasks.toml is missing modern fields: {}",
                issues
                    .into_iter()
                    .map(|issue| format!("{}: {:?}", issue.task_id, issue.missing_fields))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        Ok(_) => {}
        Err(err) => {
            std::fs::write(&tasks_path, &existing)
                .with_context(|| format!("restore {}", tasks_path.display()))?;
            return Err(err);
        }
    }

    Ok(true)
}

async fn regenerate_old_format_plans(
    workdir: &Path,
    model: Option<&str>,
    effort: Option<&str>,
    env_vars: &[(String, String)],
    plans_root: &Path,
) -> Result<usize> {
    let mut regen_count = 0usize;
    for plan_dir in old_format_plan_dirs(plans_root) {
        if regenerate_old_format_plan(workdir, model, effort, env_vars, &plan_dir).await? {
            regen_count += 1;
        }
    }
    Ok(regen_count)
}

async fn emit_prd_plan_signal(workdir: &Path, kind: Kind, body: serde_json::Value) -> Result<()> {
    let substrate = FileSubstrate::open(workdir.join(".roko"))
        .await
        .with_context(|| format!("open {}", workdir.join(".roko").display()))?;
    let signal = Engram::builder(kind)
        .body(Body::Json(body))
        .provenance(Provenance::trusted("roko.prd"))
        .build();
    substrate.put(signal).await?;
    Ok(())
}

/// Ensure the PRD directory structure exists.
pub fn ensure_dirs(workdir: &Path) -> Result<()> {
    std::fs::create_dir_all(drafts_dir(workdir))?;
    std::fs::create_dir_all(published_dir(workdir))?;
    let ideas = ideas_path(workdir);
    if !ideas.exists() {
        std::fs::write(
            &ideas,
            "# Ideas\n\nQuick captures. Run `roko prd idea \"text\"` to append.\n",
        )?;
    }
    Ok(())
}

// ─── PRD frontmatter ───────────────────────────────────────────────

/// Parsed PRD frontmatter.
#[derive(Debug, Default)]
pub struct PrdMeta {
    /// Stable PRD identifier (e.g. `prd-golem-memory`).
    pub id: String,
    /// Human-readable PRD title.
    pub title: String,
    /// Lifecycle status (`draft` or `published`).
    pub status: String,
    /// Monotonic document version number.
    pub version: u32,
    /// Creation date in `YYYY-MM-DD` format.
    pub created: String,
    /// Last update date in `YYYY-MM-DD` format.
    pub updated: String,
    /// Other PRD ids this document depends on.
    pub depends_on: Vec<String>,
    /// Crates touched by the requirements in this PRD.
    pub crates: Vec<String>,
    /// Plan ids generated from this PRD.
    pub plans_generated: Vec<String>,
    /// Coverage ratio in `[0.0, 1.0]`.
    pub coverage: f64,
    /// Free-form metadata tags.
    pub tags: Vec<String>,
    /// Optional plan generation template preset.
    pub plan_template: Option<String>,
}

impl PrdMeta {
    /// Parse frontmatter from a PRD markdown file.
    pub fn parse(content: &str) -> Option<Self> {
        let content = content.trim();
        if !content.starts_with("---") {
            return None;
        }
        let end = content[3..].find("---")?;
        let yaml = &content[3..3 + end];
        let mut meta = Self::default();
        for line in yaml.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("id:") {
                meta.id = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("title:") {
                meta.title = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("status:") {
                meta.status = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("version:") {
                meta.version = val.trim().parse().unwrap_or(1);
            } else if let Some(val) = line.strip_prefix("created:") {
                meta.created = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("updated:") {
                meta.updated = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("coverage:") {
                meta.coverage = val.trim().parse().unwrap_or(0.0);
            } else if let Some(val) = line
                .strip_prefix("plan_template:")
                .or_else(|| line.strip_prefix("plan_template ="))
            {
                let value = val.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    meta.plan_template = Some(value.to_string());
                }
            }
        }
        Some(meta)
    }
}

// ─── List PRDs ─────────────────────────────────────────────────────

/// Return sorted markdown files (`*.md`) in `dir`.
///
/// Missing or unreadable directories are treated as empty.
pub fn list_md_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Entry in the PRD listing.
pub struct PrdEntry {
    /// File slug (`<slug>.md` without extension).
    pub slug: String,
    /// Display title shown in CLI output.
    pub title: String,
    /// Lifecycle status for this entry.
    pub status: String,
    /// Coverage ratio in `[0.0, 1.0]`.
    pub coverage: f64,
}

fn read_prd_entry(path: &Path) -> PrdEntry {
    let slug = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let content = std::fs::read_to_string(path).unwrap_or_default();
    if let Some(meta) = PrdMeta::parse(&content) {
        PrdEntry {
            slug,
            title: meta.title,
            status: meta.status,
            coverage: meta.coverage,
        }
    } else {
        PrdEntry {
            slug: slug.clone(),
            title: slug,
            status: "unknown".into(),
            coverage: 0.0,
        }
    }
}

// ─── Public command handlers ───────────────────────────────────────

/// `roko prd idea "text"` — append to ideas.md.
pub fn cmd_idea(workdir: &Path, text: &str) -> Result<()> {
    ensure_dirs(workdir)?;
    let path = ideas_path(workdir);
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
    let entry = format!("- {timestamp} — {text}\n");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    std::io::Write::write_all(&mut file, entry.as_bytes())?;
    println!("💡 Captured: {text}");
    Ok(())
}

/// `roko prd list` — list all PRDs, drafts, and ideas.
pub fn cmd_list(workdir: &Path) -> Result<()> {
    ensure_dirs(workdir)?;

    println!("═══ Published PRDs ═══");
    let published = list_md_files(&published_dir(workdir));
    if published.is_empty() {
        println!("  (none)");
    } else {
        for path in &published {
            let entry = read_prd_entry(path);
            let cov = if entry.coverage > 0.0 {
                format!("{:.0}%", entry.coverage * 100.0)
            } else {
                "—".into()
            };
            println!("  {:<35} coverage: {cov}", entry.title);
        }
    }

    println!();
    println!("═══ Drafts ═══");
    let drafts = list_md_files(&drafts_dir(workdir));
    if drafts.is_empty() {
        println!("  (none)");
    } else {
        for path in &drafts {
            let entry = read_prd_entry(path);
            println!("  {:<35} [draft]", entry.title);
        }
    }

    println!();
    let ideas = ideas_path(workdir);
    let idea_count = std::fs::read_to_string(&ideas)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("- "))
        .count();
    println!("═══ Ideas ({idea_count} captured) ═══");
    // Show last 5 ideas
    let content = std::fs::read_to_string(&ideas).unwrap_or_default();
    let ideas_lines: Vec<&str> = content.lines().filter(|l| l.starts_with("- ")).collect();
    let start = ideas_lines.len().saturating_sub(5);
    for line in &ideas_lines[start..] {
        println!("  {line}");
    }
    if ideas_lines.is_empty() {
        println!("  (none)");
    }

    Ok(())
}

/// `roko prd status` — coverage report.
pub fn cmd_status(workdir: &Path, plans_dir: Option<&Path>) -> Result<()> {
    ensure_dirs(workdir)?;

    println!("══��� PRD Coverage Report ═══");
    println!();
    println!(
        "{:<35} {:<12} {:<6} {:<6} {:<8}",
        "PRD", "Status", "Plans", "Tasks", "Done"
    );
    println!(
        "{:<35} {:<12} {:<6} {:<6} {:<8}",
        "───", "──────", "─────", "─────", "────"
    );

    let all_prds: Vec<PathBuf> = list_md_files(&published_dir(workdir))
        .into_iter()
        .chain(list_md_files(&drafts_dir(workdir)))
        .collect();

    // Count tasks across all plans
    let plans_root = plans_dir.map_or_else(|| workspace_plans_dir(workdir), Path::to_path_buf);
    let mut total_plans = 0u32;
    let mut total_tasks = 0u32;
    let mut total_done = 0u32;
    if plans_root.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&plans_root) {
            for entry in entries.flatten() {
                let toml_path = entry.path().join("tasks.toml");
                if toml_path.exists() {
                    total_plans += 1;
                    let content = std::fs::read_to_string(&toml_path).unwrap_or_default();
                    total_tasks = total_tasks.saturating_add(usize_to_u32_saturating(
                        content.matches("status = ").count(),
                    ));
                    total_done = total_done.saturating_add(usize_to_u32_saturating(
                        content.matches("status = \"done\"").count(),
                    ));
                }
            }
        }
    }

    for path in &all_prds {
        let entry = read_prd_entry(path);
        println!(
            "{:<35} {:<12} {:<6} {:<6} {:<8}",
            entry.slug, entry.status, "—", "—", "—"
        );
    }

    if all_prds.is_empty() {
        println!("  (no PRDs yet — run `roko prd draft new \"title\"`)");
    }

    println!();
    println!(
        "Plans: {total_plans}  Tasks: {total_tasks}  Done: {total_done}  \
         Coverage: {:.0}%",
        if total_tasks > 0 {
            f64::from(total_done) / f64::from(total_tasks) * 100.0
        } else {
            0.0
        }
    );

    Ok(())
}

/// `roko prd draft promote <slug>` — move draft to published.
pub async fn cmd_promote(workdir: &Path, slug: &str, auto_execute: bool) -> Result<()> {
    ensure_dirs(workdir)?;
    let src = drafts_dir(workdir).join(format!("{slug}.md"));
    if !src.exists() {
        return Err(anyhow!("draft not found: {}", src.display()));
    }
    let dst = published_dir(workdir).join(format!("{slug}.md"));

    let mut content = std::fs::read_to_string(&src)?;
    content = content.replace("status: draft", "status: published");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    // Update the 'updated' field if present
    if content.contains("updated:") {
        let re_updated =
            regex::Regex::new(r"updated: .*").context("compile updated-field regex")?;
        content = re_updated
            .replace(&content, format!("updated: {today}"))
            .to_string();
    }
    std::fs::write(&dst, &content)?;
    std::fs::remove_file(&src)?;
    println!("✅ Promoted: {}", dst.display());
    if auto_plan_enabled(workdir)? {
        let plans_root = generate_plan_from_prd(slug, &dst, false).await?;
        if auto_execute {
            run_generated_plans(workdir, &plans_root).await?;
        }
    }
    Ok(())
}

async fn run_generated_plans(workdir: &Path, plans_root: &Path) -> Result<()> {
    let resolved = crate::load_layered(workdir)?;
    let metrics = Arc::new(MetricRegistry::new());
    roko_core::obs::register_standard_metrics(&metrics);

    let mut runner =
        crate::PlanRunner::from_plans_dir(plans_root, workdir, resolved.config, metrics, false)
            .await?;
    let _report = runner.run_task_plans(plans_root).await?;
    Ok(())
}

fn auto_plan_enabled(workdir: &Path) -> Result<bool> {
    let roko_toml = workdir.join("roko.toml");
    if roko_toml.is_file() {
        let text = std::fs::read_to_string(&roko_toml)
            .with_context(|| format!("read {}", roko_toml.display()))?;
        let raw: toml::Value =
            toml::from_str(&text).with_context(|| format!("parse {}", roko_toml.display()))?;
        if raw
            .get("prd")
            .and_then(|prd| prd.get("auto_plan"))
            .is_some()
        {
            let cfg: RokoConfig =
                toml::from_str(&text).with_context(|| format!("parse {}", roko_toml.display()))?;
            return Ok(cfg.prd.auto_plan);
        }
    }

    Ok(crate::load_layered(workdir)?.config.auto_plan)
}

/// Generate implementation plans from a published PRD file.
pub async fn generate_plan_from_prd(slug: &str, prd_path: &Path, dry_run: bool) -> Result<PathBuf> {
    let workdir = prd_workdir(prd_path)?;
    let result = async {
        let content = std::fs::read_to_string(prd_path)
            .with_context(|| format!("read {}", prd_path.display()))?;
        let prd_meta = PrdMeta::parse(&content).unwrap_or_default();
        let template_kind =
            crate::plan_generate::PlanTemplateKind::resolve(prd_meta.plan_template.as_deref());
        let template_guidance = crate::plan_generate::render_plan_template_guidance(template_kind);
        println!("📋 Generating plans from PRD: {slug}");

        let dry_run_workdir = if dry_run {
            Some(dry_run_fs::DryRunWorkspace::new(&workdir)?)
        } else {
            None
        };
        let workdir_ref = dry_run_workdir
            .as_ref()
            .map_or(workdir.as_path(), |temp| temp.path());

        let resolved = crate::load_layered(workdir_ref)?;
        let system = crate::plan_generate::build_generator_system_prompt(workdir_ref);
        let plans_root = workspace_plans_dir(workdir_ref);
        let tasks_before = dry_run_fs::snapshot_tasks_files(&plans_root);
        let task_prompt = format!(
            "Read the PRD at {path} and generate implementation plan directories \
             under .roko/plans/. Each REQ-XXX requirement becomes one or more tasks. \
             Each acceptance criterion becomes a task verification command. \
             Search the codebase first to understand what already exists. \
             Create plan.md and tasks.toml files directly, including per-task mcp_servers \
             when a task needs a specific MCP server.\n\n\
             {template_guidance}\n\
             PRD content:\n{content}",
            path = prd_path.display(),
            template_guidance = template_guidance,
            content = content,
        );

        let task_id = format!("prd:plan:{slug}");
        let exit_code = run_agent_logged(
            AgentExecOpts {
                prompt: &task_prompt,
                workdir: workdir_ref,
                model: resolved.config.agent.model.as_deref(),
                effort: Some(resolved.config.agent.effort.as_str()),
                system_prompt: Some(&system),
                resume_session: None,
                env_vars: &resolved.config.agent.env,
            },
            AgentExecEpisode {
                task_kind: "prd-plan-generate",
                task_id: &task_id,
            },
        )
        .await?;
        if exit_code != 0 {
            return Err(anyhow!(
                "plan generation agent failed with exit code {exit_code}"
            ));
        }

        let generated_changed = dry_run_fs::changed_tasks_files(&plans_root, &tasks_before);

        if !dry_run {
            regenerate_old_format_plans(
                workdir_ref,
                resolved.config.agent.model.as_deref(),
                Some(resolved.config.agent.effort.as_str()),
                &resolved.config.agent.env,
                &plans_root,
            )
            .await?;
        }

        let changed = dry_run_fs::changed_tasks_files(&plans_root, &tasks_before);
        if dry_run {
            if changed.is_empty() {
                return Err(anyhow!(
                    "dry-run plan generation did not produce any tasks.toml files"
                ));
            }

            for path in &changed {
                dry_run_fs::validate_and_print_preview(path)?;
            }
        } else {
            dry_run_fs::warn_on_new_or_updated_tasks(&plans_root, &tasks_before);
        }

        let (task_count, estimated_complexity) = generated_plan_stats(&generated_changed)?;
        if task_count > template_kind.max_task_count() {
            eprintln!(
                "⚠️  Generated {task_count} tasks, which exceeds the `{}` template budget of {}",
                template_kind.label(),
                template_kind.max_task_count()
            );
        }
        Ok((
            workspace_plans_dir(&workdir),
            task_count,
            estimated_complexity,
        ))
    }
    .await;

    match result {
        Ok((plans_root, task_count, estimated_complexity)) => {
            if !dry_run
                && let Err(err) = emit_prd_plan_signal(
                    &workdir,
                    Kind::Custom("prd:plan:generated".into()),
                    serde_json::json!({
                        "plan_path": plans_root.display().to_string(),
                        "task_count": task_count,
                        "estimated_complexity": estimated_complexity,
                    }),
                )
                .await
            {
                tracing::warn!("[prd] failed to emit generated-plan signal: {err}");
            }
            Ok(plans_root)
        }
        Err(err) => {
            if !dry_run
                && let Err(signal_err) = emit_prd_plan_signal(
                    &workdir,
                    Kind::Custom("prd:plan:failed".into()),
                    serde_json::json!({
                        "plan_path": workspace_plans_dir(&workdir).display().to_string(),
                        "error": format!("{err:#}"),
                    }),
                )
                .await
            {
                tracing::warn!("[prd] failed to emit failed-plan signal: {signal_err}");
            }
            Err(err)
        }
    }
}

/// Build the system prompt for agent-assisted PRD commands.
///
/// Combines the PRD quality system prompt (from [`crate::prd_prompt`]) with
/// context about existing PRDs and the specific task.
pub fn prd_agent_prompt(workdir: &Path, task: &str) -> String {
    let mut prompt = String::new();

    // Include the PRD quality standards as the foundation
    let _ = writeln!(prompt, "{}", crate::prd_prompt::PRD_SYSTEM_PROMPT);
    let _ = writeln!(prompt, "\n---\n");
    let _ = writeln!(prompt, "## Project workspace: {}\n", workdir.display());

    // Include the master index so the agent knows everything that exists
    crate::index::append_master_index_prompt(
        &mut prompt,
        workdir,
        "## Master Index (what already exists — do NOT duplicate)",
    );

    // Include the PRD index for detailed cross-references
    let prd_index = std::fs::read_to_string(prd_dir(workdir).join("INDEX.md")).unwrap_or_default();
    if !prd_index.is_empty() {
        let _ = writeln!(prompt, "## PRD Index\n{prd_index}\n---\n");
    }

    // Gather existing PRD context
    let _ = writeln!(
        prompt,
        "## Existing PRDs (for cross-references and consistency)\n"
    );
    for dir in [&published_dir(workdir), &drafts_dir(workdir)] {
        for path in list_md_files(dir) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                // Include just the frontmatter + first section as context
                let truncated: String = content.lines().take(30).collect::<Vec<_>>().join("\n");
                let _ = writeln!(prompt, "### {}\n{truncated}\n---\n", path.display());
            }
        }
    }

    // Ideas
    let ideas = std::fs::read_to_string(ideas_path(workdir)).unwrap_or_default();
    if !ideas.is_empty() {
        let _ = writeln!(prompt, "## Recent ideas\n{ideas}\n");
    }

    let _ = writeln!(prompt, "## Your task\n{task}");
    let _ = writeln!(prompt, "\n{}", crate::prd_prompt::PRD_QUALITY_CHECKLIST);
    prompt
}

/// Generate the YAML frontmatter for a new draft.
pub fn new_draft_frontmatter(slug: &str, title: &str) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    format!(
        "---\n\
         id: prd-{slug}\n\
         title: {title}\n\
         status: draft\n\
         version: 1\n\
         created: {today}\n\
         updated: {today}\n\
         depends_on: []\n\
         crates: []\n\
         plans_generated: []\n\
         coverage: 0\n\
         tags: []\n\
         ---\n\n"
    )
}

/// Returns true if a PRD markdown string contains substantive body content.
#[must_use]
pub fn has_substantive_markdown_content(content: &str) -> bool {
    let mut in_frontmatter = false;
    let mut saw_frontmatter = false;

    content.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed == "---" {
            if !saw_frontmatter {
                saw_frontmatter = true;
                in_frontmatter = true;
                return false;
            }
            if in_frontmatter {
                in_frontmatter = false;
                return false;
            }
        }

        if in_frontmatter {
            return false;
        }

        !trimmed.is_empty() && !trimmed.starts_with('#')
    })
}

/// Normalize markdown emitted by an agent and optionally prepend a scaffold.
///
/// If the model returns fenced markdown, the outer code fence is stripped.
/// When `scaffold` is provided and the returned markdown lacks YAML frontmatter,
/// the scaffold is prepended so draft creation can still recover a full PRD file.
#[must_use]
pub fn materialize_agent_markdown_output(output: &str, scaffold: Option<&str>) -> Option<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = strip_markdown_code_fence(trimmed).trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(scaffold) = scaffold
        && !normalized.starts_with("---")
    {
        return Some(format!("{scaffold}\n{normalized}"));
    }

    Some(normalized.to_string())
}

fn strip_markdown_code_fence(output: &str) -> &str {
    let trimmed = output.trim();
    if !trimmed.starts_with("```") {
        return trimmed;
    }

    let Some(first_newline) = trimmed.find('\n') else {
        return trimmed;
    };
    let inner = &trimmed[first_newline + 1..];
    let Some(closing) = inner.rfind("\n```") else {
        return trimmed;
    };
    &inner[..closing]
}

/// Slugify a title.
pub fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[must_use]
fn usize_to_u32_saturating(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn prd_workdir(prd_path: &Path) -> Result<PathBuf> {
    prd_path
        .ancestors()
        .nth(4)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            anyhow!(
                "could not derive workdir from PRD path: {}",
                prd_path.display()
            )
        })
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Agent Self-Improvement"), "agent-self-improvement");
        assert_eq!(slugify("  foo  BAR  "), "foo-bar");
        assert_eq!(slugify("hello"), "hello");
    }

    #[test]
    fn parse_frontmatter() {
        let content = "---\nid: prd-test\ntitle: Test PRD\nstatus: draft\nversion: 2\ncoverage: 0.5\nplan_template = \"strict\"\n---\n\n# Test\n";
        let meta = PrdMeta::parse(content).unwrap();
        assert_eq!(meta.id, "prd-test");
        assert_eq!(meta.title, "Test PRD");
        assert_eq!(meta.status, "draft");
        assert_eq!(meta.version, 2);
        assert!((meta.coverage - 0.5).abs() < f64::EPSILON);
        assert_eq!(meta.plan_template.as_deref(), Some("strict"));
    }

    #[test]
    fn parse_no_frontmatter() {
        assert!(PrdMeta::parse("# Just a heading").is_none());
    }

    #[test]
    fn idea_appends() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        cmd_idea(tmp.path(), "test idea 1").unwrap();
        cmd_idea(tmp.path(), "test idea 2").unwrap();
        let content = std::fs::read_to_string(ideas_path(tmp.path())).unwrap();
        assert!(content.contains("test idea 1"));
        assert!(content.contains("test idea 2"));
    }

    #[test]
    fn list_empty() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        // Should not panic
        cmd_list(tmp.path()).unwrap();
    }

    #[tokio::test]
    async fn promote_moves_file() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        let draft = drafts_dir(tmp.path()).join("test.md");
        std::fs::write(
            &draft,
            "---\nstatus: draft\nupdated: 2020-01-01\n---\n# Test\n",
        )
        .unwrap();
        cmd_promote(tmp.path(), "test", false).await.unwrap();
        assert!(!draft.exists());
        let published = published_dir(tmp.path()).join("test.md");
        assert!(published.exists());
        let content = std::fs::read_to_string(&published).unwrap();
        assert!(content.contains("status: published"));
    }

    #[test]
    fn new_draft_frontmatter_valid() {
        let fm = new_draft_frontmatter("test-prd", "Test PRD");
        assert!(fm.starts_with("---\n"));
        assert!(fm.contains("id: prd-test-prd"));
        assert!(fm.contains("title: Test PRD"));
        assert!(fm.contains("status: draft"));
    }

    #[test]
    fn has_substantive_markdown_content_ignores_headers_only() {
        let content = "---\nid: demo\n---\n# Title\n\n## Overview\n";
        assert!(!has_substantive_markdown_content(content));
    }

    #[test]
    fn has_substantive_markdown_content_detects_body_text() {
        let content = "---\nid: demo\n---\n# Title\n\nActual requirement text.\n";
        assert!(has_substantive_markdown_content(content));
    }

    #[test]
    fn materialize_agent_markdown_output_strips_fences() {
        let output = "```markdown\n---\nid: demo\n---\n# Demo\n\nBody\n```";
        let rendered = materialize_agent_markdown_output(output, None).expect("rendered");
        assert!(rendered.starts_with("---"));
        assert!(rendered.contains("Body"));
        assert!(!rendered.contains("```"));
    }

    #[test]
    fn materialize_agent_markdown_output_prepends_scaffold_when_frontmatter_missing() {
        let rendered = materialize_agent_markdown_output("Body only", Some("---\nid: demo\n---"))
            .expect("rendered");
        assert!(rendered.starts_with("---\nid: demo\n---"));
        assert!(rendered.contains("Body only"));
    }
}
