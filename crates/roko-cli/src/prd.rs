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

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agent_exec::{AgentExecOpts, run_agent};
use crate::task_parser::TasksFile;
use anyhow::{Context as _, Result, anyhow};
use roko_core::config::schema::RokoConfig;
use roko_core::obs::MetricRegistry;
use std::time::{SystemTime, UNIX_EPOCH};

// ─── Directory layout ──────────────────────────────���───────────────

fn prd_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("prd")
}
fn ideas_path(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("ideas.md")
}
fn drafts_dir(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("drafts")
}
fn published_dir(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("published")
}

fn tasks_root(workdir: &Path) -> PathBuf {
    workdir.join("plans")
}

fn collect_tasks_toml_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if !root.exists() {
        return files;
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().is_some_and(|name| name == "tasks.toml") {
                    files.push(path);
                }
            }
        }
    }

    files.sort();
    files
}

fn hash_content(content: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn snapshot_tasks_files(root: &Path) -> HashMap<PathBuf, u64> {
    let mut snapshot = HashMap::new();
    for path in collect_tasks_toml_files(root) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            snapshot.insert(path, hash_content(&content));
        }
    }
    snapshot
}

fn warn_on_tasks_quality(path: &Path) {
    let Ok(tasks_file) = TasksFile::parse(path) else {
        return;
    };
    let warnings = tasks_file.quality_warnings();
    if warnings.is_empty() {
        return;
    }

    eprintln!("⚠️  Plan quality warnings for {}:", path.display());
    for warning in warnings {
        eprintln!("  - {warning}");
    }
}

fn warn_on_new_or_updated_tasks(root: &Path, before: &HashMap<PathBuf, u64>) {
    for path in changed_tasks_files(root, before) {
        warn_on_tasks_quality(&path);
    }
}

fn changed_tasks_files(root: &Path, before: &HashMap<PathBuf, u64>) -> Vec<PathBuf> {
    let after = snapshot_tasks_files(root);
    let mut paths: Vec<PathBuf> = after
        .keys()
        .filter(|path| after.get(*path) != before.get(*path))
        .cloned()
        .collect();
    paths.sort();
    paths
}

fn copy_workspace_for_dry_run(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    copy_workspace_dir(src, dst)
}

struct DryRunWorkspace {
    path: PathBuf,
}

impl DryRunWorkspace {
    fn new(src: &Path) -> Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("compute dry-run workspace timestamp")?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "roko-prd-dry-run-{}-{}",
            std::process::id(),
            unique
        ));
        copy_workspace_for_dry_run(src, &path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DryRunWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_workspace_dir(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry.with_context(|| format!("read {}", src.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(name.as_ref(), ".git" | "target") {
            continue;
        }

        let dest = dst.join(name.as_ref());
        let ty = entry
            .file_type()
            .with_context(|| format!("inspect {}", path.display()))?;
        if ty.is_dir() {
            std::fs::create_dir_all(&dest).with_context(|| format!("create {}", dest.display()))?;
            copy_workspace_dir(&path, &dest)?;
        } else if ty.is_file() {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::copy(&path, &dest)
                .with_context(|| format!("copy {} -> {}", path.display(), dest.display()))?;
        }
    }
    Ok(())
}

fn print_tasks_preview(path: &Path, tasks_file: &TasksFile) {
    println!("📄 {}", path.display());
    println!(
        "  plan: {}  tasks: {}  status: {}  max_parallel: {}  estimated_minutes: {}",
        tasks_file.meta.plan,
        tasks_file.tasks.len(),
        tasks_file.meta.status,
        tasks_file.meta.max_parallel,
        tasks_file.meta.estimated_total_minutes,
    );
    for task in &tasks_file.tasks {
        let mut details = format!("  - {} [{}] {}", task.id, task.tier, task.title);
        details.push_str(&format!(" | files={} deps={} verify={}", task.files.len(), task.depends_on.len(), task.verify.len()));
        println!("{details}");
    }
}

fn validate_and_print_preview(path: &Path) -> Result<()> {
    let tasks_file = TasksFile::parse(path).with_context(|| format!("parse {}", path.display()))?;
    let issues = tasks_file.validate();
    if !issues.is_empty() {
        eprintln!("❌ Dry-run validation failed for {}:", path.display());
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        return Err(anyhow!("dry-run plan validation failed for {}", path.display()));
    }

    let warnings = tasks_file.quality_warnings();
    if !warnings.is_empty() {
        eprintln!("⚠️  Plan quality warnings for {}:", path.display());
        for warning in &warnings {
            eprintln!("  - {warning}");
        }
    }

    print_tasks_preview(path, &tasks_file);
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
    let plans_root = plans_dir.map_or_else(|| workdir.join("plans"), Path::to_path_buf);
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

    let mut runner = crate::PlanRunner::from_plans_dir(
        plans_root,
        workdir,
        resolved.config,
        metrics,
        false,
    )
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
            let cfg: RokoConfig = toml::from_str(&text)
                .with_context(|| format!("parse {}", roko_toml.display()))?;
            return Ok(cfg.prd.auto_plan);
        }
    }

    Ok(crate::load_layered(workdir)?.config.auto_plan)
}

