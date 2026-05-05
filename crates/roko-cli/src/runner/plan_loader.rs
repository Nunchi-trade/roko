//! Plan discovery and loading — loads `tasks.toml` without scanning `.md`
//! files or applying enrichment.
//!
//! Also provides [`scaffold_missing_crates`] which creates stub crate
//! directories for plans that reference crates not yet on disk (e.g. when a
//! plan's first task is to *create* a new crate).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use roko_fs::RokoLayout;
use tracing::info;

use crate::task_parser::TasksFile;

/// A loaded plan ready for execution.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Plan identifier (directory name).
    pub id: String,
    /// Directory containing this plan's `tasks.toml`.
    pub dir: PathBuf,
    /// Parsed task definitions.
    pub tasks: TasksFile,
    /// Short excerpt from the plan's PRD document (empty when no PRD exists).
    pub prd_excerpt: String,
}

/// Load a single plan from a directory that must contain `tasks.toml`.
pub fn load_plan(dir: &Path) -> Result<Plan> {
    let tasks_path = dir.join("tasks.toml");
    let content = match std::fs::read_to_string(&tasks_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("No tasks.toml found in {}", dir.display());
        }
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}", tasks_path.display())));
        }
    };

    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());

    let tasks = TasksFile::parse_str(&content)
        .with_context(|| format!("failed to parse {}", tasks_path.display()))?;
    let schema_issues = tasks.validate_against_schema();
    if !schema_issues.is_empty() {
        let details = schema_issues
            .iter()
            .map(|issue| format!("  - {issue}"))
            .collect::<Vec<_>>()
            .join("\n");
        bail!(
            "schema validation failed for {}:\n{}",
            tasks_path.display(),
            details
        );
    }

    let workdir = find_workspace_root(dir);
    let prd_excerpt = load_prd_excerpt_for_plan(workdir.as_deref(), &id);

    info!(plan_id = %id, task_count = tasks.tasks.len(), "loaded plan");
    Ok(Plan {
        id,
        dir: dir.to_path_buf(),
        tasks,
        prd_excerpt,
    })
}

/// Load plan(s) from a directory.
///
/// - If `dir/tasks.toml` exists, returns a single plan rooted at `dir`.
/// - Otherwise, scans immediate subdirectories for `tasks.toml` files.
/// - Never scans `.md` files or modifies anything on disk.
pub fn load_plans(dir: &Path) -> Result<Vec<Plan>> {
    // Case 1: dir itself is a plan (try reading tasks.toml directly)
    match std::fs::read_to_string(dir.join("tasks.toml")) {
        Ok(_) => return Ok(vec![load_plan(dir)?]),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(anyhow::Error::new(e).context(format!("read {}/tasks.toml", dir.display())));
        }
    }

    // Case 2: scan subdirs
    let mut plans = Vec::new();
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot read directory {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Attempt to read tasks.toml — if NotFound, this subdir is not a plan.
        match std::fs::read_to_string(path.join("tasks.toml")) {
            Ok(_) => match load_plan(&path) {
                Ok(plan) => plans.push(plan),
                Err(e) => {
                    tracing::warn!(dir = %path.display(), err = %e, "skipping plan with parse error");
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(dir = %path.display(), err = %e, "failed to probe tasks.toml");
            }
        }
    }

    if plans.is_empty() {
        bail!("No plans found in {}", dir.display());
    }

    // Sort by name for deterministic ordering.
    plans.sort_by(|a, b| a.id.cmp(&b.id));

    info!(count = plans.len(), "discovered plans");
    Ok(plans)
}

/// Walk up from `start` looking for the workspace root (a directory that
/// contains `.roko/`).  Returns `None` when no such ancestor is found.
fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if RokoLayout::for_project(&current).root().is_dir() {
            return Some(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return None,
        }
    }
}

/// Load a PRD excerpt for `plan_id` relative to `workdir`.
///
/// Checks:
/// 1. `{workdir}/.roko/prd/published/{plan_id}.md`
/// 2. `{workdir}/.roko/prd/draft/{plan_id}.md`
///
/// Returns an empty string when `workdir` is `None` or no PRD file exists.
fn load_prd_excerpt_for_plan(workdir: Option<&Path>, plan_id: &str) -> String {
    const PRD_LIMIT: usize = 2_000;
    let Some(root) = workdir else {
        return String::new();
    };
    let prd_base = RokoLayout::for_project(root).prd_dir();
    let candidates = [
        prd_base.join("published").join(format!("{plan_id}.md")),
        prd_base.join("draft").join(format!("{plan_id}.md")),
    ];
    for path in &candidates {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                return if content.len() > PRD_LIMIT {
                    let mut s = content.chars().take(PRD_LIMIT).collect::<String>();
                    s.push_str("\n[truncated]");
                    s
                } else {
                    content
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "failed to read PRD file");
                continue;
            }
        }
    }
    String::new()
}

