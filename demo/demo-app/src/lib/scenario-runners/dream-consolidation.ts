// --- src/lib/scenario-runners/dream-consolidation.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep } from '../scenario-helpers';
import { enterWorkspace, showCmd, roko, stripAnsi } from '../terminal-session';

export const dreamConsolidation: Scenario = {
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
  async run(ctx) {
    const { entries, playback, timeline, setMetric, setGate, logCommand, logCommandComplete, signal, workspaceDir } = ctx;
    const [dream, monitor] = entries;
    await enterWorkspace(dream, workspaceDir);
    await enterWorkspace(monitor, workspaceDir);
    const totalSteps = this.steps.length;

    timeline.init(this.steps);
    setMetric('model', '--');
    setMetric('cost', '--');
    setMetric('tokens', '--');
    setGate('hypnagogia', 'pending');
    setGate('nrem', 'pending');
    setGate('rem', 'pending');
    setGate('integration', 'pending');

    const waitForStep = async (stepIndex: number, label: string): Promise<boolean> => {
      await playback.waitForStep();
      if (signal.aborted) return false;
      timeline.setActive(stepIndex);
      playback.setProgress(stepIndex + 1, totalSteps, label);
      return true;
    };

    /**
     * Poll the dream terminal's output buffer for a phase keyword.
     * Returns true if the phase was detected, false on abort/timeout.
     */
    const waitForPhaseInOutput = async (
      phase: string,
      timeoutMs = 60000,
    ): Promise<boolean> => {
      const pattern = new RegExp(phase, 'i');
      const start = Date.now();
      while (Date.now() - start < timeoutMs) {
        if (signal.aborted) return false;
        const text = stripAnsi(dream.outputBuffer);
        if (pattern.test(text)) return true;
        await rawSleep(250);
      }
      // Timed out — the phase keyword wasn't detected, but don't fake it
      return false;
    };

    // Step 0: show schedule and baseline store state.
    if (!(await waitForStep(0, roko(ctx, 'knowledge dream schedule')))) return;
    logCommand(
      'dream schedule',
      'Checking the consolidation cadence before the cycle begins.',
    );
    const scheduleResult = await showCmd(dream, roko(ctx, 'knowledge dream schedule'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Shows the current dream consolidation schedule, including when the next cycle is due.',
    });
    if (scheduleResult.cost) setMetric('cost', scheduleResult.cost);
    if (scheduleResult.tokens) setMetric('tokens', scheduleResult.tokens);
    await showCmd(monitor, roko(ctx, 'knowledge stats'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Baseline knowledge stats before the dream cycle starts.',
    });

    // Step 1: seed a recent episode run so the dream cycle has material to distill.
    if (!(await waitForStep(1, roko(ctx, 'run "..."')))) return;
    logCommand(
      'seed episodes',
      'Running a fresh task so the dream cycle can consolidate new episodes into durable knowledge.',
    );
    const seedResult = await showCmd(
      dream,
      roko(ctx, 'run "Build a small Rust CLI that reads JSON from stdin and prints a summary"'),
      {
        playback,
        timeout: 180000,
        onLog: logCommand,
        onLogComplete: logCommandComplete,
        customDesc: 'Seeds the episode store with a live task run. The later dream cycle will distill this run into reusable knowledge.',
      },
    );
    if (seedResult.cost) setMetric('cost', seedResult.cost);
    if (seedResult.tokens) setMetric('tokens', seedResult.tokens);

    // Step 2: hypnagogia - replay selection while the dream run is in flight.
    if (!(await waitForStep(2, roko(ctx, 'knowledge dream run')))) return;
    logCommand(
      'hypnagogia',
      'Phase 1: replaying recent episodes and selecting the salient ones.',
    );
    const dreamRunPromise = showCmd(dream, roko(ctx, 'knowledge dream run'), {
      playback,
      timeout: 300000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Runs the full dream cycle: hypnagogia replay selection, NREM clustering, REM synthesis, and final integration.',
    });
    // Wait for real hypnagogia phase output from dream run
    const hypnOk = await waitForPhaseInOutput('hypnagog|replay|select', 30000);
    setGate('hypnagogia', hypnOk ? 'pass' : 'fail');
    await showCmd(monitor, roko(ctx, 'knowledge query "episode clusters"'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Mid-dream knowledge query while the cycle is still replaying and selecting episodes.',
    });

    // Step 3: NREM clustering - group related episodes into patterns.
    if (!(await waitForStep(3, 'NREM clustering'))) return;
    logCommand(
      'NREM',
      'Phase 2: clustering recurring patterns from the selected episodes.',
    );
    // Wait for real NREM/cluster phase output
    const nremOk = await waitForPhaseInOutput('nrem|cluster|pattern', 45000);
    setGate('nrem', nremOk ? 'pass' : 'fail');
    await showCmd(monitor, roko(ctx, 'knowledge stats'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Knowledge stats while clustering is underway. This is the live monitoring pane for the dream cycle.',
    });

    // Step 4: REM synthesis - generate new associations across clusters.
    if (!(await waitForStep(4, 'REM synthesis'))) return;
    logCommand(
      'REM',
      'Phase 3: linking clusters into new associations and candidate playbooks.',
    );
    // Wait for real REM/synthesis phase output
    const remOk = await waitForPhaseInOutput('rem|synth|link|associat', 45000);
    setGate('rem', remOk ? 'pass' : 'fail');
    await showCmd(monitor, roko(ctx, 'knowledge query "consolidation patterns"'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Mid-dream query that should surface newly synthesized consolidation patterns.',
    });

    // Step 5: integration - wait for the dream run to finish and merge the distilled output.
    if (!(await waitForStep(5, 'integration...'))) return;
    logCommand(
      'integration',
      'Phase 4: merging the distilled knowledge back into the durable store.',
    );
    const dreamResult = await dreamRunPromise;
    setGate('integration', dreamResult.ok ? 'pass' : 'fail');
    if (dreamResult.cost) setMetric('cost', dreamResult.cost);
    if (dreamResult.tokens) setMetric('tokens', dreamResult.tokens);

    // Step 6: report - clean panes and show the final consolidated state.
    if (!(await waitForStep(6, roko(ctx, 'knowledge dream report')))) return;
    dream.clearTerminal();
    monitor.clearTerminal();
    logCommand(
      'dream report',
      'Viewing the consolidation report after the cycle has merged distilled knowledge into the store.',
    );
    await showCmd(dream, roko(ctx, 'knowledge dream report'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Shows the dream consolidation report with the latest episode selection, cluster formation, synthesis, and integration details.',
    });
    await showCmd(monitor, roko(ctx, 'knowledge stats'), {
      playback,
      timeout: 30000,
      onLog: logCommand,
      onLogComplete: logCommandComplete,
      customDesc: 'Post-dream knowledge stats that reflect the newly consolidated state.',
    });
    setMetric('model', 'done');

    timeline.markAllComplete();
  },
};
