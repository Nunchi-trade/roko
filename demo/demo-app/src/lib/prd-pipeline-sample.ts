import type {
  PipelineDemoState,
  PipelineExampleId,
  PipelineScenarioExample,
} from './prd-pipeline-types';

// ── Seed content for deterministic demo playback ──────────
// Each example includes pre-built PRD, tasks.toml, and implementation files
// so the demo never waits for LLM generation.

const SIMPLE_STATUS_PRD = `---
id: prd-status-command-cli
title: Status Command CLI
status: draft
version: 1
created: 2026-04-30
updated: 2026-04-30
depends_on: []
crates: []
plans_generated: []
coverage: 0
tags: [cli, status]
---

# Status Command CLI

## Overview
Add a \`status\` subcommand to the CLI that reports operational readiness.

## Requirements
- R1: \`status\` prints \`status: ok\` on stdout and exits 0
- R2: \`status --json\` prints \`{"status":"ok"}\` on stdout
- R3: All code passes \`cargo test\` and \`cargo clippy -- -D warnings\`

## Acceptance criteria
- AC1: Running \`cargo run -- status\` prints \`status: ok\`
- AC2: Running \`cargo run -- status --json\` prints valid JSON
- AC3: \`cargo test\` passes with at least 2 tests
- AC4: \`cargo clippy -- -D warnings\` produces zero warnings
`;

const SIMPLE_STATUS_TASKS = `[meta]
plan = "status-command-cli"
iteration = 1
total = 2
done = 0
status = "ready"

[[task]]
id = "S001"
title = "Implement status subcommand"
role = "coding"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Add status and status --json output to main.rs."
files = ["src/main.rs"]
depends_on = []
max_loc = 30

[[task.verify]]
command = "cargo build"

[[task]]
id = "S002"
title = "Add unit tests"
role = "testing"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Add tests for status output in lib.rs."
files = ["src/lib.rs"]
depends_on = ["S001"]

[[task.verify]]
command = "cargo test"

[[task.verify]]
command = "cargo clippy -- -D warnings"
`;

const SIMPLE_STATUS_PLAN = `# Status Command CLI — Plan

## Overview
Implement a minimal \`status\` subcommand with human and JSON output modes.

## Tasks
1. **S001** — Implement status subcommand (mechanical, T1)
2. **S002** — Add unit tests (mechanical, T1)

## Gates
- cargo build
- cargo test
- cargo clippy -- -D warnings
`;

const SIMPLE_STATUS_MAIN = `use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "status" {
        let json = args.iter().any(|a| a == "--json");
        if json {
            println!("{}", roko_pipeline_simple::status_json());
        } else {
            println!("{}", roko_pipeline_simple::status_text());
        }
    } else {
        println!("Simple status CLI demo");
    }
}
`;

const SIMPLE_STATUS_LIB = `/// Return human-readable status line.
pub fn status_text() -> &'static str {
    "status: ok"
}

/// Return JSON status string.
pub fn status_json() -> &'static str {
    r#"{"status":"ok"}"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_output() {
        assert_eq!(status_text(), "status: ok");
    }

    #[test]
    fn json_output() {
        let j = status_json();
        assert!(j.contains(r#""status":"ok""#));
    }
}
`;

// ── release-watch ─────────────────────────────────────────

const RELEASE_WATCH_PRD = `---
id: prd-release-watch-cli
title: Release Watch CLI
status: draft
version: 1
created: 2026-04-30
updated: 2026-04-30
depends_on: []
crates: []
plans_generated: []
coverage: 0
tags: [cli, github, releases]
---

# Release Watch CLI

## Overview
CLI that checks the latest GitHub release tag for a repository and compares
it against the user's current version.

## Requirements
- R1: Accept \`--repo owner/name\` and \`--current vX.Y.Z\` arguments
- R2: Print \`up-to-date\` or \`update available: vA.B.C\`
- R3: \`--json\` flag outputs structured JSON
- R4: Unit tests use fixture JSON (no live HTTP)

## Acceptance criteria
- AC1: \`cargo test\` passes with fixture-based tests
- AC2: \`cargo clippy -- -D warnings\` is clean
`;

