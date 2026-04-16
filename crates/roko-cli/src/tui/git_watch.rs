//! Debounced git watcher for the TUI git view.
//!
//! Watches git admin paths instead of polling `git` every few seconds. Linked
//! worktrees are resolved through their `.git` indirection file so checkout,
//! commit, and fetch activity all refresh the Git tab promptly.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{
    DebounceEventHandler, DebounceEventResult, Debouncer, FileIdMap, new_debouncer_opt,
};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(500);
const CHANNEL_BOUND: usize = 4;

type NotifyDebouncer = Debouncer<notify::RecommendedWatcher, FileIdMap>;

/// Coalesced git refresh notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitRefresh {
    /// One or more git-admin filesystem changes landed in a debounce window.
    Coalesced,
}

/// Keeps the git watcher alive for the app lifetime.
pub(crate) struct GitWatchHandle {
    /// Receiver side for refresh notifications.
    pub(crate) rx: Receiver<GitRefresh>,
    _backend: GitWatchBackend,
}

#[allow(dead_code)]
enum GitWatchBackend {
    Notify(NotifyDebouncer),
    Poll(PollerHandle),
    Disabled,
}

struct PollerHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl Drop for PollerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            if let Err(err) = join.join() {
                tracing::warn!(?err, "failed to join git poll fallback thread");
            }
        }
    }
}

struct RefreshHandler {
    tx: SyncSender<GitRefresh>,
}

impl DebounceEventHandler for RefreshHandler {
    fn handle_event(&mut self, event: DebounceEventResult) {
        match event {
            Ok(events) => {
                if !events.is_empty() {
                    let _ = self.tx.try_send(GitRefresh::Coalesced);
                }
            }
            Err(errors) => {
                tracing::warn!(count = errors.len(), "git watcher debounce error");
                let _ = self.tx.try_send(GitRefresh::Coalesced);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct WatchRoot {
    path: PathBuf,
    recursive_mode: RecursiveMode,
}

#[derive(Clone, Debug)]
struct GitWatchPaths {
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl GitWatchPaths {
    fn discover(workdir: &Path) -> Result<Option<Self>> {
        let git_path = workdir.join(".git");
        if git_path.is_dir() {
            let git_dir = normalize_path(&git_path);
            return Ok(Some(Self {
                git_dir: git_dir.clone(),
                common_dir: git_dir,
            }));
        }

        if !git_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&git_path)
            .with_context(|| format!("failed to read {}", git_path.display()))?;
        let gitdir = content
            .trim()
            .strip_prefix("gitdir: ")
            .ok_or_else(|| anyhow!("unparseable gitdir pointer in {}", git_path.display()))?;
        let git_dir = normalize_path(&resolve_path(workdir, gitdir));
        let common_dir = normalize_path(&read_common_dir(&git_dir)?);
        Ok(Some(Self {
            git_dir,
            common_dir,
        }))
    }

    fn watch_roots(&self) -> Vec<WatchRoot> {
        let mut roots = vec![WatchRoot {
            path: self.git_dir.clone(),
            recursive_mode: RecursiveMode::Recursive,
        }];

        if self.common_dir != self.git_dir {
            roots.push(WatchRoot {
                path: self.common_dir.join("refs"),
                recursive_mode: RecursiveMode::Recursive,
            });
            roots.push(WatchRoot {
                path: self.common_dir.clone(),
                recursive_mode: RecursiveMode::NonRecursive,
            });
        }

        dedup_existing_roots(roots)
    }
}

impl GitWatchHandle {
    fn disabled() -> Self {
        let (_tx, rx) = mpsc::channel();
        Self {
            rx,
            _backend: GitWatchBackend::Disabled,
        }
    }

    /// Try to read the next refresh signal without blocking.
    #[must_use]
    pub(crate) fn try_recv(&self) -> std::result::Result<GitRefresh, TryRecvError> {
        self.rx.try_recv()
    }
}

/// Watch the current git repository or worktree, falling back to metadata
/// polling if `notify` cannot initialize.
pub(crate) fn watch_git_repo_with_fallback(workdir: &Path) -> GitWatchHandle {
    match GitWatchPaths::discover(workdir) {
        Ok(Some(paths)) => match watch_git_repo(paths.clone()) {
            Ok(handle) => handle,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    cadence = ?FALLBACK_POLL_INTERVAL,
                    "git fs-watch unavailable; falling back to metadata poll"
                );
                spawn_poll_fallback(paths, FALLBACK_POLL_INTERVAL)
            }
        },
        Ok(None) => {
            tracing::info!(
                path = %workdir.display(),
                "git watch disabled; workdir is not a git repo"
            );
            GitWatchHandle::disabled()
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                path = %workdir.display(),
                "failed to resolve git watch paths"
            );
            GitWatchHandle::disabled()
        }
    }
}

fn watch_git_repo(paths: GitWatchPaths) -> Result<GitWatchHandle> {
    let roots = paths.watch_roots();
    if roots.is_empty() {
        return Ok(GitWatchHandle::disabled());
    }

    let (tx, rx) = mpsc::sync_channel(CHANNEL_BOUND);
    let handler = RefreshHandler { tx };
    let mut debouncer: NotifyDebouncer = new_debouncer_opt(
        DEBOUNCE_WINDOW,
        None,
        handler,
        FileIdMap::new(),
        notify::Config::default(),
    )
    .context("failed to create debounced git watcher")?;

    for root in &roots {
        debouncer
            .watcher()
            .watch(&root.path, root.recursive_mode)
            .with_context(|| format!("failed to watch {}", root.path.display()))?;
        debouncer.cache().add_root(&root.path, root.recursive_mode);
    }

    Ok(GitWatchHandle {
        rx,
        _backend: GitWatchBackend::Notify(debouncer),
    })
}

fn spawn_poll_fallback(paths: GitWatchPaths, interval: Duration) -> GitWatchHandle {
    let roots = paths.watch_roots();
    if roots.is_empty() {
        return GitWatchHandle::disabled();
    }

    let (tx, rx) = mpsc::sync_channel(CHANNEL_BOUND);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let poll_roots = roots;
    let mut last_fingerprint = fingerprint_roots(&poll_roots);

    let join = thread::Builder::new()
        .name("tui-git-poll-fallback".into())
        .spawn(move || {
            loop {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                thread::sleep(interval);

                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                let fingerprint = fingerprint_roots(&poll_roots);
                if fingerprint != last_fingerprint {
                    last_fingerprint = fingerprint;
                    match tx.try_send(GitRefresh::Coalesced) {
                        Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                    }
                }
            }
        })
        .unwrap_or_else(|error| {
            tracing::warn!(
                error = %error,
                thread = "tui-git-poll-fallback",
                "failed to spawn git poll fallback thread"
            );
            thread::spawn(|| {})
        });

    GitWatchHandle {
        rx,
        _backend: GitWatchBackend::Poll(PollerHandle {
            stop,
            join: Some(join),
        }),
    }
}

fn read_common_dir(git_dir: &Path) -> Result<PathBuf> {
    let commondir_path = git_dir.join("commondir");
    if !commondir_path.exists() {
        return Ok(git_dir.to_path_buf());
    }

    let content = fs::read_to_string(&commondir_path)
        .with_context(|| format!("failed to read {}", commondir_path.display()))?;
    Ok(resolve_path(git_dir, content.trim()))
}

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn dedup_existing_roots(roots: Vec<WatchRoot>) -> Vec<WatchRoot> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();

