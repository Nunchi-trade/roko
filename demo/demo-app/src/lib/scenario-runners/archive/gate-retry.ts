// --- src/lib/scenario-runners/gate-retry.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../../scenarios';
import { stripAnsi } from '../../scenario-helpers';
import { showCmd, roko, trackMetrics } from '../../terminal-session';

// ── Module-level state for tracking run outcome ───────────────

let runOutcome: {
  ok: boolean;
  elapsed: number;
  cost: string | null;
  tokens: string | null;
  failedGates: string[];
  passedGates: string[];
  sawReplan: boolean;
} | null = null;

export function resetGateRetryState() {
  runOutcome = null;
}

// ── Static command definitions ────────────────────────────────

export const GATE_RETRY_COMMANDS: CommandDef[] = [
  { id: 'configure',       command: 'roko config set learning.replan_on_gate_failure true', description: 'Enable gate-failure replanning',                                              timeout: 10000,  target: { pane: 0 } },
  { id: 'run-task',        command: 'roko run "Build a small Rust async HTTP client with exponential backoff, JSON config loading, and focused tests. Keep compile, test, and clippy green." --max-retries 2', description: 'Run task with retry budget', timeout: 360000, target: { pane: 0 } },
  { id: 'gate-overview',   command: 'roko learn tune gates',                                description: 'Gate tuning and retry policy',                                                timeout: 30000,  target: { pane: 1 } },
  { id: 'status',          command: 'roko status',                                          description: 'Workspace status',                                                            timeout: 30000,  target: { pane: 1 } },
  { id: 'learn-all',       command: 'roko learn all',                                       description: 'Learning snapshot',                                                           timeout: 30000,  target: { pane: 1 } },
  { id: 'learn-efficiency',command: 'roko learn efficiency',                                description: 'Efficiency metrics',                                                          timeout: 30000,  target: { pane: 1 } },
];

// ── Gate detection patterns (same as original) ────────────────

const gateNames = ['compile', 'test', 'clippy'] as const;

const gateFailurePatterns = {
  compile: /compile.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}compile/i,
  test:    /\btest\b.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}\btest\b/i,
  clippy:  /clippy.{0,80}(?:fail(?:ed|ure)?|error|[✖✗])|(?:[✖✗]).{0,80}clippy/i,
} as const;

const gatePassPatterns = {
  compile: /compile.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}compile/i,
  test:    /\btest\b.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}\btest\b/i,
  clippy:  /clippy.{0,80}(?:pass(?:ed)?|ok|[✔✓])|(?:[✔✓]).{0,80}clippy/i,
} as const;

const replanSignals = /\b(?:attempting replan|replan(?:ned|ning)?|classification(?:=|:)?|needs_replan|transient|structural|architectural_conflict_requires_replan)\b/i;

// ── Scenario ──────────────────────────────────────────────────

