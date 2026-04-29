// --- src/lib/scenario-runners/knowledge-accumulation.ts ---
import type { Scenario } from '../scenarios';
import { enterWorkspace, showCmd, getRoko } from '../terminal-session';

export const knowledgeAccumulation: Scenario = {
  id: 'knowledge-accumulation',
  title: 'Knowledge Growth',
  subtitle: 'Watch the knowledge store grow across successive runs.',
  panes: 2,
  labels: ['task runner', 'knowledge store'],
  panel: true,
  promptBar: false,
  category: 'learning',
  features: ['Knowledge store growth', 'Successive runs', 'Tier progression'],
  durationHint: '~90s',
  accent: 'emerald',
  icon: 'knowledge',
  steps: [
    { label: 'Initial query', sublabel: 'empty store' },
    { label: 'Run 1', sublabel: 'build a CLI tool' },
    { label: 'Knowledge check', sublabel: 'query after run 1' },
    { label: 'Run 2', sublabel: 'add error handling' },
    { label: 'Knowledge growth', sublabel: 'query after run 2' },
    { label: 'Final state', sublabel: 'knowledge stats' },
  ],
  async run({ entries, playback, timeline, setMetric, logCommand, workspaceDir }) {
    const [runner, knowledge] = entries;

    await enterWorkspace(runner, workspaceDir);
    await enterWorkspace(knowledge, workspaceDir);
    const ROKO = getRoko();

    timeline.init(this.steps);
    setMetric('model', 'cascade');

    // Step 1: Initial query shows an empty store.
    await playback.waitForStep();
    timeline.setActive(0);
    playback.setProgress(1, 6, `${ROKO} knowledge stats`);
    logCommand(
      'knowledge stats',
      'Checking the initial state of the neuro knowledge store. The store should be empty before any task runs.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Checks the initial knowledge store state before any runs.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "error handling patterns"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for error handling patterns before the store has accumulated any knowledge.',
    });
    knowledge.clearTerminal();

    // Step 2: First run grows the store.
    await playback.waitForStep();
    timeline.setActive(1);
    playback.setProgress(2, 6, `${ROKO} run "Build a Rust CLI..."`);
    logCommand(
      'run 1',
      'First task run builds a small Rust CLI. Episodes and efficiency data flow into the knowledge store.',
    );
    const firstRun = await showCmd(runner, `${ROKO} run "Build a Rust CLI that parses JSON from stdin"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'First run: builds a JSON parser CLI and seeds the knowledge store with reusable patterns.',
    });
    if (firstRun.cost) setMetric('cost', firstRun.cost);
    if (firstRun.tokens) setMetric('tokens', firstRun.tokens);

    // Step 3: Query after run 1 should return richer results.
    await playback.waitForStep();
    timeline.setActive(2);
    playback.setProgress(3, 6, `${ROKO} knowledge query ...`);
    knowledge.clearTerminal();
    logCommand(
      'knowledge check 1',
      'Querying the store after the first run. The results should now include JSON parsing and CLI patterns.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the store after one run has added entries.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "JSON parsing"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for JSON parsing after the first run has seeded relevant knowledge.',
    });

    // Step 4: Second run compounds the store.
    await playback.waitForStep();
    timeline.setActive(3);
    playback.setProgress(4, 6, `${ROKO} run "Add error handling..."`);
    runner.clearTerminal();
    logCommand(
      'run 2',
      'Second task adds error handling to the existing code. Knowledge compounds with the first run.',
    );
    const secondRun = await showCmd(runner, `${ROKO} run "Add comprehensive error handling with anyhow and thiserror"`, {
      timeout: 180000,
      onLog: logCommand,
      customDesc: 'Second run: adds error handling and grows the knowledge store again.',
    });
    if (secondRun.cost) setMetric('cost', secondRun.cost);
    if (secondRun.tokens) setMetric('tokens', secondRun.tokens);

    // Step 5: Query after run 2 should be richer again.
    await playback.waitForStep();
    timeline.setActive(4);
    playback.setProgress(5, 6, `${ROKO} knowledge query ...`);
    knowledge.clearTerminal();
    logCommand(
      'knowledge check 2',
      'After two runs the store should have accumulated entries across both tasks.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows the store after two runs have accumulated more knowledge.',
    });
    await showCmd(knowledge, `${ROKO} knowledge query "error handling patterns"`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Queries for error handling patterns after the second run has added more knowledge.',
    });

    // Step 6: Final state shows learning surface area.
    await playback.waitForStep();
    timeline.setActive(5);
    playback.setProgress(6, 6, `${ROKO} knowledge stats`);
    knowledge.clearTerminal();
    logCommand(
      'final state',
      'Final knowledge store statistics after two accumulation cycles.',
    );
    await showCmd(knowledge, `${ROKO} knowledge stats`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Final stats for the knowledge store after two task runs.',
    });
    await showCmd(knowledge, `${ROKO} learn all`, {
      timeout: 30000,
      onLog: logCommand,
      customDesc: 'Shows all learning state after the accumulated runs.',
    });

    timeline.setActive(6);
  },
};
