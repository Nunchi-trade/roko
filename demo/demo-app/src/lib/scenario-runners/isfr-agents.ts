import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { showCmd, roko, trackMetrics } from '../terminal-session';
import { castBlockNumber } from '../isfr-helpers';

// ── Static command definitions ────────────────────────────────

export const ISFR_COMMANDS: CommandDef[] = [
  {
    id: 'chain-check',
    command: 'cast ... eth_blockNumber',
    description: 'Verify chain connection',
    timeout: 15000,
    target: { pane: 0 },
  },
  {
    id: 'aave-scout',
    command: 'roko run "Aave V3 scout: fetch USDC supply rate"',
    description: 'Aave scout fetches USDC rate',
    timeout: 180000,
    target: { pane: 0 },
  },
  {
    id: 'compound-scout',
    command: 'roko run "Compound V3 scout: fetch USDC supply rate"',
    description: 'Compound scout fetches rate',
    timeout: 180000,
    target: { pane: 1 },
  },
  {
    id: 'ethena-scout',
    command: 'roko run "Ethena sUSDe scout: fetch yield"',
    description: 'Ethena scout fetches yield',
    timeout: 180000,
    target: { pane: 2 },
  },
  {
    id: 'staking-scout',
    command: 'roko run "ETH staking scout: estimate yield"',
    description: 'Staking scout estimates yield',
    timeout: 180000,
    target: { pane: 3 },
  },
  {
    id: 'lending-agg',
    command: 'roko run "Lending Aggregator: TVL-weighted median"',
    description: 'Lending aggregator: TVL-weighted median',
    timeout: 180000,
    target: { pane: 4 },
  },
  {
    id: 'structured-agg',
    command: 'roko run "Structured Aggregator: Ethena + staking"',
    description: 'Structured aggregator',
    timeout: 180000,
    target: { pane: 5 },
  },
  {
    id: 'calculator',
    command: 'roko run "ISFR Calculator: weighted composite"',
    description: 'ISFR weighted composite',
    timeout: 180000,
    target: { pane: 6 },
  },
  {
    id: 'validator',
    command: 'roko run "ISFR Validator: bounds check + validate"',
    description: 'Bounds check + validate',
    timeout: 120000,
    target: { pane: 7 },
  },
];

// ── Scenario ──────────────────────────────────────────────────

