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

use crate::workspace_paths::{drafts_dir, ideas_path, plans_dir, prd_dir, published_dir, roko_dir};
use anyhow::Result;

// ─── Index paths ───────────────────────────────────────────────────

fn master_index_path(workdir: &Path) -> PathBuf {
    roko_dir(workdir).join("INDEX.md")
}
fn prd_index_path(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("INDEX.md")
}
fn plans_index_path(workdir: &Path) -> PathBuf {
    plans_dir(workdir).join("INDEX.md")
}
fn research_index_path(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("research").join("INDEX.md")
}

/// Append the master index to a prompt when it exists and is non-empty.
pub fn append_master_index_prompt(out: &mut String, workdir: &Path, heading: &str) {
    let master_index = std::fs::read_to_string(master_index_path(workdir)).unwrap_or_default();
    if master_index.trim().is_empty() {
        return;
    }
    let _ = writeln!(out, "{heading}\n{master_index}\n---\n");
}

// ─── PRD index ─────────────────────────────────────────────────────

/// Rebuild `.roko/prd/INDEX.md` from all published + draft PRDs.
pub fn rebuild_prd_index(workdir: &Path) -> Result<()> {
    let mut out = String::new();
    let _ = writeln!(out, "# PRD Index");
    let _ = writeln!(out, "\n> Auto-generated. Do not edit manually.");
    let _ = writeln!(out, "> Rebuilt on every `roko prd` command.\n");

    // Ideas count
    let ideas = ideas_path(workdir);
    let idea_count = std::fs::read_to_string(&ideas)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with("- "))
        .count();
    let _ = writeln!(out, "**Ideas**: {idea_count} captured in `ideas.md`\n");

    // Published
    let _ = writeln!(out, "## Published\n");
    let _ = writeln!(out, "| Slug | Title | Crates | Plans | Coverage |");
    let _ = writeln!(out, "|------|-------|--------|-------|----------|");
    let published = list_md_sorted(&published_dir(workdir));
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
    let drafts = list_md_sorted(&drafts_dir(workdir));
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
    let ideas_content = std::fs::read_to_string(&ideas).unwrap_or_default();
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
    let _ = writeln!(out, "> Rebuilt on every `roko plan` command.");
    if plans_dir(workdir)
        .join("_meta/IMPLEMENTATION_ORDER.md")
        .is_file()
    {
        let _ = writeln!(
            out,
            "> Execution order: [`_meta/IMPLEMENTATION_ORDER.md`](_meta/IMPLEMENTATION_ORDER.md)."
        );
    }
    let _ = writeln!(
        out,
        "> Executable totals exclude superseded and archived plans.\n"
    );

    if let Some(parent) = plans_index_path(workdir).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _ = writeln!(out, "## Executable Plans\n");
    let _ = writeln!(out, "| Plan | Tasks | Done | Ready | Status | Parallel |");
    let _ = writeln!(out, "|------|-------|------|-------|--------|----------|");

    let plan_entries = collect_plan_index_entries(workdir);
    if !plans_dir(workdir).is_dir() {
        let _ = writeln!(out, "| _(no plans directory)_ | | | | | |");
        std::fs::write(plans_index_path(workdir), &out)?;
        return Ok(());
    }

    let mut total_tasks = 0u32;
    let mut total_done = 0u32;
    let mut executable_plans = 0u32;
    let mut complete_plans = 0u32;
    let mut ready_plans = 0u32;

    for entry in plan_entries.iter().filter(|entry| !entry.is_inactive()) {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} |",
            entry.name,
            entry.tasks,
            entry.done,
            entry.ready,
            entry.status_label(),
            entry.max_parallel
        );
        total_tasks += entry.tasks;
        total_done += entry.done;
        executable_plans += 1;
        if entry.is_complete() {
            complete_plans += 1;
        } else {
            ready_plans += 1;
        }
    }

    if executable_plans == 0 {
        let _ = writeln!(out, "| _(none)_ | | | | | |");
    }

    let remaining = total_tasks.saturating_sub(total_done);
    let _ = writeln!(
        out,
        "\n**Executable Total**: {} plans, {} tasks, {} done ({:.0}%), {} remaining",
        executable_plans,
        total_tasks,
        total_done,
        if total_tasks > 0 {
            total_done as f64 / total_tasks as f64 * 100.0
        } else {
            0.0
        },
        remaining
    );
    let _ = writeln!(out, "**Complete Plans**: {complete_plans}");
    let _ = writeln!(out, "**Ready/In-Progress Plans**: {ready_plans}");

    let inactive: Vec<&PlanIndexEntry> = plan_entries
        .iter()
        .filter(|entry| entry.is_inactive())
        .collect();
    if !inactive.is_empty() {
        let inactive_tasks: u32 = inactive.iter().map(|entry| entry.tasks).sum();
        let _ = writeln!(out, "\n## Superseded / Archived\n");
        let _ = writeln!(out, "| Plan | Tasks | Status | Replaced By |");
        let _ = writeln!(out, "|------|-------|--------|-------------|");
        for entry in &inactive {
            let _ = writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                entry.name,
                entry.tasks,
                entry.status_label(),
                entry
                    .superseded_by
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("—")
            );
        }
        let _ = writeln!(
            out,
            "\n**Excluded**: {} plans, {} tasks",
            inactive.len(),
            inactive_tasks
        );
    }

    std::fs::write(plans_index_path(workdir), &out)?;
    Ok(())
}

