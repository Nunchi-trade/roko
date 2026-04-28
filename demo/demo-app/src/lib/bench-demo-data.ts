/**
 * Offline fallback data for the bench system.
 * Used when roko serve is unreachable.
 */

import type {
  BenchSuite,
  BenchModel,
  BenchRun,
  BenchTaskResult,
  BenchRunSummary,
} from './bench-types';

// ── Models ──

export const DEMO_BENCH_MODELS: BenchModel[] = [
  { id: 'claude-haiku-3-20250414', name: 'Claude Haiku 3', provider: 'Anthropic', cost_per_1k_input: 0.00025, cost_per_1k_output: 0.00125, max_tokens: 4096, context_window: 200000 },
  { id: 'claude-sonnet-4-20250514', name: 'Claude Sonnet 4', provider: 'Anthropic', cost_per_1k_input: 0.003, cost_per_1k_output: 0.015, max_tokens: 8192, context_window: 200000 },
  { id: 'claude-opus-4-20250414', name: 'Claude Opus 4', provider: 'Anthropic', cost_per_1k_input: 0.015, cost_per_1k_output: 0.075, max_tokens: 4096, context_window: 200000 },
  { id: 'gpt-4o', name: 'GPT-4o', provider: 'OpenAI', cost_per_1k_input: 0.005, cost_per_1k_output: 0.015, max_tokens: 4096, context_window: 128000 },
  { id: 'gpt-4o-mini', name: 'GPT-4o Mini', provider: 'OpenAI', cost_per_1k_input: 0.00015, cost_per_1k_output: 0.0006, max_tokens: 4096, context_window: 128000 },
  { id: 'o3-mini', name: 'o3-mini', provider: 'OpenAI', cost_per_1k_input: 0.0011, cost_per_1k_output: 0.0044, max_tokens: 16384, context_window: 200000 },
  { id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', provider: 'Google', cost_per_1k_input: 0.00125, cost_per_1k_output: 0.01, max_tokens: 8192, context_window: 1000000 },
];

// ── Suites ──

export const DEMO_BENCH_SUITES: BenchSuite[] = [
  {
    id: 'smoke',
    name: 'Smoke',
    description: 'Quick validation suite with 5 lightweight tasks',
    estimated_cost_usd: 0.05,
    difficulty_range: [1, 2],
    tasks: [
      { id: 'smoke-1', name: 'Hello World CLI', prompt: 'Create a Rust CLI that prints hello world', difficulty: 1, tags: ['rust', 'cli'] },
      { id: 'smoke-2', name: 'Add function', prompt: 'Write a function that adds two numbers', difficulty: 1, tags: ['rust', 'basic'] },
      { id: 'smoke-3', name: 'File reader', prompt: 'Read a file and count lines', difficulty: 1, tags: ['rust', 'io'] },
      { id: 'smoke-4', name: 'JSON parse', prompt: 'Parse a JSON config file into a struct', difficulty: 2, tags: ['rust', 'serde'] },
      { id: 'smoke-5', name: 'HTTP health check', prompt: 'Create a health endpoint returning JSON', difficulty: 2, tags: ['rust', 'http'] },
    ],
  },
  {
    id: 'roko-bench',
    name: 'Roko Bench',
    description: 'Self-hosting benchmark: tasks roko actually runs',
    estimated_cost_usd: 0.35,
    difficulty_range: [2, 4],
    tasks: [
      { id: 'rb-1', name: 'Wire gate pipeline', prompt: 'Connect compile+test gates to the orchestrator', difficulty: 3, tags: ['rust', 'gates'] },
      { id: 'rb-2', name: 'Episode logger', prompt: 'Log agent turns to .roko/episodes.jsonl', difficulty: 2, tags: ['rust', 'logging'] },
      { id: 'rb-3', name: 'Cascade router', prompt: 'Route tasks to optimal model based on cost/quality', difficulty: 4, tags: ['rust', 'routing'] },
      { id: 'rb-4', name: 'Prompt assembly', prompt: 'Build 9-layer system prompt from role template', difficulty: 3, tags: ['rust', 'compose'] },
      { id: 'rb-5', name: 'Plan revision', prompt: 'Replan failed gate tasks with enriched context', difficulty: 4, tags: ['rust', 'orchestration'] },
      { id: 'rb-6', name: 'MCP passthrough', prompt: 'Pass MCP config from roko.toml to agent subprocess', difficulty: 2, tags: ['rust', 'config'] },
      { id: 'rb-7', name: 'TUI dashboard', prompt: 'Wire ratatui dashboard with live metrics', difficulty: 3, tags: ['rust', 'tui'] },
      { id: 'rb-8', name: 'Safety contracts', prompt: 'Enforce role-based tool authorization', difficulty: 3, tags: ['rust', 'safety'] },
    ],
  },
  {
    id: 'codegen',
    name: 'Codegen',
    description: 'Code generation tasks across multiple languages',
    estimated_cost_usd: 0.50,
    difficulty_range: [2, 5],
    tasks: [
      { id: 'cg-1', name: 'REST API', prompt: 'Build a REST API with CRUD endpoints', difficulty: 3, tags: ['rust', 'api'] },
      { id: 'cg-2', name: 'CLI tool', prompt: 'Build a file deduplication CLI tool', difficulty: 3, tags: ['rust', 'cli'] },
      { id: 'cg-3', name: 'Parser', prompt: 'Write a markdown to HTML converter', difficulty: 4, tags: ['rust', 'parser'] },
      { id: 'cg-4', name: 'Async worker', prompt: 'Build a task queue with async workers', difficulty: 4, tags: ['rust', 'async'] },
      { id: 'cg-5', name: 'Graph algorithm', prompt: 'Implement topological sort for a DAG', difficulty: 3, tags: ['rust', 'algorithm'] },
      { id: 'cg-6', name: 'Type system', prompt: 'Build a basic type checker for a toy language', difficulty: 5, tags: ['rust', 'compiler'] },
      { id: 'cg-7', name: 'LSP server', prompt: 'Create a basic LSP server with hover support', difficulty: 5, tags: ['rust', 'lsp'] },
      { id: 'cg-8', name: 'Database layer', prompt: 'Build an SQLite abstraction layer with migrations', difficulty: 4, tags: ['rust', 'database'] },
      { id: 'cg-9', name: 'WebSocket server', prompt: 'Build a chat server with WebSocket support', difficulty: 3, tags: ['rust', 'ws'] },
      { id: 'cg-10', name: 'Config DSL', prompt: 'Create a TOML-like config DSL parser', difficulty: 4, tags: ['rust', 'parser'] },
    ],
  },
];

// ── Sample completed runs ──

function makeResults(suiteId: string, model: string, passRate: number): BenchTaskResult[] {
  const suite = DEMO_BENCH_SUITES.find((s) => s.id === suiteId);
  if (!suite) return [];
  return suite.tasks.map((task, i) => {
    const passed = i / suite.tasks.length < passRate;
    const baseCost = model.includes('haiku') ? 0.003 : model.includes('sonnet') ? 0.012 : model.includes('opus') ? 0.04 : 0.01;
    const cost = baseCost * (0.8 + Math.random() * 0.4) * task.difficulty;
    const tokens = Math.round(800 + task.difficulty * 600 + Math.random() * 400);
    return {
      task_id: task.id,
      task_name: task.name,
      status: passed ? 'pass' as const : 'fail' as const,
      cost_usd: Math.round(cost * 1000) / 1000,
      tokens_in: tokens,
      tokens_out: Math.round(tokens * 1.5),
      duration_ms: Math.round((2000 + task.difficulty * 1500 + Math.random() * 2000)),
      model,
      gate_verdicts: [
        { gate: 'compile', passed: true, duration_ms: 800 },
        { gate: 'test', passed, duration_ms: 1200 },
        ...(passed ? [{ gate: 'clippy', passed: true, duration_ms: 600 }] : []),
      ],
      error: passed ? undefined : 'Gate test failed: 1 test assertion failed',
      retries_used: passed ? 0 : 1,
    };
  });
}

function summarize(results: BenchTaskResult[]): BenchRunSummary {
  const passed = results.filter((r) => r.status === 'pass').length;
  const failed = results.filter((r) => r.status === 'fail').length;
  const skipped = results.filter((r) => r.status === 'skipped').length;
  const totalCost = results.reduce((s, r) => s + r.cost_usd, 0);
  const totalTokens = results.reduce((s, r) => s + r.tokens_in + r.tokens_out, 0);
  const totalDuration = results.reduce((s, r) => s + r.duration_ms, 0);
  return {
    total_tasks: results.length,
    passed,
    failed,
    skipped,
    total_cost_usd: Math.round(totalCost * 1000) / 1000,
    total_tokens: totalTokens,
    total_duration_ms: totalDuration,
    pass_rate: passed / results.length,
    cost_per_success_usd: passed > 0 ? Math.round((totalCost / passed) * 1000) / 1000 : 0,
    avg_duration_ms: Math.round(totalDuration / results.length),
  };
}

const run1Results = makeResults('roko-bench', 'claude-sonnet-4-20250514', 0.875);
const run2Results = makeResults('smoke', 'claude-haiku-3-20250414', 1.0);

export const DEMO_BENCH_RUNS: BenchRun[] = [
  {
    id: 'br-001',
    kind: 'suite',
    config: {
      model: 'claude-sonnet-4-20250514',
      provider: 'Anthropic',
      temperature: 0.1,
      timeout_secs: 120,
      strategy: 'full_cascade',
      retries: 1,
      gates: { compile: true, test: true, clippy: true, diff: false },
    },
    suite_id: 'roko-bench',
    suite_name: 'Roko Bench',
    status: 'completed',
    results: run1Results,
    summary: summarize(run1Results),
    started_at: '2026-04-27T14:00:00Z',
    finished_at: '2026-04-27T14:12:34Z',
  },
  {
    id: 'br-002',
    kind: 'suite',
    config: {
      model: 'claude-haiku-3-20250414',
      provider: 'Anthropic',
      temperature: 0.0,
      timeout_secs: 60,
      strategy: 'minimal',
      retries: 0,
      gates: { compile: true, test: true, clippy: false, diff: false },
    },
    suite_id: 'smoke',
    suite_name: 'Smoke',
    status: 'completed',
    results: run2Results,
    summary: summarize(run2Results),
    started_at: '2026-04-27T15:30:00Z',
    finished_at: '2026-04-27T15:32:10Z',
  },
];
