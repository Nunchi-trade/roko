//! Clack-style output primitives for CLI progress rendering.

use crate::inline::symbols;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

const FG_GREEN: &str = "\x1b[32m";
const FG_RED: &str = "\x1b[31m";
const FG_YELLOW: &str = "\x1b[33m";
const FG_CYAN: &str = "\x1b[36m";
const FG_MAGENTA: &str = "\x1b[35m";
const FG_WHITE: &str = "\x1b[97m";
const FG_GRAY: &str = "\x1b[90m";

pub fn bold(s: &str) -> String {
    format!("{BOLD}{s}{RESET}")
}

pub fn dim(s: &str) -> String {
    format!("{DIM}{FG_GRAY}{s}{RESET}")
}

pub fn green(s: &str) -> String {
    format!("{FG_GREEN}{s}{RESET}")
}

pub fn red(s: &str) -> String {
    format!("{FG_RED}{s}{RESET}")
}

pub fn yellow(s: &str) -> String {
    format!("{FG_YELLOW}{s}{RESET}")
}

pub fn cyan(s: &str) -> String {
    format!("{FG_CYAN}{s}{RESET}")
}

pub fn magenta(s: &str) -> String {
    format!("{FG_MAGENTA}{s}{RESET}")
}

/// Returns `true` if stdout is connected to a terminal.
///
/// When false, color helpers should be skipped. However, the current
/// implementation always emits ANSI codes — the caller can check `is_tty()`
/// and choose a plain-text path if needed.
pub fn is_tty() -> bool {
    use std::io::IsTerminal;

    std::io::stdout().is_terminal()
}

/// Print the intro line: `◆  <title>` in bold.
pub fn intro(title: &str) {
    println!("{} {}", symbols::START, bold(title));
}

/// Print a step line: `◇  <label>` with an optional value.
pub fn step(label: &str, value: &str) {
    if value.is_empty() {
        println!("{} {}", symbols::START_EMPTY, bold(label));
    } else {
        println!("{} {}  {}", symbols::START_EMPTY, bold(label), dim(value));
    }
}

/// Print a continuation line: `│  <text>`.
pub fn bar(text: &str) {
    println!("{}  {}", symbols::BAR, text);
}

/// Print a note line: `│  <text>` in dim style.
pub fn note(text: &str) {
    println!("{}  {}", symbols::BAR, dim(text));
}

/// Print a success line: `✔  <msg>` in green.
pub fn success(msg: &str) {
    println!("{}  {}", green(symbols::PASS), green(msg));
}

/// Print an error line: `✖  <msg>` in red.
pub fn error(msg: &str) {
    println!("{}  {}", red(symbols::FAIL), red(msg));
}

/// Print a warning line: `⚠  <msg>` in yellow.
pub fn warning(msg: &str) {
    println!("{}  {}", yellow(symbols::WARN), yellow(msg));
}

/// Print an empty `│` line (visual spacer between sections).
pub fn divider() {
    println!("{}", symbols::BAR);
}

/// Print a branch line: `├  <text>`.
pub fn branch(text: &str) {
    println!("{}  {}", symbols::BRANCH, text);
}

/// Print an end line: `└  <text>`.
pub fn end(text: &str) {
    println!("{}  {}", symbols::END, text);
}