const RELEASE_WATCH_TASKS = `[meta]
plan = "release-watch-cli"
iteration = 1
total = 3
done = 0
status = "ready"

[[task]]
id = "R001"
title = "Implement version comparison logic"
role = "coding"
status = "ready"
tier = "focused"
model_hint = "sonnet"
description = "Parse semver strings and compare versions."
files = ["src/lib.rs"]
depends_on = []
max_loc = 40

[[task.verify]]
command = "cargo build"

[[task]]
id = "R002"
title = "Wire CLI argument parsing"
role = "coding"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Parse --repo, --current, --json from argv."
files = ["src/main.rs"]
depends_on = ["R001"]

[[task.verify]]
command = "cargo build"

[[task]]
id = "R003"
title = "Add fixture tests"
role = "testing"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Test comparison logic with known version pairs."
files = ["src/lib.rs"]
depends_on = ["R001"]

[[task.verify]]
command = "cargo test"

[[task.verify]]
command = "cargo clippy -- -D warnings"
`;

const RELEASE_WATCH_PLAN = `# Release Watch CLI — Plan

## Overview
Build a CLI that compares a local version against a latest release tag.

## Tasks
1. **R001** — Implement version comparison logic (focused, T2)
2. **R002** — Wire CLI argument parsing (mechanical, T1)
3. **R003** — Add fixture tests (mechanical, T1)

## Gates
- cargo build, cargo test, cargo clippy
`;

const RELEASE_WATCH_MAIN = `use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut repo = String::new();
    let mut current = String::new();
    let mut json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--repo" if i + 1 < args.len() => { repo = args[i + 1].clone(); i += 2; }
            "--current" if i + 1 < args.len() => { current = args[i + 1].clone(); i += 2; }
            "--json" => { json = true; i += 1; }
            _ => { i += 1; }
        }
    }
    if repo.is_empty() || current.is_empty() {
        eprintln!("Usage: release-watch --repo owner/name --current vX.Y.Z [--json]");
        std::process::exit(1);
    }
    // In a real build this would fetch from GitHub; demo uses a stub latest.
    let latest = "v1.2.0";
    let result = roko_pipeline_release_watch::compare_versions(&current, latest);
    if json {
        println!(
            r#"{{"repo":"{}","current":"{}","latest":"{}","up_to_date":{}}}"#,
            repo, current, latest, result
        );
    } else if result {
        println!("up-to-date ({current})");
    } else {
        println!("update available: {latest} (current: {current})");
    }
}
`;

const RELEASE_WATCH_LIB = `/// Compare two version strings (vX.Y.Z format). Returns true if current >= latest.
pub fn compare_versions(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let s = v.strip_prefix('v').unwrap_or(v);
        let parts: Vec<u32> = s.split('.').filter_map(|p| p.parse().ok()).collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };
    parse(current) >= parse(latest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_version_is_up_to_date() {
        assert!(compare_versions("v1.2.0", "v1.2.0"));
    }

    #[test]
    fn newer_is_up_to_date() {
        assert!(compare_versions("v2.0.0", "v1.2.0"));
    }

    #[test]
    fn older_needs_update() {
        assert!(!compare_versions("v1.0.0", "v1.2.0"));
    }

    #[test]
    fn patch_comparison() {
        assert!(!compare_versions("v1.2.0", "v1.2.1"));
        assert!(compare_versions("v1.2.2", "v1.2.1"));
    }
}
`;

// ── funding-alert ─────────────────────────────────────────

const FUNDING_ALERT_PRD = `---
id: prd-btc-funding-alert-cli
title: BTC Funding Alert CLI
status: draft
version: 1
created: 2026-04-30
updated: 2026-04-30
depends_on: []
crates: []
plans_generated: []
coverage: 0
tags: [cli, defi, alerts]
---

# BTC Funding Alert CLI

## Overview
CLI that monitors BTC perpetual funding rates and triggers an alert when
funding flips negative, indicating potential market dislocation.

## Requirements
- R1: Fetch funding rate data (offline fixture for tests)
- R2: Detect negative funding flip from positive→negative transition
- R3: \`--dry-run\` prints the alert without sending email
- R4: \`--json\` outputs structured JSON
- R5: All code passes cargo test and clippy gates

## Acceptance criteria
- AC1: \`cargo test\` passes with fixture-based detector tests
- AC2: \`cargo clippy -- -D warnings\` is clean
- AC3: \`cargo run -- --dry-run\` prints alert status
`;

