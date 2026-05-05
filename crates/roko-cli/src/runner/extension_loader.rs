//! Extension chain loader — scans `.roko/extensions/` and `plugins/` for
//! extension manifests and creates [`Extension`] implementations from them.
//!
//! Each discovered plugin manifest (`plugin.toml`) becomes a
//! [`PluginExtension`] that logs lifecycle events and can be extended to
//! execute declarative hooks (tool profiles, triggers, etc.) in the future.

use std::path::Path;

use roko_core::extension::{
    Extension, ExtensionChain, ExtensionLayer, ExtensionMeta, GateEvent, InferenceRequest,
    InferenceResponse,
};
use roko_fs::RokoLayout;
use tracing::{debug, info, warn};

// ─── PluginExtension ────────────��───────────────────────────────────────

/// An [`Extension`] backed by a discovered plugin manifest.
///
/// Currently provides logging at each hook point. When a plugin declares
/// tool profiles or triggers, this wrapper is the right place to enforce
/// them.
struct PluginExtension {
    meta: ExtensionMeta,
    /// Number of prompt templates the plugin provides.
    prompt_count: usize,
    /// Number of declarative tools the plugin provides.
    tool_count: usize,
}

#[async_trait::async_trait]
impl Extension for PluginExtension {
    fn name(&self) -> &str {
        &self.meta.name
    }

    fn layer(&self) -> ExtensionLayer {
        self.meta.layer
    }

    fn meta(&self) -> ExtensionMeta {
        self.meta.clone()
    }

    async fn on_init(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            extension = %self.meta.name,
            prompts = self.prompt_count,
            tools = self.tool_count,
            "plugin extension initialized"
        );
        Ok(())
    }

    async fn on_shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!(extension = %self.meta.name, "plugin extension shutting down");
        Ok(())
    }

    async fn pre_inference(
        &self,
        request: &mut InferenceRequest,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            extension = %self.meta.name,
            plan_id = %request.plan_id,
            task = %request.task,
            "plugin pre_inference hook"
        );
        Ok(())
    }

    async fn post_inference(
        &self,
        response: &mut InferenceResponse,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            extension = %self.meta.name,
            plan_id = %response.plan_id,
            task = %response.task,
            success = response.success,
            "plugin post_inference hook"
        );
        Ok(())
    }

    async fn on_gate(
        &self,
        event: &mut GateEvent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        debug!(
            extension = %self.meta.name,
            gate = %event.gate_name,
            passed = event.passed,
            "plugin on_gate hook"
        );
        Ok(())
    }
}

// ─── Loader ───────────────────────────���─────────────────────────────────

/// Scan extension directories and populate the given [`ExtensionChain`].
///
/// Scans (in order):
/// 1. `<workdir>/.roko/extensions/`
/// 2. `<workdir>/plugins/`
///
/// Each directory is probed via [`roko_plugin::manifest::discover_plugins`].
/// Discovered plugins are wrapped as [`PluginExtension`] and added to the
/// chain sorted by layer.
///
/// Additionally, if `extension_names` is non-empty (from `roko.toml`
/// `[agent].extensions`), only plugins whose name appears in that list are
/// loaded. An empty list means "load all discovered plugins".
pub fn load_extensions(
    workdir: &Path,
    extension_names: &[String],
    chain: &mut ExtensionChain,
) -> usize {
    let layout = RokoLayout::for_project(workdir);
    let scan_dirs = [layout.extensions_dir(), workdir.join("plugins")];

    let mut loaded = 0usize;

    for dir in &scan_dirs {
        if !dir.exists() {
            debug!(dir = %dir.display(), "extension directory does not exist, skipping");
            continue;
        }

        let plugins = match roko_plugin::manifest::discover_plugins(dir) {
            Ok(p) => p,
            Err(err) => {
                warn!(
                    dir = %dir.display(),
                    error = %err,
                    "failed to scan extension directory"
                );
                continue;
            }
        };

        for plugin in plugins {
            let name = &plugin.manifest.plugin.name;

            // If an allow-list is configured, skip plugins not in it.
            if !extension_names.is_empty() && !extension_names.iter().any(|n| n == name) {
                debug!(
                    plugin = %name,
                    "plugin not in configured extensions list, skipping"
                );
                continue;
            }

            let ext = PluginExtension {
                meta: ExtensionMeta {
                    name: name.clone(),
                    layer: ExtensionLayer::Cognition, // default layer for plugin extensions
                    optional: true,                   // plugins should never be fatal
                    depends_on: plugin
                        .manifest
                        .dependencies
                        .iter()
                        .map(|d| d.name.clone())
                        .collect(),
                    version: plugin.manifest.plugin.version.clone(),
                },
                prompt_count: plugin.manifest.prompts.len(),
                tool_count: plugin.manifest.tools.len(),
            };

            info!(
                plugin = %name,
                version = %plugin.manifest.plugin.version,
                prompts = ext.prompt_count,
                tools = ext.tool_count,
                dir = %plugin.base_dir.display(),
                "loaded plugin extension"
            );

            chain.add(Box::new(ext));
            loaded += 1;
        }
    }

    if loaded > 0 {
        chain.sort_by_layer();
        info!(
            count = loaded,
            "extension chain populated from discovered plugins"
        );
    } else {
        debug!("no plugin extensions discovered");
    }

    loaded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_nonexistent_dirs_returns_zero() {
        let mut chain = ExtensionChain::new();
        let count = load_extensions(Path::new("/nonexistent/workspace"), &[], &mut chain);
        assert_eq!(count, 0);
        assert!(chain.is_empty());
    }

    #[test]
    fn load_with_empty_dir_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = RokoLayout::for_project(tmp.path()).extensions_dir();
        std::fs::create_dir_all(&ext_dir).unwrap();

        let mut chain = ExtensionChain::new();
        let count = load_extensions(tmp.path(), &[], &mut chain);
        assert_eq!(count, 0);
    }

    #[test]
    fn load_discovers_plugin_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = RokoLayout::for_project(tmp.path())
            .extensions_dir()
            .join("test-ext");
        std::fs::create_dir_all(&ext_dir).unwrap();

        std::fs::write(
            ext_dir.join("plugin.toml"),
            r#"
[plugin]
name = "test-ext"
version = "0.1.0"
description = "A test extension"

[[prompts]]
name = "test-prompt"
template = "Hello, world!"
"#,
        )
        .unwrap();

        let mut chain = ExtensionChain::new();
        let count = load_extensions(tmp.path(), &[], &mut chain);
        assert_eq!(count, 1);
        assert_eq!(chain.len(), 1);

        let meta = chain.metadata();
        assert_eq!(meta[0].name, "test-ext");
        assert_eq!(meta[0].version, "0.1.0");
        assert!(meta[0].optional);
    }

    #[test]
    fn load_respects_allow_list() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = RokoLayout::for_project(tmp.path());
        let ext_dir = layout.extensions_dir().join("allowed");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("plugin.toml"),
            r#"
[plugin]
name = "allowed-ext"
version = "0.1.0"
"#,
        )
        .unwrap();

        let skip_dir = layout.extensions_dir().join("skipped");
        std::fs::create_dir_all(&skip_dir).unwrap();
        std::fs::write(
            skip_dir.join("plugin.toml"),
            r#"
[plugin]
name = "skipped-ext"
version = "0.1.0"
"#,
        )
        .unwrap();

        let mut chain = ExtensionChain::new();
        let count = load_extensions(tmp.path(), &["allowed-ext".to_string()], &mut chain);
        assert_eq!(count, 1);
        assert_eq!(chain.metadata()[0].name, "allowed-ext");
    }
}
