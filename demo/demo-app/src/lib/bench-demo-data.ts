import type {
  BenchModel,
  BenchRun,
  BenchRunSummary,
  BenchSuite,
  BenchTaskResult,
  ParetoFrontierResponse,
} from './bench-types';

export type BenchRunComparisonFallback = {
  runs: BenchRun[];
};

export type BenchDemoFallback =
  | BenchSuite[]
  | BenchModel[]
  | BenchRun[]
  | BenchRun
  | BenchRunComparisonFallback
  | ParetoFrontierResponse
  | undefined;

export const DEMO_BENCH_MODELS: BenchModel[] = [
  { id: 'claude-haiku-3-20250414', name: 'Claude Haiku 3', provider: 'Anthropic', cost_per_1k_input: 0.00025, cost_per_1k_output: 0.00125, max_tokens: 4096, context_window: 200000 },
  { id: 'claude-sonnet-4-20250514', name: 'Claude Sonnet 4', provider: 'Anthropic', cost_per_1k_input: 0.003, cost_per_1k_output: 0.015, max_tokens: 8192, context_window: 200000 },
  { id: 'gpt-4o-mini', name: 'GPT-4o Mini', provider: 'OpenAI', cost_per_1k_input: 0.00015, cost_per_1k_output: 0.0006, max_tokens: 4096, context_window: 128000 },
  { id: 'llama3.1-8b', name: 'Llama 3.1 8B (Cerebras)', provider: 'Cerebras', cost_per_1k_input: 0.0001, cost_per_1k_output: 0.0001, max_tokens: 8192, context_window: 128000 },
];

export const DEMO_BENCH_SUITES: BenchSuite[] = [
  {
    id: 'smoke',
    name: 'Smoke',
    description: 'Quick validation suite with lightweight tasks.',
    estimated_cost_usd: 0.05,
    difficulty_range: [1, 2],
    tasks: [
      { id: 'smoke-1', name: 'Hello World CLI', prompt: 'Create a Rust CLI that prints hello world.', difficulty: 1, tags: ['rust', 'cli'] },
      { id: 'smoke-2', name: 'Add function', prompt: 'Write a function that adds two numbers.', difficulty: 1, tags: ['rust', 'basic'] },
      { id: 'smoke-3', name: 'JSON parse', prompt: 'Parse a JSON config file into a struct.', difficulty: 2, tags: ['rust', 'serde'] },
    ],
  },
  {
    id: 'learnable-rust',
    name: 'Learnable Rust',
    description: 'Offline self-learning demo suite tuned for low-cost runs.',
    estimated_cost_usd: 0.08,
    difficulty_range: [2, 4],
    tasks: [
      { id: 'lr-1', name: 'Model catalog', prompt: 'Add a selectable model to a Rust bench catalog.', difficulty: 2, tags: ['rust', 'catalog'] },
      { id: 'lr-2', name: 'Cost estimator', prompt: 'Extend a Rust cost estimator with a low-cost provider branch.', difficulty: 3, tags: ['rust', 'pricing'] },
      { id: 'lr-3', name: 'Truthful fallback', prompt: 'Keep offline fallback data labeled as demo data.', difficulty: 2, tags: ['rust', 'truth'] },
      { id: 'lr-4', name: 'Run summary', prompt: 'Summarize pass rate, cost, and duration for a completed benchmark run.', difficulty: 4, tags: ['rust', 'metrics'] },
    ],
  },
  {
    id: 'roko-bench',
    name: 'Roko Bench',
    description: 'Self-hosting benchmark tasks roko can run against itself.',
    estimated_cost_usd: 0.35,
    difficulty_range: [2, 4],
    tasks: [
      { id: 'rb-1', name: 'Wire gate pipeline', prompt: 'Connect compile and test gates to the orchestrator.', difficulty: 3, tags: ['rust', 'gates'] },
      { id: 'rb-2', name: 'Episode logger', prompt: 'Log agent turns to episode storage.', difficulty: 2, tags: ['rust', 'logging'] },
      { id: 'rb-3', name: 'Cascade router', prompt: 'Route tasks to an optimal model based on cost and quality.', difficulty: 4, tags: ['rust', 'routing'] },
      { id: 'rb-4', name: 'Safety contracts', prompt: 'Enforce role-based tool authorization.', difficulty: 3, tags: ['rust', 'safety'] },
    ],
  },
];

function makeResult(
  task: BenchSuite['tasks'][number],
  model: string,
  passed: boolean,
  index: number,
): BenchTaskResult {
  const modelRate = model.includes('llama') ? 0.0002 : model.includes('haiku') ? 0.003 : 0.012;
  const tokensIn = 800 + task.difficulty * 500 + index * 80;
  const tokensOut = 900 + task.difficulty * 650 + index * 95;
  const costUsd = Number((modelRate * task.difficulty * (1 + index * 0.08)).toFixed(4));

  return {
    task_id: task.id,
    task_name: task.name,
    status: passed ? 'pass' : 'fail',
    cost_usd: costUsd,
    tokens_in: tokensIn,
    tokens_out: tokensOut,
    duration_ms: 2200 + task.difficulty * 1400 + index * 350,
    model,
    gate_verdicts: [
      { gate: 'compile', passed: true, duration_ms: 700 + index * 40 },
      { gate: 'test', passed, duration_ms: 1100 + index * 60 },
      { gate: 'clippy', passed: passed || task.difficulty < 3, duration_ms: 500 + index * 30 },
    ],
    error: passed ? undefined : 'Gate test failed: one assertion failed',
    retries_used: passed ? 0 : 1,
  };
}

