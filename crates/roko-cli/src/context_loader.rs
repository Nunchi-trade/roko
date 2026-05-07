//! Load context files from paths, globs, and directories for prompt injection.

use std::path::{Path, PathBuf};

/// Default character budget for context loading.
pub const DEFAULT_BUDGET: usize = 50_000;

/// Directories to skip when walking recursively.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".roko",
    "__pycache__",
    "dist",
];

/// Binary extensions to skip.
const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "ico", "svg", "webp", "woff", "woff2", "ttf", "eot", "otf",
    "mp3", "mp4", "avi", "mov", "pdf", "zip", "tar", "gz", "bz2", "xz", "7z", "bin", "exe", "dll",
    "so", "dylib", "o", "a", "wasm", "pyc", "class", "jar",
];

/// A loaded context file ready for prompt inclusion.
#[derive(Debug)]
struct ContextFile {
    rel_path: String,
    content: String,
}

/// Load context files from the given paths (supports globs and directories).
///
/// Returns formatted XML-like blocks:
///   `<file path="relative/path">content</file>`
///
/// Respects a character budget; shorter files are prioritized.
pub fn load_context_files(paths: &[PathBuf], budget: usize, workdir: &Path) -> String {
    let mut candidates: Vec<PathBuf> = Vec::new();

    for path in paths {
        let path_str = path.to_string_lossy();
        // Check if it's a glob pattern
        if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
            let pattern = if path.is_relative() {
                workdir.join(path).to_string_lossy().to_string()
            } else {
                path_str.to_string()
            };
            if let Ok(entries) = glob::glob(&pattern) {
                for entry in entries.flatten() {
                    if entry.is_file() {
                        candidates.push(entry);
                    }
                }
            }
        } else {
            let resolved = if path.is_relative() {
                workdir.join(path)
            } else {
                path.to_path_buf()
            };
            if resolved.is_dir() {
                walk_dir(&resolved, &mut candidates);
            } else if resolved.is_file() {
                candidates.push(resolved);
            }
        }
    }

    // Deduplicate
    candidates.sort();
    candidates.dedup();

    // Filter out binary files
    candidates.retain(|p| !is_binary_extension(p));

    // Sort by file size (shorter first for relevance priority)
    candidates.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));

    // Load files within budget
    let mut loaded: Vec<ContextFile> = Vec::new();
    let mut used = 0usize;

    for path in &candidates {
        if used >= budget {
            break;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(workdir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let file_cost = rel.len() + content.len() + 40; // overhead for tags
        if used + file_cost > budget && !loaded.is_empty() {
            break;
        }
        used += file_cost;
        loaded.push(ContextFile {
            rel_path: rel,
            content,
        });
    }

    // Format output
    let mut out = String::new();
    for file in &loaded {
        out.push_str(&format!(
            "<file path=\"{}\">\n{}\n</file>\n",
            file.rel_path, file.content
        ));
    }

    let skipped = candidates.len().saturating_sub(loaded.len());
    if skipped > 0 {
        out.push_str(&format!(
            "\n<!-- {skipped} file(s) skipped due to context budget -->\n"
        ));
    }

    out
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if SKIP_DIRS.contains(&name) {
                continue;
            }
            walk_dir(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

fn is_binary_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| SKIP_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}