const FUNDING_ALERT_TASKS = `[meta]
plan = "btc-funding-alert-cli"
iteration = 1
total = 3
done = 0
status = "ready"

[[task]]
id = "F001"
title = "Implement funding rate types and detector"
role = "coding"
status = "ready"
tier = "focused"
model_hint = "sonnet"
description = "Define FundingRate struct and detect_flip function."
files = ["src/lib.rs"]
depends_on = []
max_loc = 60

[[task.verify]]
command = "cargo build"

[[task]]
id = "F002"
title = "Wire CLI with dry-run and JSON modes"
role = "coding"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Parse --dry-run and --json flags, print alert output."
files = ["src/main.rs"]
depends_on = ["F001"]

[[task.verify]]
command = "cargo build"

[[task]]
id = "F003"
title = "Add fixture tests for flip detection"
role = "testing"
status = "ready"
tier = "mechanical"
model_hint = "haiku"
description = "Test positive-to-negative flip and stable-positive scenarios."
files = ["src/lib.rs"]
depends_on = ["F001"]

[[task.verify]]
command = "cargo test"

[[task.verify]]
command = "cargo clippy -- -D warnings"
`;

const FUNDING_ALERT_PLAN = `# BTC Funding Alert CLI — Plan

## Overview
Build a funding rate monitor with offline-testable detection logic,
dry-run mode, and JSON output.

## Tasks
1. **F001** — Implement funding rate types and detector (focused, T2)
2. **F002** — Wire CLI with dry-run and JSON modes (mechanical, T1)
3. **F003** — Add fixture tests for flip detection (mechanical, T1)

## Gates
- cargo build, cargo test, cargo clippy
`;

const FUNDING_ALERT_MAIN = `use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let dry_run = args.iter().any(|a| a == "--dry-run");
    let json = args.iter().any(|a| a == "--json");

    // Fixture data for demo — in production this would come from Hyperliquid API
    let rates = vec![
        roko_pipeline_funding_alert::FundingRate::new(0.0100),
        roko_pipeline_funding_alert::FundingRate::new(0.0050),
        roko_pipeline_funding_alert::FundingRate::new(-0.0020),
    ];

    let alert = roko_pipeline_funding_alert::detect_flip(&rates);

    if json {
        println!(
            r#"{{"alert":{},"dry_run":{},"rates_checked":{},"latest_rate":{}}}"#,
            alert, dry_run, rates.len(), rates.last().map(|r| r.rate).unwrap_or(0.0)
        );
    } else if alert {
        let msg = "ALERT: BTC funding flipped negative";
        if dry_run {
            println!("[dry-run] {msg}");
        } else {
            println!("{msg}");
        }
    } else {
        println!("Funding stable (positive)");
    }
}
`;

const FUNDING_ALERT_LIB = `/// A single funding rate observation.
#[derive(Debug, Clone, Copy)]
pub struct FundingRate {
    pub rate: f64,
}

impl FundingRate {
    pub fn new(rate: f64) -> Self {
        Self { rate }
    }
}

/// Detect a positive-to-negative funding flip in a time series of rates.
/// Returns true if the last rate is negative and any preceding rate was positive.
pub fn detect_flip(rates: &[FundingRate]) -> bool {
    if rates.len() < 2 {
        return false;
    }
    let last = rates.last().unwrap();
    if last.rate >= 0.0 {
        return false;
    }
    // Check that at least one earlier rate was positive
    rates[..rates.len() - 1].iter().any(|r| r.rate > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_positive_to_negative_flip() {
        let rates = vec![
            FundingRate::new(0.01),
            FundingRate::new(0.005),
            FundingRate::new(-0.002),
        ];
        assert!(detect_flip(&rates));
    }

    #[test]
    fn stable_positive_no_flip() {
        let rates = vec![
            FundingRate::new(0.01),
            FundingRate::new(0.02),
            FundingRate::new(0.015),
        ];
        assert!(!detect_flip(&rates));
    }

    #[test]
    fn single_rate_no_flip() {
        assert!(!detect_flip(&[FundingRate::new(-0.01)]));
    }

    #[test]
    fn empty_rates_no_flip() {
        assert!(!detect_flip(&[]));
    }

    #[test]
    fn all_negative_no_flip() {
        let rates = vec![
            FundingRate::new(-0.01),
            FundingRate::new(-0.02),
        ];
        assert!(!detect_flip(&rates));
    }
}
`;

