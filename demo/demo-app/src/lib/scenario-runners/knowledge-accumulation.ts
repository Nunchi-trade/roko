// --- src/lib/scenario-runners/knowledge-accumulation.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

// ── Static command definitions (display layer, no ctx needed) ─

export const KNOWLEDGE_ACCUMULATION_COMMANDS: CommandDef[] = [
  { id: 'initial-stats',   command: 'roko knowledge stats',                                                          description: 'Check initial store state',            timeout: 30000,  target: { pane: 1 } },
  { id: 'initial-query',   command: 'roko knowledge query "error handling patterns"',                                description: 'Query before any runs',                timeout: 30000,  target: { pane: 1 } },
  { id: 'run-1',           command: 'roko run "Build a Rust CLI that parses JSON from stdin"',                        description: 'First run seeds knowledge',            timeout: 180000, target: { pane: 0 } },
  { id: 'check-stats-1',   command: 'roko knowledge stats',                                                          description: 'Store after run 1',                    timeout: 30000,  target: { pane: 1 } },
  { id: 'check-query-1',   command: 'roko knowledge query "JSON parsing"',                                           description: 'Query after run 1',                    timeout: 30000,  target: { pane: 1 } },
  { id: 'run-2',           command: 'roko run "Add comprehensive error handling with anyhow and thiserror"',          description: 'Second run compounds knowledge',       timeout: 180000, target: { pane: 0 } },
  { id: 'check-stats-2',   command: 'roko knowledge stats',                                                          description: 'Store after run 2',                    timeout: 30000,  target: { pane: 1 } },
  { id: 'check-query-2',   command: 'roko knowledge query "error handling patterns"',                                description: 'Query after run 2',                    timeout: 30000,  target: { pane: 1 } },
  { id: 'final-stats',     command: 'roko knowledge stats',                                                          description: 'Final store statistics',               timeout: 30000,  target: { pane: 1 } },
  { id: 'final-learn',     command: 'roko learn all',                                                                description: 'All learning state',                  timeout: 30000,  target: { pane: 1 } },
];

// ── Runtime commands factory (ctx-aware, actual command strings) ─

function knowledgeAccumulationCommands(ctx: ScenarioContext): CommandDef[] {
  return [
    { id: 'initial-stats',  command: roko(ctx, 'knowledge stats'),                                                    description: 'Check initial store state',            timeout: 30000,  target: { pane: 1 } },
    { id: 'initial-query',  command: roko(ctx, 'knowledge query "error handling patterns"'),                          description: 'Query before any runs',                timeout: 30000,  target: { pane: 1 } },
    { id: 'run-1',          command: roko(ctx, 'run "Build a Rust CLI that parses JSON from stdin"'),                  description: 'First run seeds knowledge',            timeout: 180000, target: { pane: 0 } },
    { id: 'check-stats-1',  command: roko(ctx, 'knowledge stats'),                                                    description: 'Store after run 1',                    timeout: 30000,  target: { pane: 1 } },
    { id: 'check-query-1',  command: roko(ctx, 'knowledge query "JSON parsing"'),                                     description: 'Query after run 1',                    timeout: 30000,  target: { pane: 1 } },
    { id: 'run-2',          command: roko(ctx, 'run "Add comprehensive error handling with anyhow and thiserror"'),    description: 'Second run compounds knowledge',       timeout: 180000, target: { pane: 0 } },
    { id: 'check-stats-2',  command: roko(ctx, 'knowledge stats'),                                                    description: 'Store after run 2',                    timeout: 30000,  target: { pane: 1 } },
    { id: 'check-query-2',  command: roko(ctx, 'knowledge query "error handling patterns"'),                          description: 'Query after run 2',                    timeout: 30000,  target: { pane: 1 } },
    { id: 'final-stats',    command: roko(ctx, 'knowledge stats'),                                                    description: 'Final store statistics',               timeout: 30000,  target: { pane: 1 } },
    { id: 'final-learn',    command: roko(ctx, 'learn all'),                                                          description: 'All learning state',                  timeout: 30000,  target: { pane: 1 } },
  ];
}

// ── Scenario ─────────────────────────────────────────────────

export const knowledgeAccumulation: ClickableScenario = {
  id: 'knowledge-accumulation',
  title: 'Knowledge Growth',
  subtitle: 'Watch the knowledge store grow across successive runs.',
  panes: 2,
  labels: ['task runner', 'knowledge store'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Knowledge store growth', 'Successive runs', 'Tier progression'],
  durationHint: '~90s',
  accent: 'emerald',
  icon: 'knowledge',
  steps: [
    { label: 'Initial query', sublabel: 'empty store' },
    { label: 'Run 1', sublabel: 'build a CLI tool' },
    { label: 'Knowledge check', sublabel: 'query after run 1' },
    { label: 'Run 2', sublabel: 'add error handling' },
    { label: 'Knowledge growth', sublabel: 'query after run 2' },
    { label: 'Final state', sublabel: 'knowledge stats' },
  ],
  commands: KNOWLEDGE_ACCUMULATION_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const commands = knowledgeAccumulationCommands(ctx);
    const cmd = commands.find(c => c.id === commandId);
    if (!cmd) return { ok: false, error: 'Unknown command' };

    const paneIndex = typeof cmd.target === 'object' && 'pane' in cmd.target ? cmd.target.pane : 0;
    const entry = ctx.entries[paneIndex];
    if (!entry) return { ok: false, error: 'No terminal connected' };

    // Clear terminal before certain commands
    if (commandId === 'check-stats-1' || commandId === 'check-stats-2' || commandId === 'final-stats') {
      ctx.entries[1]?.clearTerminal();
    }
    if (commandId === 'run-2') {
      ctx.entries[0]?.clearTerminal();
    }

    const result = await showCmd(entry, cmd.command, {
      timeout: cmd.timeout ?? 60000,
      customDesc: cmd.description,
      signal: ctx.signal,
      onLog: ctx.logCommand,
      onLogComplete: ctx.logCommandComplete,
    });

    // Track cost/tokens for run commands
    if (commandId === 'run-1' || commandId === 'run-2') {
      if (result.cost) ctx.setMetric('cost', result.cost);
      if (result.tokens) ctx.setMetric('tokens', result.tokens);
    }

    return { ok: result.ok };
  },
};
