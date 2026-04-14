//! Auto-maintained indexes for PRDs, plans, research, and tasks.
//!
//! Every time roko creates or modifies a PRD, plan, research artifact, or task,
//! the relevant index is rebuilt. Indexes are both human-readable (markdown)
//! and machine-parseable (structured sections with consistent formatting).
//!
//! Index files serve as:
//! 1. **Discovery** — what exists, where it lives
//! 2. **Dedup** — agents read the index before creating anything new
//! 3. **Context** — injected into agent prompts so they know the full picture
//! 4. **Cross-references** — which PRDs link to which plans, etc.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

// ─── Index paths ───────────────────────────────────────────────────

fn master_index_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("INDEX.md")
}
fn prd_index_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("prd").join("INDEX.md")
}
fn plans_index_path(workdir: &Path) -> PathBuf {
    crate::plan::plans_dir(workdir).join("INDEX.md")
}
fn research_index_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("research").join("INDEX.md")
}

// ─── PRD index ─────────────────────────────────────────────────────

/// Rebuild `.roko/prd/INDEX.md` from all published + draft PRDs.
pub fn rebuild_prd_index(workdir: &Path) -> Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# PRD Index");
    let _ = writeln!(out, "\n> Auto-generated. Do not edit manually.");
    let _ = writeln!(out, "> Rebuilt on every `roko prd` command.\n");

    // Ideas count
    let ideas_path = workdir.join(".roko/prd/ideas.md");
    let idea_count = std::fs::read_to_string(&ideas_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("- "))
        .count();
    let _ = writeln!(out, "**Ideas**: {idea_count} captured in `ideas.md`\n");

    // Published
    let _ = writeln!(out, "## Published\n");
    let _ = writeln!(out, "| Slug | Title | Crates | Plans | Coverage |");
    let _ = writeln!(out, "|------|-------|--------|-------|----------|");
    let published_dir = workdir.join(".roko/prd/published");
    let published = list_md_sorted(&published_dir);
    if published.is_empty() {
        let _ = writeln!(out, "| _(none)_ | | | | |");
    }
    for path in &published {
        let slug = file_slug(path);
        let meta = read_frontmatter(path);
        let _ = writeln!(
            out,
            "| `{slug}` | {} | {} | {} | {} |",
            meta.title,
            meta.crates,
            meta.plans_generated,
            if meta.coverage > 0.0 {
                format!("{:.0}%", meta.coverage * 100.0)
            } else {
                "—".into()
            }
        );
    }

    // Drafts
    let _ = writeln!(out, "\n## Drafts\n");
    let _ = writeln!(out, "| Slug | Title | Created |");
    let _ = writeln!(out, "|------|-------|---------|");
    let drafts_dir = workdir.join(".roko/prd/drafts");
    let drafts = list_md_sorted(&drafts_dir);
    if drafts.is_empty() {
        let _ = writeln!(out, "| _(none)_ | | |");
    }
    for path in &drafts {
        let slug = file_slug(path);
        let meta = read_frontmatter(path);
        let _ = writeln!(out, "| `{slug}` | {} | {} |", meta.title, meta.created);
    }

    // Recent ideas (last 10)
    let _ = writeln!(out, "\n## Recent Ideas\n");
    let ideas_content = std::fs::read_to_string(&ideas_path).unwrap_or_default();
    let ideas: Vec<&str> = ideas_content
        .lines()
        .filter(|l| l.starts_with("- "))
        .collect();
    let start = ideas.len().saturating_sub(10);
    for line in &ideas[start..] {
        let _ = writeln!(out, "{line}");
    }

    let idx = prd_index_path(workdir);
    if let Some(parent) = idx.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&idx, &out)?;
    Ok(())
}

// ─── Plans index ───────────────────────────────────────────────────

