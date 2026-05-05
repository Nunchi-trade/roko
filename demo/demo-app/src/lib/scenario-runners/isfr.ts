// --- src/lib/scenario-runners/isfr.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

export const ISFR_COMMANDS: CommandDef[] = [
  {
    id: 'lending-scout',
    command: 'roko do "Lending Scout: estimate USDC lending rate from Aave and Compound"',
    description: 'Lending scout gathers DeFi rates',
    timeout: 180000,
    target: { pane: 0 },
  },
  {
    id: 'staking-scout',
    command: 'roko do "Staking Scout: estimate ETH staking yield"',
    description: 'Staking scout gathers yield data',
    timeout: 180000,
    target: { pane: 1 },
  },
  {
    id: 'aggregator',
    command: 'roko do "ISFR Aggregator: combine lending and staking observations"',
    description: 'Aggregator computes composite ISFR',
    timeout: 180000,
    target: { pane: 2 },
  },
  {
    id: 'validator',
    command: 'roko do "ISFR Validator: verify rate bounds and publish recommendation"',
    description: 'Validator checks and signs off',
    timeout: 180000,
    target: { pane: 3 },
  },
];

export const isfrScenario: ClickableScenario = {
  id: 'isfr',
  title: 'ISFR',
  subtitle: 'Four specialized agents compute a DeFi risk-free rate.',
  panes: 4,
  labels: ['Lending Scout', 'Staking Scout', 'Aggregator', 'Validator'],
  panel: true,
  promptBar: false,
  mirageBar: true,
  category: 'chain',
  features: ['Agent swarm', 'Rate aggregation', 'Validation'],
  durationHint: '<2 min',
  accent: 'amber',
  icon: 'chain',
  steps: [
    { label: 'Scout', sublabel: 'lending and staking' },
    { label: 'Aggregate', sublabel: 'composite rate' },
    { label: 'Validate', sublabel: 'bounds and freshness' },
    { label: 'Publish', sublabel: 'final ISFR' },
  ],
  commands: ISFR_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const command = ISFR_COMMANDS.find((item) => item.id === commandId);
    const target = command?.target;
    if (!command || !target || typeof target === 'string' || !('pane' in target)) {
      return { ok: false, error: 'Unknown command' };
    }

    const entry = ctx.entries[target.pane];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    const result = await showCmd(entry, roko(ctx, command.command.replace(/^roko /, '')), {
      timeout: command.timeout,
      customDesc: command.description,
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    if (result.cost) ctx.setMetric('cost', result.cost);
    if (result.tokens) ctx.setMetric('tokens', result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