#[derive(Debug, Clone)]
struct PlanIndexEntry {
    name: String,
    tasks: u32,
    done: u32,
    ready: u32,
    max_parallel: String,
    meta_status: String,
    superseded_by: Option<String>,
}

impl PlanIndexEntry {
    fn is_inactive(&self) -> bool {
        matches!(self.meta_status.as_str(), "superseded" | "archived")
    }

    fn is_complete(&self) -> bool {
        self.meta_status == "done" || (self.tasks > 0 && self.done == self.tasks)
    }

    fn status_label(&self) -> String {
        match self.meta_status.as_str() {
            "superseded" => "⏭ superseded".to_string(),
            "archived" => "🗄 archived".to_string(),
            "done" => "✅ complete".to_string(),
            status if self.is_complete() => {
                if status.is_empty() {
                    "✅ complete".to_string()
                } else {
                    format!("✅ {status}")
                }
            }
            status if self.done > 0 => {
                if status.is_empty() || status == "ready" {
                    "🔄 in progress".to_string()
                } else {
                    format!("🔄 {status}")
                }
            }
            "ready" | "" => "📋 ready".to_string(),
            status => format!("📋 {status}"),
        }
    }
}

fn collect_plan_index_entries(workdir: &Path) -> Vec<PlanIndexEntry> {
    let plans_root = plans_dir(workdir);
    if !plans_root.is_dir() {
        return Vec::new();
    }

    let run_state_completed = load_run_state_completed(workdir);
    let mut plan_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&plans_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("tasks.toml").exists() {
                plan_dirs.push(path);
            }
        }
    }
    plan_dirs.sort();

    plan_dirs
        .iter()
        .map(|dir| {
            let name = dir.file_name().unwrap_or_default().to_string_lossy();
            let content = std::fs::read_to_string(dir.join("tasks.toml")).unwrap_or_default();
            let mut counts = count_top_level_tasks(&content);

            // Overlay real completion data from run-state.json if available.
            if let Some(completed_ids) = run_state_completed.get(name.as_ref()) {
                if !completed_ids.is_empty() {
                    counts.1 = completed_ids.len() as u32;
                    counts.2 = counts.0.saturating_sub(counts.1);
                }
            }

            let meta_status = extract_meta_string(&content, "status")
                .unwrap_or_default()
                .trim()
                .to_string();
            let superseded_by = extract_meta_string(&content, "superseded_by");
            let max_parallel =
                extract_meta_value(&content, "max_parallel").unwrap_or_else(|| "—".to_string());

            PlanIndexEntry {
                name: name.to_string(),
                tasks: counts.0,
                done: counts.1,
                ready: counts.2,
                max_parallel,
                meta_status,
                superseded_by,
            }
        })
        .collect()
}

