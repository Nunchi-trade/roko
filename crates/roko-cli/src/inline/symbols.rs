//! Clack-style symbols and typography for inline CLI output.
//!
//! These symbols form a visual language shared across every `roko` command.
//! The set is deliberately small and consistent — no emoji, no ambiguous
//! glyphs, no platform-dependent rendering.

/// Section start (filled diamond).
pub const START: &str = "◆";
/// Section start for pending/inactive items (empty diamond).
pub const START_EMPTY: &str = "◇";
/// Vertical continuation line.
pub const BAR: &str = "│";
/// Branch connector (more items follow).
pub const BRANCH: &str = "├";
/// Last item connector.
pub const END: &str = "└";
/// Nested branch under a branch.
pub const BRANCH_NESTED: &str = "├──";
/// Nested last item.
pub const END_NESTED: &str = "└──";

/// Pass / success indicator.
pub const PASS: &str = "✔";
/// Fail / error indicator.
pub const FAIL: &str = "✖";
/// Warning indicator.
pub const WARN: &str = "⚠";
/// Info indicator.
pub const INFO: &str = "ℹ";

/// Prompt arrow.
pub const PROMPT: &str = "❯";
/// Flow / routing arrow.
pub const ARROW: &str = "→";
/// Interpunct separator.
pub const SEP: &str = "·";

/// Progress bar filled segment.
pub const PROGRESS_FILL: &str = "━";
/// Progress bar empty segment.
pub const PROGRESS_EMPTY: &str = "░";

/// Pending / waiting indicator.
pub const PENDING: &str = "⏳";

/// Collapsed disclosure triangle (right-pointing).
pub const COLLAPSED: &str = "▸";
/// Expanded disclosure triangle (down-pointing).
pub const EXPANDED: &str = "▾";

/// Tool execution indicator.
pub const TOOL: &str = "⚙";
/// Text cursor.
pub const CURSOR: &str = "█";

/// Braille spinner frames (8-frame cycle).
pub const SPINNER_FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

/// Get the spinner frame for a given tick count.
#[must_use]
pub fn spinner_frame(tick: u64) -> &'static str {
    SPINNER_FRAMES[(tick as usize) % SPINNER_FRAMES.len()]
}

/// Render a progress bar of `width` characters at `progress` (0.0..=1.0).
#[must_use]
pub fn progress_bar(progress: f64, width: usize) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let filled = (progress * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        "{}{}",
        PROGRESS_FILL.repeat(filled),
        PROGRESS_EMPTY.repeat(empty),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_bar_empty() {
        let bar = progress_bar(0.0, 10);
        assert_eq!(bar.chars().filter(|&c| c == '░').count(), 10);
    }

    #[test]
    fn progress_bar_full() {
        let bar = progress_bar(1.0, 10);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 10);
    }

    #[test]
    fn progress_bar_half() {
        let bar = progress_bar(0.5, 10);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 5);
        assert_eq!(bar.chars().filter(|&c| c == '░').count(), 5);
    }

    #[test]
    fn progress_bar_clamps() {
        let bar = progress_bar(2.0, 10);
        assert_eq!(bar.chars().filter(|&c| c == '━').count(), 10);
    }

    #[test]
    fn spinner_cycles() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(8), "⠋"); // wraps
    }
}