/// Rebuild `.roko/plans/INDEX.md` from all plan directories.
pub fn rebuild_plans_index(workdir: &Path) -> Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# Plans Index");
    let _ = writeln!(out, "\n> Auto-generated. Do not edit manually.");
    let _ = writeln!(out, "> Rebuilt on every `roko plan` command.\n");

    if let Some(parent) = plans_index_path(workdir).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _ = writeln!(out, "| Plan | Tasks | Done | Ready | Status | Parallel |");
    let _ = writeln!(out, "|------|-------|------|-------|--------|----------|");

    let plans_dir = crate::plan::plans_dir(workdir);
    if !plans_dir.is_dir() {
        let _ = writeln!(out, "| _(no plans directory)_ | | | | | |");
        std::fs::write(plans_index_path(workdir), &out)?;
        return Ok(());
    }

    let mut plan_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&plans_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("tasks.toml").exists() {
                plan_dirs.push(path);
            }
        }
    }
    plan_dirs.sort();

    let mut total_tasks = 0u32;
    let mut total_done = 0u32;

    for dir in &plan_dirs {
        let name = dir.file_name().unwrap_or_default().to_string_lossy();
        let tasks_path = dir.join("tasks.toml");
        let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();

        let tasks: u32 = content.matches("[[task]]").count() as u32;
        let done: u32 = content.matches("status = \"done\"").count() as u32;
        let ready: u32 = content.matches("status = \"ready\"").count() as u32;
        let max_parallel = extract_toml_value(&content, "max_parallel").unwrap_or_default();

        let status = if done == tasks && tasks > 0 {
            "✅ complete"
        } else if done > 0 {
            "🔄 in progress"
        } else {
            "📋 ready"
        };

        let _ = writeln!(
            out,
            "| `{name}` | {tasks} | {done} | {ready} | {status} | {max_parallel} |"
        );
        total_tasks += tasks;
        total_done += done;
    }

    let _ = writeln!(
        out,
        "\n**Total**: {} plans, {} tasks, {} done ({:.0}%)",
        plan_dirs.len(),
        total_tasks,
        total_done,
        if total_tasks > 0 {
            total_done as f64 / total_tasks as f64 * 100.0
        } else {
            0.0
        }
    );

    std::fs::write(plans_index_path(workdir), &out)?;
    Ok(())
}

// ─── Research index ────────────────────────────────────────────────

/// Rebuild `.roko/research/INDEX.md` from all research artifacts.
pub fn rebuild_research_index(workdir: &Path) -> Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# Research Index");
    let _ = writeln!(out, "\n> Auto-generated. Do not edit manually.");
    let _ = writeln!(out, "> Rebuilt on every `roko research` command.\n");

    let _ = writeln!(out, "| Artifact | Size | Modified |");
    let _ = writeln!(out, "|----------|------|----------|");

    let research_dir = workdir.join(".roko/research");
    let files = list_md_sorted(&research_dir);

    for path in &files {
        let name = file_slug(path);
        if name == "INDEX" {
            continue;
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                Some(dt.format("%Y-%m-%d %H:%M").to_string())
            })
            .unwrap_or_else(|| "—".into());
        let _ = writeln!(out, "| `{name}` | {size} bytes | {modified} |");
    }

    if files.is_empty() || (files.len() == 1 && file_slug(&files[0]) == "INDEX") {
        let _ = writeln!(out, "| _(none)_ | | |");
    }

    let idx = research_index_path(workdir);
    if let Some(parent) = idx.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&idx, &out)?;
    Ok(())
}

// ─── Master index ──────────────────────────────────────────────────

/// Rebuild `.roko/INDEX.md` — the master index linking everything.
pub fn rebuild_master_index(workdir: &Path) -> Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# Roko Master Index");
    let _ = writeln!(out, "\n> Auto-generated. Do not edit manually.");
    let _ = writeln!(out, "> Single entry point for all roko artifacts.\n");

    // PRD summary
    let published_count = list_md_sorted(&workdir.join(".roko/prd/published")).len();
    let drafts_count = list_md_sorted(&workdir.join(".roko/prd/drafts")).len();
    let ideas_count = std::fs::read_to_string(workdir.join(".roko/prd/ideas.md"))
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("- "))
        .count();
    let _ = writeln!(
        out,
        "## PRDs ({published_count} published, {drafts_count} drafts, {ideas_count} ideas)"
    );
    let _ = writeln!(out, "→ [Full index](.roko/prd/INDEX.md)\n");

    // Plans summary
    let plans_dir = crate::plan::plans_dir(workdir);
    let mut plan_count = 0u32;
    let mut task_count = 0u32;
    let mut done_count = 0u32;
    if plans_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&plans_dir) {
            for entry in entries.flatten() {
                let tasks_path = entry.path().join("tasks.toml");
                if tasks_path.exists() {
                    plan_count += 1;
                    let content = std::fs::read_to_string(&tasks_path).unwrap_or_default();
                    task_count += content.matches("[[task]]").count() as u32;
                    done_count += content.matches("status = \"done\"").count() as u32;
                }
            }
        }
    }
    let _ = writeln!(
        out,
        "## Plans ({plan_count} plans, {task_count} tasks, {done_count} done)"
    );
    let _ = writeln!(out, "→ [Full index](.roko/plans/INDEX.md)\n");

    // Research summary
    let research_count = list_md_sorted(&workdir.join(".roko/research"))
        .iter()
        .filter(|p| file_slug(p) != "INDEX")
        .count();
    let _ = writeln!(out, "## Research ({research_count} artifacts)");
    let _ = writeln!(out, "→ [Full index](.roko/research/INDEX.md)\n");

    // Episodes summary
    let episodes_path = workdir.join(".roko/memory/episodes.jsonl");
    let episode_count = if episodes_path.exists() {
        std::fs::read_to_string(&episodes_path)
            .unwrap_or_default()
            .lines()
            .count()
    } else {
        0
    };
    let _ = writeln!(out, "## Episodes ({episode_count} recorded)");
    let _ = writeln!(out, "→ `.roko/memory/episodes.jsonl`\n");

    // Config
    let config_exists = workdir.join("roko.toml").exists();
    let _ = writeln!(out, "## Config");
    let _ = writeln!(
        out,
        "- `roko.toml`: {}",
        if config_exists {
            "✅ present"
        } else {
            "❌ missing"
        }
    );

    std::fs::write(master_index_path(workdir), &out)?;
    Ok(())
}

