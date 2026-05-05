// --- src/lib/scenario-runners/knowledge-transfer.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko, trackMetrics } from '../terminal-session';

// Module-level state to track the Beta workspace across runCommand calls
let betaWorkspaceDir: string | null = null;

export function resetKnowledgeTransferState() {
  betaWorkspaceDir = null;
}

export const KNOWLEDGE_TRANSFER_COMMANDS: CommandDef[] = [
  { id: 'alpha-build',     command: 'roko run "Build a REST API in Rust using actix-web for user management..."',      description: 'Alpha builds User API (cold start)',   timeout: 300000, target: { pane: 0 } },
  { id: 'alpha-learn',     command: 'roko learn all',                                                                  description: 'Inspect Alpha learning state',         timeout: 60000,  target: { pane: 0 } },
  { id: 'alpha-knowledge', command: 'roko knowledge stats',                                                            description: 'Show extracted knowledge',             timeout: 30000,  target: { pane: 0 } },
  { id: 'sync-knowledge',  command: 'cp -r ...',                                                                       description: 'Sync knowledge to Beta workspace',     timeout: 10000,  target: { pane: 1 } },
  { id: 'beta-build',      command: 'roko run "Build a REST API in Rust using actix-web for inventory management..."', description: 'Beta builds with knowledge',            timeout: 300000, target: { pane: 1 } },
  { id: 'compare',         command: 'roko learn efficiency',                                                           description: 'Compare efficiency',                   timeout: 30000,  target: { pane: 1 } },
];

export const knowledgeTransfer: ClickableScenario = {
  id: 'knowledge-transfer',
  title: 'Knowledge Transfer',
  subtitle: 'Two agents build similar APIs. The second one learns from the first.',
  panes: 2,
  labels: ['Agent Alpha (cold start)', 'Agent Beta (with knowledge)'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Cross-agent learning', 'Cold vs warm start', 'Knowledge reuse'],
  durationHint: '~90s',
  accent: 'emerald',
  icon: 'transfer',
  resetState: resetKnowledgeTransferState,
  steps: [
    { label: 'Setup workspaces', sublabel: 'roko init x2' },
    { label: 'Alpha builds User API', sublabel: 'roko run (cold)' },
    { label: 'Distill knowledge', sublabel: 'episodes → insights' },
    { label: 'Beta builds Inventory API', sublabel: 'roko run (warm)' },
    { label: 'Compare results', sublabel: 'efficiency metrics' },
  ],
  commands: KNOWLEDGE_TRANSFER_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, setMetric, setGate, logCommand, logCommandComplete, signal } = ctx;
    const [alpha, beta] = entries;

    if (commandId === 'alpha-build') {
      if (!alpha) return { ok: false, error: 'No alpha pane connected' };
      const tracker = trackMetrics(alpha, {
        onCost: c => setMetric('cost', c),
        onTokens: t => setMetric('tokens', t),
      });
      try {
        const result = await showCmd(alpha,
          roko(ctx, 'run "Build a REST API in Rust using actix-web for user management. ' +
          'Include CRUD endpoints for users, input validation with the validator crate, ' +
          'structured JSON error responses, and integration tests with reqwest."'),
          {
            timeout: 300000,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
            signal,
            customDesc: 'Alpha agent starts from scratch. No prior knowledge — discovers patterns through exploration.',
          },
        );
        if (result.cost) setMetric('cost', result.cost);
        setMetric('time', `${result.elapsed.toFixed(0)}s`);
        return { ok: result.ok, error: result.error };
      } finally {
        clearInterval(tracker);
      }
    }

    if (commandId === 'alpha-learn') {
      if (!alpha) return { ok: false, error: 'No alpha pane connected' };
      const result = await showCmd(alpha, roko(ctx, 'learn all'), {
        timeout: 60000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        signal,
        customDesc: 'Inspects episodes, router decisions, and efficiency metrics. The distiller extracts reusable insights.',
      });
      return { ok: result.ok, error: result.error };
    }

    if (commandId === 'alpha-knowledge') {
      if (!alpha) return { ok: false, error: 'No alpha pane connected' };
      const result = await showCmd(alpha, roko(ctx, 'knowledge stats'), {
        timeout: 30000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        signal,
        customDesc: 'Shows what knowledge entries were extracted — heuristics, strategies, and warnings.',
      });
      return { ok: result.ok, error: result.error };
    }

    if (commandId === 'sync-knowledge') {
      if (!beta) return { ok: false, error: 'No beta pane connected' };
      // Create Beta workspace if not already created
      if (!betaWorkspaceDir) {
        betaWorkspaceDir = await ctx.createWorkspace('roko-inventory-api');
      }
      const dirA = ctx.workspaceDir;
      const dirB = betaWorkspaceDir;
      await beta.execCmd(
        `cp -r ${dirA}/.roko/neuro ${dirB}/.roko/neuro 2>/dev/null; ` +
        `cp -r ${dirA}/.roko/learn ${dirB}/.roko/learn 2>/dev/null; true`,
        10000,
      );
      beta.clearTerminal();
      return { ok: true };
    }

    if (commandId === 'beta-build') {
      if (!beta) return { ok: false, error: 'No beta pane connected' };
      if (!betaWorkspaceDir) {
        betaWorkspaceDir = await ctx.createWorkspace('roko-inventory-api');
      }
      const tracker = trackMetrics(beta, {
        onCost: c => setMetric('cost', c),
        onTokens: t => setMetric('tokens', t),
      });
      try {
        const result = await showCmd(beta,
          roko(ctx,
            'run "Build a REST API in Rust using actix-web for inventory management. ' +
            'Include CRUD endpoints for products, search and filter, input validation, ' +
            'structured JSON error responses, and integration tests with reqwest."'),
          {
            timeout: 300000,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
            signal,
            workspaceDir: betaWorkspaceDir,
            customDesc: 'Beta agent starts with knowledge from Alpha. Skips exploration, uses proven patterns immediately.',
          },
        );
        return { ok: result.ok, error: result.error };
      } finally {
        clearInterval(tracker);
      }
    }

    if (commandId === 'compare') {
      if (!beta) return { ok: false, error: 'No beta pane connected' };
      const result = await showCmd(beta, roko(ctx, 'learn efficiency'), {
        timeout: 30000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        signal,
        customDesc: 'Shows efficiency comparison — cost, turns, and time savings from knowledge transfer.',
      });
      return { ok: result.ok, error: result.error };
    }

    return { ok: false, error: 'Unknown command' };
  },
};
