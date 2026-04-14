//! Shared path helpers for workspace-local `.roko` artifacts.

use std::path::{Path, PathBuf};

/// Resolve the workspace-local `.roko` root.
#[must_use]
pub fn roko_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko")
}

/// Resolve the canonical plans directory.
#[must_use]
pub fn plans_dir(workdir: &Path) -> PathBuf {
    roko_dir(workdir).join("plans")
}

/// Resolve the PRD root directory.
#[must_use]
pub fn prd_dir(workdir: &Path) -> PathBuf {
    roko_dir(workdir).join("prd")
}

/// Resolve the ideas markdown file.
#[must_use]
pub fn ideas_path(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("ideas.md")
}

/// Resolve the PRD drafts directory.
#[must_use]
pub fn drafts_dir(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("drafts")
}

/// Resolve the published PRD directory.
#[must_use]
pub fn published_dir(workdir: &Path) -> PathBuf {
    prd_dir(workdir).join("published")
}

/// Resolve the draft PRD markdown path for a slug.
#[must_use]
pub fn draft_prd_path(workdir: &Path, slug: &str) -> PathBuf {
    drafts_dir(workdir).join(format!("{slug}.md"))
}

/// Resolve the published PRD markdown path for a slug.
#[must_use]
pub fn published_prd_path(workdir: &Path, slug: &str) -> PathBuf {
    published_dir(workdir).join(format!("{slug}.md"))
}

/// Find a PRD by slug in published or draft state.
#[must_use]
pub fn find_prd_path(workdir: &Path, slug: &str) -> Option<PathBuf> {
    let published = published_prd_path(workdir, slug);
    if published.exists() {
        return Some(published);
    }

    let draft = draft_prd_path(workdir, slug);
    if draft.exists() {
        return Some(draft);
    }

    None
}
