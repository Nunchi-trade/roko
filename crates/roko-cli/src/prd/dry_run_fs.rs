use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};

use crate::task_parser::TasksFile;

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

pub(super) fn snapshot_tasks_files(root: &Path) -> HashMap<PathBuf, u64> {
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

pub(super) fn warn_on_new_or_updated_tasks(root: &Path, before: &HashMap<PathBuf, u64>) {
    for path in changed_tasks_files(root, before) {
        warn_on_tasks_quality(&path);
    }
}

pub(super) fn changed_tasks_files(root: &Path, before: &HashMap<PathBuf, u64>) -> Vec<PathBuf> {
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

pub(super) struct DryRunWorkspace {
    path: PathBuf,
}

impl DryRunWorkspace {
    pub(super) fn new(src: &Path) -> Result<Self> {
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

    pub(super) fn path(&self) -> &Path {
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
        details.push_str(&format!(
            " | files={} deps={} verify={}",
            task.files.len(),
            task.depends_on.len(),
            task.verify.len()
        ));
        println!("{details}");
    }
}

pub(super) fn validate_and_print_preview(path: &Path) -> Result<()> {
    let tasks_file = TasksFile::parse(path).with_context(|| format!("parse {}", path.display()))?;
    let issues = tasks_file.validate();
    if !issues.is_empty() {
        eprintln!("❌ Dry-run validation failed for {}:", path.display());
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        return Err(anyhow!(
            "dry-run plan validation failed for {}",
            path.display()
        ));
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
