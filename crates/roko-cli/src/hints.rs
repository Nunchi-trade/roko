use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Show a deprecation hint at most once per week.
///
/// Prints a hint like `"hint: roko run is now roko do. The old command still works."`
/// if the hint hasn't been shown in the last 7 days. State is persisted to `hints_path`.
///
/// Silently does nothing if stdout is not a TTY.
pub fn show_deprecation_hint(old: &str, new: &str, hints_path: &Path) {
    if !std::io::stdout().is_terminal() {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let seven_days: u64 = 7 * 24 * 60 * 60;

    let mut hints: HashMap<String, u64> = hints_path
        .exists()
        .then(|| std::fs::read_to_string(hints_path).ok())
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let key = format!("deprecation:{old}");
    if let Some(&last_shown) = hints.get(&key) {
        if now.saturating_sub(last_shown) < seven_days {
            return;
        }
    }

    eprintln!("hint: {old} is now {new}. The old command still works.");

    hints.insert(key, now);

    if let Some(parent) = hints_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        hints_path,
        serde_json::to_string_pretty(&hints).unwrap_or_default(),
    );
}
