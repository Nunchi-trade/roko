import type {
  PipelineDemoState,
  PipelineExampleId,
  PipelineScenarioExample,
} from './prd-pipeline-types';

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
  },
];

export const DEFAULT_PIPELINE_EXAMPLE_ID: PipelineExampleId = 'simple-status';

export function getPipelineExample(id?: string): PipelineScenarioExample {
  return (
    PIPELINE_EXAMPLES.find((example) => example.id === id) ??
    PIPELINE_EXAMPLES.find((example) => example.id === DEFAULT_PIPELINE_EXAMPLE_ID) ??
    PIPELINE_EXAMPLES[0]
  );
}

export function createPipelineIntroState(example: PipelineScenarioExample): PipelineDemoState {
  // Show rich sample state by default so the investor sees the full pipeline immediately
  const sampleState = PIPELINE_SAMPLE_STATES[example.id];
  if (sampleState) {
    return {
      ...sampleState,
      source: 'sample',
      headline: `Ready to generate: ${example.label}`,
      example,
    };
  }
  return {
    source: 'empty',
    phase: 'idle',
    headline: `Ready to generate: ${example.label}`,
    example,
    plans: [],
    events: [],
    stream: {
      sse: 'idle',
      ws: 'idle',
    },
  };
}

const SIMPLE_SAMPLE_STATE: PipelineDemoState = {
  source: 'sample',
  phase: 'tasks',
  headline: 'Simple status CLI task plan',
  example: PIPELINE_EXAMPLES[0],
  currentCommand: 'roko prd plan status-command-cli',
  lastUpdated: 'sample',
  prd: {
    slug: 'status-command-cli',
    title: 'Status Command CLI',
    status: 'planned',
    path: '.roko/prd/published/status-command-cli.md',
    excerpt:
      'Add a small status command to the demo Rust CLI. The command prints a stable text line for humans and supports a JSON option for automation.',
    requirements: [
      'REQ-001: Add a status subcommand without changing default output.',
      'REQ-002: Support status --json with deterministic field names.',
      'REQ-003: Keep verification runnable through cargo test.',
    ],
    acceptance: [
      'cargo run -- status prints status: ok',
      'cargo run -- status --json emits valid JSON',
      'cargo test passes',
    ],
  },
  plans: [
    {
      id: 'status-command-cli',
      title: 'Status Command CLI',
      path: '.roko/plans/status-command-cli',
      status: 'active',
      estimatedMinutes: 10,
      excerpt:
        'Add one command path, one JSON branch, and focused tests. The generated tasks stay small and route mostly to T1.',
      tasks: [
        {
          id: 'T1',
          title: 'Route the status command',
          description: 'Parse status and keep the existing default command behavior unchanged.',
          status: 'done',
          routeTier: 'T1',
          tier: 'T1 mechanical',
          role: 'implementer',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 35,
          files: ['src/main.rs'],
          dependsOn: [],
          verify: [{ phase: 'test', command: 'cargo test status_text_output' }],
        },
        {
          id: 'T2',
          title: 'Add deterministic JSON output',
          description: 'Emit a predictable JSON object for automation and monitoring scripts.',
          status: 'active',
          routeTier: 'T1',
          tier: 'T1 mechanical',
          role: 'implementer',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 35,
          files: ['src/main.rs', 'Cargo.toml'],
          dependsOn: ['T1'],
          verify: [{ phase: 'test', command: 'cargo test status_json_output' }],
        },
        {
          id: 'T3',
          title: 'Run compile and test gates',
          description: 'Confirm the generated implementation passes the full local gate set.',
          status: 'pending',
          routeTier: 'T1',
          tier: 'T1 verification',
          role: 'verifier',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 10,
          files: ['src/main.rs'],
          dependsOn: ['T1', 'T2'],
          verify: [
            { phase: 'compile', command: 'cargo check' },
            { phase: 'test', command: 'cargo test' },
          ],
        },
      ],
    },
  ],
  events: [
    { id: 'simple-sample-1', ts: '00:00:01', phase: 'draft', text: 'PRD generated from a one-command idea.', kind: 'success' },
    { id: 'simple-sample-2', ts: '00:00:08', phase: 'planning', text: 'Planner produced three small tasks and two verification gates.', kind: 'success' },
  ],
};

