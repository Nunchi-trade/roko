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
#[path = "plan_validate.rs"]
mod plan_validate;

use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};

use crate::agent_exec::{
    AgentExecEpisode, AgentExecOpts, run_agent_capture_logged, run_agent_logged,
};
use crate::task_parser::TasksFile;
use crate::workspace_paths::{
    drafts_dir, ideas_path, plans_dir as workspace_plans_dir, prd_dir, published_dir,
};
use anyhow::{Context as _, Result, anyhow};
use roko_core::config::schema::RokoConfig;
use roko_core::{Body, Engram, Kind, Provenance, Store};
use roko_fs::FileSubstrate;
use roko_learn::episode_logger::{Episode, EpisodeLogger};
pub use roko_learn::runtime_feedback::{ArtifactValidationReport, GenerationOutcome};
use roko_runtime::event_bus::{PublishOrigin, RokoEvent, global_event_bus};

/// Typed artifact result projected from the current PRD/plan generation outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum ArtifactOutcome {
    Valid {
        artifact_type: String,
        path: PathBuf,
        report: ArtifactValidationReport,
    },
    Invalid {
        artifact_type: String,
        path: Option<PathBuf>,
        report: Option<ArtifactValidationReport>,
    },
    NotProduced {
        artifact_type: String,
        reason: String,
    },
    ValidationUnavailable {
        artifact_type: String,
        path: Option<PathBuf>,
        reason: String,
    },
}

impl ArtifactOutcome {
    /// Adapt the legacy `GenerationOutcome` booleans without changing generation behavior.
    #[must_use]
    pub fn from_generation_outcome(
        artifact_type: impl Into<String>,
        path: Option<PathBuf>,
        outcome: &GenerationOutcome,
    ) -> Self {
        let artifact_type = artifact_type.into();
        if !outcome.process_success {
            return Self::NotProduced {
                artifact_type,
                reason: "generation process failed".to_string(),
            };
        }

        if !outcome.artifact_valid {
            return Self::Invalid {
                artifact_type,
                path,
                report: outcome.validation_report.clone(),
            };
        }

        let Some(path) = path else {
            return Self::NotProduced {
                artifact_type,
                reason: "generation process succeeded but no artifact path was provided"
                    .to_string(),
            };
        };

        match &outcome.validation_report {
            Some(report) => Self::Valid {
                artifact_type,
                path,
                report: report.clone(),
            },
            None => Self::ValidationUnavailable {
                artifact_type,
                path: Some(path),
                reason: "artifact validation report was not available".to_string(),
            },
        }
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid { .. })
    }
}

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
            role: Some("strategist"),
            allowed_tools: None,
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

async fn append_prd_published_episode(
    workdir: &Path,
    slug: &str,
    path: &Path,
    published_at: chrono::DateTime<chrono::Utc>,
    origin: PublishOrigin,
) -> Result<()> {
    let logger = EpisodeLogger::new(workdir.join(".roko").join("episodes.jsonl"));
    let mut episode = Episode::new("roko-cli", slug);
    episode.kind = "prd_published".to_string();
    episode.agent_template = "cli".to_string();
    episode.trigger_kind = "prd_publish".to_string();
    episode.timestamp = published_at;
    episode.started_at = published_at;
    episode.completed_at = published_at;
    episode.success = true;
    episode
        .extra
        .insert("slug".to_string(), serde_json::json!(slug));
    episode.extra.insert(
        "path".to_string(),
        serde_json::json!(path.display().to_string()),
    );
    episode.extra.insert(
        "origin".to_string(),
        serde_json::to_value(origin).unwrap_or(serde_json::Value::Null),
    );
    episode.extra.insert(
        "published_at".to_string(),
        serde_json::json!(published_at.to_rfc3339()),
    );
    logger.append(&episode).await?;
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
    // §14.2: Refuse to silently overwrite an existing published PRD.
    if dst.exists() {
        return Err(anyhow!(
            "published PRD already exists at {}; remove or rename it first",
            dst.display()
        ));
    }

    let mut content = std::fs::read_to_string(&src)?;
    if !has_substantive_markdown_content(&content) {
        return Err(anyhow!(
            "draft has no substantive content; cannot promote. \
             Re-run `roko prd draft edit {slug}` to populate it first."
        ));
    }
    // §14.4: Only replace status within YAML frontmatter, not in body text.
    content = replace_in_frontmatter(&content, "status: draft", "status: published");
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
    let published_at = chrono::Utc::now();
    if let Err(err) =
        append_prd_published_episode(workdir, slug, &dst, published_at, PublishOrigin::Cli).await
    {
        eprintln!("warning: failed to append PRD publish audit event: {err:#}");
    }
    global_event_bus().emit(RokoEvent::PrdPublished {
        slug: slug.to_string(),
        path: dst.clone(),
        published_at,
        origin: PublishOrigin::Cli,
    });
    let _ = maybe_generate_plan_after_promote(workdir, slug, &dst, auto_execute).await?;
    Ok(())
}

async fn maybe_generate_plan_after_promote(
    workdir: &Path,
    slug: &str,
    prd_path: &Path,
    auto_execute: bool,
) -> Result<Option<PathBuf>> {
    maybe_generate_plan_after_promote_with(
        workdir,
        slug.to_string(),
        prd_path.to_path_buf(),
        auto_execute,
        |slug, path, dry_run| async move {
            generate_plan_from_prd_with_outcome(&slug, &path, dry_run, None, None).await
        },
    )
    .await
}

async fn maybe_generate_plan_after_promote_with<F, Fut>(
    workdir: &Path,
    slug: String,
    prd_path: PathBuf,
    auto_execute: bool,
    generator: F,
) -> Result<Option<PathBuf>>
where
    F: FnOnce(String, PathBuf, bool) -> Fut,
    Fut: Future<Output = Result<(PathBuf, GenerationOutcome)>>,
{
    if !auto_plan_enabled(workdir)? {
        return Ok(None);
    }

    let prd_path_display = prd_path.display().to_string();
    match generator(slug, prd_path.clone(), false).await {
        Ok((plans_root, outcome)) => {
            if outcome.fully_successful() {
                println!("Plan generated: {}", plans_root.display());
                if auto_execute {
                    run_generated_plans(workdir, &plans_root).await?;
                }
            } else if outcome.process_success {
                eprintln!(
                    "warning: plan generation completed but artifact validation failed ({})",
                    outcome.status_label()
                );
            } else {
                eprintln!(
                    "warning: plan generation reported {} for {}",
                    outcome.status_label(),
                    prd_path_display
                );
            }
            if auto_execute && !outcome.fully_successful() {
                eprintln!(
                    "warning: skipping auto-execute because generated artifact was not fully successful"
                );
            }
            Ok(Some(plans_root))
        }
        Err(err) => {
            // §14.6: Surface actionable feedback — the PRD was promoted
            // but no plan was generated. Tell the user how to recover.
            eprintln!("error: auto plan generation failed for '{prd_path_display}': {err:#}");
            eprintln!(
                "  → PRD is promoted. Run `roko prd plan <slug>` manually to generate a plan."
            );
            Ok(None)
        }
    }
}

