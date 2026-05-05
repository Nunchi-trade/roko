//! Inline terminal wiring for the plan runner.
//!
//! This is intentionally runner-local glue around the existing
//! `crate::inline` primitives. It does not introduce a second output sink
//! abstraction; `stream_to_stderr = false` maps to a disabled renderer.

use std::collections::HashMap;
use std::io::{self, Write as _};
use std::time::Instant;

use ratatui::text::{Line, Span};
use serde_json::Value;
use tracing::debug;

use crate::inline::plaintext::lines_to_plain;
use crate::inline::primitives::{CostMeter, DiffBlockData, DiffEntry, ToolCallBlock};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::InlineTerminal;
use crate::tui::Theme;

use super::types::{GateCompletion, GateCompletionKind};

enum InlineTarget {
    Inline(InlineTerminal),
    Plain,
    Disabled,
}

struct PendingToolCall {
    name: String,
    started_at: Instant,
}

/// Runner-owned inline output state.
pub(crate) struct RunnerInlineTerminal {
    target: InlineTarget,
    theme: Theme,
    cost: CostMeter,
    pending_tools: HashMap<String, PendingToolCall>,
}

impl RunnerInlineTerminal {
    pub(crate) fn new(stream_to_stderr: bool) -> Self {
        let mut theme = Theme::from_env();
        let target = if stream_to_stderr {
            match InlineTerminal::new() {
                Ok(term) => {
                    theme = *term.theme();
                    InlineTarget::Inline(term)
                }
                Err(err) => {
                    debug!(error = %err, "inline terminal unavailable; using structured plain output");
                    InlineTarget::Plain
                }
            }
        } else {
            InlineTarget::Disabled
        };

        Self {
            target,
            theme,
            cost: CostMeter::new(),
            pending_tools: HashMap::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        !matches!(self.target, InlineTarget::Disabled)
    }

    pub(crate) fn warm_cache_started(&mut self) {
        self.push_lines(&[styled::section_start(
            &self.theme,
            "warm",
            "cargo cache",
            None,
        )]);
    }

    pub(crate) fn warm_cache_completed(&mut self, warm_ms: u64) {
        self.push_lines(&[styled::section_end(
            &self.theme,
            "warm",
            &format!("ready in {warm_ms}ms"),
        )]);
    }

    pub(crate) fn task_started(
        &mut self,
        task_id: &str,
        role: &str,
        title: &str,
        attempt_num: u32,
    ) {
        let attempt = (attempt_num > 1).then(|| format!("attempt {attempt_num}"));
        self.push_blank();
        self.push_lines(&[
            styled::section_start(&self.theme, "task", task_id, attempt.as_deref()),
            styled::continuation(&self.theme, "role", role, None),
            styled::continuation(&self.theme, "title", &truncate_chars(title, 120), None),
        ]);
    }

    pub(crate) fn agent_started(&mut self, provider: &str, model: &str, pid: Option<u32>) {
        let detail = pid.map(|pid| format!("{provider} pid {pid}"));
        self.push_lines(&[styled::continuation(
            &self.theme,
            "agent",
            model,
            detail.as_deref(),
        )]);
    }

    pub(crate) fn agent_text(&mut self, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        let rendered: Vec<Line<'static>> = lines
            .iter()
            .map(|line| styled::indented_line(&self.theme, line, 2))
            .collect();
        self.push_lines(&rendered);
    }

    pub(crate) fn tool_call_started(&mut self, id: &str, name: &str) {
        self.pending_tools.insert(
            id.to_string(),
            PendingToolCall {
                name: name.to_string(),
                started_at: Instant::now(),
            },
        );
        let block = ToolCallBlock::from_start(name, &Value::Null);
        self.push_lines(&block.to_lines(&self.theme));
    }

    pub(crate) fn tool_output(&mut self, id: &str, output: &str) {
        if let Some(pending) = self.pending_tools.remove(id) {
            let mut block = ToolCallBlock::from_start(&pending.name, &Value::Null);
            block.set_result(output, pending.started_at.elapsed().as_secs_f64(), true);
            self.push_lines(&block.to_lines(&self.theme));
        }

        let first_line = output.lines().next().unwrap_or("").trim();
        if !first_line.is_empty() {
            let preview = truncate_chars(first_line, 80);
            self.push_lines(&[styled::indented_line(&self.theme, &preview, 4)]);
        }
    }

    pub(crate) fn token_usage(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        model: &str,
    ) {
        self.cost.input_tokens += input_tokens;
        self.cost.output_tokens += output_tokens;
        self.cost.cache_hits += cache_read_tokens;
        self.cost.cache_misses += cache_write_tokens;
        *self.cost.model_tokens.entry(model.to_string()).or_default() +=
            input_tokens + output_tokens;
        self.push_cost_line(model);
    }

    pub(crate) fn agent_turn_completed(
        &mut self,
        total_cost_usd: Option<f64>,
        is_error: bool,
        model: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
    ) {
        if let Some(cost) = total_cost_usd {
            self.cost.total_cost = cost;
        }
        self.cost.input_tokens = total_input_tokens;
        self.cost.output_tokens = total_output_tokens;
        self.cost.run_count += 1;

        let value = if is_error {
            format!(
                "failed {} ${:.4} {} {} in / {} out",
                symbols::SEP,
                self.cost.total_cost,
                symbols::SEP,
                self.cost.input_tokens,
                self.cost.output_tokens
            )
        } else {
            format!(
                "complete {} ${:.4} {} {} in / {} out {} {model}",
                symbols::SEP,
                self.cost.total_cost,
                symbols::SEP,
                self.cost.input_tokens,
                self.cost.output_tokens,
                symbols::SEP
            )
        };
        self.push_lines(&[styled::section_end(&self.theme, "agent", &value)]);
    }

    pub(crate) fn agent_error(&mut self, message: &str) {
        let msg = truncate_chars(message, 120);
        self.push_lines(&[Line::from(vec![
            Span::styled(symbols::END.to_string(), self.theme.muted()),
            Span::raw(" "),
            Span::styled("error     ", self.theme.danger()),
            Span::styled(msg, self.theme.danger()),
        ])]);
    }

    pub(crate) fn gate_completed(&mut self, completion: &GateCompletion) {
        if completion.kind != GateCompletionKind::Gate {
            return;
        }
        for verdict in &completion.verdicts {
            let icon = if verdict.passed {
                symbols::PASS
            } else {
                symbols::FAIL
            };
            let secs = completion.duration_ms / 1000;
            self.push_lines(&[styled::continuation(
                &self.theme,
                "gate",
                &format!("{icon} {}", verdict.gate_name),
                Some(&format!(
                    "rung {} {} {secs}s",
                    completion.rung,
                    symbols::SEP
                )),
            )]);
        }
    }

    pub(crate) fn gate_retry(&mut self, next_attempt: u32, cooldown_ms: u64) {
        let delay_s = cooldown_ms / 1000;
        self.push_lines(&[styled::continuation(
            &self.theme,
            "retry",
            "gate failed",
            Some(&format!(
                "attempt {next_attempt} {} backoff {delay_s}s",
                symbols::SEP
            )),
        )]);
    }

    pub(crate) fn diff_block(&mut self, entries: &[DiffEntry]) {
        if entries.is_empty() {
            return;
        }
        let block = DiffBlockData {
            entries: entries.to_vec(),
            expanded: true,
        };
        self.push_lines(&block.to_lines(&self.theme));
    }

    pub(crate) fn task_done(&mut self, completed: usize, total: usize, total_task_ms: u64) {
        let secs = total_task_ms / 1000;
        self.push_lines(&[styled::section_end(
            &self.theme,
            "done",
            &format!("{secs}s {} {completed}/{total} tasks", symbols::SEP),
        )]);
    }

    pub(crate) fn task_failed(&mut self, reason: &str) {
        let summary = truncate_chars(reason.lines().next().unwrap_or("failed"), 120);
        self.push_lines(&[Line::from(vec![
            Span::styled(symbols::END.to_string(), self.theme.muted()),
            Span::raw(" "),
            Span::styled("failed    ", self.theme.danger()),
            Span::styled(summary, self.theme.danger()),
        ])]);
    }

    fn push_cost_line(&mut self, model: &str) {
        self.push_lines(&[styled::continuation(
            &self.theme,
            "cost",
            &format!("${:.4}", self.cost.total_cost),
            Some(&format!(
                "{} in / {} out {} {model}",
                self.cost.input_tokens,
                self.cost.output_tokens,
                symbols::SEP
            )),
        )]);
    }

    fn push_blank(&mut self) {
        self.push_lines(&[Line::raw("")]);
    }

    fn push_lines(&mut self, lines: &[Line<'static>]) {
        if lines.is_empty() {
            return;
        }

        match &mut self.target {
            InlineTarget::Inline(term) => {
                if let Err(err) = term.push_lines(lines) {
                    debug!(error = %err, "inline terminal render failed; switching to structured plain output");
                    self.target = InlineTarget::Plain;
                    write_plain(lines);
                }
            }
            InlineTarget::Plain => write_plain(lines),
            InlineTarget::Disabled => {}
        }
    }
}

fn write_plain(lines: &[Line<'_>]) {
    let text = lines_to_plain(lines);
    let mut stderr = io::stderr().lock();
    let _ = stderr.write_all(text.as_bytes());
    let _ = stderr.flush();
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}
