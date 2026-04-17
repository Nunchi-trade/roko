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

use anyhow::{Context, Result};
use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{
    DebounceEventHandler, DebounceEventResult, Debouncer, FileIdMap, new_debouncer_opt,
};

const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);
const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(1);
const CHANNEL_BOUND: usize = 4;

type NotifyDebouncer = Debouncer<notify::RecommendedWatcher, FileIdMap>;

/// Coalesced filesystem refresh signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsRefresh {
    /// One or more filesystem events landed in a debounce window.
    Coalesced,
}

/// Keeps the filesystem watcher alive for the app lifetime.
pub struct FsWatchHandle {
    /// Receiver side for refresh notifications.
    pub rx: Receiver<FsRefresh>,
    _backend: FsWatchBackend,
}

#[allow(dead_code)]
enum FsWatchBackend {
    Notify(NotifyDebouncer),
    Poll(PollerHandle),
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
                tracing::warn!(?err, "failed to join .roko poll fallback thread");
            }
        }
    }
}

struct RefreshHandler {
    tx: SyncSender<FsRefresh>,
}

impl DebounceEventHandler for RefreshHandler {
    fn handle_event(&mut self, event: DebounceEventResult) {
        match event {
            Ok(events) => {
                if !events.is_empty() {
                    let _ = self.tx.try_send(FsRefresh::Coalesced);
                }
            }
            Err(errors) => {
                tracing::warn!(count = errors.len(), "filesystem watcher debounce error");
                let _ = self.tx.try_send(FsRefresh::Coalesced);
            }
        }
    }
}

impl FsWatchHandle {
    /// Try to read the next refresh signal without blocking.
    #[must_use]
    pub fn try_recv(&self) -> std::result::Result<FsRefresh, TryRecvError> {
        self.rx.try_recv()
    }
}

/// Watch `.roko/` recursively and coalesce bursts into one refresh event.
pub fn watch_roko_dir(workdir: &Path) -> Result<FsWatchHandle> {
    let roko_dir = prepare_roko_dir(workdir)?;
    let (tx, rx) = mpsc::sync_channel(CHANNEL_BOUND);
    let handler = RefreshHandler { tx };

    let mut debouncer: NotifyDebouncer = new_debouncer_opt(
        DEBOUNCE_WINDOW,
        None,
        handler,
        FileIdMap::new(),
        notify::Config::default(),
    )
    .context("failed to create debounced filesystem watcher")?;
    debouncer
        .watcher()
        .watch(&roko_dir, RecursiveMode::Recursive)
        .with_context(|| format!("failed to watch {}", roko_dir.display()))?;
    debouncer
        .cache()
        .add_root(&roko_dir, RecursiveMode::Recursive);

    Ok(FsWatchHandle {
        rx,
        _backend: FsWatchBackend::Notify(debouncer),
    })
}

/// Watch `.roko/` and fall back to a 1 s poll if notify cannot initialize.
pub fn watch_roko_dir_with_fallback(workdir: &Path) -> FsWatchHandle {
    match watch_roko_dir(workdir) {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(
                error = %error,
                cadence = ?FALLBACK_POLL_INTERVAL,
                "notify unavailable; falling back to poll watcher"
            );
            spawn_poll_fallback_with_interval(workdir.to_path_buf(), FALLBACK_POLL_INTERVAL)
        }
    }
}

fn spawn_poll_fallback_with_interval(workdir: PathBuf, interval: Duration) -> FsWatchHandle {
    let roko_dir = workdir.join(".roko");
    if let Err(error) = fs::create_dir_all(&roko_dir) {
        tracing::warn!(
            error = %error,
            path = %roko_dir.display(),
            "could not create .roko before starting poll fallback"
        );
    }

    let (tx, rx) = mpsc::sync_channel(CHANNEL_BOUND);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let poll_root = roko_dir.clone();
    let mut last_fingerprint = roko_dir_fingerprint(&poll_root);

    let join = thread::Builder::new()
        .name("tui-fs-poll-fallback".into())
        .spawn(move || {
            loop {
                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                thread::sleep(interval);

                if stop_thread.load(Ordering::Relaxed) {
                    break;
                }

                let fingerprint = roko_dir_fingerprint(&poll_root);
                if fingerprint != last_fingerprint {
                    last_fingerprint = fingerprint;
                    match tx.try_send(FsRefresh::Coalesced) {
                        Ok(()) | Err(mpsc::TrySendError::Full(_)) => {}
                        Err(mpsc::TrySendError::Disconnected(_)) => break,
                    }
                }
            }
        })
        .unwrap_or_else(|error| {
            tracing::warn!(
                error = %error,
                thread = "tui-fs-poll-fallback",
                "failed to spawn fallback filesystem poller"
            );
            thread::spawn(|| {})
        });

    FsWatchHandle {
        rx,
        _backend: FsWatchBackend::Poll(PollerHandle {
            stop,
            join: Some(join),
        }),
    }
}

fn prepare_roko_dir(workdir: &Path) -> Result<PathBuf> {
    let roko_dir = workdir.join(".roko");
    fs::create_dir_all(&roko_dir)
        .with_context(|| format!("failed to create {}", roko_dir.display()))?;
    Ok(roko_dir)
}

fn roko_dir_fingerprint(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_path(path, &mut hasher);
    hasher.finish()
}

fn hash_path(path: &Path, hasher: &mut DefaultHasher) {
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
        hash_path(&child, hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_roko_dir_emits_refresh_within_500ms() {
        let tempdir = tempfile::tempdir().unwrap();
        let handle = spawn_poll_fallback_with_interval(
            tempdir.path().to_path_buf(),
            Duration::from_millis(50),
        );
        let target = tempdir.path().join(".roko").join("watch-trigger.json");
        std::fs::write(&target, br#"{"ok":true}"#).unwrap();

        let refresh = handle.rx.recv_timeout(Duration::from_millis(500));
        assert_eq!(refresh.unwrap(), FsRefresh::Coalesced);
    }
}
