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

/// Format a token count with thin-space thousands grouping.
/// 1234567 -> "1 234 567"
fn fmt_tokens(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
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

/// Print the agent identity block at the start of a run.
///
/// Prints:
/// ```text
/// ◆  <agent>
/// │  model    <model>
/// │  routing  <routing>
/// ```
///
/// `agent` is the agent name (e.g., `"researcher"`).
/// `model` is the resolved model string (e.g., `"claude-sonnet-4-6-20251001"`).
/// `routing` is a human-readable routing decision string such as
/// `"cascade-router → claude-sonnet-4-6-20251001"` or `"direct"`.
pub fn print_identity(agent: &str, model: &str, routing: &str) {
    intro(agent);
    bar(&format!(
        "{}  {}",
        dim(&format!("{:<8}", "model")),
        cyan(model)
    ));

    let routing_display = if let Some((before, after)) = routing.split_once(" → ") {
        format!("{} {} {}", magenta(before), symbols::ARROW, cyan(after))
    } else {
        routing.to_string()
    };
    bar(&format!(
        "{}  {}",
        dim(&format!("{:<8}", "routing")),
        routing_display
    ));
}

/// Print a cost prediction block before execution starts.
///
/// Prints:
/// ```text
/// ◇  Cost estimate
/// │  ~<tokens> tokens
/// │  ~$<cost> USD
/// ```
///
/// `estimated_tokens` is the predicted token count (input + output combined).
/// `estimated_cost_usd` is the predicted cost in US dollars.
pub fn print_cost_prediction(estimated_tokens: u64, estimated_cost_usd: f64) {
    step("Cost estimate", "");
    bar(&format!("~{} tokens", cyan(&fmt_tokens(estimated_tokens))));
    bar(&format!(
        "~{} USD",
        yellow(&format!("${:.4}", estimated_cost_usd))
    ));
}

/// Print actual cost with delta against the prediction.
///
/// Prints:
/// ```text
/// ◇  Cost actual
/// │  <tokens> tokens
/// │  $<cost> USD
/// │  Δ <sign>$<delta> (<direction> estimate)
/// ```
///
/// The delta line is green if under budget, yellow if over, and omitted when
/// the delta is zero (within $0.0001 tolerance).
///
/// `tokens` is the measured token count (input + output).
/// `cost` is the actual cost in USD.
/// `predicted` is the pre-execution cost estimate from `print_cost_prediction`.
pub fn print_cost_actual(tokens: u64, cost: f64, predicted: f64) {
    step("Cost actual", "");
    bar(&format!("{} tokens", cyan(&fmt_tokens(tokens))));
    bar(&format!("{} USD", cyan(&format!("${:.4}", cost))));

    let delta = cost - predicted;
    if delta.abs() < 0.0001 {
        // No meaningful delta — skip the line.
        return;
    }

    let (sign, direction, color_fn): (&str, &str, fn(&str) -> String) = if delta < 0.0 {
        ("\u{2212}", "under estimate", green as fn(&str) -> String)
    } else {
        ("+", "over estimate", yellow as fn(&str) -> String)
    };

    bar(&color_fn(&format!(
        "\u{0394} {}${:.4} ({})",
        sign,
        delta.abs(),
        direction,
    )));
}

/// Print the knowledge loading status block.
///
/// Prints:
/// ```text
/// ◇  Knowledge
/// │  N facts loaded (avg confidence: 0.XX)
/// ```
///
/// When `fact_count` is 0, prints:
/// ```text
/// ◇  Knowledge
/// │  no facts loaded
/// ```
///
/// `fact_count` is the number of knowledge facts retrieved from the neuro store.
/// `avg_confidence` is the mean confidence score across loaded facts (0.0-1.0).
pub fn print_knowledge_loaded(fact_count: usize, avg_confidence: f64) {
    step("Knowledge", "");
    if fact_count == 0 {
        note("no facts loaded");
    } else {
        bar(&format!(
            "{} facts loaded {}",
            cyan(&fact_count.to_string()),
            dim(&format!("(avg confidence: {:.2})", avg_confidence)),
        ));
    }
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

/// Print a single gate result line.
///
/// Prints:
/// ```text
/// ├  ✔ <name>   <duration>ms
/// ```
/// for a passing gate, or:
/// ```text
/// ├  ✖ <name>   <duration>ms
/// │    <first line of error output>
/// │    <second line of error output>
/// ```
/// for a failing gate. Error output is truncated to the first 3 lines.
///
/// `name` is the gate name (e.g., `"compile"`, `"test"`).
/// `passed` indicates whether the gate passed.
/// `duration_ms` is the gate execution time in milliseconds.
/// `error_output` is the stderr/error content (only shown when `!passed`).
pub fn print_gate_result(name: &str, passed: bool, duration_ms: u64, error_output: &str) {
    let icon = if passed {
        green(symbols::PASS)
    } else {
        red(symbols::FAIL)
    };
    let name_display = if passed { green(name) } else { red(name) };
    let duration = dim(&format!("{}ms", duration_ms));

    println!(
        "{}  {} {:<12} {}",
        symbols::BRANCH,
        icon,
        name_display,
        duration,
    );

    if !passed && !error_output.is_empty() {
        for line in error_output.lines().filter(|line| !line.is_empty()).take(3) {
            println!("{}    {}", symbols::BAR, dim(line));
        }
    }
}

/// Print an end line: `└  <text>`.
pub fn end(text: &str) {
    println!("{}  {}", symbols::END, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_split_on_arrow() {
        // Smoke test: just ensure the function doesn't panic with typical inputs.
        // (We can't assert on exact output because ANSI codes vary.)
        print_identity(
            "researcher",
            "claude-sonnet-4-6",
            "cascade-router → claude-sonnet-4-6",
        );
        print_identity("coder", "gpt-4o", "direct");
    }

    #[test]
    fn fmt_tokens_small() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(999), "999");
    }

    #[test]
    fn fmt_tokens_thousands() {
        assert_eq!(fmt_tokens(1_000), "1\u{202F}000");
        assert_eq!(fmt_tokens(8_400), "8\u{202F}400");
        assert_eq!(fmt_tokens(1_234_567), "1\u{202F}234\u{202F}567");
    }

    #[test]
    fn cost_prediction_no_panic() {
        print_cost_prediction(8_400, 0.042);
        print_cost_prediction(0, 0.0);
    }

    #[test]
    fn cost_actual_under_budget_no_panic() {
        print_cost_actual(6_200, 0.031, 0.04);
    }

    #[test]
    fn cost_actual_over_budget_no_panic() {
        print_cost_actual(10_400, 0.052, 0.04);
    }

    #[test]
    fn cost_actual_exact_match_no_panic() {
        print_cost_actual(8_400, 0.04, 0.04);
    }

    #[test]
    fn cost_actual_zero_tokens_no_panic() {
        print_cost_actual(0, 0.0, 0.0);
    }

    #[test]
    fn knowledge_loaded_no_panic() {
        print_knowledge_loaded(12, 0.87);
        print_knowledge_loaded(0, 0.0);
        print_knowledge_loaded(1, 1.0);
    }

    #[test]
    fn knowledge_loaded_zero_facts() {
        // Just verify the zero-fact branch doesn't panic.
        print_knowledge_loaded(0, 0.5);
    }

    #[test]
    fn gate_result_pass_no_panic() {
        print_gate_result("compile", true, 12, "");
        print_gate_result("clippy", true, 45, "");
    }

    #[test]
    fn gate_result_fail_no_panic() {
        print_gate_result(
            "test",
            false,
            340,
            "error[E0308]: mismatched types\n  --> src/main.rs:42:5",
        );
    }

    #[test]
    fn gate_result_fail_long_error_truncates() {
        let long_error = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        print_gate_result("test", false, 500, &long_error);
    }

    #[test]
    fn gate_result_fail_empty_error_no_panic() {
        print_gate_result("lint", false, 10, "");
    }
}
