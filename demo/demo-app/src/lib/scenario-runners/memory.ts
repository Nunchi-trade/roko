// --- src/lib/scenario-runners/memory.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

const COLD_TASK = 'Build a Rust CLI that parses CSV and outputs JSON';
const WARM_TASK = 'Build a Rust CLI that parses TOML and outputs JSON';

export const MEMORY_COMMANDS: CommandDef[] = [
  {
    id: 'cold',
    command: `roko do "${COLD_TASK}"`,
    description: 'Cold run writes reusable knowledge',
    timeout: 300000,
    target: { pane: 0 },
  },
  {
    id: 'warm',
    command: `roko do "${WARM_TASK}"`,
    description: 'Warm run consumes prior knowledge',
    timeout: 300000,
    target: { pane: 1 },
  },
];

// ANSI escape code stripping
function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, '');
}

function pollMetrics(
  entry: { outputBuffer: string },
  prefix: string,
  ctx: ScenarioContext,
  signal?: AbortSignal,
): () => void {
  const seen = new Set<string>();
  const emit = (key: string, value: string) => {
    const tag = `${key}:${value}`;
    if (seen.has(tag)) return;
    seen.add(tag);
    ctx.setMetric(key, value);
  };

  const interval = setInterval(() => {
    const text = stripAnsi(entry.outputBuffer);
    const modelMatch = text.match(/\bmodel\s+([a-zA-Z0-9][\w./-]*)/i);
    if (modelMatch) emit('model', modelMatch[1]);
    const costMatch = text.match(/\$(\d+\.\d+)/) || text.match(/\bcost\s+(\d+\.\d+)/i);
    if (costMatch) {
      emit(`${prefix}-cost`, `$${costMatch[1]}`);
      emit('cost', `$${costMatch[1]}`);
    }
    const tokenMatch = text.match(/(\d[\d,]*)\s*(?:tokens?|tok)\b/i) || text.match(/\btokens?\s+(\d[\d,]*)/i);
    if (tokenMatch) {
      emit(`${prefix}-tokens`, tokenMatch[1]);
      emit('tokens', tokenMatch[1]);
    }
  }, 500);

  const cleanup = () => clearInterval(interval);
  signal?.addEventListener('abort', cleanup, { once: true });
  return cleanup;
}

export const memoryScenario: ClickableScenario = {
  id: 'memory',
  title: 'Memory',
  subtitle: 'The first agent learns from scratch. The second starts with that knowledge — solving a similar task faster and cheaper.',
  panes: 2,
  labels: ['First run (no prior knowledge)', 'Second run (uses learned knowledge)'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Solve from scratch', 'Save what was learned', 'Reuse knowledge on a similar task'],
  durationHint: '<2 min',
  accent: 'emerald',
  icon: 'knowledge',
  steps: [
    { label: 'First run', sublabel: 'solve CSV → JSON from scratch' },
    { label: 'Save knowledge', sublabel: 'persist learnings' },
    { label: 'Second run', sublabel: 'solve TOML → JSON with prior knowledge' },
    { label: 'Compare', sublabel: 'measure cost and speed difference' },
  ],
  commands: MEMORY_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'cold' && commandId !== 'warm') return { ok: false, error: 'Unknown command' };

    const entry = commandId === 'cold' ? ctx.entries[0] : ctx.entries[1];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    const prefix = commandId; // 'cold' or 'warm'
    const stopPolling = pollMetrics(entry, prefix, ctx, ctx.signal);

    const task = commandId === 'cold' ? COLD_TASK : WARM_TASK;
    const result = await showCmd(entry, roko(ctx, `do "${task}"`), {
      timeout: 300000,
      customDesc: commandId === 'cold'
        ? 'Cold run that should generate reusable knowledge.'
        : 'Warm run that should consume knowledge from the cold run.',
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    stopPolling();

    if (result.cost) ctx.setMetric(`${prefix}-cost`, result.cost);
    if (result.tokens) ctx.setMetric(`${prefix}-tokens`, result.tokens);
    ctx.setMetric(`${prefix}-elapsed`, String(result.elapsed ?? 0));
    ctx.setMetric(`${prefix}-calls`, '1');

    // Feed sidebar stats (model/cost/tokens/time) for provenance
    if (result.model) ctx.setMetric('model', result.model);
    if (result.cost) ctx.setMetric('cost', result.cost);
    if (result.tokens) ctx.setMetric('tokens', result.tokens);
    ctx.setMetric('time', `${(result.elapsed ?? 0).toFixed(1)}s`);

    return { ok: result.ok, error: result.error };
  },
};
