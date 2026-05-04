// --- src/lib/scenario-runners/chain-intelligence.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko } from '../terminal-session';

// ── Static command definitions ────────────────────────────────

export const CHAIN_INTELLIGENCE_COMMANDS: CommandDef[] = [
  {
    id: 'health-alpha',
    command: 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"',
    description: 'Check mirage health (Alpha)',
    timeout: 10000,
    target: { pane: 0 },
  },
  {
    id: 'health-beta',
    command: 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"',
    description: 'Check mirage health (Beta)',
    timeout: 10000,
    target: { pane: 1 },
  },
  {
    id: 'block-check',
    command: 'curl -sf -X POST http://localhost:8545 -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}\' | head -c 200',
    description: 'Verify fork is live',
    timeout: 10000,
    target: { pane: 0 },
  },
  {
    id: 'fund-alpha',
    command: 'cast rpc anvil_setBalance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 0x8AC7230489E80000 --rpc-url http://localhost:8545 2>/dev/null && echo "Alpha wallet funded: 10 ETH"',
    description: 'Fund Alpha wallet (10 ETH)',
    timeout: 15000,
    target: { pane: 0 },
  },
  {
    id: 'fund-beta',
    command: 'cast rpc anvil_setBalance 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC 0x5F68E8131ECFFF0000 --rpc-url http://localhost:8545 2>/dev/null && echo "Beta wallet funded: 110 ETH"',
    description: 'Fund Beta wallet (110 ETH)',
    timeout: 15000,
    target: { pane: 1 },
  },
  {
    id: 'alpha-research',
    command: 'roko run "Analyze yield opportunities for 500K USDC across Aave V3 and Uniswap V3 on this Ethereum fork. Research rates, compare options, and post your findings to the knowledge graph using chain.post_insight. Available: chain.balance, chain.get_pool_info, chain.post_insight, chain.search_insights."',
    description: 'Alpha researches yield opportunities',
    timeout: 300000,
    target: { pane: 0 },
  },
  {
    id: 'beta-hedge',
    command: 'roko run "Hedge a 100 ETH long position using Aave V3 borrows and Uniswap V3 LP. IMPORTANT: Before doing your own research, check the knowledge graph with chain.search_insights for existing rate and pool data. If you find relevant insights, use them and call chain.confirm_insight. Post your own findings with chain.post_insight."',
    description: 'Beta hedges with knowledge',
    timeout: 300000,
    target: { pane: 1 },
  },
  {
    id: 'alpha-status',
    command: 'roko status',
    description: 'Alpha workspace status',
    timeout: 30000,
    target: { pane: 0 },
  },
  {
    id: 'beta-status',
    command: 'roko status',
    description: 'Beta workspace status',
    timeout: 30000,
    target: { pane: 1 },
  },
  {
    id: 'cross-pollination',
    command: 'roko run "Review the knowledge graph for new strategy insights from other agents using chain.search_insights. If you find a carry trade or hedge strategy, analyze it, confirm it with chain.confirm_insight, and post a meta-insight comparing yield strategies."',
    description: 'Cross-pollination of insights',
    timeout: 180000,
    target: { pane: 0 },
  },
  {
    id: 'alpha-learn',
    command: 'roko learn all',
    description: 'Alpha learning state',
    timeout: 30000,
    target: { pane: 0 },
  },
  {
    id: 'beta-learn',
    command: 'roko learn all',
    description: 'Beta learning state',
    timeout: 30000,
    target: { pane: 1 },
  },
];

// ── Scenario ──────────────────────────────────────────────────

export const chainIntelligence: ClickableScenario = {
  id: 'chain-intelligence',
  title: 'Chain Intelligence',
  subtitle: 'Two DeFi agents share insights through an on-chain knowledge graph on forked Ethereum.',
  panes: 2,
  labels: ['Yield Scout (Alpha)', 'Risk Hedger (Beta)'],
  panel: true,
  promptBar: false,
  category: 'chain',
  features: ['On-chain knowledge graph', 'DeFi agents', 'Forked Ethereum'],
  durationHint: '~120s',
  accent: 'violet',
  icon: 'chain',
  steps: [
    { label: 'Connect to fork', sublabel: 'mirage-rs mainnet' },
    { label: 'Alpha researches yields', sublabel: 'Aave + Uniswap' },
    { label: 'Beta picks up knowledge', sublabel: 'knowledge graph query' },
    { label: 'Both execute strategies', sublabel: 'on-chain transactions' },
    { label: 'Cross-pollination', sublabel: 'insights compound' },
    { label: 'Results', sublabel: 'efficiency metrics' },
  ],
  commands: CHAIN_INTELLIGENCE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const def = CHAIN_INTELLIGENCE_COMMANDS.find(c => c.id === commandId);
    if (!def) return { ok: false, error: 'Unknown command' };

    const paneIndex = def.target && typeof def.target === 'object' && 'pane' in def.target ? def.target.pane : 0;
    const entry = ctx.entries[paneIndex];
    if (!entry) return { ok: false, error: 'No terminal connected' };

    // For roko commands, resolve the binary; for raw commands, use as-is
    const isRokoCmd = commandId === 'alpha-research' || commandId === 'beta-hedge' ||
      commandId === 'alpha-status' || commandId === 'beta-status' ||
      commandId === 'cross-pollination' || commandId === 'alpha-learn' || commandId === 'beta-learn';

    let command: string;
    if (isRokoCmd) {
      // Strip leading "roko " and pass the rest to roko()
      const sub = def.command.replace(/^roko\s+/, '');
      command = roko(ctx, sub);
    } else {
      command = def.command;
    }

    const result = await showCmd(entry, command, {
      timeout: def.timeout ?? 60000,
      customDesc: def.description,
      signal: ctx.signal,
      onGate: (commandId === 'alpha-research' || commandId === 'beta-hedge')
        ? (name, status) => ctx.setMetric('gates', `${name}: ${status}`)
        : undefined,
    });

    if (commandId === 'alpha-research' || commandId === 'beta-hedge') {
      if (result.cost) ctx.setMetric('cost', result.cost);
      if (result.tokens) ctx.setMetric('tokens', result.tokens);
    }

    return { ok: result.ok, error: result.error };
  },
};
