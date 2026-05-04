// --- src/lib/scenario-runners/race.ts ---
import type { ClickableScenario, CommandDef, CommandTarget, ScenarioContext } from '../scenarios';
import { showCmd, roko, trackMetrics } from '../terminal-session';

export const RACE_COMMANDS: CommandDef[] = [
  { id: 'naive',   command: 'roko run "Build a CLI calculator in Rust" --no-replan', description: 'Run naive (no replan)',      timeout: 180000, target: { pane: 0 } },
  { id: 'cascade', command: 'roko run "Build a CLI calculator in Rust"',             description: 'Run cascade (full pipeline)', timeout: 180000, target: { pane: 1 } },
  { id: 'compare', command: 'roko learn efficiency',                                  description: 'Compare efficiency',         timeout: 30000,  target: 'all' as CommandTarget },
];

export const race: ClickableScenario = {
  id: 'race',
  title: 'Cost Race',
  subtitle: 'Same task, two approaches. Left: naive single-model. Right: cascade-routed.',
  panes: 2,
  labels: ['naive (no replan)', 'cascade (full pipeline)'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['Naive vs cascade routing', 'Cost comparison', 'Side-by-side'],
  durationHint: '~60s',
  accent: 'teal',
  icon: 'race',
  steps: [
    { label: 'Naive run', sublabel: '--no-replan' },
    { label: 'Cascade run', sublabel: 'full pipeline' },
  ],
  commands: RACE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, setMetric } = ctx;
    const [left, right] = entries;

    if (commandId === 'naive') {
      if (!left) return { ok: false, error: 'No left pane connected' };
      const tracker = trackMetrics(left, {
        onCost: c => setMetric('cost-left', c),
        onTokens: t => setMetric('tokens-left', t),
      });
      try {
        const result = await showCmd(left, roko(ctx, 'run "Build a CLI calculator in Rust" --no-replan'), {
          timeout: 180000,
          customDesc: 'Runs with --no-replan: uses a single model without cascade routing or gate-failure replanning. The baseline approach.',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      } finally {
        clearInterval(tracker);
      }
    }

    if (commandId === 'cascade') {
      if (!right) return { ok: false, error: 'No right pane connected' };
      const tracker = trackMetrics(right, {
        onCost: c => setMetric('cost-right', c),
        onTokens: t => setMetric('tokens-right', t),
      });
      try {
        const result = await showCmd(right, roko(ctx, 'run "Build a CLI calculator in Rust"'), {
          timeout: 180000,
          customDesc: 'Runs with full pipeline: cascade router picks optimal models per-turn, gates validate, and failures trigger automatic replanning.',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      } finally {
        clearInterval(tracker);
      }
    }

    if (commandId === 'compare') {
      const panes = [left, right].filter(Boolean);
      if (panes.length === 0) return { ok: false, error: 'No panes connected' };
      const results = await Promise.all(
        panes.map(pane =>
          showCmd(pane!, roko(ctx, 'learn efficiency'), {
            timeout: 30000,
            customDesc: 'Compare efficiency metrics between the two approaches.',
            workspaceDir: ctx.workspaceDir,
            signal: ctx.signal,
          }),
        ),
      );
      const failed = results.find(r => !r.ok);
      return failed ? { ok: false, error: failed.error } : { ok: true };
    }

    return { ok: false, error: 'Unknown command' };
  },
};
