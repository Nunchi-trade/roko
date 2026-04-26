//! Visual demo of the inline rendering engine.
//!
//! Run with: cargo run -p roko-cli --example inline_demo

use std::time::Duration;

use crossterm::event::{self, Event, KeyCode};
use roko_cli::inline::markdown;
use roko_cli::inline::primitives::{CostMeter, KnowledgeInfo, RunBlockData, ToolCallBlock, ToolCallInfo};
use roko_cli::inline::styled;
use roko_cli::inline::symbols;
use roko_cli::inline::terminal::InlineTerminal;
use roko_cli::tui::Theme;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const REVEAL: Duration = Duration::from_millis(30);
const PAUSE: Duration = Duration::from_millis(400);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut term = InlineTerminal::new()?;
    let theme = *term.theme();

    // ── Intro ───────────────────────────────────────────
    std::thread::sleep(Duration::from_millis(200));
    term.push_lines_revealed(
        &[
            styled::section_start(&theme, "roko", "inline rendering engine", Some("v0.1")),
            styled::continuation(&theme, "", "press any key to advance through each beat", None),
        ],
        REVEAL,
    )?;
    term.push_blank()?;

    wait_key(&mut term, &theme)?;

    // ── Beat 1: Full RunBlock ───────────────────────────
    term.push_separator()?;
    std::thread::sleep(Duration::from_millis(100));

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
            ToolCallInfo { name: "ReadFile".into(), summary: "src/auth.rs (247 lines)".into(), duration_s: 0.3 },
            ToolCallInfo { name: "Edit".into(), summary: "src/auth.rs:42 (+3 -1)".into(), duration_s: 0.1 },
            ToolCallInfo { name: "Bash".into(), summary: "cargo test --lib".into(), duration_s: 2.1 },
        ],
        deposited_count: 2,
        deposited_path: Some("/infra/payments-svc".into()),
        chain_block: Some(4821),
    };

    term.push_lines_revealed(&block.to_lines(&theme), REVEAL)?;
    term.push_blank()?;

    wait_key(&mut term, &theme)?;

    // ── Beat 2: Markdown ────────────────────────────────
    term.push_separator()?;
    std::thread::sleep(Duration::from_millis(100));

    let md = r#"## Analysis Summary

The Q3 earnings data shows **significant margin compression** across mid-cap fintech names:

| Company | Revenue | Margin |
|---------|---------|--------|
| Stripe  | $4.2B   | 23%    |
| Block   | $5.1B   | 18%    |
| Adyen   | $2.1B   | 31%    |

Key findings:

- Interchange revenue declined *12% QoQ*
- Payment processing margins compressed by ~200bps
- Cross-border volumes remained strong

```rust
fn calculate_margin(revenue: f64, costs: f64) -> f64 {
    (revenue - costs) / revenue * 100.0
}
```

> This analysis was generated from 7 engrams across 3 agents
> with an average confidence of 0.91.
"#;

    term.push_lines_revealed(
        &[styled::section_start(&theme, "markdown", "rendered LLM output", None)],
        REVEAL,
    )?;

    let md_lines = markdown::render_markdown_with_bar(md, &theme);
    term.push_lines_revealed(&md_lines, REVEAL)?;
    term.push_lines(&[styled::section_end(&theme, "", "")])?;
    term.push_blank()?;

    wait_key(&mut term, &theme)?;

    // ── Beat 3: Tool calls ──────────────────────────────
    term.push_separator()?;
    std::thread::sleep(Duration::from_millis(100));

    let tools: Vec<(String, serde_json::Value, f64)> = vec![
        ("ReadFile".into(), serde_json::json!({"file_path": "/Users/will/dev/project/src/payments/handler.rs"}), 0.3),
        ("Grep".into(), serde_json::json!({"pattern": "AWS_SECRET"}), 0.2),
        ("Bash".into(), serde_json::json!({"command": "cargo test --workspace -- payments"}), 1.8),
        ("Edit".into(), serde_json::json!({"file_path": "src/payments/handler.rs"}), 0.1),
    ];

    term.push_lines_revealed(
        &[styled::section_start(&theme, "tools", "4 tool calls", None)],
        REVEAL,
    )?;

    for (name, input, dur) in &tools {
        let mut block = ToolCallBlock::from_start(name, input);
        block.set_result("ok", *dur, true);
        term.push_lines_revealed(&block.to_lines(&theme), REVEAL)?;
        std::thread::sleep(PAUSE);
    }

    term.push_lines(&[styled::section_end(&theme, "", "")])?;
    term.push_blank()?;

    wait_key(&mut term, &theme)?;

    // ── Beat 4: Cost summary ────────────────────────────
    term.push_separator()?;
    std::thread::sleep(Duration::from_millis(100));

    let mut meter = CostMeter::new();
    meter.record_run(0.031, 4821, 1203, "haiku", 0.93);
    meter.record_run(0.022, 3100, 890, "haiku", 0.71);
    meter.record_run(0.029, 4200, 1100, "haiku", 0.88);

    let ratio = meter.savings_ratio();
    let summary_lines = vec![
        styled::section_start(&theme, "session", "cost summary", None),
        styled::continuation(&theme, "runs", &meter.run_count.to_string(), None),
        styled::continuation(
            &theme,
            "total",
            &format!("${:.4}", meter.total_cost),
            Some(&format!("baseline: ${:.4}", meter.naive_baseline)),
        ),
        styled::continuation(
            &theme,
            "tokens",
            &format!("{} in / {} out", meter.input_tokens, meter.output_tokens),
            None,
        ),
        styled::continuation(
            &theme,
            "savings",
            &format!("{ratio:.1}x vs naive baseline"),
            None,
        ),
        styled::section_end(&theme, "model", meter.primary_model().unwrap_or("—")),
    ];

    term.push_lines_revealed(&summary_lines, Duration::from_millis(60))?;
    term.push_blank()?;

    wait_key(&mut term, &theme)?;

    // ── Beat 5: Live spinner ────────────────────────────
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
                Constraint::Length(1), // blank
                Constraint::Min(1),    // spinner area
                Constraint::Length(1), // blank
                Constraint::Length(1), // status bar
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
                Some((elapsed / 10.0).min(1.0)),
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
    std::thread::sleep(Duration::from_millis(200));

    term.push_lines_revealed(
        &[Line::from(vec![
            Span::styled(
                format!("{} ", symbols::PASS),
                theme.success(),
            ),
            Span::styled("demo complete".to_string(), theme.success()),
        ])],
        REVEAL,
    )?;
    term.push_blank()?;

    drop(term);
    Ok(())
}

fn wait_key(term: &mut InlineTerminal, theme: &Theme) -> Result<(), Box<dyn std::error::Error>> {
    term.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{} ", symbols::PROMPT),
                    theme.accent(),
                ),
                Span::styled("press any key to continue".to_string(), theme.muted()),
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
