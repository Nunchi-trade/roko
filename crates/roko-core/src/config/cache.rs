//! Config cache with file-watch invalidation (redesign Phase 4.2 / Phase 10.3).
//!
//! [`ConfigCache`] loads once at construction, then atomically swaps when the
//! underlying config file changes. Zero-copy reads via [`arc_swap::ArcSwap`].

use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use notify::{Event, EventKind, RecommendedWatcher, Watcher};

use super::LoadConfigError;
use super::loader::{
    LoadOptions, discover_project_config, global_config_path,
    load_config_with_options,
};
use super::schema::RokoConfig;

/// A config loaded once and atomically refreshed when the underlying
/// file changes on disk.
///
/// `ConfigCache` is `Send + Sync` and cheap to share via `Arc`.
pub struct ConfigCache {
    /// Shared swap slot: both the watcher closure and [`Self::get`] read/write
    /// through the same `ArcSwap` so that file-change reloads are visible to
    /// callers immediately.
    config: Arc<ArcSwap<RokoConfig>>,
    /// Keeps the watcher alive for the lifetime of the cache.
    _watcher: Option<RecommendedWatcher>,
}

impl ConfigCache {
    /// Load config from `workdir` and start watching for changes.
    ///
    /// Both the project `roko.toml` and the global `~/.roko/config.toml` are
    /// watched. When either changes, the config is reloaded and atomically
    /// swapped.
    ///
    /// If the file watcher fails to start, the cache still works but won't
    /// pick up live changes (a warning is logged).
    pub fn new(workdir: &Path) -> Result<Arc<Self>, LoadConfigError> {
        let opts = LoadOptions::default();
        let config = load_config_with_options(workdir, &opts)?;
        let swap = Arc::new(ArcSwap::from_pointee(config));

        // Identify which files to watch.
        let project_path = discover_project_config(workdir);
        let global_path = {
            let p = global_config_path();
            if p.exists() { Some(p) } else { None }
        };

        let swap_clone = Arc::clone(&swap);
        let workdir_owned = workdir.to_path_buf();
        let opts_clone = opts.clone();

        let watcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| {
                let Ok(event) = res else { return };
                if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    match load_config_with_options(&workdir_owned, &opts_clone) {
                        Ok(new_config) => {
                            tracing::info!(
                                workdir = %workdir_owned.display(),
                                "roko.toml changed \u{2014} config reloaded"
                            );
                            swap_clone.store(Arc::new(new_config));
                        }
                        Err(e) => {
                            tracing::warn!(
                                workdir = %workdir_owned.display(),
                                error = %e,
                                "roko.toml changed but reload failed \u{2014} keeping previous config"
                            );
                        }
                    }
                }
            });

        let watcher = match watcher {
            Ok(mut w) => {
                if let Some(ref p) = project_path {
                    if let Err(e) = w.watch(p, notify::RecursiveMode::NonRecursive) {
                        tracing::warn!(
                            path = %p.display(),
                            error = %e,
                            "failed to watch project config"
                        );
                    }
                }
                if let Some(ref p) = global_path {
                    if let Err(e) = w.watch(p, notify::RecursiveMode::NonRecursive) {
                        tracing::warn!(
                            path = %p.display(),
                            error = %e,
                            "failed to watch global config"
                        );
                    }
                }
                Some(w)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config file watcher unavailable; live reload disabled"
                );
                None
            }
        };

        Ok(Arc::new(Self {
            config: swap,
            _watcher: watcher,
        }))
    }

    /// Load config without file watching (useful for tests and short-lived CLIs).
    pub fn new_static(workdir: &Path) -> Result<Arc<Self>, LoadConfigError> {
        let opts = LoadOptions::default();
        let config = load_config_with_options(workdir, &opts)?;
        Ok(Arc::new(Self {
            config: Arc::new(ArcSwap::from_pointee(config)),
            _watcher: None,
        }))
    }

    /// Read the current config. Zero-copy -- returns an `Arc` into the swap slot.
    pub fn get(&self) -> Arc<RokoConfig> {
        self.config.load_full()
    }

    /// Force a reload from disk right now, ignoring file-watcher events.
    pub fn force_reload(&self, workdir: &Path) {
        let opts = LoadOptions::default();
        match load_config_with_options(workdir, &opts) {
            Ok(new_config) => {
                tracing::info!(
                    workdir = %workdir.display(),
                    "config force-reloaded"
                );
                self.config.store(Arc::new(new_config));
            }
            Err(e) => {
                tracing::warn!(
                    workdir = %workdir.display(),
                    error = %e,
                    "config force-reload failed; keeping previous config"
                );
            }
        }
    }
}

impl std::fmt::Debug for ConfigCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigCache").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_loads_default_when_no_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ConfigCache::new_static(dir.path()).unwrap();
        let config = cache.get();
        assert_eq!(*config, RokoConfig::default());
    }

    #[test]
    fn cache_loads_from_roko_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.cache-test]
kind = "openai_compat"
base_url = "https://cache-test.example/v1"
"#,
        )
        .unwrap();

        let cache = ConfigCache::new_static(dir.path()).unwrap();
        let config = cache.get();
        assert!(config.providers.contains_key("cache-test"));
    }

    #[test]
    fn watched_cache_sees_file_change() {
        // Regression: ConfigCache::new() previously created two independent
        // ArcSwap instances -- the watcher wrote to one while get() read the
        // other. After the fix, both share the same Arc<ArcSwap>.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.initial-provider]
kind = "openai_compat"
base_url = "https://initial.example/v1"
"#,
        )
        .unwrap();

        let cache = ConfigCache::new(dir.path()).unwrap();
        assert!(cache.get().providers.contains_key("initial-provider"));
        assert!(!cache.get().providers.contains_key("added-provider"));

        // Mutate the file; the watcher should pick up the change.
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.initial-provider]
kind = "openai_compat"
base_url = "https://initial.example/v1"

[providers.added-provider]
kind = "openai_compat"
base_url = "https://added.example/v1"
"#,
        )
        .unwrap();

        // Poll for up to 5 seconds for the watcher to fire.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut saw_change = false;
        while std::time::Instant::now() < deadline {
            if cache.get().providers.contains_key("added-provider") {
                saw_change = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            saw_change,
            "ConfigCache::get() must reflect file changes via the watcher; \
             this likely means the watcher and get() use separate ArcSwap instances"
        );
    }

    #[test]
    fn force_reload_updates_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("roko.toml"), "config_version = 2\n").unwrap();

        let cache = ConfigCache::new_static(dir.path()).unwrap();
        assert!(cache.get().providers.is_empty());

        // Write a new config with a provider.
        std::fs::write(
            dir.path().join("roko.toml"),
            r#"
config_version = 2
schema_version = 2

[providers.reload-test]
kind = "openai_compat"
base_url = "https://reload-test.example/v1"
"#,
        )
        .unwrap();

        cache.force_reload(dir.path());
        assert!(cache.get().providers.contains_key("reload-test"));
    }
}
