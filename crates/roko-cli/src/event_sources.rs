//! `roko event-sources` subcommands.
//!
//! This command inspects the configured `roko.toml` scheduler and watcher
//! sections and prints the effective cron schedules plus filesystem watch
//! roots.

use anyhow::{Context as _, Result};
use roko_core::config::schema::{RokoConfig, WatcherPathConfig};
use roko_plugin::CronEventSource;
use serde::Serialize;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
struct CronScheduleRow {
    name: String,
    expression: String,
    signal_kind: String,
    next_fire: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FileWatcherRow {
    directory: String,
    include: Vec<String>,
    exclude: Vec<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct EventSourcesSnapshot {
    cron_schedules: Vec<CronScheduleRow>,
    file_watchers: Vec<FileWatcherRow>,
}

/// List configured cron schedules and filesystem watchers.
pub fn cmd_list(workdir: &Path, json: bool) -> Result<()> {
    let config = load_roko_config(workdir)?;
    let cron_source = CronEventSource::from_config(config.scheduler.clone());

    let mut cron_schedules = cron_source
        .schedules()
        .context("build cron schedule list")?
        .into_iter()
        .map(|schedule| CronScheduleRow {
            name: schedule.name,
            expression: schedule.expression,
            signal_kind: schedule.signal_kind,
            next_fire: schedule.next_fire.map(|instant| instant.to_rfc3339()),
        })
        .collect::<Vec<_>>();
    cron_schedules.sort_unstable_by(|a, b| a.name.cmp(&b.name));

    let mut file_watchers = config
        .watcher
        .paths
        .iter()
        .map(|path| file_watcher_row(workdir, path))
        .collect::<Vec<_>>();
    file_watchers.sort_unstable_by(|a, b| a.directory.cmp(&b.directory));

    let snapshot = EventSourcesSnapshot {
        cron_schedules,
        file_watchers,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
    } else {
        print!("{}", format_event_sources(&snapshot));
    }

    Ok(())
}

fn load_roko_config(workdir: &Path) -> Result<RokoConfig> {
    let path = workdir.join("roko.toml");
    if !path.exists() {
        return Ok(RokoConfig::default());
    }

    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn file_watcher_row(workdir: &Path, path: &WatcherPathConfig) -> FileWatcherRow {
    let resolved_directory = resolve_watch_directory(workdir, &path.directory);
    FileWatcherRow {
        directory: path.directory.display().to_string(),
        include: path.include.clone(),
        exclude: path.exclude.clone(),
        status: watch_status(&resolved_directory),
    }
}

fn resolve_watch_directory(workdir: &Path, directory: &Path) -> PathBuf {
    if directory.is_absolute() {
        directory.to_path_buf()
    } else {
        workdir.join(directory)
    }
}

fn watch_status(directory: &Path) -> String {
    match fs::metadata(directory) {
        Ok(metadata) if metadata.is_dir() => "ready".to_string(),
        Ok(_) => "not a directory".to_string(),
        Err(_) => "missing".to_string(),
    }
}

fn format_event_sources(snapshot: &EventSourcesSnapshot) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "Cron schedules");
    let _ = writeln!(out, "{}", format_cron_schedule_list(&snapshot.cron_schedules));
    let _ = writeln!(out);
    let _ = writeln!(out, "File watchers");
    let _ = writeln!(out, "{}", format_file_watcher_list(&snapshot.file_watchers));

    out
}

fn format_cron_schedule_list(rows: &[CronScheduleRow]) -> String {
    if rows.is_empty() {
        return "no cron schedules configured\n".to_string();
    }

    let mut widths = [4usize, 10, 12, 9];
    for row in rows {
        widths[0] = widths[0].max(row.name.len());
        widths[1] = widths[1].max(row.expression.len());
        widths[2] = widths[2].max(row.signal_kind.len());
        widths[3] = widths[3].max(row.next_fire.as_deref().unwrap_or("-").len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<name_w$}  {:<expr_w$}  {:<signal_w$}  {:<next_w$}",
        "NAME",
        "EXPRESSION",
        "SIGNAL",
        "NEXT FIRE",
        name_w = widths[0],
        expr_w = widths[1],
        signal_w = widths[2],
        next_w = widths[3],
    );
    let _ = writeln!(
        out,
        "{:-<name_w$}  {:-<expr_w$}  {:-<signal_w$}  {:-<next_w$}",
        "",
        "",
        "",
        "",
        name_w = widths[0],
        expr_w = widths[1],
        signal_w = widths[2],
        next_w = widths[3],
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<name_w$}  {:<expr_w$}  {:<signal_w$}  {:<next_w$}",
            row.name,
            row.expression,
            row.signal_kind,
            row.next_fire.as_deref().unwrap_or("-"),
            name_w = widths[0],
            expr_w = widths[1],
            signal_w = widths[2],
            next_w = widths[3],
        );
    }
    out
}

fn format_file_watcher_list(rows: &[FileWatcherRow]) -> String {
    if rows.is_empty() {
        return "no file watchers configured\n".to_string();
    }

    let mut widths = [9usize, 6, 6, 6];
    for row in rows {
        widths[0] = widths[0].max(row.directory.len());
        widths[1] = widths[1].max(join_globs(&row.include).len());
        widths[2] = widths[2].max(join_globs(&row.exclude).len());
        widths[3] = widths[3].max(row.status.len());
    }

    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<dir_w$}  {:<include_w$}  {:<exclude_w$}  {:<status_w$}",
        "DIRECTORY",
        "INCLUDE",
        "EXCLUDE",
        "STATUS",
        dir_w = widths[0],
        include_w = widths[1],
        exclude_w = widths[2],
        status_w = widths[3],
    );
    let _ = writeln!(
        out,
        "{:-<dir_w$}  {:-<include_w$}  {:-<exclude_w$}  {:-<status_w$}",
        "",
        "",
        "",
        "",
        dir_w = widths[0],
        include_w = widths[1],
        exclude_w = widths[2],
        status_w = widths[3],
    );
    for row in rows {
        let _ = writeln!(
            out,
            "{:<dir_w$}  {:<include_w$}  {:<exclude_w$}  {:<status_w$}",
            row.directory,
            join_globs(&row.include),
            join_globs(&row.exclude),
            row.status,
            dir_w = widths[0],
            include_w = widths[1],
            exclude_w = widths[2],
            status_w = widths[3],
        );
    }
    out
}

fn join_globs(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn format_event_sources_handles_empty_config() {
        let text = format_event_sources(&EventSourcesSnapshot {
            cron_schedules: vec![],
            file_watchers: vec![],
        });

        assert!(text.contains("no cron schedules configured"));
        assert!(text.contains("no file watchers configured"));
    }

    #[test]
    fn watch_status_reports_missing_directory() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("roko-event-source-{unique}"));

        assert_eq!(watch_status(&path), "missing");
    }
}