/// Rebuild ALL indexes. Call this after any mutation.
pub fn rebuild_all(workdir: &Path) -> Result<()> {
    // Silently skip if directories don't exist yet
    let _ = rebuild_prd_index(workdir);
    let _ = rebuild_plans_index(workdir);
    let _ = rebuild_research_index(workdir);
    rebuild_master_index(workdir)?;
    Ok(())
}

// ─── Helpers ───────────────────────────────────────────────────────

fn list_md_sorted(dir: &Path) -> Vec<PathBuf> {
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

fn file_slug(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

struct FrontmatterBrief {
    title: String,
    created: String,
    crates: String,
    plans_generated: String,
    coverage: f64,
}

fn read_frontmatter(path: &Path) -> FrontmatterBrief {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let slug = file_slug(path);
    let mut brief = FrontmatterBrief {
        title: slug,
        created: "—".into(),
        crates: "—".into(),
        plans_generated: "—".into(),
        coverage: 0.0,
    };
    for line in content.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("title:") {
            brief.title = val.trim().trim_matches('"').to_string();
        } else if let Some(val) = line.strip_prefix("created:") {
            brief.created = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("crates:") {
            brief.crates = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("plans_generated:") {
            brief.plans_generated = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("coverage:") {
            brief.coverage = val.trim().parse().unwrap_or(0.0);
        }
    }
    brief
}

fn extract_toml_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(val) = rest.trim().strip_prefix('=') {
                return Some(val.trim());
            }
        }
    }
    None
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_all_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/prd/published")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/prd/drafts")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/research")).unwrap();
        std::fs::write(tmp.path().join(".roko/prd/ideas.md"), "# Ideas\n").unwrap();
        rebuild_all(tmp.path()).unwrap();
        assert!(master_index_path(tmp.path()).exists());
        assert!(prd_index_path(tmp.path()).exists());
        assert!(research_index_path(tmp.path()).exists());
    }

    #[test]
    fn prd_index_includes_drafts() {
        let tmp = tempfile::tempdir().unwrap();
        let drafts = tmp.path().join(".roko/prd/drafts");
        std::fs::create_dir_all(&drafts).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/prd/published")).unwrap();
        std::fs::write(tmp.path().join(".roko/prd/ideas.md"), "# Ideas\n").unwrap();
        std::fs::write(
            drafts.join("test-prd.md"),
            "---\ntitle: Test PRD\nstatus: draft\ncreated: 2026-04-08\n---\n# Test\n",
        )
        .unwrap();
        rebuild_prd_index(tmp.path()).unwrap();
        let content = std::fs::read_to_string(prd_index_path(tmp.path())).unwrap();
        assert!(content.contains("test-prd"));
        assert!(content.contains("Test PRD"));
    }

    #[test]
    fn plans_index_counts_tasks() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = tmp.path().join(".roko/plans/test-plan");
        std::fs::create_dir_all(&plan).unwrap();
        std::fs::write(
            plan.join("tasks.toml"),
            "[meta]\nplan = \"test\"\nmax_parallel = 2\n\n\
             [[task]]\nid = \"T1\"\nstatus = \"done\"\n\n\
             [[task]]\nid = \"T2\"\nstatus = \"ready\"\n",
        )
        .unwrap();
        rebuild_plans_index(tmp.path()).unwrap();
        let content = std::fs::read_to_string(plans_index_path(tmp.path())).unwrap();
        assert!(content.contains("test-plan"));
        assert!(content.contains("2")); // 2 tasks
        assert!(content.contains("1")); // 1 done
    }

    #[test]
    fn plans_index_writes_under_dot_roko() {
        let tmp = tempfile::tempdir().unwrap();

        rebuild_plans_index(tmp.path()).unwrap();

        assert!(plans_index_path(tmp.path()).exists());
    }

    #[test]
    fn master_index_has_all_sections() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/prd/published")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/prd/drafts")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/research")).unwrap();
        std::fs::write(tmp.path().join(".roko/prd/ideas.md"), "# Ideas\n").unwrap();
        rebuild_all(tmp.path()).unwrap();
        let content = std::fs::read_to_string(master_index_path(tmp.path())).unwrap();
        assert!(content.contains("## PRDs"));
        assert!(content.contains("## Plans"));
        assert!(content.contains("## Research"));
        assert!(content.contains("## Episodes"));
        assert!(content.contains("## Config"));
    }
}
