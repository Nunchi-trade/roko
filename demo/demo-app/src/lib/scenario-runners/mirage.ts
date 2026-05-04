// --- src/lib/scenario-runners/mirage.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { enterWorkspace, showCmd } from '../terminal-session';

// ── Static command definitions (display layer, no ctx needed) ─

export const MIRAGE_COMMANDS: CommandDef[] = [
  { id: 'health',  command: 'curl -sf http://localhost:8545/health; echo',                                                                                                                                                                                                                                                          description: 'Check mirage sidecar health',   timeout: 10000  },
  { id: 'blocks',  command: 'for i in 1 2 3; do curl -sf -X POST http://localhost:8545 -H "Content-Type: application/json" -d \'{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}\'; echo; sleep 1; done',                                                                                                            description: 'Watch block production',         timeout: 12000  },
  { id: 'mutate',  command: 'cast rpc anvil_setBalance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 0x8AC7230489E80000 --rpc-url http://localhost:8545 >/dev/null && printf "funded alpha wallet: " && cast balance 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 --ether --rpc-url http://localhost:8545', description: 'Mutate local fork state',         timeout: 15000  },
  { id: 'api',     command: 'curl -sf http://localhost:8545/api/health; echo; curl -sf http://localhost:8545/api/stats | head -c 500; echo',                                                                                                                                                                                        description: 'Read chain API surface',         timeout: 10000  },
];

// ── Runtime commands factory (ctx-aware, actual command strings) ─

function mirageCommands(_ctx: ScenarioContext): CommandDef[] {
  return MIRAGE_COMMANDS;
}

// ── Scenario ─────────────────────────────────────────────────

export const mirage: ClickableScenario = {
  id: 'mirage',
  title: 'Mirage',
  subtitle: 'Fork any EVM chain locally. Stream blocks in real-time with configurable block times.',
  panes: 1,
  labels: ['mirage'],
  panel: false,
  promptBar: false,
  mirageBar: true,
  steps: [
    { label: 'Connect', sublabel: 'mirage sidecar' },
    { label: 'Probe RPC', sublabel: 'block production' },
    { label: 'Mutate State', sublabel: 'anvil-compatible RPC' },
    { label: 'Inspect API', sublabel: 'knowledge substrate' },
  ],
  category: 'chain',
  features: ['EVM fork', 'Real-time blocks', 'Configurable block times'],
  durationHint: '~30s',
  accent: 'amber',
  icon: 'evm',
  commands: MIRAGE_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const commands = mirageCommands(ctx);
    const cmd = commands.find(c => c.id === commandId);
    if (!cmd) return { ok: false, error: 'Unknown command' };

    const [main] = ctx.entries;
    if (!main) return { ok: false, error: 'No terminal connected' };

    await enterWorkspace(main, ctx.workspaceDir);

    const result = await showCmd(main, cmd.command, {
      timeout: cmd.timeout ?? 60000,
      customDesc: cmd.description,
      signal: ctx.signal,
    });

    return { ok: result.ok, error: result.error };
  },
};