async fn run_generated_plans(workdir: &Path, plans_root: &Path) -> Result<()> {
    let plans = crate::runner::load_plans(plans_root)?;
    let roko_config = roko_core::config::loader::load_config_unified(workdir)
        .with_context(|| format!("load roko config from {}", workdir.display()))?;
    let run_config = crate::runner::RunConfig::from_roko_config(
        workdir.to_path_buf(),
        plans_root.to_path_buf(),
        roko_config,
    );
    let state_hub = crate::state_hub::StateHub::default_capacity();
    let report = crate::runner::run(
        plans,
        &run_config,
        &state_hub,
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    if !report.all_succeeded() {
        return Err(anyhow!("generated plan execution failed"));
    }
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
    let (plans_root, _) =
        generate_plan_from_prd_with_outcome(slug, prd_path, dry_run, None, None).await?;
    Ok(plans_root)
}

/// Generate implementation plans from a published PRD file using an
/// explicit resolved model key from the caller.
pub async fn generate_plan_from_prd_with_model(
    slug: &str,
    prd_path: &Path,
    dry_run: bool,
    model: Option<&str>,
) -> Result<PathBuf> {
    let (plans_root, _) =
        generate_plan_from_prd_with_outcome(slug, prd_path, dry_run, None, model).await?;
    Ok(plans_root)
}

/// Generate implementation plans from a published PRD file with optional
/// failure context injected into the planning prompt.
pub async fn generate_plan_from_prd_with_failure_context(
    slug: &str,
    prd_path: &Path,
    dry_run: bool,
    failure_context: Option<&str>,
    model: Option<&str>,
) -> Result<PathBuf> {
    let (plans_root, _) =
        generate_plan_from_prd_with_outcome(slug, prd_path, dry_run, failure_context, model)
            .await?;
    Ok(plans_root)
}

async fn generate_plan_from_prd_with_outcome(
    slug: &str,
    prd_path: &Path,
    dry_run: bool,
    failure_context: Option<&str>,
    model: Option<&str>,
) -> Result<(PathBuf, GenerationOutcome)> {
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
        let system = augment_generator_system_prompt(
            crate::plan_generate::build_generator_system_prompt(workdir_ref),
            failure_context,
        );
        let plans_root = workspace_plans_dir(workdir_ref);
        let tasks_before = dry_run_fs::snapshot_tasks_files(&plans_root);

        // Build repo context to ground the planning agent in actual repository
        // structure. Keywords come from the PRD slug and title.
        let prd_title = prd_meta.title.as_str();
        let mut prd_feature_keywords: Vec<String> = slug
            .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
            .chain(prd_title.split(|c: char| c == '-' || c == '_' || c.is_whitespace()))
            .filter(|w| w.len() >= 3)
            .map(|w| w.to_lowercase())
            .collect();
        prd_feature_keywords.sort_unstable();
        prd_feature_keywords.dedup();
        prd_feature_keywords.truncate(10);
        let prd_keyword_refs: Vec<&str> = prd_feature_keywords.iter().map(String::as_str).collect();
        // Skip repo context scanning for workspaces without source code
        // (e.g. freshly-initialized workspaces from `roko init`).
        let has_source_code = workdir_ref.join("src").is_dir()
            || workdir_ref.join("crates").is_dir()
            || workdir_ref.join("lib").is_dir()
            || workdir_ref.join("Cargo.toml").is_file()
            || workdir_ref.join("package.json").is_file();
        let repo_context_section: Option<String> = if has_source_code {
            match crate::repo_context::build_repo_context(workdir_ref, &prd_keyword_refs).await {
                Ok(repo_context) => {
                    if !repo_context.context_root_verified {
                        eprintln!(
                            "warning: repository context not verified for keywords {:?}; \
                             generated plan may reference nonexistent code.",
                            prd_feature_keywords
                        );
                    }
                    Some(repo_context.to_prompt_section())
                }
                Err(err) => {
                    eprintln!(
                        "warning: repository context unavailable for keywords {:?}: {err}",
                        prd_feature_keywords
                    );
                    None
                }
            }
        } else {
            None // Empty workspace — skip context scanning
        };
        let prd_context_suffix = repo_context_section
            .as_deref()
            .map(|ctx| format!("\n\n---\n\n{ctx}"))
            .unwrap_or_default();

        // Trim PRD content to keep prompt size manageable for smaller models.
        let max_prd_chars = 8000;
        let trimmed_content = if content.len() > max_prd_chars {
            let boundary = content.floor_char_boundary(max_prd_chars);
            format!(
                "{}\n\n[PRD content truncated at {max_prd_chars} chars]",
                &content[..boundary]
            )
        } else {
            content.clone()
        };

        let task_prompt = format!(
            "Generate an implementation plan from the PRD below.\n\n\
             IMPORTANT: The PRD content is included inline — do NOT read {path} \
             again. You may read up to 5 codebase files to understand existing \
             structure, but then you MUST produce your output.\n\n\
             Each REQ-XXX requirement becomes one or more tasks. \
             Each acceptance criterion becomes a task verification command.\n\n\
             Do NOT create files directly. Instead, output the plan content \
             as follows:\n\n\
             1. Output a fenced block tagged `toml` containing the tasks.toml content.\n\
             2. Optionally output a fenced block tagged `plan.md` containing the plan narrative.\n\n\
             Include per-task mcp_servers when a task needs a specific MCP server.\n\n\
             {template_guidance}\n\
             PRD content:\n{trimmed_content}{prd_context_suffix}",
            path = prd_path.display(),
            template_guidance = template_guidance,
            trimmed_content = trimmed_content,
            prd_context_suffix = prd_context_suffix,
        );

        let task_id = format!("prd:plan:{slug}");
        let spinner = crate::spinner::cli_spinner(format!("Generating plan from PRD: {slug}"));
        let (exit_code, output) = run_agent_capture_logged(
            AgentExecOpts {
                prompt: &task_prompt,
                workdir: workdir_ref,
                model: model.or_else(|| resolved.config.agent.model.as_deref()),
                effort: Some(resolved.config.agent.effort.as_str()),
                system_prompt: Some(&system),
                resume_session: None,
                env_vars: &resolved.config.agent.env,
                role: Some("strategist"),
                allowed_tools: Some("Read,Grep,Glob"),
            },
            AgentExecEpisode {
                task_kind: "prd-plan-generate",
                task_id: &task_id,
            },
        )
        .await?;
        if exit_code == 0 {
            spinner.finish_with_message(format!("Plan generated for: {slug}"));
        } else {
            spinner.finish_with_message("Plan generation failed");
        }
        tracing::info!(
            exit_code,
            output_len = output.len(),
            output_trimmed_len = output.trim().len(),
            "prd plan: agent returned"
        );
        if exit_code != 0 {
            return Err(anyhow!(
                "plan generation agent failed with exit code {exit_code} \
                 ({} bytes of output)",
                output.len()
            ));
        }
        if output.trim().is_empty() {
            return Err(anyhow!(
                "plan generation agent returned empty output for {slug} — \
                 the model may not support the required output format"
            ));
        }

        // Write files from agent output (strategist can't write files directly).
        // Try fenced ```toml block first, then ```tasks.toml, then unfenced TOML.
        let toml_content = extract_fenced_block(&output, "toml")
            .or_else(|| extract_fenced_block(&output, "tasks.toml"))
            .or_else(|| extract_toml_content_fallback(&output));
        tracing::info!(
            has_toml_block = toml_content.is_some(),
            toml_block_len = toml_content.map(|s| s.len()).unwrap_or(0),
            "prd plan: fenced block extraction"
        );
        if let Some(toml_content) = toml_content {
            // Post-generation validation: fix typos, bad model hints, truncated
            // slug, and reject structurally broken TOML before writing to disk.
            let validated_toml = validate_and_fix_generated_plan(
                toml_content,
                slug,
                &resolved.config.models,
                resolved.config.agent.model.as_deref(),
            )
            .with_context(|| {
                format!(
                    "post-generation validation failed for plan '{slug}'; \
                     the agent produced invalid TOML"
                )
            })?;

            let plan_dir = plans_root.join(slug);
            std::fs::create_dir_all(&plan_dir)
                .with_context(|| format!("create plan dir {}", plan_dir.display()))?;
            std::fs::write(plan_dir.join("tasks.toml"), &validated_toml)
                .with_context(|| format!("write tasks.toml to {}", plan_dir.display()))?;
            println!(
                "📋 Wrote tasks.toml ({} bytes) to {}",
                validated_toml.len(),
                plan_dir.display()
            );
            let plan_md_content = extract_fenced_block(&output, "plan.md")
                .or_else(|| extract_fenced_block(&output, "markdown"))
                .or_else(|| extract_fenced_block(&output, "md"));
            if let Some(plan_md) = plan_md_content {
                std::fs::write(plan_dir.join("plan.md"), plan_md)
                    .with_context(|| format!("write plan.md to {}", plan_dir.display()))?;
                println!(
                    "📋 Wrote plan.md ({} bytes) to {}",
                    plan_md.len(),
                    plan_dir.display()
                );
            } else {
                // Write minimal plan.md so plan discovery tools can find this directory.
                let minimal_plan_md = format!(
                    "---\nplan: {slug}\ntitle: {slug}\n---\n\n# {slug}\n\nGenerated plan.\n"
                );
                std::fs::write(plan_dir.join("plan.md"), &minimal_plan_md)
                    .with_context(|| format!("write plan.md to {}", plan_dir.display()))?;
            }
        } else {
            // No fenced toml block found — fail with an actionable error rather
            // than silently exiting 0 with no tasks.toml produced.
            let preview: String = output.chars().take(500).collect();
            let has_toml_like = output.contains("[meta]") || output.contains("[[task]]");
            let toml_hint = if has_toml_like {
                "\nhint: The model output TOML without proper fencing. \
                 Try a more capable model or check the plan_generate system prompt."
            } else {
                ""
            };
            return Err(anyhow!(
                "Plan generation failed: no tasks.toml was produced.\n\
                 The agent output ({} bytes) did not contain a fenced ```toml block.\n\
                 Output preview:\n---\n{preview}\n---\n\
                 hint: Try again, or create plans/{slug}/tasks.toml manually.{toml_hint}",
                output.len()
            ));
        }

        let generated_changed = dry_run_fs::changed_tasks_files(&plans_root, &tasks_before);

        if !dry_run {
            if let Err(e) = regenerate_old_format_plans(
                workdir_ref,
                model.or_else(|| resolved.config.agent.model.as_deref()),
                Some(resolved.config.agent.effort.as_str()),
                &resolved.config.agent.env,
                &plans_root,
            )
            .await
            {
                eprintln!("warning: old-format plan regeneration failed (non-fatal): {e}");
            }
        }

        let changed = dry_run_fs::changed_tasks_files(&plans_root, &tasks_before);
        let mut artifact_valid = true;
        let mut validation_report: Option<ArtifactValidationReport> = None;

        if dry_run {
            if changed.is_empty() {
                artifact_valid = false;
                eprintln!("warning: dry-run plan generation did not produce any tasks.toml files");
            } else {
                for path in &changed {
                    if let Err(err) = dry_run_fs::validate_and_print_preview(path) {
                        artifact_valid = false;
                        eprintln!(
                            "warning: dry-run validation failed for {}: {err:#}",
                            path.display()
                        );
                    }
                }
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

        match self::plan_validate::validate_plans_dir_with_workdir(
            &plans_root,
            None,
            Some(workdir_ref),
        ) {
            Ok(report) => {
                if report.totals.errors > 0 {
                    artifact_valid = false;
                    eprintln!(
                        "warning: artifact validation found {} error(s) and {} warning(s) for {}",
                        report.totals.errors, report.totals.warnings, slug
                    );
                }
                validation_report = serde_json::to_value(&report).ok();
                if validation_report.is_none() {
                    artifact_valid = false;
                }
            }
            Err(err) => {
                artifact_valid = false;
                eprintln!(
                    "warning: artifact validation could not be completed for {}: {err:#}",
                    slug
                );
            }
        }

        let outcome = GenerationOutcome {
            process_success: true,
            artifact_valid,
            validation_report,
        };

        Ok((
            workspace_plans_dir(&workdir),
            task_count,
            estimated_complexity,
            outcome,
        ))
    }
    .await;

    match result {
        Ok((plans_root, task_count, estimated_complexity, outcome)) => {
            if !dry_run {
                let signal_kind = if outcome.fully_successful() {
                    Some(Kind::Custom("prd:plan:generated".into()))
                } else if outcome.process_success {
                    Some(Kind::Custom("prd:plan:partial_success".into()))
                } else {
                    Some(Kind::Custom("prd:plan:failed".into()))
                };

                if let Some(kind) = signal_kind
                    && let Err(err) = emit_prd_plan_signal(
                        &workdir,
                        kind,
                        serde_json::json!({
                            "plan_path": plans_root.display().to_string(),
                            "task_count": task_count,
                            "estimated_complexity": estimated_complexity,
                            "status": outcome.status_label(),
                            "process_success": outcome.process_success,
                            "artifact_valid": outcome.artifact_valid,
                            "validation_report": outcome.validation_report,
                        }),
                    )
                    .await
                {
                    tracing::warn!("[prd] failed to emit plan signal: {err}");
                }
            }
            Ok((plans_root, outcome))
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

pub(crate) fn augment_generator_system_prompt(
    mut system_prompt: String,
    failure_context: Option<&str>,
) -> String {
    let Some(failure_context) = failure_context.map(str::trim).filter(|ctx| !ctx.is_empty()) else {
        return system_prompt;
    };

    system_prompt.push_str("\n\n## Failure context for replanning\n");
    system_prompt.push_str(failure_context);
    system_prompt.push_str(
        "\n\nUse this failure context to revise the plan first. Do not reproduce the same task shape.\n",
    );
    system_prompt
}

/// Build the system prompt for agent-assisted PRD commands.
///
/// Combines the PRD quality system prompt (from [`crate::prd_prompt`]) with
/// context about existing PRDs and the specific task.
/// Default limit for how many PRDs to include in the agent system prompt.
const PRD_CONTEXT_LIMIT: usize = 20;

pub fn prd_agent_prompt(workdir: &Path, task: &str) -> String {
    prd_agent_prompt_with_limit(workdir, task, PRD_CONTEXT_LIMIT)
}

pub fn prd_agent_prompt_with_limit(workdir: &Path, task: &str, prd_limit: usize) -> String {
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

    // Gather existing PRD context (most recent first, limited to prd_limit)
    let _ = writeln!(
        prompt,
        "## Existing PRDs (for cross-references and consistency)\n"
    );
    let mut all_prd_files: Vec<PathBuf> = Vec::new();
    for dir in [&published_dir(workdir), &drafts_dir(workdir)] {
        all_prd_files.extend(list_md_files(dir));
    }
    // Sort by modification time descending so newest PRDs come first
    all_prd_files.sort_by(|a, b| {
        let mtime = |p: &Path| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        };
        mtime(b).cmp(&mtime(a))
    });
    let total = all_prd_files.len();
    for path in all_prd_files.into_iter().take(prd_limit) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            // Include just the frontmatter + first section as context
            let truncated: String = content.lines().take(30).collect::<Vec<_>>().join("\n");
            let _ = writeln!(prompt, "### {}\n{truncated}\n---\n", path.display());
        }
    }
    if total > prd_limit {
        let _ = writeln!(
            prompt,
            "_({} older PRDs omitted — see .roko/prd/ for full list)_\n",
            total - prd_limit,
        );
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

/// Replace `from` with `to` only within the YAML frontmatter (between first
/// pair of `---` delimiters). Body content is left untouched.
fn replace_in_frontmatter(content: &str, from: &str, to: &str) -> String {
    // Frontmatter is the region between the first `---\n` and the next `---\n`.
    if let Some(start) = content.find("---") {
        let after_first = start + 3;
        if let Some(end_offset) = content[after_first..].find("---") {
            let end = after_first + end_offset + 3;
            let frontmatter = &content[..end];
            let body = &content[end..];
            return format!("{}{}", frontmatter.replace(from, to), body);
        }
    }
    // No frontmatter delimiters found — fall back to global replace.
    content.replace(from, to)
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

/// Extract the contents of a fenced code block tagged with `tag` from agent output.
///
/// Looks for `` ```tag `` or `` ```<tag> `` and returns the inner content.
/// Handles nested fences by matching the closing `` ``` `` that sits alone
/// on a line (possibly with trailing whitespace).
fn extract_fenced_block<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let fence_plain = format!("```{tag}");
    let fence_angle = format!("```<{tag}>");
    let start = text
        .find(&fence_plain)
        .or_else(|| text.find(&fence_angle))?;
    let after_fence = &text[start..];
    let newline = after_fence.find('\n')? + 1;
    let inner = &after_fence[newline..];

    // Find a closing ``` that is alone on a line (not followed by more text
    // like ```toml or ```python — those are nested openers).
    let mut search_from = 0;
    loop {
        let candidate = inner[search_from..].find("\n```")?;
        let abs = search_from + candidate;
        let after_ticks = abs + 4; // position after \n```
        // Closing fence: either end-of-string, or next char is \n or whitespace-then-\n
        let rest = &inner[after_ticks..];
        if rest.is_empty()
            || rest.starts_with('\n')
            || rest.trim_start().starts_with('\n')
            || rest.trim_start().is_empty()
        {
            let content = inner[..abs].trim();
            return if content.is_empty() {
                None
            } else {
                Some(&inner[..abs])
            };
        }
        search_from = after_ticks;
    }
}

/// Fallback TOML extraction for agent output that lacks fenced code blocks.
///
/// Scans for a `[meta]` section header and returns everything from that point
/// to the end of the string, provided it also contains at least one `[[task]]`.
fn extract_toml_content_fallback(output: &str) -> Option<&str> {
    let meta_start = output.find("[meta]")?;
    // Find the start of the line containing [meta]
    let line_start = output[..meta_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let candidate = output[line_start..].trim();
    if candidate.contains("[[task]]") {
        Some(candidate)
    } else {
        None
    }
}

// ---- post-generation plan TOML validation --------------------------------

/// Known field names for the `[meta]` section.
const KNOWN_META_FIELDS: &[&str] = &[
    "plan",
    "iteration",
    "total",
    "done",
    "status",
    "max_parallel",
    "estimated_total_minutes",
    "skip_enrichment",
];

/// Required field names for the `[meta]` section.
const REQUIRED_META_FIELDS: &[&str] = &["plan", "total", "status"];

/// Known field names for a `[[task]]` entry.
const KNOWN_TASK_FIELDS: &[&str] = &[
    "id",
    "title",
    "description",
    "role",
    "status",
    "tier",
    "frequency",
    "model_hint",
    "replan_strategy",
    "max_loc",
    "files",
    "write_files",
    "allowed_tools",
    "denied_tools",
    "mcp_servers",
    "depends_on",
    "depends_on_plan",
    "split_into",
    "context",
    "verify",
    "timeout_secs",
    "max_retries",
    "acceptance",
    "acceptance_contract",
    "domain",
    "gate_rung",
];

/// Required field names for each `[[task]]`.
const REQUIRED_TASK_FIELDS: &[&str] = &["id", "title", "status", "role", "tier"];

/// Known field names for each `[[task.verify]]` entry.
const KNOWN_VERIFY_FIELDS: &[&str] = &["phase", "command", "fail_msg", "timeout_ms"];

/// Required field names for each `[[task.verify]]` entry.
const REQUIRED_VERIFY_FIELDS: &[&str] = &["phase", "command"];

/// Common typos the LLM produces and their corrections.
const FIELD_TYPO_CORRECTIONS: &[(&str, &str)] = &[
    ("pha", "phase"),
    ("phas", "phase"),
    ("cmd", "command"),
    ("comand", "command"),
    ("commnad", "command"),
    ("commmand", "command"),
    ("descrption", "description"),
    ("descripion", "description"),
    ("desc", "description"),
    ("stat", "status"),
    ("staus", "status"),
    ("tite", "title"),
    ("titl", "title"),
    ("modle_hint", "model_hint"),
    ("model", "model_hint"),
    ("modelhint", "model_hint"),
    ("depnds_on", "depends_on"),
    ("dependson", "depends_on"),
    ("depend_on", "depends_on"),
    ("filse", "files"),
    ("fles", "files"),
    ("verfy", "verify"),
    ("verfiy", "verify"),
    ("tiemout_secs", "timeout_secs"),
    ("fail_message", "fail_msg"),
    ("failure_msg", "fail_msg"),
    ("timeout", "timeout_ms"),
];

/// Suggest a correction for a possibly-misspelled field.
/// Returns an owned `String` to avoid lifetime issues with the caller.
fn suggest_field_correction(field: &str, known: &[&str]) -> Option<String> {
    // Check explicit typo table first.
    for &(typo, correction) in FIELD_TYPO_CORRECTIONS {
        if field == typo {
            return Some(correction.to_string());
        }
    }
    // Fallback: find the closest known field by edit distance (threshold <= 2).
    let mut best: Option<(&str, usize)> = None;
    for &known_field in known {
        let dist = strsim_distance(field, known_field);
        if dist > 0 && dist <= 2 {
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((known_field, dist));
            }
        }
    }
    best.map(|(s, _)| s.to_string())
}

/// Minimal Levenshtein distance (no allocations for short strings).
fn strsim_distance(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let m = a_bytes.len();
    let n = b_bytes.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Check whether a model identifier is present in the config model table.
fn model_in_config(
    model: &str,
    models: &std::collections::HashMap<String, roko_core::config::schema::ModelProfile>,
) -> bool {
    models.contains_key(model) || models.values().any(|p| p.slug == model)
}

/// Validate and fix a generated plan TOML string.
///
/// Checks:
/// 1. TOML syntax.
/// 2. Required fields in `[meta]` and `[[task]]`.
/// 3. Unknown / misspelled fields (with suggested corrections applied).
/// 4. `model_hint` values validated against the config model table.
/// 5. `meta.plan` matched against the expected slug.
///
/// On fixable issues the TOML is patched and a warning is logged to stderr.
/// On unfixable issues an error is returned.
fn validate_and_fix_generated_plan(
    toml_str: &str,
    slug: &str,
    models: &std::collections::HashMap<String, roko_core::config::schema::ModelProfile>,
    default_model: Option<&str>,
) -> Result<String> {
    // 1. Parse syntax.
    let mut root: toml::Value =
        toml::from_str(toml_str).map_err(|e| anyhow!("generated plan has invalid TOML: {e}"))?;

    let root_table = root
        .as_table_mut()
        .ok_or_else(|| anyhow!("generated plan TOML root is not a table"))?;

    let mut errors: Vec<String> = Vec::new();

    // -- [meta] validation ---------------------------------------------------
    if let Some(meta_val) = root_table.get_mut("meta") {
        if let Some(meta) = meta_val.as_table_mut() {
            // Flag unknown meta fields.
            let meta_keys: Vec<String> = meta.keys().cloned().collect();
            for key in &meta_keys {
                if !KNOWN_META_FIELDS.contains(&key.as_str()) {
                    if let Some(correction) = suggest_field_correction(key, KNOWN_META_FIELDS) {
                        if let Some(value) = meta.remove(key.as_str()) {
                            eprintln!(
                                "warning: [meta] field '{key}' is unknown; \
                                 corrected to '{correction}'"
                            );
                            meta.insert(correction, value);
                        }
                    } else {
                        eprintln!("warning: [meta] has unknown field '{key}'");
                    }
                }
            }
            // Check required meta fields.
            for &required in REQUIRED_META_FIELDS {
                match meta.get(required) {
                    None => errors.push(format!("[meta] is missing required field '{required}'")),
                    Some(v) if v.as_str().is_some_and(|s| s.trim().is_empty()) => {
                        errors.push(format!("[meta].{required} is empty"));
                    }
                    _ => {}
                }
            }
            // Fix meta.plan if truncated or wrong.
            if let Some(plan_val) = meta.get("plan") {
                if let Some(plan_str) = plan_val.as_str() {
                    if plan_str != slug {
                        if slug.starts_with(plan_str) {
                            eprintln!(
                                "warning: meta.plan '{plan_str}' appears truncated; \
                                 corrected to '{slug}'"
                            );
                        } else {
                            eprintln!(
                                "warning: meta.plan '{plan_str}' does not match \
                                 expected slug '{slug}'; corrected"
                            );
                        }
                        meta.insert("plan".to_string(), toml::Value::String(slug.to_string()));
                    }
                }
            }
        }
    } else {
        errors.push("[meta] section is missing".to_string());
    }

    // -- [[task]] validation --------------------------------------------------
    if let Some(tasks_val) = root_table.get_mut("task") {
        if let Some(tasks) = tasks_val.as_array_mut() {
            if tasks.is_empty() {
                errors.push("[[task]] array is present but empty".to_string());
            }
            for (i, task_val) in tasks.iter_mut().enumerate() {
                if let Some(task) = task_val.as_table_mut() {
                    let task_id_label: String = task
                        .get("id")
                        .and_then(toml::Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| format!("task #{}", i + 1));

                    // Flag unknown task fields.
                    let task_keys: Vec<String> = task.keys().cloned().collect();
                    for key in &task_keys {
                        if !KNOWN_TASK_FIELDS.contains(&key.as_str()) {
                            if let Some(correction) =
                                suggest_field_correction(key, KNOWN_TASK_FIELDS)
                            {
                                if let Some(value) = task.remove(key.as_str()) {
                                    eprintln!(
                                        "warning: {task_id_label}: field '{key}' is unknown; \
                                         corrected to '{correction}'"
                                    );
                                    task.insert(correction, value);
                                }
                            } else {
                                eprintln!(
                                    "warning: {task_id_label}: unknown field '{key}'"
                                );
                            }
                        }
                    }

                    // Check required task fields.
                    for &required in REQUIRED_TASK_FIELDS {
                        match task.get(required) {
                            None => errors.push(format!(
                                "{task_id_label} is missing required field '{required}'"
                            )),
                            Some(v) if v.as_str().is_some_and(|s| s.trim().is_empty()) => {
                                errors.push(format!(
                                    "{task_id_label}: field '{required}' is empty"
                                ));
                            }
                            _ => {}
                        }
                    }

                    // Validate model_hint.
                    if let Some(hint_val) = task.get("model_hint").cloned() {
                        if let Some(hint) = hint_val.as_str() {
                            let normalized = crate::task_parser::normalize_model_alias(hint);
                            if !model_in_config(normalized, models) {
                                let replacement = default_model.unwrap_or("claude-sonnet-4-6");
                                eprintln!(
                                    "warning: {task_id_label}: model_hint '{hint}' \
                                     not in config, using '{replacement}'"
                                );
                                task.insert(
                                    "model_hint".to_string(),
                                    toml::Value::String(replacement.to_string()),
                                );
                            } else if normalized != hint {
                                // Replace short alias with canonical name.
                                task.insert(
                                    "model_hint".to_string(),
                                    toml::Value::String(normalized.to_string()),
                                );
                            }
                        }
                    }

                    // Validate [[task.verify]] sub-entries.
                    if let Some(verify_val) = task.get_mut("verify") {
                        if let Some(steps) = verify_val.as_array_mut() {
                            for (si, step_val) in steps.iter_mut().enumerate() {
                                if let Some(step) = step_val.as_table_mut() {
                                    let step_keys: Vec<String> = step.keys().cloned().collect();
                                    for key in &step_keys {
                                        if !KNOWN_VERIFY_FIELDS.contains(&key.as_str()) {
                                            if let Some(correction) = suggest_field_correction(
                                                key,
                                                KNOWN_VERIFY_FIELDS,
                                            ) {
                                                if let Some(value) = step.remove(key.as_str()) {
                                                    eprintln!(
                                                        "warning: {task_id_label} verify[{si}]: \
                                                         field '{key}' corrected to '{correction}'"
                                                    );
                                                    step.insert(correction, value);
                                                }
                                            } else {
                                                eprintln!(
                                                    "warning: {task_id_label} verify[{si}]: \
                                                     unknown field '{key}'"
                                                );
                                            }
                                        }
                                    }

                                    // Check required verify fields.
                                    for &required in REQUIRED_VERIFY_FIELDS {
                                        if step
                                            .get(required)
                                            .and_then(toml::Value::as_str)
                                            .is_none_or(|s| s.trim().is_empty())
                                        {
                                            errors.push(format!(
                                                "{task_id_label} verify[{si}]: \
                                                 missing required field '{required}'"
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    } else {
        errors.push("[[task]] array is missing".to_string());
    }

    if !errors.is_empty() {
        let joined = errors.join("\n  - ");
        return Err(anyhow!(
            "generated plan TOML has {n} validation error(s):\n  - {joined}",
            n = errors.len()
        ));
    }

    // Serialize the (possibly patched) TOML back to a string.
    let mut serialized = toml::to_string_pretty(&root)
        .map_err(|e| anyhow!("failed to re-serialize fixed plan TOML: {e}"))?;

    // -- Angle-bracket placeholder replacement --------------------------------
    // LLMs sometimes emit literal `<relevant-lib>`, `<crate>`, `<path>` etc.
    // instead of concrete values. Replace known placeholders with slug-derived
    // defaults so downstream tooling doesn't choke on them.
    let path_default = format!("crates/{slug}/src/lib.rs");
    let replacements: &[(&str, &str)] = &[
        ("<relevant-lib>", slug),
        ("<binary-crate>", slug),
        ("<crate>", slug),
        ("<module>", "lib"),
        ("<path>", &path_default),
        ("<file>", &path_default),
        ("<test_name>", "test_placeholder"),
    ];
    for &(placeholder, replacement) in replacements {
        if serialized.contains(placeholder) {
            eprintln!(
                "plan validation: replaced placeholder '{}' with '{}'",
                placeholder, replacement
            );
            serialized = serialized.replace(placeholder, replacement);
        }
    }

    // If we did any replacements, verify the TOML still parses.
    if replacements
        .iter()
        .any(|(ph, _)| toml_str.contains(*ph))
    {
        let _: toml::Value = toml::from_str(&serialized)
            .map_err(|e| anyhow!("TOML became invalid after placeholder replacement: {e}"))?;
    }

    Ok(serialized)
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

// ─── PRD artifact validation ───────────────────────────────────────

/// Severity of a PRD validation issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrdValidationSeverity {
    /// Blocking: `artifact_valid` is set to `false`.
    Error,
    /// Non-blocking: printed as a warning, does not affect `artifact_valid`.
    Warning,
}

/// A single validation issue found in a generated PRD.
#[derive(Debug, Clone)]
pub struct PrdValidationIssue {
    pub severity: PrdValidationSeverity,
    pub category: &'static str,
    pub message: String,
}

/// Outcome of post-generation PRD artifact validation.
///
/// `artifact_valid = false` means the PRD should not be accepted as a
/// successful generation outcome — learning gates should withhold rewards.
#[derive(Debug, Clone)]
pub struct PrdArtifactReport {
    /// Slug of the PRD being validated.
    pub slug: String,
    /// Whether the underlying agent process succeeded (exit 0).
    pub process_success: bool,
    /// Whether the artifact itself passes all blocking checks.
    ///
    /// Set to `false` when any [`PrdValidationSeverity::Error`] issue is found.
    pub artifact_valid: bool,
    /// All issues found during validation (errors + warnings).
    pub issues: Vec<PrdValidationIssue>,
}

impl PrdArtifactReport {
    fn new(slug: &str, process_success: bool) -> Self {
        Self {
            slug: slug.to_string(),
            process_success,
            artifact_valid: true,
            issues: Vec::new(),
        }
    }

    fn push(&mut self, issue: PrdValidationIssue) {
        if issue.severity == PrdValidationSeverity::Error {
            self.artifact_valid = false;
        }
        self.issues.push(issue);
    }

    /// Print all issues to stderr and a summary to stdout.
    pub fn print_summary(&self) {
        for issue in &self.issues {
            let label = match issue.severity {
                PrdValidationSeverity::Error => "ERROR",
                PrdValidationSeverity::Warning => "WARNING",
            };
            eprintln!("[{}] {}: {}", label, issue.category, issue.message);
        }
        let errors = self
            .issues
            .iter()
            .filter(|i| i.severity == PrdValidationSeverity::Error)
            .count();
        let warnings = self
            .issues
            .iter()
            .filter(|i| i.severity == PrdValidationSeverity::Warning)
            .count();
        if self.artifact_valid {
            println!("PRD artifact validation: PASSED ({warnings} warnings)");
        } else {
            println!("PRD artifact validation: FAILED ({errors} errors, {warnings} warnings)");
        }
    }
}

/// Extract a `## <heading>` markdown section body (case-insensitive).
///
/// Returns the lines between the matched heading and the next `##`-level
/// heading, joined as a single string. Returns `None` when the heading is
/// not found or the matched section is empty.
fn extract_prd_section(content: &str, heading: &str) -> Option<String> {
    let heading_lower = heading.to_lowercase();
    let mut in_section = false;
    let mut lines: Vec<&str> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let trimmed_lower = trimmed.to_lowercase();
        if trimmed_lower.starts_with("## ") {
            if in_section {
                // Reached the next ## heading — stop collecting.
                break;
            }
            let heading_text = trimmed_lower.trim_start_matches("## ").trim();
            if heading_text.starts_with(heading_lower.as_str()) {
                in_section = true;
                continue;
            }
        } else if in_section {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Extract all relative file paths referenced in the grounding section text.
///
/// A path is recognised when it starts with `crates/`, `src/`, `tests/`,
/// `plans/`, `apps/`, or `docs/` — i.e. plausible workspace-relative paths.
fn extract_referenced_paths(grounding_text: &str) -> Vec<String> {
    let prefixes = ["crates/", "src/", "tests/", "plans/", "apps/", "docs/"];
    let mut paths: Vec<String> = Vec::new();
    for line in grounding_text.lines() {
        for word in line.split_whitespace() {
            // Strip leading punctuation like `-`, `*`, `` ` ``, `(`.
            let word = word
                .trim_start_matches(['-', '*', '`', '(', '['])
                .trim_end_matches(['`', ')', ']', ',', '.']);
            if prefixes.iter().any(|p| word.starts_with(p)) {
                paths.push(word.to_string());
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Check that a generated PRD contains the required `## Repository Grounding`
/// section.
///
/// **R4_B02**: This check is **blocking** — a missing section sets
/// `artifact_valid = false` on the returned report, which prevents learning
/// gates from treating the generation as successful.
///
/// Returns a [`PrdArtifactReport`] whose `artifact_valid` field reflects
/// whether the section was found.
#[must_use]
pub fn check_grounding_section(
    prd_content: &str,
    slug: &str,
    process_success: bool,
) -> PrdArtifactReport {
    let mut report = PrdArtifactReport::new(slug, process_success);
    let has_section = prd_content.lines().any(|line| {
        line.trim()
            .to_lowercase()
            .starts_with("## repository grounding")
    });
    if !has_section {
        report.push(PrdValidationIssue {
            severity: PrdValidationSeverity::Error,
            category: "missing_section",
            message: format!(
                "PRD '{}' is missing required '## Repository Grounding' section — PRD rejected",
                slug
            ),
        });
    }
    report
}

/// Validate the `## Repository Grounding` section of a generated PRD.
///
/// **R4_B02**: Missing grounding section is an `Error` (blocking).
/// **R4_B03**: Referenced source files that don't exist on disk are `Error`
///             (blocking). Duplicate crate proposals are also `Error`.
///
/// `workdir` is the workspace root (used to resolve relative source paths).
/// `workspace_members` is the list of crate directory names under `crates/`.
#[must_use]
pub fn validate_prd_grounding(
    prd_content: &str,
    slug: &str,
    workdir: &Path,
    workspace_members: &[String],
    process_success: bool,
) -> PrdArtifactReport {
    // Start with the blocking grounding-section check (R4_B02).
    let mut report = check_grounding_section(prd_content, slug, process_success);

    let Some(grounding_text) = extract_prd_section(prd_content, "repository grounding") else {
        // Section missing — already recorded as Error above; nothing more to validate.
        return report;
    };

    let text_lower = grounding_text.to_lowercase();

    // R4_B03a: "no existing crates" claim is suspicious when the workspace has members.
    if (text_lower.contains("no existing crates") || text_lower.contains("no relevant crates"))
        && !workspace_members.is_empty()
    {
        report.push(PrdValidationIssue {
            severity: PrdValidationSeverity::Warning,
            category: "false_negative",
            message: format!(
                "PRD claims no existing crates but workspace has {} crate(s): {}",
                workspace_members.len(),
                workspace_members
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }

    // R4_B03b: "new crate X" proposals that duplicate existing workspace members.
    let new_crate_patterns = ["new crate: ", "new crate `", "create crate ", "add crate "];
    for line in grounding_text.lines() {
        let line_lower = line.to_lowercase();
        for pat in &new_crate_patterns {
            if let Some(after_offset) = line_lower.find(pat) {
                let rest = &line[after_offset + pat.len()..];
                let proposed = rest
                    .trim()
                    .trim_start_matches('`')
                    .trim_start_matches('"')
                    .split(|c: char| {
                        c.is_whitespace() || c == '`' || c == '"' || c == ',' || c == ')'
                    })
                    .next()
                    .unwrap_or("")
                    .trim();
                if !proposed.is_empty() && proposed.starts_with("roko-") {
                    if workspace_members
                        .iter()
                        .any(|m| m.to_lowercase() == proposed.to_lowercase())
                    {
                        report.push(PrdValidationIssue {
                            severity: PrdValidationSeverity::Error,
                            category: "duplicate_crate",
                            message: format!(
                                "PRD proposes creating crate '{}' which already exists in the workspace",
                                proposed
                            ),
                        });
                    }
                }
            }
        }
    }

    // R4_B03c: Referenced source files must exist in the workspace.
    let referenced_paths = extract_referenced_paths(&grounding_text);
    for rel_path in &referenced_paths {
        let abs_path = workdir.join(rel_path);
        if !abs_path.exists() {
            report.push(PrdValidationIssue {
                severity: PrdValidationSeverity::Error,
                category: "missing_file_ref",
                message: format!(
                    "PRD references '{}' in Repository Grounding but that path does not exist in the workspace",
                    rel_path
                ),
            });
        }
    }

    report
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
    fn generation_outcome_labels_distinguish_process_from_artifact() {
        let success = GenerationOutcome {
            process_success: true,
            artifact_valid: true,
            validation_report: None,
        };
        let partial = GenerationOutcome {
            process_success: true,
            artifact_valid: false,
            validation_report: None,
        };
        let failure = GenerationOutcome {
            process_success: false,
            artifact_valid: true,
            validation_report: None,
        };

        assert!(success.fully_successful());
        assert_eq!(success.status_label(), "success");
        assert!(!partial.fully_successful());
        assert_eq!(partial.status_label(), "partial_success");
        assert!(!failure.fully_successful());
        assert_eq!(failure.status_label(), "failure");
    }

    #[test]
    fn artifact_outcome_valid_requires_process_artifact_path_and_report() {
        let outcome = GenerationOutcome {
            process_success: true,
            artifact_valid: true,
            validation_report: Some(serde_json::json!({"totals": {"errors": 0}})),
        };
        let path = PathBuf::from(".roko/plans/demo");

        let artifact =
            ArtifactOutcome::from_generation_outcome("prd-plan", Some(path.clone()), &outcome);

        assert_eq!(
            artifact,
            ArtifactOutcome::Valid {
                artifact_type: "prd-plan".to_string(),
                path,
                report: serde_json::json!({"totals": {"errors": 0}}),
            }
        );
        assert!(artifact.is_valid());
    }

    #[test]
    fn artifact_outcome_invalid_is_not_success() {
        let outcome = GenerationOutcome {
            process_success: true,
            artifact_valid: false,
            validation_report: Some(serde_json::json!({"totals": {"errors": 2}})),
        };
        let path = PathBuf::from(".roko/plans/demo");

        let artifact =
            ArtifactOutcome::from_generation_outcome("prd-plan", Some(path.clone()), &outcome);

        assert_eq!(
            artifact,
            ArtifactOutcome::Invalid {
                artifact_type: "prd-plan".to_string(),
                path: Some(path),
                report: Some(serde_json::json!({"totals": {"errors": 2}})),
            }
        );
        assert!(!artifact.is_valid());
    }

    #[test]
    fn artifact_outcome_process_failure_is_not_produced() {
        let outcome = GenerationOutcome {
            process_success: false,
            artifact_valid: true,
            validation_report: Some(serde_json::json!({"ignored": true})),
        };

        let artifact = ArtifactOutcome::from_generation_outcome(
            "prd-plan",
            Some(PathBuf::from(".roko/plans/demo")),
            &outcome,
        );

        assert_eq!(
            artifact,
            ArtifactOutcome::NotProduced {
                artifact_type: "prd-plan".to_string(),
                reason: "generation process failed".to_string(),
            }
        );
        assert!(!artifact.is_valid());
    }

    #[test]
    fn artifact_outcome_validation_unavailable_is_not_success() {
        let outcome = GenerationOutcome {
            process_success: true,
            artifact_valid: true,
            validation_report: None,
        };
        let path = PathBuf::from(".roko/plans/demo");

        let artifact =
            ArtifactOutcome::from_generation_outcome("prd-plan", Some(path.clone()), &outcome);

        assert_eq!(
            artifact,
            ArtifactOutcome::ValidationUnavailable {
                artifact_type: "prd-plan".to_string(),
                path: Some(path),
                reason: "artifact validation report was not available".to_string(),
            }
        );
        assert!(!artifact.is_valid());
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
            "---\nstatus: draft\nupdated: 2020-01-01\n---\n# Test\n\nThis PRD describes a real feature with substantive content.\n",
        )
        .unwrap();
        cmd_promote(tmp.path(), "test", false).await.unwrap();
        assert!(!draft.exists());
        let published = published_dir(tmp.path()).join("test.md");
        assert!(published.exists());
        let content = std::fs::read_to_string(&published).unwrap();
        assert!(content.contains("status: published"));
    }

    #[tokio::test]
    async fn promote_follow_on_generation_failure_is_non_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("roko.toml"), "[prd]\nauto_plan = true\n").unwrap();
        let prd_path = published_dir(tmp.path()).join("test.md");

        let outcome = maybe_generate_plan_after_promote_with(
            tmp.path(),
            "test".to_string(),
            prd_path.clone(),
            false,
            |_slug, _path, _dry_run| async move { Err(anyhow!("synthetic generation failure")) },
        )
        .await
        .unwrap();

        assert!(outcome.is_none());
    }

    #[tokio::test]
    async fn promote_rejects_empty_draft() {
        let tmp = tempfile::tempdir().unwrap();
        ensure_dirs(tmp.path()).unwrap();
        let draft = drafts_dir(tmp.path()).join("empty.md");
        std::fs::write(&draft, "---\nstatus: draft\n---\n# Empty\n").unwrap();
        let err = cmd_promote(tmp.path(), "empty", false).await.unwrap_err();
        assert!(
            err.to_string().contains("no substantive content"),
            "got: {err}"
        );
        assert!(draft.exists(), "draft should not be deleted on reject");
    }

    #[test]
    fn extract_fenced_block_finds_toml() {
        let text = "Some text\n```toml\n[tasks]\nname = \"test\"\n```\nMore text";
        let block = extract_fenced_block(text, "toml").unwrap();
        assert!(block.contains("[tasks]"));
        assert!(block.contains("name = \"test\""));
    }

    #[test]
    fn extract_fenced_block_returns_none_for_missing() {
        assert!(extract_fenced_block("no blocks here", "toml").is_none());
    }

    #[test]
    fn extract_fenced_block_skips_nested_fences() {
        // Agent output might include code samples with their own fences
        let text = "Here is the plan:\n```toml\n[[tasks]]\nid = \"T1\"\n\
                    # Example bash:\n```bash\necho hello\n```\n\
                    verify = \"cargo test\"\n```\nDone.";
        let block = extract_fenced_block(text, "toml").unwrap();
        assert!(block.contains("id = \"T1\""), "should contain task");
        assert!(block.contains("```bash"), "should include the nested fence");
    }

    #[test]
    fn extract_fenced_block_handles_angle_bracket_tag() {
        let text = "Output:\n```<plan.md>\n# My Plan\n\nSteps here.\n```\n";
        let block = extract_fenced_block(text, "plan.md").unwrap();
        assert!(block.contains("# My Plan"));
    }

    #[test]
    fn extract_fenced_block_returns_none_for_empty_block() {
        let text = "```toml\n\n```\n";
        assert!(extract_fenced_block(text, "toml").is_none());
    }

    #[test]
    fn extract_fenced_block_multiple_blocks_gets_first() {
        let text = "```toml\nfirst = true\n```\n\n```toml\nsecond = true\n```\n";
        let block = extract_fenced_block(text, "toml").unwrap();
        assert!(block.contains("first = true"));
        assert!(!block.contains("second = true"));
    }

    #[test]
    fn augment_generator_system_prompt_skips_empty_context() {
        let prompt = augment_generator_system_prompt("base prompt".to_string(), Some("   "));
        assert_eq!(prompt, "base prompt");
    }

    #[test]
    fn augment_generator_system_prompt_includes_failure_context() {
        let prompt = augment_generator_system_prompt(
            "base prompt".to_string(),
            Some("task_id = \"demo\"\nreason = \"gate failure\""),
        );
        assert!(prompt.starts_with("base prompt"));
        assert!(prompt.contains("## Failure context for replanning"));
        assert!(prompt.contains("task_id = \"demo\""));
        assert!(prompt.contains("gate failure"));
        assert!(prompt.contains("Do not reproduce the same task shape."));
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
    fn replace_in_frontmatter_only_affects_frontmatter() {
        let content = "---\nstatus: draft\ntitle: Demo\n---\n\n# Body\n\nThe status: draft is mentioned here too.\n";
        let result = replace_in_frontmatter(content, "status: draft", "status: published");
        assert!(result.contains("status: published"));
        assert!(
            result.contains("The status: draft is mentioned here too."),
            "body should be untouched"
        );
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

    // ─── R4_B02 / R4_B03 validation tests ─────────────────────────

    #[test]
    fn check_grounding_section_rejects_missing_section() {
        let content = "---\nid: prd-demo\n---\n# Demo\n\n## Requirements\n\nSome req.\n";
        let report = check_grounding_section(content, "demo", true);
        assert!(
            !report.artifact_valid,
            "missing section must set artifact_valid=false"
        );
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.severity == PrdValidationSeverity::Error
                    && i.category == "missing_section"),
            "expected missing_section error"
        );
    }

    #[test]
    fn check_grounding_section_accepts_present_section() {
        let content = "---\nid: prd-demo\n---\n# Demo\n\n## Repository Grounding\n\nExisting crates: roko-core.\n";
        let report = check_grounding_section(content, "demo", true);
        assert!(report.artifact_valid, "present section must pass");
        assert!(report.issues.is_empty(), "no issues expected");
    }

    #[test]
    fn check_grounding_section_case_insensitive() {
        let content = "# Demo\n\n## REPOSITORY GROUNDING\n\nContent here.\n";
        let report = check_grounding_section(content, "demo", true);
        assert!(report.artifact_valid, "case-insensitive match must pass");
    }

    #[test]
    fn validate_prd_grounding_flags_no_existing_crates_claim() {
        let content = "# Demo\n\n## Repository Grounding\n\nNo existing crates are relevant.\n";
        let members = vec!["roko-core".to_string(), "roko-agent".to_string()];
        let report = validate_prd_grounding(content, "demo", Path::new("/tmp"), &members, true);
        assert!(
            report.issues.iter().any(|i| i.category == "false_negative"),
            "expected false_negative warning"
        );
        // Warning only — still valid
        assert!(report.artifact_valid);
    }

    #[test]
    fn validate_prd_grounding_blocks_duplicate_crate_proposal() {
        let content = "# Demo\n\n## Repository Grounding\n\nnew crate: roko-core\n";
        let members = vec!["roko-core".to_string()];
        let report = validate_prd_grounding(content, "demo", Path::new("/tmp"), &members, true);
        assert!(
            !report.artifact_valid,
            "duplicate crate must set artifact_valid=false"
        );
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == "duplicate_crate"
                    && i.severity == PrdValidationSeverity::Error)
        );
    }

    #[test]
    fn validate_prd_grounding_blocks_nonexistent_file_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let content = "# Demo\n\n## Repository Grounding\n\n**Source files**:\n- crates/roko-cli/src/no_such_file.rs — does not exist\n";
        let report = validate_prd_grounding(content, "demo", tmp.path(), &[], true);
        assert!(
            !report.artifact_valid,
            "nonexistent file ref must set artifact_valid=false"
        );
        assert!(report.issues.iter().any(
            |i| i.category == "missing_file_ref" && i.severity == PrdValidationSeverity::Error
        ));
    }

    #[test]
    fn validate_prd_grounding_allows_existing_file_reference() {
        let tmp = tempfile::tempdir().unwrap();
        // Create the file so it "exists"
        let file_path = tmp.path().join("crates").join("roko-cli").join("src");
        std::fs::create_dir_all(&file_path).unwrap();
        std::fs::write(file_path.join("prd.rs"), "// prd").unwrap();
        let content = "# Demo\n\n## Repository Grounding\n\n**Source files**:\n- crates/roko-cli/src/prd.rs — PRD logic\n";
        let report = validate_prd_grounding(content, "demo", tmp.path(), &[], true);
        assert!(report.artifact_valid, "existing file ref must pass");
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == "missing_file_ref"),
            "no missing_file_ref issues expected"
        );
    }

    #[test]
    fn extract_prd_section_returns_none_when_missing() {
        let content = "# Title\n\n## Overview\n\nSome text.\n";
        assert!(extract_prd_section(content, "repository grounding").is_none());
    }

    #[test]
    fn extract_prd_section_extracts_body() {
        let content =
            "# Title\n\n## Repository Grounding\n\nCrates: roko-core.\n\n## References\n\nRef 1.\n";
        let body = extract_prd_section(content, "repository grounding").unwrap();
        assert!(body.contains("Crates: roko-core."), "body: {body}");
        assert!(!body.contains("## References"), "must stop at next heading");
    }

    #[test]
    fn extract_referenced_paths_finds_crate_paths() {
        let text = "- crates/roko-cli/src/prd.rs — PRD logic\n- crates/roko-core/src/lib.rs";
        let paths = extract_referenced_paths(text);
        assert!(paths.contains(&"crates/roko-cli/src/prd.rs".to_string()));
        assert!(paths.contains(&"crates/roko-core/src/lib.rs".to_string()));
    }

    // ---- validate_and_fix_generated_plan tests -------------------------------

    fn empty_models() -> std::collections::HashMap<String, roko_core::config::schema::ModelProfile> {
        std::collections::HashMap::new()
    }

    fn sample_models() -> std::collections::HashMap<String, roko_core::config::schema::ModelProfile>
    {
        // Build from TOML to avoid enumerating every ModelProfile field.
        let toml_str = r#"
[models.claude-sonnet-4-6]
provider = "anthropic"
slug = "claude-sonnet-4-6"

[models.claude-haiku-4-5]
provider = "anthropic"
slug = "claude-haiku-4-5"
"#;
        let cfg: roko_core::config::schema::RokoConfig =
            roko_core::config::schema::RokoConfig::from_toml(toml_str).unwrap();
        cfg.models
    }

    #[test]
    fn validate_valid_plan_passes() {
        let toml = r#"
[meta]
plan = "my-plan"
total = 2
status = "pending"

[[task]]
id = "T1"
title = "First task"
status = "pending"
role = "implementer"
tier = "focused"

[[task.verify]]
phase = "test"
command = "cargo test"

[[task]]
id = "T2"
title = "Second task"
status = "pending"
role = "implementer"
tier = "mechanical"
depends_on = ["T1"]
"#;
        let result = validate_and_fix_generated_plan(toml, "my-plan", &empty_models(), None);
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
    }

    #[test]
    fn validate_fixes_truncated_slug() {
        let toml = r#"
[meta]
plan = "my-pl"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"
"#;
        let result =
            validate_and_fix_generated_plan(toml, "my-plan", &empty_models(), None).unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        assert_eq!(
            parsed["meta"]["plan"].as_str().unwrap(),
            "my-plan",
            "slug should be corrected"
        );
    }

    #[test]
    fn validate_fixes_wrong_slug() {
        let toml = r#"
[meta]
plan = "wrong-slug"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"
"#;
        let result =
            validate_and_fix_generated_plan(toml, "correct-slug", &empty_models(), None).unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        assert_eq!(parsed["meta"]["plan"].as_str().unwrap(), "correct-slug");
    }

    #[test]
    fn validate_fixes_verify_field_typo() {
        let toml = r#"
[meta]
plan = "test"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"

[[task.verify]]
pha = "test"
command = "cargo test"
"#;
        let result =
            validate_and_fix_generated_plan(toml, "test", &empty_models(), None).unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        let verify = parsed["task"][0]["verify"][0].as_table().unwrap();
        assert!(verify.contains_key("phase"), "pha should be corrected to phase");
        assert!(!verify.contains_key("pha"), "pha should be removed");
    }

    #[test]
    fn validate_fixes_unknown_model_hint() {
        let toml = r#"
[meta]
plan = "test"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"
model_hint = "gpt-nonexistent"
"#;
        let models = sample_models();
        let result = validate_and_fix_generated_plan(
            toml,
            "test",
            &models,
            Some("claude-sonnet-4-6"),
        )
        .unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        assert_eq!(
            parsed["task"][0]["model_hint"].as_str().unwrap(),
            "claude-sonnet-4-6",
            "unknown model should be replaced with default"
        );
    }

    #[test]
    fn validate_normalizes_model_alias() {
        let toml = r#"
[meta]
plan = "test"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"
model_hint = "haiku"
"#;
        let models = sample_models();
        let result =
            validate_and_fix_generated_plan(toml, "test", &models, None).unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        assert_eq!(
            parsed["task"][0]["model_hint"].as_str().unwrap(),
            "claude-haiku-4-5",
            "alias should be normalized to full name"
        );
    }

    #[test]
    fn validate_rejects_missing_meta() {
        let toml = r#"
[[task]]
id = "T1"
title = "Do thing"
status = "pending"
role = "implementer"
tier = "focused"
"#;
        let result = validate_and_fix_generated_plan(toml, "test", &empty_models(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("[meta] section is missing"), "msg: {msg}");
    }

    #[test]
    fn validate_rejects_missing_task_array() {
        let toml = r#"
[meta]
plan = "test"
total = 0
status = "pending"
"#;
        let result = validate_and_fix_generated_plan(toml, "test", &empty_models(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("[[task]] array is missing"), "msg: {msg}");
    }

    #[test]
    fn validate_rejects_missing_required_task_fields() {
        let toml = r#"
[meta]
plan = "test"
total = 1
status = "pending"

[[task]]
id = "T1"
"#;
        let result = validate_and_fix_generated_plan(toml, "test", &empty_models(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("missing required field 'title'"), "msg: {msg}");
        assert!(msg.contains("missing required field 'status'"), "msg: {msg}");
        assert!(msg.contains("missing required field 'role'"), "msg: {msg}");
        assert!(msg.contains("missing required field 'tier'"), "msg: {msg}");
    }

    #[test]
    fn validate_rejects_invalid_toml_syntax() {
        let toml = "this is not valid toml {{{}}}";
        let result = validate_and_fix_generated_plan(toml, "test", &empty_models(), None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("invalid TOML"), "msg: {msg}");
    }

    #[test]
    fn validate_fixes_task_field_typo() {
        let toml = r#"
[meta]
plan = "test"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Do thing"
stat = "pending"
role = "implementer"
tier = "focused"
"#;
        let result =
            validate_and_fix_generated_plan(toml, "test", &empty_models(), None).unwrap();
        let parsed: toml::Value = toml::from_str(&result).unwrap();
        let task = parsed["task"][0].as_table().unwrap();
        assert!(
            task.contains_key("status"),
            "stat should be corrected to status"
        );
        assert!(!task.contains_key("stat"), "stat should be removed");
    }

    #[test]
    fn validate_fixes_placeholder_crate_names() {
        let toml = r#"
[meta]
plan = "btc-funding-alert-cli"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Set up crate structure"
status = "pending"
role = "implementer"
tier = "focused"
files = ["crates/<relevant-lib>/src/lib.rs"]

[[task.verify]]
phase = "build"
command = "cargo check -p <crate>"
"#;
        let result = validate_and_fix_generated_plan(
            toml,
            "btc-funding-alert-cli",
            &empty_models(),
            None,
        )
        .unwrap();
        assert!(
            !result.contains("<relevant-lib>"),
            "placeholder <relevant-lib> should be replaced"
        );
        assert!(
            !result.contains("<crate>"),
            "placeholder <crate> should be replaced"
        );
        assert!(
            result.contains("btc-funding-alert-cli"),
            "slug should appear in output"
        );
        // Verify it's still valid TOML.
        let _parsed: toml::Value = toml::from_str(&result).unwrap();
    }

    #[test]
    fn validate_fixes_placeholder_in_verify_command() {
        let toml = r#"
[meta]
plan = "my-cool-tool"
total = 1
status = "pending"

[[task]]
id = "T1"
title = "Implement module"
status = "pending"
role = "implementer"
tier = "focused"
files = ["crates/<binary-crate>/src/<module>.rs"]

[[task.verify]]
phase = "build"
command = "cargo check -p <binary-crate>"

[[task.verify]]
phase = "test"
command = "cargo test -p <crate> -- <test_name>"
"#;
        let result = validate_and_fix_generated_plan(
            toml,
            "my-cool-tool",
            &empty_models(),
            None,
        )
        .unwrap();
        // <binary-crate> and <crate> replaced with slug.
        assert!(
            !result.contains("<binary-crate>"),
            "placeholder <binary-crate> should be replaced"
        );
        assert!(
            !result.contains("<crate>"),
            "placeholder <crate> should be replaced"
        );
        assert!(
            !result.contains("<module>"),
            "placeholder <module> should be replaced"
        );
        assert!(
            !result.contains("<test_name>"),
            "placeholder <test_name> should be replaced"
        );
        assert!(
            result.contains("cargo check -p my-cool-tool"),
            "verify command should contain slug: {result}"
        );
        assert!(
            result.contains("cargo test -p my-cool-tool"),
            "verify command should contain slug: {result}"
        );
        // Verify it's still valid TOML.
        let _parsed: toml::Value = toml::from_str(&result).unwrap();
    }

    #[test]
    fn strsim_distance_basic() {
        assert_eq!(strsim_distance("phase", "phase"), 0);
        assert_eq!(strsim_distance("pha", "phase"), 2);
        assert_eq!(strsim_distance("stat", "status"), 2);
        assert_eq!(strsim_distance("", "abc"), 3);
        assert_eq!(strsim_distance("abc", ""), 3);
    }

    #[test]
    fn suggest_correction_finds_typos() {
        assert_eq!(
            suggest_field_correction("pha", KNOWN_VERIFY_FIELDS),
            Some("phase".to_string())
        );
        assert_eq!(
            suggest_field_correction("stat", KNOWN_TASK_FIELDS),
            Some("status".to_string())
        );
        // Unknown field with no close match returns None.
        assert_eq!(
            suggest_field_correction("zzzzunknown", KNOWN_TASK_FIELDS),
            None
        );
    }
}