export const PIPELINE_EXAMPLES: PipelineScenarioExample[] = [
  {
    id: 'simple-status',
    label: 'Simple status CLI',
    complexity: 'Super simple',
    prdTitle: 'Status Command CLI',
    slug: 'status-command-cli',
    workspacePrefix: 'roko-prd-simple',
    repoName: 'roko_pipeline_simple',
    setupDescription: 'Minimal Rust CLI with one existing main.rs file.',
    idea:
      'Add a status command to this Rust CLI. It should print status: ok for humans, support status --json for automation, and include cargo-testable verification.',
    why: [
      'One local command, no network, no secrets.',
      'Shows that even a tiny feature becomes explicit tasks and gates.',
      'Most generated work should route to T1 mechanical implementation.',
    ],
    seedPrd: SIMPLE_STATUS_PRD,
    seedTasksToml: SIMPLE_STATUS_TASKS,
    seedPlanMd: SIMPLE_STATUS_PLAN,
    seedFiles: {
      'src/main.rs': SIMPLE_STATUS_MAIN,
      'src/lib.rs': SIMPLE_STATUS_LIB,
    },
  },
  {
    id: 'release-watch',
    label: 'GitHub release watcher',
    complexity: 'Slightly more complex',
    prdTitle: 'Release Watch CLI',
    slug: 'release-watch-cli',
    workspacePrefix: 'roko-prd-release',
    repoName: 'roko_pipeline_release_watch',
    setupDescription: 'Rust CLI skeleton that will need HTTP, JSON parsing, and fixture tests.',
    idea:
      'Build a CLI that checks the latest GitHub release for a repository, compares it against a provided current version, and prints either up-to-date or update available. Support --json output and keep unit tests offline with fixture JSON.',
    why: [
      'Adds HTTP and JSON parsing without requiring paid services.',
      'Forces task separation between API client, comparison logic, CLI rendering, and tests.',
      'Should mix T1 verification tasks with T2 implementation tasks.',
    ],
    seedPrd: RELEASE_WATCH_PRD,
    seedTasksToml: RELEASE_WATCH_TASKS,
    seedPlanMd: RELEASE_WATCH_PLAN,
    seedFiles: {
      'src/main.rs': RELEASE_WATCH_MAIN,
      'src/lib.rs': RELEASE_WATCH_LIB,
    },
  },
  {
    id: 'funding-alert',
    label: 'BTC funding alert',
    complexity: 'Stage job',
    prdTitle: 'BTC Funding Alert CLI',
    slug: 'btc-funding-alert-cli',
    workspacePrefix: 'roko-prd-funding',
    repoName: 'roko_pipeline_funding_alert',
    setupDescription: 'Rust CLI skeleton for a DeFi data and email integration workflow.',
    stageQuote:
      'Build a CLI that fetches BTC funding rates from Hyperliquid and emails me an alert when funding flips negative.',
    idea:
      'Build a CLI that fetches BTC funding rates from Hyperliquid and emails me an alert when funding flips negative. Separate the Hyperliquid API client, funding flip detector, state persistence, and email notifier so tests can run without live network or SMTP. Include dry-run mode, environment-based email settings, and verification gates for cargo test, clippy, and an offline smoke command.',
    why: [
      'Concrete investor-facing output: a tool people immediately understand.',
      'Multi-skill: Rust, DeFi market data, email integration, and verification.',
      'Good routing story: T1 for fixtures/gates, T2 for implementation, T3 for integration risk.',
      'Cold-start versus warm execution can support the >5x cost delta story.',
    ],
    seedPrd: FUNDING_ALERT_PRD,
    seedTasksToml: FUNDING_ALERT_TASKS,
    seedPlanMd: FUNDING_ALERT_PLAN,
    seedFiles: {
      'src/main.rs': FUNDING_ALERT_MAIN,
      'src/lib.rs': FUNDING_ALERT_LIB,
    },
  },
];

export const DEFAULT_PIPELINE_EXAMPLE_ID: PipelineExampleId = 'funding-alert';

export function getPipelineExample(id?: string): PipelineScenarioExample {
  return (
    PIPELINE_EXAMPLES.find((example) => example.id === id) ??
    PIPELINE_EXAMPLES.find((example) => example.id === DEFAULT_PIPELINE_EXAMPLE_ID) ??
    PIPELINE_EXAMPLES[0]
  );
}

export function createPipelineIntroState(example: PipelineScenarioExample): PipelineDemoState {
  return {
    source: 'empty',
    phase: 'idle',
    headline: `Ready to generate: ${example.label}`,
    example,
    plans: [],
    events: [],
  };
}
