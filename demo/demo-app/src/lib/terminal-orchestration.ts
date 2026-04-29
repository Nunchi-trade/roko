// --- src/lib/terminal-orchestration.ts ---
// Re-exports shared utilities that scenario runners need.
// This file provides a single import point for runner files instead of
// importing from both terminal-session.ts and scenario-helpers.ts.

export {
  setSpeedMultiplier,
  resolveRoko,
  getRoko,
  setupWorkspace,
  joinWorkspace,
  enterWorkspace,
  showCmd,
  trackMetrics,
} from './terminal-session';

export {
  rawSleep,
  stripAnsi,
  compactTime,
  pipelineEvent,
} from './scenario-helpers';
