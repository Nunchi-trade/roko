// --- src/lib/scenario-runners/providers.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../../scenarios';
import { stripAnsi } from '../../scenario-helpers';
import { showCmd, roko } from '../../terminal-session';

// ── Static command definitions (display layer, no ctx needed) ─

export const PROVIDERS_COMMANDS: CommandDef[] = [
  { id: 'zhipu',     command: 'roko run "Build a hello-world web server" --provider zhipu',     description: 'Dispatch to Zhipu provider',     timeout: 180000, target: { pane: 0 } },
  { id: 'openai',    command: 'roko run "Build a hello-world web server" --provider openai',    description: 'Dispatch to OpenAI provider',    timeout: 180000, target: { pane: 1 } },
  { id: 'anthropic', command: 'roko run "Build a hello-world web server" --provider anthropic', description: 'Dispatch to Anthropic provider', timeout: 180000, target: { pane: 2 } },
  { id: 'moonshot',  command: 'roko run "Build a hello-world web server" --provider moonshot',  description: 'Dispatch to Moonshot provider',  timeout: 180000, target: { pane: 3 } },
];

// ── Runtime commands factory (ctx-aware, actual command strings) ─

function providersCommands(ctx: ScenarioContext): CommandDef[] {
  return [
    { id: 'zhipu',     command: roko(ctx, 'run "Build a hello-world web server"') + ' --provider zhipu',     description: 'Dispatch to Zhipu provider',     timeout: 180000, target: { pane: 0 } },
    { id: 'openai',    command: roko(ctx, 'run "Build a hello-world web server"') + ' --provider openai',    description: 'Dispatch to OpenAI provider',    timeout: 180000, target: { pane: 1 } },
    { id: 'anthropic', command: roko(ctx, 'run "Build a hello-world web server"') + ' --provider anthropic', description: 'Dispatch to Anthropic provider', timeout: 180000, target: { pane: 2 } },
    { id: 'moonshot',  command: roko(ctx, 'run "Build a hello-world web server"') + ' --provider moonshot',  description: 'Dispatch to Moonshot provider',  timeout: 180000, target: { pane: 3 } },
  ];
}

// ── Scenario ─────────────────────────────────────────────────

export const providers: ClickableScenario = {
  id: 'providers',
  title: 'Providers',
  subtitle: 'One prompt, four providers, simultaneously. Provider-agnostic by design.',
  panes: 4,
  labels: ['zhipu', 'openai', 'anthropic', 'moonshot'],
  panel: true,
  promptBar: false,
  category: 'comparison',
  features: ['4 providers simultaneously', 'Provider-agnostic', 'Live output'],
  durationHint: '~45s',
  accent: 'amber',
  icon: 'grid',
  steps: [
    { label: 'Zhipu', sublabel: 'dispatch' },
    { label: 'OpenAI', sublabel: 'dispatch' },
    { label: 'Anthropic', sublabel: 'dispatch' },
    { label: 'Moonshot', sublabel: 'dispatch' },
  ],
  commands: PROVIDERS_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const commands = providersCommands(ctx);
    const cmd = commands.find(c => c.id === commandId);
    if (!cmd) return { ok: false, error: 'Unknown command' };

    const paneIndex = typeof cmd.target === 'object' && 'pane' in cmd.target ? cmd.target.pane : 0;
    const entry = ctx.entries[paneIndex];
    if (!entry) return { ok: false, error: 'No terminal connected' };

    await showCmd(entry, cmd.command, {
      timeout: cmd.timeout ?? 60000,
      customDesc: cmd.description,
      signal: ctx.signal,
      onLog: ctx.logCommand,
      onLogComplete: ctx.logCommandComplete,
    });

    // Check for provider-not-configured errors
    const buf = stripAnsi(entry.outputBuffer);
    if (/not configured|no.*api.*key|error.*provider|missing.*key/i.test(buf)) {
      ctx.logCommand(
        commandId,
        `Provider ${commandId} not configured — set the API key env var to enable.`,
      );
    }

    return { ok: true };
  },
};