const RELEASE_SAMPLE_STATE: PipelineDemoState = {
  source: 'sample',
  phase: 'tasks',
  headline: 'Release watcher task plan',
  example: PIPELINE_EXAMPLES[1],
  currentCommand: 'roko prd plan release-watch-cli',
  lastUpdated: 'sample',
  prd: {
    slug: 'release-watch-cli',
    title: 'Release Watch CLI',
    status: 'planned',
    path: '.roko/prd/published/release-watch-cli.md',
    excerpt:
      'Build a CLI that checks the latest GitHub release for a repository, compares it with a supplied current version, and supports text or JSON output.',
    requirements: [
      'REQ-001: Accept repository and current version inputs.',
      'REQ-002: Fetch and parse latest release metadata.',
      'REQ-003: Keep comparison and rendering testable offline.',
      'REQ-004: Support JSON output for automation.',
    ],
    acceptance: [
      'Fixture tests parse GitHub release JSON.',
      'Version comparison reports update available and up-to-date states.',
      'cargo test and cargo clippy pass.',
    ],
  },
  plans: [
    {
      id: 'release-watch-cli',
      title: 'Release Watch CLI',
      path: '.roko/plans/release-watch-cli',
      status: 'active',
      estimatedMinutes: 24,
      excerpt:
        'Separate CLI parsing, GitHub API data modeling, comparison logic, offline fixtures, and final gates.',
      tasks: [
        {
          id: 'T1',
          title: 'Define CLI inputs and output contract',
          description: 'Add repo, current-version, and --json arguments with predictable error handling.',
          status: 'done',
          routeTier: 'T1',
          tier: 'T1 mechanical',
          role: 'implementer',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 45,
          files: ['src/main.rs', 'Cargo.toml'],
          dependsOn: [],
          verify: [{ phase: 'compile', command: 'cargo check' }],
        },
        {
          id: 'T2',
          title: 'Add GitHub release API client',
          description: 'Fetch latest release JSON and parse tag name, release URL, and published time.',
          status: 'active',
          routeTier: 'T2',
          tier: 'T2 implementation',
          role: 'api implementer',
          modelHint: 'claude-sonnet-4-6',
          maxLoc: 85,
          files: ['src/github.rs', 'src/main.rs', 'Cargo.toml'],
          dependsOn: ['T1'],
          verify: [
            { phase: 'test', command: 'cargo test github_release_fixture' },
            { phase: 'compile', command: 'cargo check' },
          ],
        },
        {
          id: 'T3',
          title: 'Compare versions and render results',
          description: 'Normalize v-prefixed tags and render both human and JSON update states.',
          status: 'pending',
          routeTier: 'T2',
          tier: 'T2 implementation',
          role: 'implementer',
          modelHint: 'claude-sonnet-4-6',
          maxLoc: 75,
          files: ['src/version.rs', 'src/output.rs', 'src/main.rs'],
          dependsOn: ['T2'],
          verify: [{ phase: 'test', command: 'cargo test version_comparison' }],
        },
        {
          id: 'T4',
          title: 'Add offline fixture tests',
          description: 'Cover API parsing, comparison, and JSON output without live GitHub calls.',
          status: 'pending',
          routeTier: 'T1',
          tier: 'T1 verification',
          role: 'verifier',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 45,
          files: ['tests/fixtures/latest_release.json', 'tests/release_watch.rs'],
          dependsOn: ['T2', 'T3'],
          verify: [{ phase: 'test', command: 'cargo test --test release_watch' }],
        },
        {
          id: 'T5',
          title: 'Run lint and smoke gates',
          description: 'Validate the full generated CLI through clippy and an offline smoke path.',
          status: 'pending',
          routeTier: 'T1',
          tier: 'T1 verification',
          role: 'verifier',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 15,
          files: ['src/main.rs'],
          dependsOn: ['T1', 'T2', 'T3', 'T4'],
          verify: [
            { phase: 'clippy', command: 'cargo clippy --all-targets -- -D warnings' },
            { phase: 'smoke', command: 'cargo run -- --fixture tests/fixtures/latest_release.json --repo owner/repo --current v1.0.0' },
          ],
        },
      ],
    },
  ],
  events: [
    { id: 'release-sample-1', ts: '00:00:01', phase: 'draft', text: 'PRD captured HTTP, parsing, output, and offline-test requirements.', kind: 'success' },
    { id: 'release-sample-2', ts: '00:00:15', phase: 'planning', text: 'Planner split API work into T2 and verification into T1 tasks.', kind: 'success' },
  ],
};

