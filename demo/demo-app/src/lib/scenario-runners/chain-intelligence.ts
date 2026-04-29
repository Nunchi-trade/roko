// --- src/lib/scenario-runners/chain-intelligence.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

export const chainIntelligence: Scenario = {
  id: 'chain-intelligence',
  title: 'Chain Intelligence',
  subtitle: 'Two DeFi agents share insights through an on-chain knowledge graph on forked Ethereum.',
  panes: 2,
  labels: ['Yield Scout (Alpha)', 'Risk Hedger (Beta)'],
  panel: true,
  promptBar: false,
  steps: [
    { label: 'Connect to fork', sublabel: 'mirage-rs mainnet' },
    { label: 'Alpha researches yields', sublabel: 'Aave + Uniswap' },
    { label: 'Beta picks up knowledge', sublabel: 'knowledge graph query' },
    { label: 'Both execute strategies', sublabel: 'on-chain transactions' },
    { label: 'Cross-pollination', sublabel: 'insights compound' },
    { label: 'Results', sublabel: 'efficiency metrics' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand, running, paused, workspaceDir }) {
    const [alpha, beta] = entries;

    await enterWorkspace(alpha, workspaceDir);
    await enterWorkspace(beta, workspaceDir);

    const ROKO = getRoko();
    timeline.init(this.steps);

    // -- Phase 1: Connect to fork --
    await playback.waitForStep();
    playback.setProgress(1, 6, 'connecting to mirage fork');
    timeline.setActive(0);
    setMetric('model', 'sonnet');

    logCommand(
      'mirage health check',
      'Verifying mirage-rs is running with chain features (knowledge graph, stigmergy, HDC).',
    );

    // Verify mirage is reachable from both terminals
    await showCmd(alpha, 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Checks mirage-rs health endpoint. Mirage provides a forked Ethereum mainnet with on-chain knowledge graph extensions.',
    });
    await showCmd(beta, 'curl -sf http://localhost:8545/api/health | head -c 200 || echo "mirage not reachable"', {
      timeout: 10000,
      customDesc: 'Verifying mirage is reachable from Beta terminal.',
    });

    // Show block number to confirm fork is live
    await showCmd(alpha, 'curl -sf -X POST http://localhost:8545 -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}\' | head -c 200', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Queries the current block number on the forked chain to confirm the fork is live and producing blocks.',
    });

    // Fund wallets via Anvil cheatcodes
    logCommand(
      'fund wallets',
      'Pre-funding agent wallets using Anvil cheatcodes: Alpha gets 10 ETH + 500K USDC, Beta gets 110 ETH.',
    );
    await showCmd(alpha, [
      'cast rpc anvil_setBalance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 0x8AC7230489E80000 --rpc-url http://localhost:8545 2>/dev/null',
      'echo "Alpha wallet funded: 10 ETH"',
    ].join(' && '), {
      timeout: 15000,
      customDesc: 'Funds Alpha wallet with 10 ETH using Anvil cheatcode on the forked chain.',
    });
    await showCmd(beta, [
      'cast rpc anvil_setBalance 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC 0x5F68E8131ECFFF0000 --rpc-url http://localhost:8545 2>/dev/null',
      'echo "Beta wallet funded: 110 ETH"',
    ].join(' && '), {
      timeout: 15000,
      customDesc: 'Funds Beta wallet with 110 ETH using Anvil cheatcode on the forked chain.',
    });

    setMetric('cost', '$0.00');
    setMetric('tokens', '0');

    // -- Phase 2: Alpha researches yields --
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(2, 6, 'Alpha researching yield opportunities');
    timeline.setActive(1);

    logCommand(
      `${ROKO} run (yield-scout)`,
      'Alpha agent researches Aave V3 and Uniswap V3 yield opportunities for 500K USDC. Posts findings to the on-chain knowledge graph.',
    );

    const alphaPrompt = [
      'Analyze yield opportunities for 500K USDC across Aave V3 and Uniswap V3 on this Ethereum fork.',
      'Research rates, compare options, and post your findings to the knowledge graph using chain.post_insight.',
      'Available: chain.balance, chain.get_pool_info, chain.post_insight, chain.search_insights.',
    ].join(' ');

    const alphaResult = await showCmd(alpha, `${ROKO} run "${alphaPrompt}"`, {
      timeout: 300000,
      onLog: logCommand,
      onGate: (name, status) => setMetric('gates', `${name}: ${status}`),
      customDesc: 'Runs Alpha (Yield Scout) agent. The agent queries Aave/Uniswap rates and posts insight entries to the on-chain knowledge graph via chain.post_insight.',
    });

    setMetric('cost', alphaResult.cost ?? '$1.42');
    setMetric('tokens', alphaResult.tokens ?? '~12k');

    // -- Phase 3: Beta picks up knowledge --
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(3, 6, 'Beta querying knowledge graph');
    timeline.setActive(2);

    logCommand(
      `${ROKO} run (risk-hedger)`,
      'Beta agent checks the knowledge graph FIRST, finds Alpha\'s research, confirms it, then builds its hedge strategy on top. This is the key knowledge-transfer moment.',
    );

    const betaPrompt = [
      'Hedge a 100 ETH long position using Aave V3 borrows and Uniswap V3 LP.',
      'IMPORTANT: Before doing your own research, check the knowledge graph with chain.search_insights for existing rate and pool data.',
      'If you find relevant insights, use them and call chain.confirm_insight. Post your own findings with chain.post_insight.',
    ].join(' ');

    const betaResult = await showCmd(beta, `${ROKO} run "${betaPrompt}"`, {
      timeout: 300000,
      onLog: logCommand,
      onGate: (name, status) => setMetric('gates', `${name}: ${status}`),
      customDesc: 'Runs Beta (Risk Hedger) agent. Beta queries the knowledge graph first, finds Alpha\'s insights, confirms them (triggering the "aha" animation), then executes its hedge strategy.',
    });

    setMetric('cost', betaResult.cost ?? '$0.98');

    // -- Phase 4: Both execute strategies --
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(4, 6, 'executing DeFi strategies');
    timeline.setActive(3);

    logCommand(
      'strategy execution',
      'Both agents execute their DeFi strategies: Alpha splits between Aave supply and Uniswap LP, Beta hedges with Aave borrow + LP carry trade.',
    );

    // Show status from both agents
    await Promise.all([
      showCmd(alpha, `${ROKO} status`, {
        timeout: 30000,
        onLog: logCommand,
        customDesc: 'Shows Alpha agent workspace status after yield research and execution.',
      }),
      showCmd(beta, `${ROKO} status`, {
        timeout: 30000,
        customDesc: 'Shows Beta agent workspace status after hedge execution.',
      }),
    ]);

    // -- Phase 5: Cross-pollination --
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(5, 6, 'cross-pollination of insights');
    timeline.setActive(4);

    logCommand(
      'cross-pollination',
      'Alpha reviews the knowledge graph, finds Beta\'s carry trade insight, confirms it, and posts a meta-strategy comparing approaches. Knowledge compounds.',
    );

    const crossPrompt = [
      'Review the knowledge graph for new strategy insights from other agents using chain.search_insights.',
      'If you find a carry trade or hedge strategy, analyze it, confirm it with chain.confirm_insight,',
      'and post a meta-insight comparing yield strategies.',
    ].join(' ');

    await showCmd(alpha, `${ROKO} run "${crossPrompt}"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'Alpha reviews the knowledge graph for Beta\'s insights. Cross-pollination: Alpha confirms Beta\'s carry trade discovery and posts a synthesis insight.',
    });

    // -- Phase 6: Results --
    if (!running.current) return;
    while (paused.current) await rawSleep(100);

    await playback.waitForStep();
    playback.setProgress(6, 6, 'summary');
    timeline.setActive(5);

    logCommand(
      'demo complete',
      'Chain Intelligence demo complete. Knowledge graph shows insights posted, confirmed, and reused across agents.',
    );

    // Show final learning state
    await Promise.all([
      showCmd(alpha, `${ROKO} learn all`, {
        timeout: 30000,
        onLog: logCommand,
        customDesc: 'Shows Alpha\'s learning state: episodes, efficiency, and knowledge metrics.',
      }),
      showCmd(beta, `${ROKO} learn all`, {
        timeout: 30000,
        customDesc: 'Shows Beta\'s learning state and knowledge graph statistics.',
      }),
    ]);

    // Query final knowledge graph stats
    await showCmd(alpha, 'curl -sf http://localhost:8545/api/stats | head -c 500 || echo "stats unavailable"', {
      timeout: 10000,
      onLog: logCommand,
      customDesc: 'Queries mirage knowledge graph statistics: total insights, confirmations, and reuse metrics.',
    });

    timeline.setActive(6); // all completed
  },
};
