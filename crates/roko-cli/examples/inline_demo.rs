//! Visual demo of all inline rendering primitives.
//!
//! Run with: cargo run -p roko-cli --example inline_demo

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode};
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use roko_cli::inline::markdown;
use roko_cli::inline::primitives::*;
use roko_cli::inline::styled;
use roko_cli::inline::symbols;
use roko_cli::inline::terminal::InlineTerminal;
use roko_cli::tui::Theme;

const REVEAL: Duration = Duration::from_millis(25);
const FAST: Duration = Duration::from_millis(12);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut term = InlineTerminal::new()?;
    let theme = *term.theme();

    pause(200);
    term.push_lines_revealed(
        &[
            styled::section_start(
                &theme,
                "roko",
                "inline rendering engine",
                Some("all primitives"),
            ),
            styled::continuation(&theme, "", "press any key to advance", None),
        ],
        REVEAL,
    )?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 1. RunBlock ─────────────────────────────────────
    term.push_separator()?;
    pause(100);

    let block = RunBlockData {
        agent_name: "auditor@v1".into(),
        identity: Some("eid://roko/auditor.v1".into()),
        attested: true,
        predicted_cost: Some(0.043),
        predicted_time: Some(12.4),
        predicted_route: Some("haiku".into()),
        gate_verdicts: vec![
            ("compile".into(), true),
            ("test".into(), true),
            ("clippy".into(), true),
            ("secret_scan".into(), false),
        ],
        knowledge_loaded: Some(KnowledgeInfo {
            count: 7,
            topic: "/infra/payments-svc".into(),
            agent_count: 3,
            avg_confidence: 0.91,
        }),
        actual_cost: Some(0.031),
        actual_route: Some("haiku".into()),
        actual_time: Some(9.8),
        tool_calls: vec![
            ToolCallInfo {
                name: "ReadFile".into(),
                summary: "src/auth.rs (247 lines)".into(),
                duration_s: 0.3,
            },
            ToolCallInfo {
                name: "Edit".into(),
                summary: "src/auth.rs:42 (+3 -1)".into(),
                duration_s: 0.1,
            },
            ToolCallInfo {
                name: "Bash".into(),
                summary: "cargo test --lib".into(),
                duration_s: 2.1,
            },
        ],
        deposited_count: 2,
        deposited_path: Some("/infra/payments-svc".into()),
        chain_block: Some(4821),
    };
    term.push_lines_revealed(&block.to_lines(&theme), REVEAL)?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 2. GateBlock ────────────────────────────────────
    term.push_separator()?;
    pause(100);

    let gates = GateBlockData {
        policy: Some("prod-sec".into()),
        rungs: vec![
            GateRung {
                name: "compile".into(),
                status: GateStatus::Passed {
                    summary: "0 errors (142 crates)".into(),
                    duration_s: 2.1,
                },
            },
            GateRung {
                name: "clippy".into(),
                status: GateStatus::Passed {
                    summary: "0 warnings".into(),
                    duration_s: 0.8,
                },
            },
            GateRung {
                name: "test".into(),
                status: GateStatus::Passed {
                    summary: "11/11 pass".into(),
                    duration_s: 3.4,
                },
            },
            GateRung {
                name: "secret_scan".into(),
                status: GateStatus::Failed {
                    reason: "AWS_SECRET in env.yaml:14".into(),
                    duration_s: 0.3,
                },
            },
            GateRung {
                name: "diff".into(),
                status: GateStatus::Skipped,
            },
            GateRung {
                name: "llm_judge".into(),
                status: GateStatus::Pending,
            },
            GateRung {
                name: "verify".into(),
                status: GateStatus::Pending,
            },
        ],
    };
    term.push_lines_revealed(&gates.to_lines(&theme), Duration::from_millis(50))?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 3. ErrorBlock + ReplanBlock ─────────────────────
    term.push_separator()?;
    pause(100);

    let error = ErrorBlockData {
        severity: ErrorSeverity::Error,
        source: "compile".into(),
        summary: "error[E0308]: mismatched types".into(),
        location: Some("src/handler.rs:42:18".into()),
        details: vec![
            "42 │     let cost: i32 = calculate_cost();".into(),
            "   │                     ^^^^^^^^^^^^^^^^ expected i32, found String".into(),
        ],
        retry: Some(RetryInfo {
            attempt: 1,
            max_attempts: 3,
            retry_in_s: 10.0,
            strategy: "exponential backoff".into(),
        }),
    };
    term.push_lines_revealed(&error.to_lines(&theme), REVEAL)?;
    pause(400);

    let replan = ReplanBlockData {
        failed_gate: "test".into(),
        failure_reason: "assertion error in handler.rs:42".into(),
        strategy: "escalate to sonnet".into(),
        confidence_before: Some(0.67),
        confidence_required: Some(0.91),
        attempt: 2,
        max_attempts: 3,
        retry_succeeded: true,
        retry_duration_s: 4.2,
        replan_cost_usd: 0.027,
    };
    term.push_lines_revealed(&replan.to_lines(&theme), REVEAL)?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 4. ProgressTree ─────────────────────────────────
    term.push_separator()?;
    pause(100);

    let tree = ProgressTreeData {
        plan_name: "deploy-audit".into(),
        waves: vec![
            TreeWave {
                number: 1,
                tasks: vec![
                    TreeTask {
                        id: "T01".into(),
                        title: "dep-scan".into(),
                        status: TaskProgress::Done {
                            cost_usd: 0.012,
                            duration_s: 2.1,
                        },
                    },
                    TreeTask {
                        id: "T02".into(),
                        title: "secret-scan".into(),
                        status: TaskProgress::Done {
                            cost_usd: 0.008,
                            duration_s: 1.4,
                        },
                    },
                    TreeTask {
                        id: "T03".into(),
                        title: "policy-check".into(),
                        status: TaskProgress::Done {
                            cost_usd: 0.031,
                            duration_s: 4.2,
                        },
                    },
                ],
            },
            TreeWave {
                number: 2,
                tasks: vec![
                    TreeTask {
                        id: "T04".into(),
                        title: "integration-test".into(),
                        status: TaskProgress::Running { elapsed_s: 6.2 },
                    },
                    TreeTask {
                        id: "T05".into(),
                        title: "diff-review".into(),
                        status: TaskProgress::Blocked {
                            blocked_by: "T04".into(),
                        },
                    },
                    TreeTask {
                        id: "T06".into(),
                        title: "cost-analysis".into(),
                        status: TaskProgress::Blocked {
                            blocked_by: "T04".into(),
                        },
                    },
                ],
            },
            TreeWave {
                number: 3,
                tasks: vec![
                    TreeTask {
                        id: "T07".into(),
                        title: "episode-log".into(),
                        status: TaskProgress::Pending,
                    },
                    TreeTask {
                        id: "T08".into(),
                        title: "chain-anchor".into(),
                        status: TaskProgress::Pending,
                    },
                ],
            },
        ],
    };
    term.push_lines_revealed(&tree.to_lines(&theme), Duration::from_millis(40))?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 5. DiffBlock ────────────────────────────────────
    term.push_separator()?;
    pause(100);

    let diff = DiffBlockData {
        entries: vec![
            DiffEntry {
                path: "deploy/env.yaml".into(),
                additions: 1,
                deletions: 1,
                summary: Some("rotated AWS secret".into()),
            },
            DiffEntry {
                path: "src/handler.rs".into(),
                additions: 8,
                deletions: 3,
                summary: Some("Secrets Manager integration".into()),
            },
            DiffEntry {
                path: "tests/auth_test.rs".into(),
                additions: 22,
                deletions: 0,
                summary: Some("new test coverage".into()),
            },
        ],
        expanded: true,
    };
    term.push_lines_revealed(&diff.to_lines(&theme), REVEAL)?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 6. Markdown ─────────────────────────────────────
    term.push_separator()?;
    pause(100);

    let md = r#"## Analysis Summary