function makeResults(suiteId: string, model: string, passCount: number): BenchTaskResult[] {
  const suite = DEMO_BENCH_SUITES.find((candidate) => candidate.id === suiteId);
  if (!suite) return [];

  return suite.tasks.map((task, index) => makeResult(task, model, index < passCount, index));
}

function summarize(results: BenchTaskResult[]): BenchRunSummary {
  const passed = results.filter((result) => result.status === 'pass').length;
  const failed = results.filter((result) => result.status === 'fail').length;
  const skipped = results.filter((result) => result.status === 'skipped').length;
  const totalCost = results.reduce((sum, result) => sum + result.cost_usd, 0);
  const totalTokens = results.reduce((sum, result) => sum + result.tokens_in + result.tokens_out, 0);
  const totalDuration = results.reduce((sum, result) => sum + result.duration_ms, 0);

  return {
    total_tasks: results.length,
    passed,
    failed,
    skipped,
    total_cost_usd: Number(totalCost.toFixed(4)),
    total_tokens: totalTokens,
    total_duration_ms: totalDuration,
    pass_rate: results.length > 0 ? passed / results.length : 0,
    cost_per_success_usd: passed > 0 ? Number((totalCost / passed).toFixed(4)) : 0,
    avg_duration_ms: results.length > 0 ? Math.round(totalDuration / results.length) : 0,
  };
}

const learnableRustResults = makeResults('learnable-rust', 'llama3.1-8b', 4);
const rokoBenchResults = makeResults('roko-bench', 'claude-sonnet-4-20250514', 3);
const smokeResults = makeResults('smoke', 'claude-haiku-3-20250414', 3);

export const DEMO_BENCH_RUNS: BenchRun[] = [
  {
    id: 'br-003',
    kind: 'suite',
    config: {
      model: 'llama3.1-8b',
      provider: 'Cerebras',
      temperature: 0,
      timeout_secs: 90,
      strategy: 'full_cascade',
      retries: 1,
      gates: { compile: true, test: true, clippy: true, diff: false },
    },
    suite_id: 'learnable-rust',
    suite_name: 'Learnable Rust',
    status: 'completed',
    results: learnableRustResults,
    summary: summarize(learnableRustResults),
    started_at: '2026-04-27T13:00:00Z',
    finished_at: '2026-04-27T13:09:42Z',
  },
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
    results: rokoBenchResults,
    summary: summarize(rokoBenchResults),
    started_at: '2026-04-27T14:00:00Z',
    finished_at: '2026-04-27T14:12:34Z',
  },
  {
    id: 'br-002',
    kind: 'suite',
    config: {
      model: 'claude-haiku-3-20250414',
      provider: 'Anthropic',
      temperature: 0,
      timeout_secs: 60,
      strategy: 'minimal',
      retries: 0,
      gates: { compile: true, test: true, clippy: false, diff: false },
    },
    suite_id: 'smoke',
    suite_name: 'Smoke',
    status: 'completed',
    results: smokeResults,
    summary: summarize(smokeResults),
    started_at: '2026-04-27T15:30:00Z',
    finished_at: '2026-04-27T15:32:10Z',
  },
];

export const DEMO_BENCH_PARETO: ParetoFrontierResponse = {
  points: DEMO_BENCH_RUNS
    .filter((run): run is BenchRun & { summary: BenchRunSummary } => Boolean(run.summary))
    .map((run) => ({
      run_id: run.id,
      model: run.config.model,
      suite_id: run.suite_id,
      pass_rate: run.summary.pass_rate,
      cost_usd: run.summary.total_cost_usd,
      duration_ms: run.summary.total_duration_ms,
    })),
};

export function getDemoBenchRun(runId: string): BenchRun | undefined {
  return DEMO_BENCH_RUNS.find((run) => run.id === runId);
}

export function getDemoBenchRunComparison(runIds: string[]): BenchRunComparisonFallback {
  const runs = runIds
    .map((runId) => getDemoBenchRun(runId))
    .filter((run): run is BenchRun => Boolean(run));

  return { runs };
}

export function getDemoBenchFallback(path: string): BenchDemoFallback {
  let url: URL;
  try {
    url = new URL(path, 'http://demo.local');
  } catch {
    return undefined;
  }

  const { pathname, searchParams } = url;
  if (pathname === '/api/bench/suites') return DEMO_BENCH_SUITES;
  if (pathname === '/api/bench/models') return DEMO_BENCH_MODELS;
  if (pathname === '/api/bench/runs') return DEMO_BENCH_RUNS;
  if (pathname === '/api/bench/pareto') return DEMO_BENCH_PARETO;

  if (pathname === '/api/bench/runs/compare') {
    const runIds = (searchParams.get('ids') ?? '')
      .split(',')
      .map((runId) => runId.trim())
      .filter(Boolean);

    return getDemoBenchRunComparison(runIds);
  }

  const runDetailMatch = pathname.match(/^\/api\/bench\/runs\/([^/]+)$/);
  if (runDetailMatch) return getDemoBenchRun(decodeURIComponent(runDetailMatch[1]));

  const exportMatch = pathname.match(/^\/api\/bench\/export\/([^/]+)$/);
  if (exportMatch) return getDemoBenchRun(decodeURIComponent(exportMatch[1]));

  return undefined;
}