fn load_run_state_completed(workdir: &Path) -> std::collections::HashMap<String, Vec<String>> {
    // tasks.toml is never updated by plan run; completion state lives in
    // run-state.json. This is RunStateSnapshot, not executor.json.
    let run_state_path = workdir.join(".roko/state/run-state.json");
    if !run_state_path.exists() {
        return std::collections::HashMap::new();
    }

    std::fs::read_to_string(&run_state_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|val| {
            val.get("completed_tasks")
                .and_then(|ct| serde_json::from_value(ct.clone()).ok())
        })
        .unwrap_or_default()
}

fn count_top_level_tasks(content: &str) -> (u32, u32, u32) {
    let Ok(parsed) = toml::from_str::<toml::Value>(content) else {
        return (
            content.matches("[[task]]").count() as u32,
            content.matches("status = \"done\"").count() as u32,
            content.matches("status = \"ready\"").count() as u32,
        );
    };

    let Some(tasks) = parsed.get("task").and_then(toml::Value::as_array) else {
        return (0, 0, 0);
    };

    let mut done = 0u32;
    let mut ready = 0u32;
    for task in tasks {
        match task
            .as_table()
            .and_then(|table| table.get("status"))
            .and_then(toml::Value::as_str)
        {
            Some("done") => done += 1,
            Some("ready") | None => ready += 1,
            _ => {}
        }
    }

    (tasks.len() as u32, done, ready)
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
    let published_count = list_md_sorted(&published_dir(workdir)).len();
    let drafts_count = list_md_sorted(&drafts_dir(workdir)).len();
    let ideas_count = std::fs::read_to_string(ideas_path(workdir))
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
    let plan_entries = collect_plan_index_entries(workdir);
    let active_entries: Vec<&PlanIndexEntry> = plan_entries
        .iter()
        .filter(|entry| !entry.is_inactive())
        .collect();
    let plan_count = active_entries.len() as u32;
    let task_count: u32 = active_entries.iter().map(|entry| entry.tasks).sum();
    let done_count: u32 = active_entries.iter().map(|entry| entry.done).sum();
    let complete_count = active_entries
        .iter()
        .filter(|entry| entry.is_complete())
        .count();
    let superseded_count = plan_entries
        .iter()
        .filter(|entry| entry.is_inactive())
        .count();
    let remaining_count = task_count.saturating_sub(done_count);
    let _ = writeln!(
        out,
        "## Plans ({plan_count} executable, {complete_count} complete, {remaining_count} tasks remaining, {superseded_count} superseded)"
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
    let episodes_path = workdir.join(".roko/episodes.jsonl");
    let episode_count = if episodes_path.exists() {
        std::fs::read_to_string(&episodes_path)
            .unwrap_or_default()
            .lines()
            .count()
    } else {
        0
    };
    let _ = writeln!(out, "## Episodes ({episode_count} recorded)");
    let _ = writeln!(out, "→ `.roko/episodes.jsonl`\n");

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

fn extract_meta_string(content: &str, key: &str) -> Option<String> {
    extract_meta_value(content, key).map(|value| value.trim_matches('"').to_string())
}

fn extract_meta_value(content: &str, key: &str) -> Option<String> {
    let parsed = toml::from_str::<toml::Value>(content).ok()?;
    let value = parsed
        .get("meta")
        .and_then(toml::Value::as_table)?
        .get(key)?;

    match value {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Integer(i) => Some(i.to_string()),
        toml::Value::Float(f) => Some(f.to_string()),
        toml::Value::Boolean(b) => Some(b.to_string()),
        _ => Some(value.to_string()),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_paths::{drafts_dir, ideas_path, plans_dir, published_dir};

    #[test]
    fn rebuild_all_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(published_dir(tmp.path())).unwrap();
        std::fs::create_dir_all(drafts_dir(tmp.path())).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/research")).unwrap();
        std::fs::write(ideas_path(tmp.path()), "# Ideas\n").unwrap();
        rebuild_all(tmp.path()).unwrap();
        assert!(master_index_path(tmp.path()).exists());
        assert!(prd_index_path(tmp.path()).exists());
        assert!(research_index_path(tmp.path()).exists());
    }

    #[test]
    fn prd_index_includes_drafts() {
        let tmp = tempfile::tempdir().unwrap();
        let drafts = drafts_dir(tmp.path());
        std::fs::create_dir_all(&drafts).unwrap();
        std::fs::create_dir_all(published_dir(tmp.path())).unwrap();
        std::fs::write(ideas_path(tmp.path()), "# Ideas\n").unwrap();
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
        let plan = plans_dir(tmp.path()).join("test-plan");
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
    fn plans_index_ignores_nested_acceptance_contract_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = plans_dir(tmp.path()).join("contract-plan");
        std::fs::create_dir_all(&plan).unwrap();
        std::fs::write(
            plan.join("tasks.toml"),
            "[meta]\nplan = \"contract\"\nmax_parallel = 1\n\n\
             [[task]]\nid = \"T1\"\ntitle = \"Task\"\nstatus = \"ready\"\n\n\
             [task.acceptance_contract]\nversion = 1\n\n\
             [[task.acceptance_contract.gates]]\nid = \"compile\"\nkind = \"compile\"\ncommand = \"cargo check\"\n",
        )
        .unwrap();

        rebuild_plans_index(tmp.path()).unwrap();

        let content = std::fs::read_to_string(plans_index_path(tmp.path())).unwrap();
        assert!(
            content.contains("| `contract-plan` | 1 | 0 | 1 |"),
            "nested gate id/status changed plan counts: {content}"
        );
    }

    #[test]
    fn plans_index_excludes_superseded_from_executable_totals() {
        let tmp = tempfile::tempdir().unwrap();
        let active = plans_dir(tmp.path()).join("active-plan");
        let old = plans_dir(tmp.path()).join("old-plan");
        std::fs::create_dir_all(&active).unwrap();
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            active.join("tasks.toml"),
            "[meta]\nplan = \"active\"\nstatus = \"ready\"\nmax_parallel = 1\n\n\
             [[task]]\nid = \"T1\"\nstatus = \"done\"\n\n\
             [[task]]\nid = \"T2\"\nstatus = \"ready\"\n",
        )
        .unwrap();
        std::fs::write(
            old.join("tasks.toml"),
            "[meta]\nplan = \"old\"\nstatus = \"superseded\"\n\
             superseded_by = \"active-plan\"\nmax_parallel = 1\n\n\
             [[task]]\nid = \"T1\"\nstatus = \"ready\"\n\n\
             [[task]]\nid = \"T2\"\nstatus = \"ready\"\n",
        )
        .unwrap();

        rebuild_plans_index(tmp.path()).unwrap();

        let content = std::fs::read_to_string(plans_index_path(tmp.path())).unwrap();
        assert!(content.contains("| `active-plan` | 2 | 1 | 1 |"));
        assert!(content.contains("## Superseded / Archived"));
        assert!(content.contains("| `old-plan` | 2 | ⏭ superseded | active-plan |"));
        assert!(content.contains("**Executable Total**: 1 plans, 2 tasks, 1 done"));
        assert!(content.contains("**Excluded**: 1 plans, 2 tasks"));
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
        std::fs::create_dir_all(published_dir(tmp.path())).unwrap();
        std::fs::create_dir_all(drafts_dir(tmp.path())).unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko/research")).unwrap();
        std::fs::write(ideas_path(tmp.path()), "# Ideas\n").unwrap();
        rebuild_all(tmp.path()).unwrap();
        let content = std::fs::read_to_string(master_index_path(tmp.path())).unwrap();
        assert!(content.contains("## PRDs"));
        assert!(content.contains("## Plans"));
        assert!(content.contains("## Research"));
        assert!(content.contains("## Episodes"));
        assert!(content.contains("## Config"));
    }

    #[test]
    fn append_master_index_prompt_skips_empty_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mut prompt = String::from("prefix\n");

        append_master_index_prompt(&mut prompt, tmp.path(), "## Existing");

        assert_eq!(prompt, "prefix\n");
    }

    #[test]
    fn append_master_index_prompt_includes_heading_and_separator() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".roko")).unwrap();
        std::fs::write(master_index_path(tmp.path()), "# Master\n").unwrap();

        let mut prompt = String::new();
        append_master_index_prompt(&mut prompt, tmp.path(), "## Existing");

        assert!(prompt.contains("## Existing"));
        assert!(prompt.contains("# Master"));
        assert!(prompt.ends_with("---\n\n"));
    }
}