The Q3 earnings data shows **significant margin compression** across mid-cap fintech:

| Company | Revenue | Margin | QoQ    |
|---------|---------|--------|--------|
| Stripe  | $4.2B   | 23%    | -2.1pp |
| Block   | $5.1B   | 18%    | -3.4pp |
| Adyen   | $2.1B   | 31%    | +0.8pp |

Key findings:

- Interchange revenue declined *12% QoQ*
- Payment processing margins compressed by ~200bps
- Cross-border volumes remained strong (+14% YoY)

```rust
fn calculate_margin(revenue: f64, costs: f64) -> f64 {
    (revenue - costs) / revenue * 100.0
}
```

> This analysis was generated from 7 engrams across 3 agents
> with an average confidence of 0.91.
"#;

    term.push_lines_revealed(
        &[styled::section_start(
            &theme,
            "markdown",
            "LLM output rendering",
            None,
        )],
        REVEAL,
    )?;
    let md_lines = markdown::render_markdown_with_bar(md, &theme);
    term.push_lines_revealed(&md_lines, FAST)?;
    term.push_lines(&[styled::section_end(&theme, "", "")])?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 7. Tool calls ───────────────────────────────────
    term.push_separator()?;
    pause(100);

    term.push_lines_revealed(
        &[styled::section_start(&theme, "tools", "4 tool calls", None)],
        REVEAL,
    )?;
    let tools: Vec<(&str, serde_json::Value, f64)> = vec![
        (
            "ReadFile",
            serde_json::json!({"file_path": "/Users/will/dev/project/src/payments/handler.rs"}),
            0.3,
        ),
        ("Grep", serde_json::json!({"pattern": "AWS_SECRET"}), 0.2),
        (
            "Bash",
            serde_json::json!({"command": "cargo test --workspace -- payments"}),
            1.8,
        ),
        (
            "Edit",
            serde_json::json!({"file_path": "src/payments/handler.rs"}),
            0.1,
        ),
    ];
    for (name, input, dur) in &tools {
        let mut block = ToolCallBlock::from_start(name, input);
        block.set_result("ok", *dur, true);
        term.push_lines_revealed(&block.to_lines(&theme), REVEAL)?;
        pause(200);
    }
    term.push_lines(&[styled::section_end(&theme, "", "")])?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 8. CostWaterfall ────────────────────────────────
    term.push_separator()?;
    pause(100);

    let waterfall = CostWaterfallData {
        baseline_usd: 2.61,
        entries: vec![
            WaterfallEntry {
                label: "prompt caching".into(),
                savings_usd: 1.31,
                factor: 5.0,
            },
            WaterfallEntry {
                label: "cascade routing (haiku)".into(),
                savings_usd: 0.78,
                factor: 3.1,
            },
            WaterfallEntry {
                label: "knowledge pre-load".into(),
                savings_usd: 0.29,
                factor: 1.4,
            },
            WaterfallEntry {
                label: "gate early-exit".into(),
                savings_usd: 0.14,
                factor: 1.2,
            },
        ],
        actual_usd: 0.084,
    };
    term.push_lines_revealed(&waterfall.to_lines(&theme), Duration::from_millis(60))?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 9. SessionSummary ───────────────────────────────
    term.push_separator()?;
    pause(100);

    let mut meter = CostMeter::new();
    meter.record_run(0.031, 4821, 1203, "haiku", 0.93);
    meter.record_run(0.022, 3100, 890, "haiku", 0.71);
    meter.record_run(0.029, 4200, 1100, "haiku", 0.88);
    for _ in 0..87 {
        meter.record_cache(true);
    }
    for _ in 0..13 {
        meter.record_cache(false);
    }

    let summary = SessionSummaryData {
        cost: meter,
        gates_total: 24,
        gates_passed: 22,
        replans: 2,
        elapsed_s: 32.4,
    };
    term.push_lines_revealed(&summary.to_lines(&theme), Duration::from_millis(50))?;
    term.push_blank()?;
    wait_key(&mut term, &theme)?;

    // ── 10. Live spinner ────────────────────────────────
    term.push_separator()?;
    term.push_lines(&[Line::from(vec![
        Span::styled("  Spinner demo ", theme.text()),
        Span::styled("— press any key to exit", theme.muted()),
    ])])?;

    let start = std::time::Instant::now();
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let tick = (elapsed * 10.0) as u64;

        term.draw(|frame| {
            let area = frame.area();
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);

            let spinner = styled::spinner_line(&theme, tick, "Processing agent task...", elapsed);
            frame.render_widget(Paragraph::new(spinner), chunks[1]);

            let status = styled::status_bar(
                &theme,
                0.082,
                12121,
                3193,
                "haiku",
                Some((elapsed / 8.0).min(1.0)),
            );
            frame.render_widget(Paragraph::new(status), chunks[3]);
        })?;

        if event::poll(Duration::from_millis(33))? {
            if let Event::Key(_) = event::read()? {
                break;
            }
        }
    }

    // ── Finish ──────────────────────────────────────────
    term.push_separator()?;
    pause(200);
    term.push_lines_revealed(
        &[Line::from(vec![
            Span::styled(format!("{} ", symbols::PASS), theme.success()),
            Span::styled("all 11 primitives rendered".to_string(), theme.success()),
        ])],
        REVEAL,
    )?;
    term.push_blank()?;

    drop(term);
    Ok(())
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn wait_key(term: &mut InlineTerminal, theme: &Theme) -> Result<(), Box<dyn std::error::Error>> {
    term.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{} ", symbols::PROMPT), theme.accent()),
                Span::styled("press any key".to_string(), theme.muted()),
            ])),
            chunks[1],
        );
    })?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if !matches!(key.code, KeyCode::Modifier(_)) {
                    return Ok(());
                }
            }
        }
    }
}