export const isfrAgents: ClickableScenario = {
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
  commands: ISFR_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, setMetric, setGate, logCommand, logCommandComplete, signal } = ctx;

    switch (commandId) {
      case 'chain-check': {
        const entry = entries[0];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        const result = await showCmd(entry, castBlockNumber(), {
          timeout: 15000,
          customDesc: 'Verify connection to mirage-rs Ethereum fork',
          signal,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
        });
        if (result.ok) {
          setMetric('chain', 'connected');
          setGate('chain-fork', 'pass');
        } else {
          setMetric('status', 'chain offline');
          setGate('chain-fork', 'fail');
        }
        return { ok: result.ok, error: result.error };
      }

      case 'aave-scout': {
        const entry = entries[0];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        const tracker = trackMetrics(entry, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
        });
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the Aave V3 scout. Fetch the USDC supply rate from Aave V3 on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call getReserveData(address) on 0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2 with USDC address 0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48. Convert the raw liquidityRate (ray, 1e27) to an annual percentage rate. Post the Aave USDC supply rate as a knowledge insight."`),
          {
            timeout: 180000,
            customDesc: 'Aave scout fetches USDC supply rate via getReserveData',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        clearInterval(tracker);
        setGate('aave-data', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      case 'compound-scout': {
        const entry = entries[1];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the Compound V3 scout. Fetch the USDC supply rate from Compound V3 on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call getSupplyRate(uint256) on 0xc3d688B66703497DAA19211EEdff47f25384cdc3 with arg 0. The result is per-second scaled by 1e18; convert to annual percentage rate. Post the Compound USDC supply rate as a knowledge insight."`),
          {
            timeout: 180000,
            customDesc: 'Compound scout fetches USDC supply rate via getSupplyRate',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        setGate('compound-data', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      case 'ethena-scout': {
        const entry = entries[2];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the Ethena sUSDe scout. Fetch the sUSDe yield on the Ethereum fork at http://127.0.0.1:8545 using cast call. Call totalAssets() and totalSupply() on 0x9D39A5DE30e57443BfF2A8307A4256c8797A3497. Compute the exchange rate (totalAssets/totalSupply) and estimate the annualized yield. Post the Ethena sUSDe yield as a knowledge insight."`),
          {
            timeout: 180000,
            customDesc: 'Ethena scout fetches sUSDe yield via totalAssets/totalSupply',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        setGate('ethena-data', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      case 'staking-scout': {
        const entry = entries[3];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the ETH staking scout. Estimate the ETH beacon chain staking yield on the Ethereum fork at http://127.0.0.1:8545. Check the deposit contract balance at 0x00000000219ab540356cBB839Cbe05303d7705Fa using cast balance. Use the total staked ETH to estimate the current annualized staking yield. Post the ETH staking yield as a knowledge insight."`),
          {
            timeout: 180000,
            customDesc: 'Staking scout estimates ETH beacon chain yield via deposit contract',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        setGate('staking-data', result.ok ? 'pass' : 'fail');
        // All 4 scouts have individual gates; mark data-collection pass optimistically per scout
        setGate('data-collection', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      case 'lending-agg': {
        const entry = entries[4];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        entry.clearTerminal();
        const tracker = trackMetrics(entry, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
        });
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the Lending Aggregator. Search the knowledge graph for the Aave USDC supply rate and Compound USDC supply rate insights posted by the scout agents. Compute the TVL-weighted median for the LENDING asset class. Report the aggregated LENDING class rate in basis points. Post the LENDING class rate as a knowledge insight."`),
          {
            timeout: 180000,
            customDesc: 'Lending aggregator computes TVL-weighted median from Aave + Compound',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        clearInterval(tracker);
        return { ok: result.ok, error: result.error };
      }

      case 'structured-agg': {
        const entry = entries[5];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        entry.clearTerminal();
        const tracker = trackMetrics(entry, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
        });
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the Structured Aggregator. Search the knowledge graph for the Ethena sUSDe yield and ETH staking yield insights posted by the scout agents. Compute the weighted rate for the STRUCTURED asset class (Ethena) and the STAKING class (ETH staking). Report each class rate in basis points. Post the STRUCTURED and STAKING class rates as knowledge insights."`),
          {
            timeout: 180000,
            customDesc: 'Structured aggregator processes Ethena + staking yields',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        clearInterval(tracker);
        return { ok: result.ok, error: result.error };
      }

      case 'calculator': {
        const entry = entries[6];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        entry.clearTerminal();
        const tracker = trackMetrics(entry, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
        });
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the ISFR Calculator. Retrieve all class rates from the knowledge graph: LENDING, STRUCTURED, FUNDING (use 0 if not available), and STAKING. Compute the Internet Secured Funding Rate as a weighted composite: ISFR = 0.60 * LENDING + 0.25 * STRUCTURED + 0.10 * FUNDING + 0.05 * STAKING. Express the result in basis points. Post the composite ISFR rate as a final insight."`),
          {
            timeout: 180000,
            customDesc: 'Calculator applies ISFR weights to compute final composite rate',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        clearInterval(tracker);
        setGate('computation', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      case 'validator': {
        const entry = entries[7];
        if (!entry) return { ok: false, error: 'No terminal connected' };
        entry.clearTerminal();
        const tracker = trackMetrics(entry, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
        });
        const result = await showCmd(
          entry,
          roko(ctx, `run "You are the ISFR Validator. Retrieve the ISFR composite rate from the knowledge graph. Validate that the rate is within reasonable bounds (0 to 5000 basis points, i.e. 0% to 50%). Check that all component class rates are present and non-negative. If valid, confirm the insight. Report the final validated ISFR rate and its component breakdown."`),
          {
            timeout: 120000,
            customDesc: 'Validator checks bounds and confirms the ISFR composite rate',
            signal,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: setGate,
          },
        );
        clearInterval(tracker);
        setGate('validation', result.ok ? 'pass' : 'fail');
        return { ok: result.ok, error: result.error };
      }

      default:
        return { ok: false, error: 'Unknown command' };
    }
  },
};
