//! `roko note` command — instant, append-only note capture.

use anyhow::{Result, bail};
use std::fmt::Write as _;
use std::path::Path;

/// Write a timestamped note to `.roko/notes/`.
pub(crate) fn cmd_note(
    workdir: &Path,
    text: Vec<String>,
    tags: Vec<String>,
    json: bool,
) -> Result<i32> {
    let text = text.join(" ");
    let text = text.trim();
    if text.is_empty() {
        bail!("provide note text: roko note \"my thought here\"");
    }

    let now = chrono::Local::now();
    let filename = now.format("%Y-%m-%d-%H%M%S.md").to_string();
    let notes_dir = workdir.join(".roko").join("notes");
    std::fs::create_dir_all(&notes_dir)?;

    let mut body = String::new();
    body.push_str("# Note\n\n");
    let _ = writeln!(body, "**Date:** {}", now.format("%Y-%m-%d %H:%M:%S"));
    if !tags.is_empty() {
        let _ = writeln!(body, "**Tags:** {}", tags.join(", "));
    }
    let _ = write!(body, "\n{text}\n");

    let path = notes_dir.join(&filename);
    std::fs::write(&path, &body)?;

    if json {
        let payload = serde_json::json!({
            "path": path,
            "timestamp": now.to_rfc3339(),
            "tags": tags,
            "text": text,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("Saved {}", path.display());
    }

    Ok(0)
}
