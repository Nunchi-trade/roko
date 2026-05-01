import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, roko, trackMetrics } from '../terminal-session';
import { castBlockNumber } from '../isfr-helpers';

export const isfrAgents: Scenario = {
  id: 'isfr-agents',
  title: 'ISFR Agents',
  subtitle: '8 agents compute Internet Secured Funding Rate from live DeFi protocol data.',
  panes: 8,
  labels: [
    'Aave Scout',
    'Compound Scout',
    'Ethena Scout',
    'Staking Scout',
    'Lending Aggregator',
    'Structured Aggregator',
    'ISFR Calculator',
    'ISFR Validator',
  ],
  panel: true,
  promptBar: false,
  mirageBar: true,
  category: 'chain',
  features: ['Multi-agent coordination', 'DeFi rate aggregation', 'Knowledge sharing', 'On-chain validation'],
  durationHint: '~120s',
  accent: 'amber',
  icon: 'chain',
  steps: [
    { label: 'Setup', sublabel: 'Connect to mirage-rs fork' },
    { label: 'Data collection', sublabel: 'Scouts fetch DeFi rates' },
    { label: 'Aggregation', sublabel: 'Compute class medians' },
    { label: 'ISFR computation', sublabel: 'Weighted composite rate' },
    { label: 'Validation', sublabel: 'Bounds check + publish' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete, signal } = ctx;
    const [aaveScout, compoundScout, ethenaScout, stakingScout, lendingAgg, structuredAgg, calculator, validator] = entries;
    timeline.init(this.steps);

    // -- Phase 0: Setup --
    timeline.setActive(0);
    playback.setProgress(0, 5, 'Setting up workspaces');

    const dir = ctx.workspaceDir;
    await Promise.all(entries.map(e => enterWorkspace(e, dir)));

    // Verify chain connection on the first pane
    const blockCheck = await showCmd(aaveScout, castBlockNumber(), {
      playback,
      timeout: 15000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Verify connection to mirage-rs Ethereum fork',
    });

    if (!blockCheck.ok) {
      setMetric('status', 'chain offline');
      return;
    }
    setMetric('chain', 'connected');
    setGate('chain-fork', 'pass');

    if (signal.aborted) return;

    // -- Phase 1: Data collection (4 scouts in parallel) --
    await playback.waitForStep();
    timeline.setActive(1);
    playback.setProgress(1, 5, 'Scouts fetching DeFi rates');

    const scoutTracker = trackMetrics(aaveScout, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    // All 4 scouts run in parallel
    const [aaveResult, compoundResult, ethenaResult, stakingResult] = await Promise.all([
      showCmd(aaveScout,
        roko(ctx, `run "You are the Aave V3 scout. Fetch the USDC supply rate from Aave V3 on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call getReserveData(address) on 0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2 with USDC address 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48. Convert the raw liquidityRate (ray, 1e27) to an annual percentage rate. Post the Aave USDC supply rate as a knowledge insight."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Aave scout fetches USDC supply rate via getReserveData',
        },
      ),
      showCmd(compoundScout,
        roko(ctx, `run "You are the Compound V3 scout. Fetch the USDC supply rate from Compound V3 on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call getSupplyRate(uint256) on 0xc3d688B66703497DAA19211EEdff47f25384cdc3 with arg 0. The result is per-second scaled by 1e18; convert to annual percentage rate. Post the Compound USDC supply rate as a knowledge insight."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Compound scout fetches USDC supply rate via getSupplyRate',
        },
      ),
      showCmd(ethenaScout,
        roko(ctx, `run "You are the Ethena sUSDe scout. Fetch the sUSDe yield on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call totalAssets() and totalSupply() on 0x9D39A5DE30e57443BfF2A8307A4256c8797A3497. Compute the exchange rate (totalAssets/totalSupply) and estimate the annualized yield. Post the Ethena sUSDe yield as a knowledge insight."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Ethena scout fetches sUSDe yield via totalAssets/totalSupply',
        },
      ),
      showCmd(stakingScout,
        roko(ctx, `run "You are the ETH staking scout. Estimate the ETH beacon chain staking yield on the Ethereum fork at http://127.0.0.1:8545. Check the deposit contract balance at 0x00000000219ab540356cBB839Cbe05303d7705Fa using cast balance. Use the total staked ETH to estimate the current annualized staking yield. Post the ETH staking yield as a knowledge insight."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Staking scout estimates ETH beacon chain yield via deposit contract',
        },
      ),
    ]);
    clearInterval(scoutTracker);

    const scoutsOk = aaveResult.ok && compoundResult.ok && ethenaResult.ok && stakingResult.ok;
    setGate('data-collection', scoutsOk ? 'pass' : 'fail');
    const firstScoutCost = aaveResult.cost ?? compoundResult.cost ?? ethenaResult.cost ?? stakingResult.cost;
    if (firstScoutCost) setMetric('cost', firstScoutCost);

    if (signal.aborted) return;

    // -- Phase 2: Aggregation (2 aggregators in parallel) --
    await playback.waitForStep();
    timeline.setActive(2);
    playback.setProgress(2, 5, 'Aggregating class rates');

    lendingAgg.clearTerminal();
    structuredAgg.clearTerminal();

    const aggTracker = trackMetrics(lendingAgg, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    const [lendingAggResult, structuredAggResult] = await Promise.all([
      showCmd(lendingAgg,
        roko(ctx, `run "You are the Lending Aggregator. Search the knowledge graph for the Aave USDC supply rate and Compound USDC supply rate insights posted by the scout agents. Compute the TVL-weighted median for the LENDING asset class. Report the aggregated LENDING class rate in basis points. Post the LENDING class rate as a knowledge insight."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Lending aggregator computes TVL-weighted median from Aave + Compound',
        },
      ),
      showCmd(structuredAgg,
        roko(ctx, `run "You are the Structured Aggregator. Search the knowledge graph for the Ethena sUSDe yield and ETH staking yield insights posted by the scout agents. Compute the weighted rate for the STRUCTURED asset class (Ethena) and the STAKING class (ETH staking). Report each class rate in basis points. Post the STRUCTURED and STAKING class rates as knowledge insights."`),
        {
          playback,
          timeout: 180000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          onGate: setGate,
          customDesc: 'Structured aggregator processes Ethena + staking yields',
        },
      ),
    ]);
    clearInterval(aggTracker);
    setGate('aggregation', lendingAggResult.ok && structuredAggResult.ok ? 'pass' : 'fail');

    if (signal.aborted) return;

    // -- Phase 3: ISFR computation --
    await playback.waitForStep();
    timeline.setActive(3);
    playback.setProgress(3, 5, 'Computing ISFR composite');

    calculator.clearTerminal();
    const calcTracker = trackMetrics(calculator, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    const calcResult = await showCmd(calculator,
      roko(ctx, `run "You are the ISFR Calculator. Retrieve all class rates from the knowledge graph: LENDING, STRUCTURED, FUNDING (use 0 if not available), and STAKING. Compute the Internet Secured Funding Rate as a weighted composite: ISFR = 0.60 * LENDING + 0.25 * STRUCTURED + 0.10 * FUNDING + 0.05 * STAKING. Express the result in basis points. Post the composite ISFR rate as a final insight."`),
      {
        playback,
        timeout: 180000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        customDesc: 'Calculator applies ISFR weights to compute final composite rate',
      },
    );
    clearInterval(calcTracker);
    setGate('computation', calcResult.ok ? 'pass' : 'fail');

    if (signal.aborted) return;

    // -- Phase 4: Validation --
    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(4, 5, 'Validating ISFR');

    validator.clearTerminal();
    const valTracker = trackMetrics(validator, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
    });

    const valResult = await showCmd(validator,
      roko(ctx, `run "You are the ISFR Validator. Retrieve the ISFR composite rate from the knowledge graph. Validate that the rate is within reasonable bounds (0 to 5000 basis points, i.e. 0% to 50%). Check that all component class rates are present and non-negative. If valid, confirm the insight. Report the final validated ISFR rate and its component breakdown."`),
      {
        playback,
        timeout: 120000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        onGate: setGate,
        customDesc: 'Validator checks bounds and confirms the ISFR composite rate',
      },
    );
    clearInterval(valTracker);
    setGate('validation', valResult.ok ? 'pass' : 'fail');

    timeline.markAllComplete();
  },
};
