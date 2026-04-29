// --- src/lib/scenario-runners/mirage.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep } from '../scenario-helpers';

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
  steps: [],
  async run({ entries }) {
    const e = entries[0];

    // Wait for WS to connect
    const start = Date.now();
    while (Date.now() - start < 8000) {
      if (e.ws && e.ws.readyState === WebSocket.OPEN) break;
      await rawSleep(100);
    }
    await e.waitForPrompt(10000);
    e.clearTerminal();
  },
};
