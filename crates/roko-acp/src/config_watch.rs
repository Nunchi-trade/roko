//! Best-effort live config change detection for ACP.
//!
//! `ConfigWatcher` serves two roles:
//! 1. **Change detection** -- the `changed()` poll used by the ACP handler loop
//!    to know when to reload config via `AcpConfig::load_roko_config()`.
//! 2. **Cached current config** -- the `current()` accessor backed by
//!    [`roko_core::config::ConfigCache`] for zero-copy reads. When the
//!    underlying file changes, the cache is atomically swapped by the
//!    `ConfigCache` watcher thread.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::config::AcpConfig;

/// Watches ACP config files and reports when they should be reloaded.
///
/// Optionally holds a [`roko_core::config::ConfigCache`] that auto-reloads
/// on file change, providing a zero-copy `current()` accessor.
pub struct ConfigWatcher {
    _watcher: Option<RecommendedWatcher>,
    rx: mpsc::Receiver<()>,
    /// Optional config cache for zero-copy reads. When present, `current()`
    /// returns the latest config without going through `AcpConfig::load_roko_config()`.
    cache: Option<Arc<roko_core::config::ConfigCache>>,
}

impl ConfigWatcher {
    /// Start watching existing config files. Missing files are reported by the
    /// ACP startup diagnostics and simply skipped here.
    #[must_use]
    pub fn start(config: &AcpConfig) -> Self {
        let (tx, rx) = mpsc::channel(8);
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if result.is_ok() {
                let _ = tx.try_send(());
            }
        });

        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::warn!(error = %error, "ACP config watch unavailable");
                return Self {
                    _watcher: None,
                    rx,
                    cache: None,
                };
            }
        };

        let mut watched_targets = BTreeSet::new();
        for path in watched_paths(config) {
            watch_config_path(&mut watcher, path, &mut watched_targets);
        }

        // Best-effort: create a ConfigCache for zero-copy reads.
        let cache = match roko_core::config::ConfigCache::new(&config.workdir) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "ACP ConfigCache creation failed; falling back to direct loads"
                );
                None
            }
        };

        Self {
            _watcher: Some(watcher),
            rx,
            cache,
        }
    }

    /// Drain all pending file-change notifications.
    pub fn changed(&mut self) -> bool {
        let mut changed = false;
        while self.rx.try_recv().is_ok() {
            changed = true;
        }
        changed
    }

    /// Return the current config from the cache (zero-copy via `ArcSwap`).
    ///
    /// Falls back to `None` if the cache was not created (e.g. initial load
    /// failed or the watcher could not be started).
    pub fn current(&self) -> Option<Arc<roko_core::config::schema::RokoConfig>> {
        self.cache.as_ref().map(|c| c.get())
    }
}

fn watch_config_path(
    watcher: &mut RecommendedWatcher,
    path: PathBuf,
    watched_targets: &mut BTreeSet<PathBuf>,
) {
    let target = if path.is_file() {
        path.clone()
    } else {
        match path
            .parent()
            .map(PathBuf::from)
            .filter(|parent| parent.is_dir())
        {
            Some(parent) => parent,
            None => {
                tracing::warn!(path = %path.display(), "ACP config watch target does not exist");
                return;
            }
        }
    };

    if !watched_targets.insert(target.clone()) {
        tracing::debug!(path = %target.display(), "ACP config watch target already registered");
        return;
    }

    if let Err(error) = watcher.watch(&target, RecursiveMode::NonRecursive) {
        tracing::warn!(path = %target.display(), error = %error, "failed to watch ACP config path");
    } else if target == path {
        tracing::info!(path = %target.display(), "watching ACP config file");
    } else {
        tracing::info!(
            path = %path.display(),
            parent = %target.display(),
            "watching ACP config parent for file creation"
        );
    }
}

fn watched_paths(config: &AcpConfig) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 1. Explicit --global-config, or implicit ~/.roko/config.toml when absent.
    //    The core loader always merges the implicit global config, so we must
    //    watch it to detect changes even when --global-config is not set.
    match config.global_config_path.as_ref() {
        Some(path) => paths.push(path.clone()),
        None => {
            let implicit = roko_core::config::loader::global_config_path();
            paths.push(implicit);
        }
    }

    // 2. Project config: explicit --config or workspace roko.toml.
    match config.config_path.as_ref() {
        Some(path) => paths.push(path.clone()),
        None => paths.push(config.workdir.join("roko.toml")),
    }

    // 3. ROKO_CONFIG env var.
    if let Ok(path) = std::env::var("ROKO_CONFIG")
        && !path.trim().is_empty()
    {
        paths.push(PathBuf::from(path));
    }

    paths.sort();
    paths.dedup();
    paths
}