export const gateRetry: ClickableScenario = {
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
  resetState: resetGateRetryState,
  steps: [
    { label: 'First attempt',     sublabel: 'roko run' },
    { label: 'Gate failure',      sublabel: 'compile/test/clippy' },
    { label: 'Classification',    sublabel: 'transient vs structural' },
    { label: 'Strategy adjust',   sublabel: 'replan' },
    { label: 'Retry',             sublabel: 'second attempt' },
    { label: 'Pass',              sublabel: 'gates green' },
  ],
  commands: GATE_RETRY_COMMANDS,

  async runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }> {
    const { entries, setMetric, setGate, logCommand, logCommandComplete, signal } = ctx;
    const [task, gates] = entries;

    switch (commandId) {
      case 'configure': {
        if (!task) return { ok: false, error: 'No pane 0 connected' };
        await task.execCmd(roko(ctx, 'config set learning.replan_on_gate_failure true'), 10000);
        gateNames.forEach(name => setGate(name, 'pending'));
        task.clearTerminal();
        return { ok: true };
      }

      case 'run-task': {
        if (!task) return { ok: false, error: 'No pane 0 connected' };

        const tracker = trackMetrics(task, {
          onCost: c => setMetric('cost', c),
          onTokens: t => setMetric('tokens', t),
          onGate: (name, status) => setGate(name, status),
        });

        try {
          gateNames.forEach(name => setGate(name, 'pending'));

          const prompt = 'Build a small Rust async HTTP client with exponential backoff, JSON config loading, and focused tests. Keep compile, test, and clippy green.';
          const runCmd = roko(ctx, `run "${prompt}" --max-retries 2`);

          const result = await showCmd(task, runCmd, {
            timeout: 360000,
            onLog: logCommand,
            onLogComplete: logCommandComplete,
            onGate: (name, status) => setGate(name, status),
            signal,
            customDesc:
              'Runs the task with gate-failure replanning enabled. The first attempt may fail compile, test, or clippy, after which the runner classifies the failure and retries with a revised plan.',
          });

          const output = stripAnsi(task.getOutputBuffer());
          const failedGateNames = gateNames.filter(name => gateFailurePatterns[name].test(output));
          const passedGateNames = gateNames.filter(name => gatePassPatterns[name].test(output));
          const sawReplanDetected = replanSignals.test(output);
          const failureObserved = failedGateNames.length > 0 || sawReplanDetected;
          const failureThenRetry = failureObserved && result.ok;

          // Store outcome for subsequent commands
          runOutcome = {
            ok: result.ok,
            elapsed: result.elapsed,
            cost: result.cost ?? null,
            tokens: result.tokens ?? null,
            failedGates: [...failedGateNames],
            passedGates: [...passedGateNames],
            sawReplan: sawReplanDetected,
          };

          // Set gate statuses based on detected patterns
          if (failureThenRetry && failedGateNames.length > 0) {
            for (const name of failedGateNames) setGate(name, 'fail');
            await new Promise(r => setTimeout(r, 250));
            for (const name of failedGateNames) setGate(name, 'pass');
          } else if (!failureThenRetry && failedGateNames.length > 0) {
            for (const name of failedGateNames) setGate(name, 'fail');
          } else {
            for (const name of passedGateNames) setGate(name, 'pass');
          }

          return { ok: result.ok, error: result.error };
        } finally {
          clearInterval(tracker);
        }
      }

      case 'gate-overview': {
        if (!gates) return { ok: false, error: 'No pane 1 connected' };
        gates.clearTerminal();
        const result = await showCmd(gates, roko(ctx, 'learn tune gates'), {
          timeout: 30000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          signal,
          customDesc: 'Gate tuning and retry policy',
        });
        return { ok: result.ok, error: result.error };
      }

      case 'status': {
        if (!gates) return { ok: false, error: 'No pane 1 connected' };
        gates.clearTerminal();
        const statusDesc = runOutcome
          ? runOutcome.failedGates.length > 0 && runOutcome.ok
            ? 'Final workspace status after the failure-and-retry cycle. The gate bar reflects the recovered state.'
            : runOutcome.failedGates.length > 0
            ? 'Final workspace status after the failed run. The gate bar stays red because the retry budget was exhausted.'
            : 'Final workspace status after a clean first attempt.'
          : 'Workspace status';
        const result = await showCmd(gates, roko(ctx, 'status'), {
          timeout: 30000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          signal,
          customDesc: statusDesc,
        });
        return { ok: result.ok, error: result.error };
      }

      case 'learn-all': {
        if (!gates) return { ok: false, error: 'No pane 1 connected' };
        gates.clearTerminal();
        const learnDesc = runOutcome
          ? runOutcome.failedGates.length > 0 && runOutcome.ok
            ? 'Learning snapshot after the recovered gate failure. Replan history and task outcomes are visible here.'
            : runOutcome.failedGates.length > 0
            ? 'Learning snapshot after an unrecovered gate failure.'
            : 'Learning snapshot after a clean first attempt.'
          : 'Learning snapshot';
        const result = await showCmd(gates, roko(ctx, 'learn all'), {
          timeout: 30000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          signal,
          customDesc: learnDesc,
        });
        return { ok: result.ok, error: result.error };
      }

      case 'learn-efficiency': {
        if (!gates) return { ok: false, error: 'No pane 1 connected' };
        gates.clearTerminal();
        const effDesc = runOutcome
          ? runOutcome.failedGates.length > 0 && runOutcome.ok
            ? 'Efficiency snapshot showing the retry overhead from the recovered run.'
            : runOutcome.failedGates.length > 0
            ? 'Efficiency snapshot showing the failed attempt overhead.'
            : 'Efficiency snapshot showing the baseline overhead for a first-try success.'
          : 'Efficiency metrics';
        const result = await showCmd(gates, roko(ctx, 'learn efficiency'), {
          timeout: 30000,
          onLog: logCommand,
          onLogComplete: logCommandComplete,
          signal,
          customDesc: effDesc,
        });
        // Set final cost/tokens metrics from stored runOutcome
        if (runOutcome) {
          if (runOutcome.cost) setMetric('cost', runOutcome.cost);
          if (runOutcome.tokens) setMetric('tokens', runOutcome.tokens);
        }
        return { ok: result.ok, error: result.error };
      }

      default:
        return { ok: false, error: 'Unknown command' };
    }
  },
};
