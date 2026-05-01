// --- src/lib/scenario-runners/mirage.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd } from '../terminal-session';

export const mirage: Scenario = {
  id: 'mirage',
  title: 'Mirage',
  subtitle: 'Fork any EVM chain locally. Stream blocks in real-time with configurable block times.',
  panes: 1,
  labels: ['mirage'],
  panel: false,
  promptBar: false,
  mirageBar: true,
  category: 'chain',
  features: ['EVM fork', 'Real-time blocks', 'Configurable block times'],
  durationHint: '~30s',
  accent: 'amber',
  icon: 'evm',
  steps: [
    { label: 'Connect', sublabel: 'mirage sidecar' },
    { label: 'Probe RPC', sublabel: 'block production' },
    { label: 'Mutate State', sublabel: 'anvil-compatible RPC' },
    { label: 'Inspect API', sublabel: 'knowledge substrate' },
  ],
  async run(ctx) {
    const { entries, playback, timeline, logCommand, logCommandComplete, setMetric, signal, workspaceDir } = ctx;
    const e = entries[0];

    await enterWorkspace(e, workspaceDir);
    timeline.init(this.steps);
    setMetric('model', '--');
    setMetric('cost', '--');
    setMetric('tokens', '--');
    setMetric('gates', '--');

    await playback.waitForStep();
    playback.setProgress(1, 4, 'checking mirage sidecar');
    timeline.setActive(0);
    await showCmd(e, 'curl -sf http://localhost:8545/health; echo', {
      playback,
      timeout: 10000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      signal,
      customDesc: 'Checks the Mirage sidecar health endpoint exposed by the Railway-style container.',
    });

    if (signal.aborted) return;
    await playback.waitForStep();
    playback.setProgress(2, 4, 'watching block production');
    timeline.setActive(1);
    await showCmd(e, [
      'for i in 1 2 3; do',
      'curl -sf -X POST http://localhost:8545 -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}\';',
      'echo;',
      'sleep 1;',
      'done',
    ].join(' '), {
      playback,
      timeout: 12000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      signal,
      customDesc: 'Calls eth_blockNumber repeatedly to show the local fork producing blocks.',
    });

    if (signal.aborted) return;
    await playback.waitForStep();
    playback.setProgress(3, 4, 'mutating local fork state');
    timeline.setActive(2);
    await showCmd(e, [
      'cast rpc anvil_setBalance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 0x8AC7230489E80000 --rpc-url http://localhost:8545 >/dev/null',
      '&& printf "funded alpha wallet: "',
      '&& cast balance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 --ether --rpc-url http://localhost:8545',
    ].join(' '), {
      playback,
      timeout: 15000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      signal,
      customDesc: 'Uses Anvil-compatible RPC methods through Mirage, then reads the mutated balance back.',
    });

    if (signal.aborted) return;
    await playback.waitForStep();
    playback.setProgress(4, 4, 'reading chain API surface');
    timeline.setActive(3);
    await showCmd(e, 'curl -sf http://localhost:8545/api/health; echo; curl -sf http://localhost:8545/api/stats | head -c 500; echo', {
      playback,
      timeout: 10000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      signal,
      customDesc: 'Reads Mirage chain-extension API health and stats for knowledge/stigmergy subsystems.',
    });

    timeline.markAllComplete();
    playback.setProgress(4, 4, 'mirage smoke complete');
  },
};
