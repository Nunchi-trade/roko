// --- src/lib/scenario-runners/oracle.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

const BLOCK_NUMBER_COMMAND =
  'curl -s http://localhost:8545 -X POST -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}\' | jq .result';

export const ORACLE_COMMANDS: CommandDef[] = [
  {
    id: 'chain-check',
    command: BLOCK_NUMBER_COMMAND,
    description: 'Verify local Ethereum fork connection',
    timeout: 15000,
    target: { pane: 0 },
  },
  {
    id: 'data-agent',
    command: 'roko do "Query Aave V3 and Compound lending rates on the local Anvil fork. Write structured analysis to knowledge store."',
    description: 'Data agent writes DeFi rate analysis',
    timeout: 240000,
    target: { pane: 0 },
  },
  {
    id: 'strategy-agent',
    command: 'roko do "Read DeFi rate analysis from knowledge store. Recommend optimal USDC allocation across protocols."',
    description: 'Strategy agent consumes knowledge',
    timeout: 240000,
    target: { pane: 1 },
  },
];

export const oracleScenario: ClickableScenario = {
  id: 'oracle',
  title: 'Oracle',
  subtitle: 'On-chain data becomes reusable agent knowledge.',
  panes: 2,
  labels: ['data collection', 'strategy synthesis'],
  panel: true,
  promptBar: false,
  mirageBar: true,
  category: 'chain',
  features: ['Chain data', 'Knowledge write', 'Strategy read'],
  durationHint: '<2 min',
  accent: 'violet',
  icon: 'evm',
  steps: [
    { label: 'Connect', sublabel: 'local fork' },
    { label: 'Scan', sublabel: 'lending rates' },
    { label: 'Write', sublabel: 'knowledge store' },
    { label: 'Recommend', sublabel: 'USDC allocation' },
  ],
  commands: ORACLE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const command = ORACLE_COMMANDS.find((item) => item.id === commandId);
    if (!command) return { ok: false, error: 'Unknown command' };

    const target = command.target;
    const paneIdx = target && typeof target === 'object' && 'pane' in target ? target.pane : 0;
    const entry = ctx.entries[paneIdx];
    if (!entry) return { ok: false, error: 'Terminal pane is not connected' };

    if (commandId === 'chain-check') {
      const result = await showCmd(entry, BLOCK_NUMBER_COMMAND, {
        timeout: 15000,
        customDesc: 'Verify local Ethereum fork connection.',
        workspaceDir: ctx.workspaceDir,
        signal: ctx.signal,
      });
      if (result.ok) ctx.setMetric('chain-checked', '1');
      return { ok: result.ok, error: result.error };
    }

    const result = await showCmd(entry, roko(ctx, command.command.replace(/^roko /, '')), {
      timeout: command.timeout,
      customDesc: command.description,
      workspaceDir: ctx.workspaceDir,
      signal: ctx.signal,
    });

    const prefix = commandId === 'data-agent' ? 'data' : 'strategy';
    if (result.cost) ctx.setMetric(`${prefix}-cost`, result.cost);
    if (result.tokens) ctx.setMetric(`${prefix}-tokens`, result.tokens);
    ctx.setMetric(`${prefix}-elapsed`, String(result.elapsed ?? 0));
    ctx.setMetric(`${prefix}-calls`, '1');

    // Also feed sidebar stats
    if (result.cost) ctx.setMetric('cost', result.cost);
    if (result.tokens) ctx.setMetric('tokens', result.tokens);

    return { ok: result.ok, error: result.error };
  },
};
