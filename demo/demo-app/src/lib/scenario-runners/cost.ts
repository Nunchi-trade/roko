// --- src/lib/scenario-runners/cost.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

const PRIME_TASK = 'build a function that checks if a number is prime';

export const COST_COMMANDS: CommandDef[] = [
  {
    id: 'naive',
    command: `roko do "${PRIME_TASK}" --no-cascade`,
    description: 'Run the baseline without cascade routing',
    timeout: 180000,
    target: { pane: 0 },
  },
  {
    id: 'cascade',
    command: `roko do "${PRIME_TASK}"`,
    description: 'Run the cascade-routed version',
    timeout: 180000,
    target: { pane: 1 },
  },
];

// ANSI escape code stripping
function stripAnsi(s: string): string {
  // eslint-disable-next-line no-control-regex
  return s.replace(/\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])/g, '');
}

/**
 * Poll terminal output buffer for cost/token metrics in real time.
 * Fires ctx.setMetric for intermediate detections before showCmd returns.
 */
function pollCostMetrics(
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

    // Model detection (real-time)
    const modelMatch = text.match(/\bmodel\s+([a-zA-Z0-9][\w./-]*)/i);
    if (modelMatch) emit('model', modelMatch[1]);

    // Cost detection
    const costMatch = text.match(/\$(\d+\.\d+)/) || text.match(/\bcost\s+(\d+\.\d+)/i);
    if (costMatch) {
      emit(`${prefix}-cost`, `$${costMatch[1]}`);
      emit('cost', `$${costMatch[1]}`);
    }

    // Token detection
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

export const costScenario: ClickableScenario = {
  id: 'cost',
  title: 'Cost Comparison',
  subtitle: 'Two agents solve the same task — one with a fixed model, one with smart routing. Compare the cost.',
  panes: 2,
  labels: ['Direct (single model)', 'Cascade (smart routing)'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['Fixed-model baseline', 'Cascade routing (start cheap, escalate if needed)', 'Side-by-side cost comparison'],
  durationHint: '<2 min',
  accent: 'teal',
  icon: 'race',
  steps: [
    { label: 'Direct run', sublabel: 'single expensive model' },
    { label: 'Cascade run', sublabel: 'start cheap, escalate if needed' },
    { label: 'Compare', sublabel: 'cost, tokens, and time' },
  ],
  commands: COST_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'naive' && commandId !== 'cascade') return { ok: false, error: 'Unknown command' };

    const entry = commandId === 'naive' ? ctx.entries[0] : ctx.entries[1];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    const prefix = commandId === 'naive' ? 'naive' : 'cascade';

    // Start real-time metric polling
    const stopPolling = pollCostMetrics(entry, prefix, ctx, ctx.signal);

    const subcommand = commandId === 'naive'
      ? `do "${PRIME_TASK}" --no-cascade`
      : `do "${PRIME_TASK}"`;
    const result = await showCmd(entry, roko(ctx, subcommand), {
      timeout: 180000,
      customDesc: commandId === 'naive'
        ? 'Baseline execution without cascade routing.'
        : 'Cascade-routed execution for the same task.',
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    stopPolling();

    // Final metrics from showCmd result (authoritative)
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