const FUNDING_SAMPLE_STATE: PipelineDemoState = {
  source: 'sample',
  phase: 'tasks',
  headline: 'Stage job task plan',
  example: PIPELINE_EXAMPLES[2],
  currentCommand: 'roko prd plan btc-funding-alert-cli',
  lastUpdated: 'sample',
  prd: {
    slug: 'btc-funding-alert-cli',
    title: 'BTC Funding Alert CLI',
    status: 'planned',
    path: '.roko/prd/published/btc-funding-alert-cli.md',
    excerpt:
      'Build a CLI that fetches BTC funding rates from Hyperliquid, detects when BTC funding flips negative, and sends an email alert with dry-run and offline-test support.',
    requirements: [
      'REQ-001: Fetch BTC funding data from Hyperliquid through a testable client.',
      'REQ-002: Detect positive-to-negative funding flips using persisted prior state.',
      'REQ-003: Send email alerts through environment-configured SMTP settings.',
      'REQ-004: Provide dry-run output for stage demos without sending email.',
      'REQ-005: Verify with offline fixtures, cargo test, clippy, and smoke gates.',
    ],
    acceptance: [
      'Fixture data with a negative flip triggers one alert.',
      'Dry-run mode prints the email subject and funding values without SMTP.',
      'Missing SMTP env vars fail with a clear error outside dry-run mode.',
      'cargo test and cargo clippy pass.',
    ],
  },
  plans: [
    {
      id: 'btc-funding-alert-cli',
      title: 'BTC Funding Alert CLI',
      path: '.roko/plans/btc-funding-alert-cli',
      status: 'active',
      estimatedMinutes: 36,
      excerpt:
        'Split the stage job into DeFi data ingestion, flip detection, email integration, orchestration, and verification gates.',
      tasks: [
        {
          id: 'T1',
          title: 'Define CLI contract and dry-run config',
          description: 'Add flags for symbol, state file, dry-run, and SMTP environment validation.',
          status: 'done',
          routeTier: 'T1',
          tier: 'T1 mechanical',
          role: 'implementer',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 55,
          files: ['src/main.rs', 'Cargo.toml'],
          dependsOn: [],
          verify: [{ phase: 'compile', command: 'cargo check' }],
        },
        {
          id: 'T2',
          title: 'Implement Hyperliquid funding client',
          description: 'Fetch BTC perpetual funding data and normalize the response into a small domain type.',
          status: 'active',
          routeTier: 'T2',
          tier: 'T2 implementation',
          role: 'defi api implementer',
          modelHint: 'claude-sonnet-4-6',
          maxLoc: 95,
          files: ['src/hyperliquid.rs', 'src/funding.rs', 'Cargo.toml'],
          dependsOn: ['T1'],
          verify: [
            { phase: 'test', command: 'cargo test hyperliquid_fixture_parses_btc_funding' },
            { phase: 'compile', command: 'cargo check' },
          ],
        },
        {
          id: 'T3',
          title: 'Detect negative funding flips',
          description: 'Compare current funding against persisted previous funding and emit a single flip event.',
          status: 'pending',
          routeTier: 'T2',
          tier: 'T2 implementation',
          role: 'domain implementer',
          modelHint: 'claude-sonnet-4-6',
          maxLoc: 80,
          files: ['src/funding.rs', 'src/state.rs'],
          dependsOn: ['T2'],
          verify: [{ phase: 'test', command: 'cargo test detects_positive_to_negative_flip' }],
        },
        {
          id: 'T4',
          title: 'Wire email notifier integration',
          description: 'Send one concise funding alert through SMTP while keeping dry-run and unit tests offline.',
          status: 'pending',
          routeTier: 'T3',
          tier: 'T3 integration risk',
          role: 'integration implementer',
          modelHint: 'claude-opus-4-1',
          maxLoc: 120,
          files: ['src/email.rs', 'src/main.rs', 'Cargo.toml'],
          dependsOn: ['T1', 'T3'],
          verify: [
            { phase: 'test', command: 'cargo test email_dry_run_renders_subject' },
            { phase: 'test', command: 'cargo test missing_smtp_env_is_actionable' },
          ],
        },
        {
          id: 'T5',
          title: 'Compose end-to-end CLI flow',
          description: 'Connect fetch, flip detection, state persistence, notifier, and exit behavior.',
          status: 'pending',
          routeTier: 'T2',
          tier: 'T2 orchestration',
          role: 'implementer',
          modelHint: 'claude-sonnet-4-6',
          maxLoc: 90,
          files: ['src/main.rs', 'src/state.rs'],
          dependsOn: ['T2', 'T3', 'T4'],
          verify: [{ phase: 'smoke', command: 'cargo run -- --fixture tests/fixtures/hyperliquid_funding.json --dry-run' }],
        },
        {
          id: 'T6',
          title: 'Run full gates and capture demo receipt',
          description: 'Run compile, unit, lint, and dry-run gates so the stage UI can show defensible verification.',
          status: 'pending',
          routeTier: 'T1',
          tier: 'T1 verification',
          role: 'verifier',
          modelHint: 'claude-haiku-4-5',
          maxLoc: 20,
          files: ['tests/fixtures/hyperliquid_funding.json', 'src/main.rs'],
          dependsOn: ['T1', 'T2', 'T3', 'T4', 'T5'],
          verify: [
            { phase: 'compile', command: 'cargo check' },
            { phase: 'test', command: 'cargo test' },
            { phase: 'clippy', command: 'cargo clippy --all-targets -- -D warnings' },
            { phase: 'smoke', command: 'cargo run -- --fixture tests/fixtures/hyperliquid_funding.json --dry-run' },
          ],
        },
      ],
    },
  ],
  events: [
    { id: 'funding-sample-1', ts: '00:00:01', phase: 'draft', text: 'PRD captured DeFi data, email settings, dry-run, and offline verification.', kind: 'success' },
    { id: 'funding-sample-2', ts: '00:00:18', phase: 'planning', text: 'Planner generated a mixed T1/T2/T3 task graph for the stage job.', kind: 'success' },
  ],
};

export const PIPELINE_SAMPLE_STATES: Record<PipelineExampleId, PipelineDemoState> = {
  'simple-status': SIMPLE_SAMPLE_STATE,
  'release-watch': RELEASE_SAMPLE_STATE,
  'funding-alert': FUNDING_SAMPLE_STATE,
};

export function getPipelineSampleState(id?: string): PipelineDemoState {
  const example = getPipelineExample(id);
  return PIPELINE_SAMPLE_STATES[example.id];
}

export const PIPELINE_SAMPLE_STATE = SIMPLE_SAMPLE_STATE;
