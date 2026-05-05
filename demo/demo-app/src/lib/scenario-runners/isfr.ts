// --- src/lib/scenario-runners/isfr.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

export const ISFR_COMMANDS: CommandDef[] = [
  {
    id: 'isfr-keeper',
    command: 'roko isfr start',
    description: 'ISFRKeeper polls mock rate sources and publishes composite rates',
    timeout: 180000,
    target: { pane: 0 },
  },
  {
    id: 'lending-scout',
    command: 'roko do "Analyze current USDC lending rates across Aave V3 and Compound. Compare APYs, utilization rates, and recommend the best lending strategy."',
    description: 'Lending Scout analyzes lending rates across protocols',
    timeout: 120000,
    target: { pane: 1 },
  },
  {
    id: 'staking-scout',
    command: 'roko do "Research ETH staking yields across major liquid staking protocols. Compare validator APR, slashing risk, and withdrawal times."',
    description: 'Staking Scout analyzes liquid staking yields',
    timeout: 120000,
    target: { pane: 2 },
  },
  {
    id: 'isfr-oracle',
    command: 'roko do "As the ISFR Oracle, read the latest rate observations from the knowledge store and publish a final composite risk-free rate recommendation with confidence intervals."',
    description: 'ISFR Oracle synthesizes rate data into a final recommendation',
    timeout: 120000,
    target: { pane: 3 },
  },
];

export const isfrScenario: ClickableScenario = {
  id: 'isfr',
  title: 'ISFR',
  subtitle: 'Four specialized agents compute a DeFi risk-free rate.',
  panes: 4,
  labels: ['ISFRKeeper', 'Lending Scout', 'Staking Scout', 'ISFR Oracle'],
  panel: true,
  promptBar: false,
  mirageBar: true,
  category: 'chain',
  features: ['Agent swarm', 'Rate aggregation', 'Oracle synthesis'],
  durationHint: '<3 min',
  accent: 'amber',
  icon: 'chain',
  steps: [
    { label: 'Poll', sublabel: 'rate sources' },
    { label: 'Scout', sublabel: 'lending and staking' },
    { label: 'Monitor', sublabel: 'composite rate' },
    { label: 'Synthesize', sublabel: 'final ISFR' },
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

    // The keeper uses a raw command (not a roko subcommand wrapper), while
    // the scout and oracle agents use roko do.
    const cmd = commandId === 'isfr-keeper'
      ? roko(ctx, 'isfr start')
      : roko(ctx, command.command.replace(/^roko /, ''));

    const result = await showCmd(entry, cmd, {
      timeout: command.timeout,
      customDesc: command.description,
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    if (result.cost) ctx.setMetric(`cost-${commandId}`, result.cost);
    if (result.tokens) ctx.setMetric(`tokens-${commandId}`, result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