/// Validate that a crate name is safe and follows Rust naming conventions.
///
/// Rejects:
/// - Empty names
/// - Names starting with `-` or `.`
/// - Names containing `..` (directory traversal)
/// - Names containing `/` or `\` (path separators)
/// - Names with characters outside `[a-zA-Z0-9_-]`
fn is_valid_crate_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.starts_with('.')
        && !name.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Scan all tasks across `plans` for file references to crates that don't
/// exist yet, then scaffold those crates (`Cargo.toml` + `src/lib.rs`) and
/// register them in the workspace `Cargo.toml` members list.
///
/// This handles the common case where a plan's first task is to *create* a new
/// crate — the gate would fail (`cargo check` can't find the crate) unless a
/// minimal scaffold is present.
///
/// Returns the names of any newly created crates.
pub fn scaffold_missing_crates(workdir: &Path, plans: &[Plan]) -> Result<Vec<String>> {
    let crates_dir = workdir.join("crates");
    let mut scaffolded: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    info!(
        workdir = %workdir.display(),
        plan_count = plans.len(),
        "scaffold_missing_crates: scanning plans"
    );

    // First pass: collect crate names and whether any file ref mentions src/main.rs.
    let mut crate_needs_main: HashSet<String> = HashSet::new();
    for plan in plans {
        let task_count = plan.tasks.tasks.len();
        let total_files: usize = plan.tasks.tasks.iter().map(|t| t.files.len()).sum();
        info!(
            plan_id = %plan.id,
            task_count,
            total_files,
            "scaffold: scanning plan"
        );
        for task in &plan.tasks.tasks {
            for file_ref in &task.files {
                let parts: Vec<&str> = file_ref.splitn(3, '/').collect();
                if parts.len() >= 2 && parts[0] == "crates" {
                    let crate_name = parts[1].to_string();
                    if crate_name.is_empty() || crate_name.contains('*') {
                        continue;
                    }
                    // Validate crate name to prevent directory traversal and
                    // invalid Rust crate names.
                    if !is_valid_crate_name(&crate_name) {
                        info!(
                            crate_name = %crate_name,
                            "scaffold: skipping invalid crate name"
                        );
                        continue;
                    }
                    // Track if any file ref for this crate mentions src/main.rs.
                    if parts.len() == 3 && parts[2] == "src/main.rs" {
                        crate_needs_main.insert(crate_name.clone());
                    }
                    if !seen.insert(crate_name.clone()) {
                        continue;
                    }
                    let crate_dir = crates_dir.join(&crate_name);
                    if crate_dir.exists() {
                        continue;
                    }

                    scaffolded.push(crate_name);
                }
            }
        }
    }

    // Second pass: create scaffold files for each new crate.
    for crate_name in &scaffolded {
        let crate_dir = crates_dir.join(crate_name);
        let src_dir = crate_dir.join("src");
        std::fs::create_dir_all(&src_dir)
            .with_context(|| format!("scaffold: create {}", src_dir.display()))?;

        let is_bin = crate_needs_main.contains(crate_name.as_str());

        // Infer inter-crate dependencies from task graph:
        // If any task targets this crate and depends_on tasks in other crates,
        // those other crates are likely dependencies.
        let mut deps: Vec<String> = Vec::new();
        for plan in plans {
            for task in &plan.tasks.tasks {
                let targets_this = task.files.iter().any(|f| {
                    f.starts_with(&format!("crates/{crate_name}/"))
                        || f.starts_with(&format!("crates/{}/", crate_name.replace('-', "_")))
                });
                if targets_this {
                    for dep_id in &task.depends_on {
                        // Find the dependency task and extract its target crate
                        for other_task in &plan.tasks.tasks {
                            if &other_task.id == dep_id {
                                for f in &other_task.files {
                                    if let Some(rest) = f.strip_prefix("crates/") {
                                        if let Some(dep_crate) = rest.split('/').next() {
                                            if dep_crate != crate_name
                                                && !deps.contains(&dep_crate.to_string())
                                            {
                                                deps.push(dep_crate.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let deps_section = if deps.is_empty() {
            String::new()
        } else {
            let mut s = String::from("\n[dependencies]\n");
            for dep in &deps {
                s.push_str(&format!(
                    "{} = {{ path = \"../{dep}\" }}\n",
                    dep.replace('-', "_")
                ));
            }
            s
        };
        tracing::debug!(
            crate_name,
            dep_count = deps.len(),
            deps = ?deps,
            "scaffold: inferred inter-crate dependencies"
        );

        let cargo_toml = if is_bin {
            format!(
                "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{crate_name}\"\npath = \"src/main.rs\"\n{deps_section}"
            )
        } else {
            format!(
                "[package]\nname = \"{crate_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n{deps_section}"
            )
        };
        std::fs::write(crate_dir.join("Cargo.toml"), cargo_toml)
            .with_context(|| format!("scaffold: write {}/Cargo.toml", crate_dir.display()))?;

        if is_bin {
            std::fs::write(src_dir.join("main.rs"), "fn main() {}\n")
                .with_context(|| format!("scaffold: write {}/src/main.rs", crate_dir.display()))?;
        } else {
            std::fs::write(src_dir.join("lib.rs"), "")
                .with_context(|| format!("scaffold: write {}/src/lib.rs", crate_dir.display()))?;
        }

        info!("scaffolded new crate crates/{crate_name}/ (bin={is_bin})");
    }

    // Register scaffolded crates in workspace Cargo.toml members.
    if !scaffolded.is_empty() {
        let ws_cargo_path = workdir.join("Cargo.toml");

        // Try to read the existing workspace Cargo.toml. If it doesn't exist,
        // create a minimal manifest so `cargo check` can succeed.
        let ws_content = match std::fs::read_to_string(&ws_cargo_path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let minimal = "[workspace]\nresolver = \"2\"\nmembers = [\n]\n".to_string();
                std::fs::write(&ws_cargo_path, &minimal)
                    .context("scaffold: create workspace Cargo.toml")?;
                info!(
                    "created minimal workspace Cargo.toml at {}",
                    ws_cargo_path.display()
                );
                minimal
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context("scaffold: read workspace Cargo.toml"));
            }
        };

        let mut new_content = ws_content.clone();
        for name in &scaffolded {
            let member_entry = format!("\"crates/{name}\"");
            if new_content.contains(&member_entry) {
                continue;
            }
            // Find the `members = [` line, then locate the matching `]`.
            // Skip comment lines and nested brackets to handle:
            //   members = [
            //     # core crates
            //     "crates/roko-core",
            //   ]
            if let Some(members_pos) = new_content.find("members") {
                if let Some(open_bracket) = new_content[members_pos..].find('[') {
                    let search_start = members_pos + open_bracket + 1;
                    let mut depth = 1i32;
                    let mut close_pos = None;
                    for (i, ch) in new_content[search_start..].char_indices() {
                        match ch {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    close_pos = Some(search_start + i);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(insert_at) = close_pos {
                        let insertion = format!("    {member_entry},\n");
                        new_content.insert_str(insert_at, &insertion);
                    }
                }
            }
        }

        if new_content != ws_content {
            std::fs::write(&ws_cargo_path, &new_content)
                .context("scaffold: write workspace Cargo.toml")?;
            info!(
                "added {} new crate(s) to workspace members: {:?}",
                scaffolded.len(),
                scaffolded
            );
        }
    }

    Ok(scaffolded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_tasks_toml(dir: &Path, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("tasks.toml"), content).unwrap();
    }

    const MINIMAL_TASKS: &str = r#"
[meta]
plan = "test-plan"

[[task]]
id = "T1"
title = "Do something"
role = "researcher"
"#;

    #[test]
    fn load_single_plan() {
        let tmp = tempfile::tempdir().unwrap();
        let plan_dir = tmp.path().join("my-plan");
        write_tasks_toml(&plan_dir, MINIMAL_TASKS);

        let plan = load_plan(&plan_dir).unwrap();
        assert_eq!(plan.id, "my-plan");
        assert_eq!(plan.tasks.tasks.len(), 1);
        assert_eq!(plan.tasks.meta.plan, "test-plan");
    }

    #[test]
    fn load_plan_missing_tasks_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plan(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn load_plan_rejects_schema_issues() {
        let tmp = tempfile::tempdir().unwrap();
        let plan_dir = tmp.path().join("bad-plan");
        write_tasks_toml(
            &plan_dir,
            r#"
[meta]
plan = "bad-plan"

[[task]]
id = "T1"
title = "Missing implementer fields"
role = "implementer"
"#,
        );

        let error = load_plan(&plan_dir).unwrap_err().to_string();
        assert!(error.contains("schema validation failed"));
        assert!(error.contains("missing 'verify'"));
        assert!(error.contains("missing 'files'"));
    }

    #[test]
    fn load_plans_direct() {
        let tmp = tempfile::tempdir().unwrap();
        write_tasks_toml(tmp.path(), MINIMAL_TASKS);

        let plans = load_plans(tmp.path()).unwrap();
        assert_eq!(plans.len(), 1);
    }

    #[test]
    fn load_plans_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        write_tasks_toml(&tmp.path().join("plan-a"), MINIMAL_TASKS);
        write_tasks_toml(
            &tmp.path().join("plan-b"),
            &MINIMAL_TASKS.replace("test-plan", "plan-b"),
        );

        let plans = load_plans(tmp.path()).unwrap();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].id, "plan-a");
        assert_eq!(plans[1].id, "plan-b");
    }

    #[test]
    fn load_plans_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let result = load_plans(tmp.path());
        assert!(result.is_err());
    }

    const TASKS_WITH_CRATE_FILES: &str = r#"
[meta]
plan = "test-plan"

[[task]]
id = "T1"
title = "Create new crate"
role = "researcher"
files = ["crates/my-new-crate/src/lib.rs", "crates/my-new-crate/Cargo.toml"]
"#;

    #[test]
    fn scaffold_creates_missing_crate() {
        let tmp = tempfile::tempdir().unwrap();
        let plan_dir = tmp.path().join("plan");
        write_tasks_toml(&plan_dir, TASKS_WITH_CRATE_FILES);

        let plans = load_plans(&plan_dir).unwrap();
        let scaffolded = scaffold_missing_crates(tmp.path(), &plans).unwrap();

        assert_eq!(scaffolded, vec!["my-new-crate"]);
        assert!(tmp.path().join("crates/my-new-crate/src/lib.rs").exists());
        assert!(tmp.path().join("crates/my-new-crate/Cargo.toml").exists());
        // Workspace Cargo.toml should have been created.
        let ws = fs::read_to_string(tmp.path().join("Cargo.toml")).unwrap();
        assert!(ws.contains("\"crates/my-new-crate\""));
    }

    #[test]
    fn scaffold_skips_existing_crate() {
        let tmp = tempfile::tempdir().unwrap();
        // Pre-create the crate directory.
        fs::create_dir_all(tmp.path().join("crates/my-new-crate/src")).unwrap();
        fs::write(tmp.path().join("crates/my-new-crate/Cargo.toml"), "").unwrap();

        let plan_dir = tmp.path().join("plan");
        write_tasks_toml(&plan_dir, TASKS_WITH_CRATE_FILES);

        let plans = load_plans(&plan_dir).unwrap();
        let scaffolded = scaffold_missing_crates(tmp.path(), &plans).unwrap();

        assert!(scaffolded.is_empty());
    }

    #[test]
    fn scaffold_ignores_non_crate_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks = r#"
[meta]
plan = "test-plan"

[[task]]
id = "T1"
title = "Edit root file"
role = "researcher"
files = ["src/main.rs", "README.md"]
"#;
        let plan_dir = tmp.path().join("plan");
        write_tasks_toml(&plan_dir, tasks);

        let plans = load_plans(&plan_dir).unwrap();
        let scaffolded = scaffold_missing_crates(tmp.path(), &plans).unwrap();
        assert!(scaffolded.is_empty());
    }

    #[test]
    fn scaffold_creates_bin_crate_for_main_rs() {
        let tmp = tempfile::tempdir().unwrap();
        let tasks = r#"
[meta]
plan = "test-plan"

[[task]]
id = "T1"
title = "Create CLI crate"
role = "researcher"
files = ["crates/my-cli/src/main.rs", "crates/my-cli/Cargo.toml"]
"#;
        let plan_dir = tmp.path().join("plan");
        write_tasks_toml(&plan_dir, tasks);

        let plans = load_plans(&plan_dir).unwrap();
        let scaffolded = scaffold_missing_crates(tmp.path(), &plans).unwrap();

        assert_eq!(scaffolded, vec!["my-cli"]);
        // Should have main.rs, not lib.rs.
        assert!(tmp.path().join("crates/my-cli/src/main.rs").exists());
        assert!(!tmp.path().join("crates/my-cli/src/lib.rs").exists());
        // main.rs should contain fn main().
        let main_content =
            fs::read_to_string(tmp.path().join("crates/my-cli/src/main.rs")).unwrap();
        assert!(main_content.contains("fn main()"));
        // Cargo.toml should have [[bin]] section.
        let cargo = fs::read_to_string(tmp.path().join("crates/my-cli/Cargo.toml")).unwrap();
        assert!(cargo.contains("[[bin]]"));
    }
}