/// Generate implementation plans from a published PRD file.
pub async fn generate_plan_from_prd(slug: &str, prd_path: &Path, dry_run: bool) -> Result<PathBuf> {
    let workdir = prd_workdir(prd_path)?;
    let content = std::fs::read_to_string(prd_path)
        .with_context(|| format!("read {}", prd_path.display()))?;
    println!("📋 Generating plans from PRD: {slug}");

    let dry_run_workdir = if dry_run {
        Some(DryRunWorkspace::new(&workdir)?)
    } else {
        None
    };
    let workdir_ref = dry_run_workdir
        .as_ref()
        .map_or(workdir.as_path(), |temp| temp.path());

    let resolved = crate::load_layered(workdir_ref)?;
    let system = crate::plan_generate::PLAN_GENERATOR_SYSTEM_PROMPT;
    let plans_root = tasks_root(workdir_ref);
    let tasks_before = snapshot_tasks_files(&plans_root);
    let task_prompt = format!(
        "Read the PRD at {path} and generate implementation plan directories \
         under plans/. Each REQ-XXX requirement becomes one or more tasks. \
         Each acceptance criterion becomes a task verification command. \
         Search the codebase first to understand what already exists. \
         Create plan.md and tasks.toml files directly, including per-task mcp_servers \
         when a task needs a specific MCP server.\n\n\
         PRD content:\n{content}",
        path = prd_path.display()
    );

    let exit_code = run_agent(AgentExecOpts {
        prompt: &task_prompt,
        workdir: workdir_ref,
        model: resolved.config.agent.model.as_deref(),
        effort: Some(resolved.config.agent.effort.as_str()),
        system_prompt: Some(system),
        resume_session: None,
        env_vars: &resolved.config.agent.env,
    })
    .await?;
    if exit_code != 0 {
        return Err(anyhow!(
            "plan generation agent failed with exit code {exit_code}"
        ));
    }

    if dry_run {
        let changed = changed_tasks_files(&plans_root, &tasks_before);
        if changed.is_empty() {
            return Err(anyhow!(
                "dry-run plan generation did not produce any tasks.toml files"
            ));
        }

        for path in &changed {
            validate_and_print_preview(path)?;
        }
    } else {
        warn_on_new_or_updated_tasks(&plans_root, &tasks_before);
    }

    Ok(workdir.join("plans"))
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
    let master_index = std::fs::read_to_string(workdir.join(".roko/INDEX.md")).unwrap_or_default();
    if !master_index.is_empty() {
        let _ = writeln!(
            prompt,
            "## Master Index (what already exists — do NOT duplicate)\n"
        );
        let _ = writeln!(prompt, "{master_index}\n---\n");
    }

    // Include the PRD index for detailed cross-references
    let prd_index = std::fs::read_to_string(workdir.join(".roko/prd/INDEX.md")).unwrap_or_default();
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
        .ok_or_else(|| anyhow!("could not derive workdir from PRD path: {}", prd_path.display()))
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
        let content = "---\nid: prd-test\ntitle: Test PRD\nstatus: draft\nversion: 2\ncoverage: 0.5\n---\n\n# Test\n";
        let meta = PrdMeta::parse(content).unwrap();
        assert_eq!(meta.id, "prd-test");
        assert_eq!(meta.title, "Test PRD");
        assert_eq!(meta.status, "draft");
        assert_eq!(meta.version, 2);
        assert!((meta.coverage - 0.5).abs() < f64::EPSILON);
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

    #[test]
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
}
