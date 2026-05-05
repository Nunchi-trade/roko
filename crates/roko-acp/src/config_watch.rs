//! Best-effort live config change detection for ACP.

use std::collections::BTreeSet;
use std::path::PathBuf;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::config::AcpConfig;

/// Watches ACP config files and reports when they should be reloaded.
pub struct ConfigWatcher {
    _watcher: Option<RecommendedWatcher>,
    rx: mpsc::Receiver<()>,
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
                return Self { _watcher: None, rx };
            }
        };

        let mut watched_targets = BTreeSet::new();
        for path in watched_paths(config) {
            watch_config_path(&mut watcher, path, &mut watched_targets);
        }

        Self {
            _watcher: Some(watcher),
            rx,
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
    match config.config_path.as_ref() {
        Some(path) => paths.push(path.clone()),
        None => paths.push(config.workdir.join("roko.toml")),
    }
    if let Some(path) = config.global_config_path.as_ref() {
        paths.push(path.clone());
    }
    if let Ok(path) = std::env::var("ROKO_CONFIG")
        && !path.trim().is_empty()
    {
        paths.push(PathBuf::from(path));
    }
    paths.sort();
    paths.dedup();
    paths
}