    for root in roots {
        let normalized = normalize_path(&root.path);
        if !normalized.exists() {
            continue;
        }

        let key = (
            normalized.clone(),
            root.recursive_mode == RecursiveMode::Recursive,
        );
        if seen.insert(key) {
            deduped.push(WatchRoot {
                path: normalized,
                recursive_mode: root.recursive_mode,
            });
        }
    }

    deduped
}

fn fingerprint_roots(roots: &[WatchRoot]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for root in roots {
        hash_path(
            &root.path,
            root.recursive_mode == RecursiveMode::Recursive,
            &mut hasher,
        );
    }
    hasher.finish()
}

fn hash_path(path: &Path, recursive: bool, hasher: &mut DefaultHasher) {
    path.hash(hasher);

    let Ok(metadata) = fs::metadata(path) else {
        false.hash(hasher);
        return;
    };

    metadata.is_dir().hash(hasher);
    metadata.len().hash(hasher);
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .hash(hasher);

    if !metadata.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        false.hash(hasher);
        return;
    };

    let mut children: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    children.sort();

    for child in children {
        child.hash(hasher);
        if recursive {
            hash_path(&child, true, hasher);
            continue;
        }

        let Ok(child_meta) = fs::metadata(&child) else {
            false.hash(hasher);
            continue;
        };

        child_meta.is_dir().hash(hasher);
        child_meta.len().hash(hasher);
        child_meta
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_git_path_returns_disabled_handle() {
        let tempdir = tempfile::tempdir().unwrap();
        let handle = watch_git_repo_with_fallback(tempdir.path());
        assert!(handle.try_recv().is_err());
    }
}
