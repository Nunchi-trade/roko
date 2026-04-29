// --- src/lib/scenario-runners/gate-retry.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep, stripAnsi } from '../scenario-helpers';
import { enterWorkspace, showCmd, getRoko, trackMetrics } from '../terminal-session';

export const gateRetry: Scenario = {
  id: 'gate-retry',
  title: 'Gate Retry',
  subtitle: 'Watch a task fail gates, get classified, and retry with an adjusted strategy.',
  panes: 2,
  labels: ['task execution', 'gate status'],
  panel: true,
  promptBar: false,
  category: 'pipeline',
  features: ['Gate failure detection', 'Auto-replan', 'Adjusted retry'],
  durationHint: '~75s',
  accent: 'amber',
  icon: 'gate',
  steps: [
    { label: 'First attempt', sublabel: 'roko run' },
    { label: 'Gate failure', sublabel: 'compile/test/clippy' },
    { label: 'Classification', sublabel: 'transient vs structural' },
    { label: 'Strategy adjust', sublabel: 'replan' },
    { label: 'Retry', sublabel: 'second attempt' },
    { label: 'Pass', sublabel: 'gates green' },
  ],
  async run({ entries, playback, timeline, setMetric, setGate, logCommand, running, paused, workspaceDir }) {
    const [task, gates] = entries;
    await enterWorkspace(task, workspaceDir);
    await enterWorkspace(gates, workspaceDir);

    const ROKO = getRoko();
    const totalSteps = 6;
    const gateNames = ['compile', 'test', 'clippy'] as const;
    const advance = async (stepIndex: number, label: string): Promise<boolean> => {
      await playback.waitForStep();
      if (!running.current) return false;
      while (paused.current) await rawSleep(100);
      timeline.setActive(stepIndex);
      playback.setProgress(stepIndex + 1, totalSteps, label);
      return true;
    };
    const showGateMonitor = async (command: string, customDesc: string, timeout = 30000) => {
      gates.clearTerminal();
      return showCmd(gates, command, {
        timeout,
        onLog: logCommand,
        customDesc,
      });
    };
    const gateFailurePatterns = {
      compile: /compile.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}compile/i,
      test: /\btest\b.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}\btest\b/i,
      clippy: /clippy.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}clippy/i,
    } as const;
    const gatePassPatterns = {
      compile: /compile.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}compile/i,
      test: /\btest\b.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}\btest\b/i,
      clippy: /clippy.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}clippy/i,
    } as const;
    const replanSignals = /\b(?:attempting replan|replan(?:ned|ning)?|classification(?:=|:)?|needs_replan|transient|structural|architectural_conflict_requires_replan)\b/i;

    timeline.init(this.steps);
    setMetric('model', 'cascade');

    await task.execCmd(`${ROKO} config set learning.replan_on_gate_failure true`, 10000);
    task.clearTerminal();

    let taskMetricsTracker: ReturnType<typeof setInterval> | null = trackMetrics(task, {
      onCost: c => setMetric('cost', c),
      onTokens: t => setMetric('tokens', t),
      onGate: (name, status) => setGate(name, status),
    });

    try {
      gateNames.forEach(name => setGate(name, 'pending'));

      const prompt = 'Build a small Rust async HTTP client with exponential backoff, JSON config loading, and focused tests. Keep compile, test, and clippy green.';
      const runCmd = `${ROKO} run "${prompt}" --max-retries 2`;

      if (!(await advance(0, runCmd))) return;
      logCommand(
        'first attempt',
        'Running with gate-failure replanning enabled and a two-retry budget. The left pane executes the task while the right pane shows gate tuning and learning snapshots.',
      );

      const gateOverviewPromise = showGateMonitor(
        `${ROKO} learn tune gates`,
        'Shows the gate tuning and retry policy in the monitoring pane while the first attempt runs.',
      );

      const runResult = await showCmd(task, runCmd, {
        timeout: 360000,
        onLog: logCommand,
        onGate: (name, status) => setGate(name, status),
        customDesc:
          'Runs the task with gate-failure replanning enabled. The first attempt may fail compile, test, or clippy, after which the runner classifies the failure and retries with a revised plan.',
      });

      await gateOverviewPromise;

      const output = stripAnsi(task.getOutputBuffer());
      const failedGateNames = gateNames.filter(name => gateFailurePatterns[name].test(output));
      const passedGateNames = gateNames.filter(name => gatePassPatterns[name].test(output));
      const sawReplanSignals = replanSignals.test(output);
      const failureObserved = failedGateNames.length > 0 || sawReplanSignals;
      const failureThenRetry = failureObserved && runResult.ok;

      if (taskMetricsTracker) {
        clearInterval(taskMetricsTracker);
        taskMetricsTracker = null;
      }

      if (failureThenRetry && failedGateNames.length > 0) {
        for (const name of failedGateNames) setGate(name, 'fail');
        await rawSleep(250);
        for (const name of failedGateNames) setGate(name, 'pass');
      } else if (!failureThenRetry && failedGateNames.length > 0) {
        for (const name of failedGateNames) setGate(name, 'fail');
      } else {
        for (const name of passedGateNames) setGate(name, 'pass');
      }

      if (failureThenRetry) {
        if (!(await advance(1, 'gate failure detected'))) return;
        logCommand(
          'gate failure',
          failedGateNames.length > 0
            ? `Detected failures in ${failedGateNames.join(', ')} on the first attempt. The runner recovered with an internal retry.`
            : 'Detected a gate failure and a successful retry in the same run.',
        );

        await showGateMonitor(
          `${ROKO} status`,
          'Final workspace status after the failure-and-retry cycle. The gate bar reflects the recovered state.',
        );

        if (!(await advance(2, sawReplanSignals ? 'failure classified' : 'retry planner engaged'))) return;
        logCommand(
          'classification',
          sawReplanSignals
            ? 'The runner emitted explicit replan/classification signals before retrying.'
            : 'The runner retried after the gate failure even though the terminal output did not spell out the classification explicitly.',
        );

        await showGateMonitor(
          `${ROKO} learn all`,
          'Learning snapshot after the recovered gate failure. Replan history and task outcomes are visible here.',
        );

        if (!(await advance(3, 'strategy adjusted'))) return;
        logCommand(
          'strategy adjust',
          'The retry used a revised plan instead of repeating the same failing path.',
        );

        await showGateMonitor(
          `${ROKO} learn efficiency`,
          'Efficiency snapshot showing the retry overhead from the recovered run.',
        );

        if (!(await advance(4, 'retry succeeded'))) return;
        logCommand(
          'retry',
          `The adjusted retry completed successfully in ${runResult.elapsed.toFixed(1)}s.`,
        );

        if (!(await advance(5, 'gates green'))) return;
        logCommand(
          'result',
          `Recovered from the gate failure. Cost: ${runResult.cost ?? 'unknown'}; tokens: ${runResult.tokens ?? 'unknown'}.`,
        );
      } else if (failureObserved) {
        if (!(await advance(1, 'gate failure detected'))) return;
        logCommand(
          'gate failure',
          failedGateNames.length > 0
            ? `Detected failures in ${failedGateNames.join(', ')} but the run did not recover within the retry budget.`
            : 'The runner emitted gate-failure signals, but the retry budget was exhausted before recovery.',
        );

        await showGateMonitor(
          `${ROKO} status`,
          'Final workspace status after the failed run. The gate bar stays red because the retry budget was exhausted.',
        );

        if (!(await advance(2, 'no replan recovery'))) return;
        logCommand(
          'classification',
          sawReplanSignals
            ? 'The terminal output included classification signals, but the run still failed before recovery.'
            : 'No successful replan was emitted before the run stopped.',
        );

        await showGateMonitor(
          `${ROKO} learn all`,
          'Learning snapshot after an unrecovered gate failure.',
        );

        if (!(await advance(3, 'strategy unchanged'))) return;
        logCommand(
          'strategy adjust',
          'The retry budget was exhausted before a repaired plan could land.',
        );

        await showGateMonitor(
          `${ROKO} learn efficiency`,
          'Efficiency snapshot showing the failed attempt overhead.',
        );

        if (!(await advance(4, 'retry budget exhausted'))) return;
        logCommand(
          'completion',
          `The run stopped after ${runResult.elapsed.toFixed(1)}s without a successful retry.`,
        );

        if (!(await advance(5, 'final gate status'))) return;
        logCommand(
          'result',
          `Task did not pass all gates. Cost: ${runResult.cost ?? 'unknown'}; tokens: ${runResult.tokens ?? 'unknown'}.`,
        );
      } else {
        if (!(await advance(1, 'first attempt passed'))) return;
        logCommand(
          'first-try pass',
          'The task satisfied compile, test, and clippy on the first attempt, so the replan path stayed idle.',
        );

        await showGateMonitor(
          `${ROKO} status`,
          'Final workspace status after a clean first attempt.',
        );

        if (!(await advance(2, 'no classification needed'))) return;
        logCommand(
          'classification',
          'No gate failure was detected, so no classification or replan was needed.',
        );

        await showGateMonitor(
          `${ROKO} learn all`,
          'Learning snapshot after a clean first attempt.',
        );

        if (!(await advance(3, 'strategy stayed stable'))) return;
        logCommand(
          'strategy adjust',
          'The initial strategy held; no retry was needed.',
        );

        await showGateMonitor(
          `${ROKO} learn efficiency`,
          'Efficiency snapshot showing the baseline overhead for a first-try success.',
        );

        if (!(await advance(4, 'no retry needed'))) return;
        logCommand(
          'completion',
          `Completed in ${runResult.elapsed.toFixed(1)}s without needing a retry.`,
        );

        if (!(await advance(5, 'gates green'))) return;
        logCommand(
          'result',
          `Task passed on the first attempt. Cost: ${runResult.cost ?? 'unknown'}; tokens: ${runResult.tokens ?? 'unknown'}.`,
        );
      }

      if (runResult.cost) setMetric('cost', runResult.cost);
      if (runResult.tokens) setMetric('tokens', runResult.tokens);

      timeline.setActive(6);
    } finally {
      if (taskMetricsTracker) clearInterval(taskMetricsTracker);
    }
  },
};
