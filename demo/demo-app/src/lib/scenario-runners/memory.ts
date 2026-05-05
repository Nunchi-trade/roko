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

export const memoryScenario: ClickableScenario = {
  id: 'memory',
  title: 'Memory',
  subtitle: 'Second run inherits useful knowledge from the first run.',
  panes: 2,
  labels: ['cold run', 'warm run'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Knowledge ingestion', 'Knowledge reuse', 'Delta comparison'],
  durationHint: '<2 min',
  accent: 'emerald',
  icon: 'knowledge',
  steps: [
    { label: 'Cold run', sublabel: 'CSV to JSON' },
    { label: 'Ingest', sublabel: 'write knowledge' },
    { label: 'Warm run', sublabel: 'TOML to JSON' },
    { label: 'Delta', sublabel: 'faster and cheaper' },
  ],
  commands: MEMORY_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'cold' && commandId !== 'warm') return { ok: false, error: 'Unknown command' };

    const entry = commandId === 'cold' ? ctx.entries[0] : ctx.entries[1];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    const task = commandId === 'cold' ? COLD_TASK : WARM_TASK;
    const result = await showCmd(entry, roko(ctx, `do "${task}"`), {
      timeout: 300000,
      customDesc: commandId === 'cold'
        ? 'Cold run that should generate reusable knowledge.'
        : 'Warm run that should consume knowledge from the cold run.',
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    if (result.cost) ctx.setMetric(commandId === 'cold' ? 'cold-cost' : 'warm-cost', result.cost);
    if (result.tokens) ctx.setMetric(commandId === 'cold' ? 'cold-tokens' : 'warm-tokens', result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
