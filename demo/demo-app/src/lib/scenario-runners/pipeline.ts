// --- src/lib/scenario-runners/pipeline.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

const PIPELINE_TASK = 'Build a Rust CLI that converts temperatures between Celsius and Fahrenheit';

export const PIPELINE_COMMANDS: CommandDef[] = [
  {
    id: 'run',
    command: `roko do "${PIPELINE_TASK}"`,
    description: 'Run idea-to-code pipeline',
    timeout: 300000,
    target: { pane: 0 },
  },
];

export const pipelineScenario: ClickableScenario = {
  id: 'pipeline',
  title: 'Pipeline',
  subtitle: 'One command takes an idea to working, validated code.',
  panes: 1,
  labels: ['pipeline'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['Classify', 'Plan', 'Execute', 'Gate'],
  durationHint: '<2 min',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Classify', sublabel: 'scope the request' },
    { label: 'Plan', sublabel: 'create tasks' },
    { label: 'Execute', sublabel: 'write code' },
    { label: 'Gate', sublabel: 'validate result' },
    { label: 'Done', sublabel: 'summarize outcome' },
  ],
  commands: PIPELINE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    if (commandId !== 'run') return { ok: false, error: 'Unknown command' };
    const [entry] = ctx.entries;
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    const result = await showCmd(entry, roko(ctx, `do "${PIPELINE_TASK}"`), {
      timeout: 300000,
      customDesc: 'Runs the redesigned one-command idea-to-code pipeline.',
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    if (result.cost) ctx.setMetric('cost', result.cost);
    if (result.tokens) ctx.setMetric('tokens', result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
