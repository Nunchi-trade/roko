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
