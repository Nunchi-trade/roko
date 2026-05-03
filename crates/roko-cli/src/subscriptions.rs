//! `roko subscription` subcommands.
//!
//! These commands manage the on-disk subscription files used by the server
//! and by the combined registry loader.

use anyhow::{Context as _, Result, anyhow};
use roko_core::config::schema::{RokoConfig, SubscriptionConfig};
use roko_serve::dispatch::SubscriptionRegistry;
use serde::Serialize;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
struct SubscriptionRow {
    id: String,
    template: String,
    trigger: String,
    enabled: bool,
}

/// List all subscriptions in the workspace.
pub fn cmd_list(workdir: &Path, json: bool) -> Result<()> {
    let registry = load_registry(workdir)?;
    let mut rows: Vec<SubscriptionRow> = registry
        .all()
        .into_iter()
        .map(|subscription| SubscriptionRow {
            id: subscription.id,
            template: subscription.template,
            trigger: subscription.trigger,
            enabled: subscription.enabled,
        })
        .collect();
    rows.sort_unstable_by(|a, b| a.id.cmp(&b.id));

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "subscriptions": rows }))?
        );
    } else {
        print!("{}", format_subscription_list(&rows));
    }

    Ok(())
}

/// Create a new subscription file under `.roko/subscriptions/`.
pub fn cmd_add(workdir: &Path, template: &str, trigger: &str) -> Result<()> {
    let template = template.trim();
    let trigger = trigger.trim();
    if template.is_empty() {
        return Err(anyhow!("subscription template must not be empty"));
    }
    if trigger.is_empty() {
        return Err(anyhow!("subscription trigger must not be empty"));
    }

    let registry = load_registry(workdir)?;
    let config = SubscriptionConfig {
        template: template.to_string(),
        trigger: trigger.to_string(),
        ..SubscriptionConfig::default()
    };
    let id = next_subscription_id(workdir, &registry, &config);
    let path = subscription_path(workdir, &id);
    write_subscription_file(&path, &config)?;

    println!(
        "created subscription {} at {}",
        id,
        path.strip_prefix(workdir)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    );
    Ok(())
}

/// Remove a subscription file.
pub fn cmd_remove(workdir: &Path, id: &str) -> Result<()> {
    let current = load_subscription_by_id(workdir, id)?;
    let path = subscription_path(workdir, id);
    if !path.exists() {
        return Err(anyhow!(
            "subscription '{id}' is stored in roko.toml and cannot be removed with this command"
        ));
    }

    fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    println!(
        "removed subscription {} ({}, {})",
        current.id, current.template, current.trigger
    );
    Ok(())
}

/// Enable or disable a subscription file.
pub fn cmd_set_enabled(workdir: &Path, id: &str, enabled: bool) -> Result<()> {
    let current = load_subscription_by_id(workdir, id)?;
    let path = subscription_path(workdir, id);
    if !path.exists() {
        return Err(anyhow!(
            "subscription '{id}' is stored in roko.toml and cannot be modified with this command"
        ));
    }

    let mut config = current.to_config();
    config.enabled = enabled;
    write_subscription_file(&path, &config)?;

    let state = if enabled { "enabled" } else { "disabled" };
    println!(
        "{} subscription {} ({}, {})",
        state, current.id, current.template, current.trigger
    );
    Ok(())
}

fn load_registry(workdir: &Path) -> Result<SubscriptionRegistry> {
    let config = load_roko_config(workdir)?;
    Ok(SubscriptionRegistry::load_from_project(workdir, &config))
}

fn load_subscription_by_id(workdir: &Path, id: &str) -> Result<roko_serve::dispatch::Subscription> {
    let registry = load_registry(workdir)?;
    registry
        .get_by_id(id)
        .ok_or_else(|| anyhow!("subscription '{id}' not found"))
}

fn subscription_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("subscriptions")
}

fn subscription_path(workdir: &Path, id: &str) -> PathBuf {
    subscription_dir(workdir).join(format!("{id}.toml"))
}

fn next_subscription_id(
    workdir: &Path,
    registry: &SubscriptionRegistry,
    config: &SubscriptionConfig,
) -> String {
    let base = slugify_subscription_id(&config.template, &config.trigger);
    let mut candidate = base.clone();
    let mut suffix = 2usize;

    while registry.get_by_id(&candidate).is_some()
        || subscription_path(workdir, &candidate).exists()
    {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }

    candidate
}

fn slugify_subscription_id(template: &str, trigger: &str) -> String {
    let mut slug = format!("{template}-{trigger}");
    slug = slug
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "subscription".to_string()
    } else {
        slug
    }
}

fn format_subscription_list(rows: &[SubscriptionRow]) -> String {
    if rows.is_empty() {
        return "no subscriptions found\n".to_string();
    }

    let mut widths = [2usize, 8, 7, 7];
    for row in rows {
        widths[0] = widths[0].max(row.id.len());
        widths[1] = widths[1].max(row.template.len());
        widths[2] = widths[2].max(row.trigger.len());
        widths[3] = widths[3].max(subscription_enabled_label(row.enabled).len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<id_w$}  {:<template_w$}  {:<trigger_w$}  {:<enabled_w$}",
        "ID",
        "TEMPLATE",
        "TRIGGER",
        "ENABLED",
        id_w = widths[0],
        template_w = widths[1],
        trigger_w = widths[2],
        enabled_w = widths[3],
    );
    let _ = writeln!(
        out,
        "{:-<id_w$}  {:-<template_w$}  {:-<trigger_w$}  {:-<enabled_w$}",
        "",
        "",
        "",
        "",
        id_w = widths[0],
        template_w = widths[1],
        trigger_w = widths[2],
        enabled_w = widths[3],
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<id_w$}  {:<template_w$}  {:<trigger_w$}  {:<enabled_w$}",
            row.id,
            row.template,
            row.trigger,
            subscription_enabled_label(row.enabled),
            id_w = widths[0],
            template_w = widths[1],
            trigger_w = widths[2],
            enabled_w = widths[3],
        );
    }
    out
}

fn subscription_enabled_label(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

fn write_subscription_file(path: &Path, config: &SubscriptionConfig) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("invalid subscription path"))?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let rendered = toml::to_string_pretty(config).context("serialize subscription")?;
    fs::write(path, rendered).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    roko_core::config::loader::load_config_unified(workdir).map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_subscription_id_replaces_punctuation() {
        assert_eq!(
            slugify_subscription_id("pr-reviewer", "github:pull_request:*"),
            "pr-reviewer-github-pull-request"
        );
    }

    #[test]
    fn format_subscription_list_renders_table() {
        let text = format_subscription_list(&[
            SubscriptionRow {
                id: "sub-1".into(),
                template: "pr-reviewer".into(),
                trigger: "github:pull_request:*".into(),
                enabled: true,
            },
            SubscriptionRow {
                id: "sub-2".into(),
                template: "release-watcher".into(),
                trigger: "github:release:*".into(),
                enabled: false,
            },
        ]);

        assert!(text.contains("ID"));
        assert!(text.contains("sub-1"));
        assert!(text.contains("disabled"));
    }
}
