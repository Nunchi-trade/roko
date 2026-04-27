//! Non-interactive autoplay of all inline primitives.
//! Outputs to stdout without raw mode — works in pipes and CI.
//!
//! Run with: cargo run -p roko-cli --example inline_autoplay

use roko_cli::inline::markdown;
use roko_cli::inline::plaintext::print_plain;
use roko_cli::inline::primitives::*;
use roko_cli::inline::styled;
use roko_cli::inline::symbols;
use roko_cli::tui::Theme;

fn main() {
    let theme = Theme::dark();

    section("1. RunBlock — completed run summary");
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
    print_plain(&block.to_lines(&theme));

    section("2. GateBlock — gate pipeline with per-rung status");
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
    print_plain(&gates.to_lines(&theme));

    section("3. ErrorBlock + ReplanBlock — failure + self-healing");
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
    print_plain(&error.to_lines(&theme));
    println!();
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
    print_plain(&replan.to_lines(&theme));

    section("4. ProgressTree — hierarchical plan execution");
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
                ],
            },
            TreeWave {
                number: 3,
                tasks: vec![TreeTask {
                    id: "T06".into(),
                    title: "chain-anchor".into(),
                    status: TaskProgress::Pending,
                }],
            },
        ],
    };
    print_plain(&tree.to_lines(&theme));

    section("5. DiffBlock — file changes");
    let diff = DiffBlockData {
        entries: vec![
            DiffEntry {
                path: "deploy/env.yaml".into(),
                additions: 1,
                deletions: 1,
                summary: Some("rotated secret".into()),
            },
            DiffEntry {
                path: "src/handler.rs".into(),
                additions: 8,
                deletions: 3,
                summary: Some("Secrets Manager".into()),
            },
            DiffEntry {
                path: "tests/auth_test.rs".into(),
                additions: 22,
                deletions: 0,
                summary: Some("new coverage".into()),
            },
        ],
        expanded: true,
    };
    print_plain(&diff.to_lines(&theme));

    section("6. Markdown — rendered LLM output");
    let md = r#"## Analysis Summary

The Q3 earnings data shows **significant margin compression**:

| Company | Revenue | Margin |
|---------|---------|--------|
| Stripe  | $4.2B   | 23%    |
| Block   | $5.1B   | 18%    |

Key findings:
- Interchange revenue declined *12% QoQ*
- Cross-border volumes remained strong

```rust
fn calculate_margin(revenue: f64, costs: f64) -> f64 {
    (revenue - costs) / revenue * 100.0
}
```

> Generated from 7 engrams across 3 agents.
"#;
    let md_lines = markdown::render_markdown_with_bar(md, &theme);
    print_plain(&md_lines);

    section("7. ToolCalls — collapsed tool invocations");
    for (name, input, dur) in [
        (
            "ReadFile",
            serde_json::json!({"file_path": "src/payments/handler.rs"}),
            0.3,
        ),
        ("Grep", serde_json::json!({"pattern": "AWS_SECRET"}), 0.2),
        (
            "Bash",
            serde_json::json!({"command": "cargo test --workspace -- payments"}),
            1.8,
        ),
    ] {
        let mut tc = ToolCallBlock::from_start(name, &input);
        tc.set_result("ok", dur, true);
        print_plain(&tc.to_lines(&theme));
    }

    section("8. CostWaterfall — decomposed savings");
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
    print_plain(&waterfall.to_lines(&theme));

    section("9. SessionSummary — end-of-session roll-up");
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
    print_plain(&summary.to_lines(&theme));

    println!();
    println!("{} all 9 primitives rendered", symbols::PASS);
}

fn section(title: &str) {
    println!();
    println!("─── {title} ───");
    println!();
}
