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

export const costScenario: ClickableScenario = {
  id: 'cost',
  title: 'Cost',
  subtitle: 'Same task, same model class. Cascade routing is the variable.',
  panes: 2,
  labels: ['naive', 'cascade'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['Naive baseline', 'Cascade routing', 'Cost delta'],
  durationHint: '<2 min',
  accent: 'teal',
  icon: 'race',
  steps: [
    { label: 'Baseline', sublabel: '--no-cascade' },
    { label: 'Cascade', sublabel: 'routed execution' },
    { label: 'Compare', sublabel: 'cost, tokens, time' },
  ],
  commands: COST_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'naive' && commandId !== 'cascade') return { ok: false, error: 'Unknown command' };

    const entry = commandId === 'naive' ? ctx.entries[0] : ctx.entries[1];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

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

    if (result.cost) ctx.setMetric(commandId === 'naive' ? 'cost-left' : 'cost-right', result.cost);
    if (result.tokens) ctx.setMetric(commandId === 'naive' ? 'tokens-left' : 'tokens-right', result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
