// --- src/lib/scenario-runners/prd-research-loop.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../../scenarios';
import { showCmd, roko, trackMetrics } from '../../terminal-session';

// ── Static command definitions (display layer, no ctx needed) ─

export const RESEARCH_LOOP_COMMANDS: CommandDef[] = [
  { id: 'idea',       command: 'roko prd idea "Add config validation with schema checking and helpful error messages"', description: 'Capture raw work item into PRD backlog',            timeout: 45000  },
  { id: 'draft',      command: 'roko prd draft new cli-config-validation',                                              description: 'Agent expands idea into structured PRD',            timeout: 120000 },
  { id: 'research',   command: 'roko research enhance-prd cli-config-validation',                                       description: 'Research agent enriches PRD with prior art',        timeout: 180000 },
  { id: 'plan',       command: 'roko prd plan cli-config-validation',                                                   description: 'Generate implementation plan from research-enhanced PRD', timeout: 180000 },
  { id: 'run',        command: 'roko plan run .roko/plans --max-retries 1',                                             description: 'Execute plan: agents implement, gates validate',    timeout: 300000 },
  { id: 'learn-all',  command: 'roko learn all',                                                                        description: 'Full learning state: router, experiments, thresholds', timeout: 30000 },
  { id: 'learn-tune', command: 'roko learn tune routing',                                                               description: 'Cascade router tuning: model confidence scores',    timeout: 30000  },
  { id: 'status',     command: 'roko status',                                                                           description: 'Workspace status: signals, episodes, health',       timeout: 30000  },
  { id: 'efficiency', command: 'roko learn efficiency',                                                                 description: 'Per-turn efficiency: tokens, cost, latency, model selection', timeout: 30000 },
];

// ── Runtime commands factory (ctx-aware, actual command strings) ─

function researchLoopCommands(ctx: ScenarioContext): CommandDef[] {
  return [
    { id: 'idea',       command: roko(ctx, 'prd idea "Add config validation with schema checking and helpful error messages"'), description: 'Capture raw work item into PRD backlog',            timeout: 45000  },
    { id: 'draft',      command: roko(ctx, 'prd draft new cli-config-validation'),                                              description: 'Agent expands idea into structured PRD',            timeout: 120000 },
    { id: 'research',   command: roko(ctx, 'research enhance-prd cli-config-validation'),                                       description: 'Research agent enriches PRD with prior art',        timeout: 180000 },
    { id: 'plan',       command: roko(ctx, 'prd plan cli-config-validation'),                                                   description: 'Generate implementation plan from research-enhanced PRD', timeout: 180000 },
    { id: 'run',        command: roko(ctx, 'plan run .roko/plans --max-retries 1'),                                             description: 'Execute plan: agents implement, gates validate',    timeout: 300000 },
    { id: 'learn-all',  command: roko(ctx, 'learn all'),                                                                        description: 'Full learning state: router, experiments, thresholds', timeout: 30000 },
    { id: 'learn-tune', command: roko(ctx, 'learn tune routing'),                                                               description: 'Cascade router tuning: model confidence scores',    timeout: 30000  },
    { id: 'status',     command: roko(ctx, 'status'),                                                                           description: 'Workspace status: signals, episodes, health',       timeout: 30000  },
    { id: 'efficiency', command: roko(ctx, 'learn efficiency'),                                                                 description: 'Per-turn efficiency: tokens, cost, latency, model selection', timeout: 30000 },
  ];
}

// ── Scenario ─────────────────────────────────────────────────

export const prdResearchLoop: ClickableScenario = {
  id: 'prd-research-loop',
  title: 'Research Loop',
  subtitle: 'Full pipeline: idea, draft, research, plan, execute, gates, learn.',
  panes: 1,
  labels: ['full pipeline'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['Full PRD lifecycle', 'Research enhancement', '7 gates'],
  durationHint: '~90s',
  accent: 'rose',
  icon: 'pipeline',
  steps: [
    { label: 'Capture idea', sublabel: 'prd idea' },
    { label: 'Draft PRD', sublabel: 'prd draft new' },
    { label: 'Research enhance', sublabel: 'research enhance-prd' },
    { label: 'Generate plan', sublabel: 'prd plan' },
    { label: 'Execute plan', sublabel: 'plan run' },
    { label: 'Gate results', sublabel: 'compile + test + clippy' },
    { label: 'Learn', sublabel: 'learn all' },
    { label: 'Summary', sublabel: 'status + efficiency' },
  ],
  commands: RESEARCH_LOOP_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const commands = researchLoopCommands(ctx);
    const cmd = commands.find(c => c.id === commandId);
    if (!cmd) return { ok: false, error: 'Unknown command' };

    const [main] = ctx.entries;
    if (!main) return { ok: false, error: 'No terminal connected' };

    const tracker = trackMetrics(main, {
      onCost: (c) => ctx.setMetric('cost', c),
      onTokens: (t) => ctx.setMetric('tokens', t),
    }, 250);

    try {
      if (commandId === 'learn-all' || commandId === 'status') {
        main.clearTerminal();
      }

      if (commandId === 'run') {
        ctx.setGate('compile', 'pending');
        ctx.setGate('test', 'pending');
        ctx.setGate('clippy', 'pending');
      }

      const result = await showCmd(main, cmd.command, {
        timeout: cmd.timeout ?? 60000,
        customDesc: cmd.description,
        workspaceDir: ctx.workspaceDir,
        signal: ctx.signal,
        onGate: commandId === 'run' ? (name, status) => ctx.setGate(name, status) : undefined,
      });

      if (result.cost) ctx.setMetric('cost', result.cost);
      if (result.tokens) ctx.setMetric('tokens', result.tokens);

      return { ok: result.ok, error: result.error };
    } finally {
      clearInterval(tracker);
    }
  },
};
