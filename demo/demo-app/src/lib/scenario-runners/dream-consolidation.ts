// --- src/lib/scenario-runners/dream-consolidation.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

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
  async run({ entries, playback, timeline, setMetric, setGate, logCommand, running, paused, workspaceDir }) {
    const [dream, monitor] = entries;
    await enterWorkspace(dream, workspaceDir);
    await enterWorkspace(monitor, workspaceDir);
    const ROKO = getRoko();
    const totalSteps = this.steps.length;

    timeline.init(this.steps);
    setMetric('model', 'dream engine');
    setMetric('cost', 'pending');
    setMetric('tokens', 'pending');
    setGate('hypnagogia', 'pending');
    setGate('nrem', 'pending');
    setGate('rem', 'pending');
    setGate('integration', 'pending');

    const waitForStep = async (stepIndex: number, label: string): Promise<boolean> => {
      await playback.waitForStep();
      if (!running.current) return false;
      while (paused.current) await rawSleep(100);
      timeline.setActive(stepIndex);
      playback.setProgress(stepIndex + 1, totalSteps, label);
      return true;
    };

    const allowPhaseTime = async (ms: number): Promise<boolean> => {
      const start = Date.now();
      while (Date.now() - start < ms) {
        if (!running.current) return false;
        while (paused.current) await rawSleep(100);
        await rawSleep(100);
      }
      return true;
    };

    // Step 0: show schedule and baseline store state.
    if (!(await waitForStep(0, `${ROKO} knowledge dream schedule`))) return;
    logCommand(
      'dream schedule',
      'Checking the consolidation cadence before the cycle begins.',
    );
    const scheduleResult = await showCmd(dream, `${ROKO} knowledge dream schedule`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the current dream consolidation schedule, including when the next cycle is due.',
    });
    if (scheduleResult.cost) setMetric('cost', scheduleResult.cost);
    if (scheduleResult.tokens) setMetric('tokens', scheduleResult.tokens);
    await showCmd(monitor, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Baseline knowledge stats before the dream cycle starts.',
    });

    // Step 1: seed a recent episode run so the dream cycle has material to distill.
    if (!(await waitForStep(1, `${ROKO} run "..."`))) return;
    logCommand(
      'seed episodes',
      'Running a fresh task so the dream cycle can consolidate new episodes into durable knowledge.',
    );
    const seedResult = await showCmd(
      dream,
      `${ROKO} run "Build a small Rust CLI that reads JSON from stdin and prints a summary"`,
      {
        timeout: 180000,
        onLog: logCommand,
        customDesc: 'Seeds the episode store with a live task run. The later dream cycle will distill this run into reusable knowledge.',
      },
    );
    if (seedResult.cost) setMetric('cost', seedResult.cost);
    if (seedResult.tokens) setMetric('tokens', seedResult.tokens);

    // Step 2: hypnagogia - replay selection while the dream run is in flight.
    if (!(await waitForStep(2, `${ROKO} knowledge dream run`))) return;
    logCommand(
      'hypnagogia',
      'Phase 1: replaying recent episodes and selecting the salient ones.',
    );
    const dreamRunPromise = showCmd(dream, `${ROKO} knowledge dream run`, {
      timeout: 300000,
      onLog: logCommand,
      customDesc: 'Runs the full dream cycle: hypnagogia replay selection, NREM clustering, REM synthesis, and final integration.',
    });
    if (!(await allowPhaseTime(1500))) return;
    setGate('hypnagogia', 'pass');
    await showCmd(monitor, `${ROKO} knowledge query "episode clusters"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Mid-dream knowledge query while the cycle is still replaying and selecting episodes.',
    });

    // Step 3: NREM clustering - group related episodes into patterns.
    if (!(await waitForStep(3, 'NREM clustering'))) return;
    logCommand(
      'NREM',
      'Phase 2: clustering recurring patterns from the selected episodes.',
    );
    if (!(await allowPhaseTime(1200))) return;
    setGate('nrem', 'pass');
    await showCmd(monitor, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Knowledge stats while clustering is underway. This is the live monitoring pane for the dream cycle.',
    });

    // Step 4: REM synthesis - generate new associations across clusters.
    if (!(await waitForStep(4, 'REM synthesis'))) return;
    logCommand(
      'REM',
      'Phase 3: linking clusters into new associations and candidate playbooks.',
    );
    if (!(await allowPhaseTime(1200))) return;
    setGate('rem', 'pass');
    await showCmd(monitor, `${ROKO} knowledge query "consolidation patterns"`, {
      timeout: 30000,
      onLog: logCommand,
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
    if (!(await waitForStep(6, `${ROKO} knowledge dream report`))) return;
    dream.clearTerminal();
    monitor.clearTerminal();
    logCommand(
      'dream report',
      'Viewing the consolidation report after the cycle has merged distilled knowledge into the store.',
    );
    await showCmd(dream, `${ROKO} knowledge dream report`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the dream consolidation report with the latest episode selection, cluster formation, synthesis, and integration details.',
    });
    await showCmd(monitor, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Post-dream knowledge stats that reflect the newly consolidated state.',
    });
    setMetric('model', 'consolidated');

    timeline.setActive(totalSteps);
  },
};
