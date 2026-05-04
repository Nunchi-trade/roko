// --- src/lib/scenario-runners/dream-consolidation.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { stripAnsi } from '../scenario-helpers';
import { showCmd, roko } from '../terminal-session';

// ── Static command definitions (display layer, no ctx needed) ─

export const DREAM_COMMANDS: CommandDef[] = [
  { id: 'schedule',       command: 'roko knowledge dream schedule',                                                       description: 'Check dream schedule',                                        timeout: 30000,  target: { pane: 0 } },
  { id: 'baseline-stats', command: 'roko knowledge stats',                                                                description: 'Baseline knowledge stats',                                     timeout: 30000,  target: { pane: 1 } },
  { id: 'seed',           command: 'roko run "Build a small Rust CLI that reads JSON from stdin and prints a summary"',   description: 'Seed episodes for dream cycle',                                timeout: 180000, target: { pane: 0 } },
  { id: 'dream-run',      command: 'roko knowledge dream run',                                                            description: 'Full dream cycle (hypnagogia → NREM → REM → integration)',    timeout: 300000, target: { pane: 0 } },
  { id: 'dream-report',   command: 'roko knowledge dream report',                                                         description: 'Dream consolidation report',                                   timeout: 30000,  target: { pane: 0 } },
  { id: 'final-stats',    command: 'roko knowledge stats',                                                                description: 'Post-dream knowledge stats',                                   timeout: 30000,  target: { pane: 1 } },
];

// ── Scenario ─────────────────────────────────────────────────

export const dreamConsolidation: ClickableScenario = {
  id: 'dream-consolidation',
  title: 'Dream Cycle',
  subtitle: 'Offline consolidation - episodes distilled into durable knowledge.',
  panes: 2,
  labels: ['dream engine', 'knowledge monitor'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Offline consolidation', 'Episode distillation', 'Durable knowledge'],
  durationHint: '~60s',
  accent: 'violet',
  icon: 'dream',
  steps: [
    { label: 'Trigger check', sublabel: 'dream schedule' },
    { label: 'Seed episodes', sublabel: 'roko run' },
    { label: 'Hypnagogia', sublabel: 'replay selection' },
    { label: 'NREM clustering', sublabel: 'pattern extraction' },
    { label: 'REM synthesis', sublabel: 'creative linking' },
    { label: 'Integration', sublabel: 'knowledge merge' },
    { label: 'Report', sublabel: 'dream report' },
  ],
  commands: DREAM_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const [pane0, pane1] = ctx.entries;
    if (!pane0) return { ok: false, error: 'No terminal connected' };

    switch (commandId) {
      case 'schedule': {
        const result = await showCmd(pane0, roko(ctx, 'knowledge dream schedule'), {
          timeout: 30000,
          customDesc: 'Check dream schedule',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      }

      case 'baseline-stats': {
        const target = pane1 ?? pane0;
        const result = await showCmd(target, roko(ctx, 'knowledge stats'), {
          timeout: 30000,
          customDesc: 'Baseline knowledge stats',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      }

      case 'seed': {
        const result = await showCmd(
          pane0,
          roko(ctx, 'run "Build a small Rust CLI that reads JSON from stdin and prints a summary"'),
          {
            timeout: 180000,
            customDesc: 'Seed episodes for dream cycle',
            workspaceDir: ctx.workspaceDir,
            signal: ctx.signal,
          },
        );
        if (result.cost) ctx.setMetric('cost', result.cost);
        if (result.tokens) ctx.setMetric('tokens', result.tokens);
        return { ok: result.ok, error: result.error };
      }

      case 'dream-run': {
        const result = await showCmd(pane0, roko(ctx, 'knowledge dream run'), {
          timeout: 300000,
          customDesc: 'Full dream cycle (hypnagogia → NREM → REM → integration)',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        // outputBuffer still holds the last command's output until the next command clears it
        const output = stripAnsi(pane0.outputBuffer);
        ctx.setGate('hypnagogia', /hypnagog|replay|select/i.test(output) ? 'pass' : 'fail');
        ctx.setGate('nrem',       /nrem|cluster|pattern/i.test(output)    ? 'pass' : 'fail');
        ctx.setGate('rem',        /rem|synth|link|associat/i.test(output)  ? 'pass' : 'fail');
        ctx.setGate('integration', result.ok ? 'pass' : 'fail');
        if (result.cost) ctx.setMetric('cost', result.cost);
        if (result.tokens) ctx.setMetric('tokens', result.tokens);
        return { ok: result.ok, error: result.error };
      }

      case 'dream-report': {
        pane0.clearTerminal();
        const result = await showCmd(pane0, roko(ctx, 'knowledge dream report'), {
          timeout: 30000,
          customDesc: 'Dream consolidation report',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      }

      case 'final-stats': {
        const target = pane1 ?? pane0;
        target.clearTerminal();
        const result = await showCmd(target, roko(ctx, 'knowledge stats'), {
          timeout: 30000,
          customDesc: 'Post-dream knowledge stats',
          workspaceDir: ctx.workspaceDir,
          signal: ctx.signal,
        });
        return { ok: result.ok, error: result.error };
      }

      default:
        return { ok: false, error: 'Unknown command' };
    }
  },
};
