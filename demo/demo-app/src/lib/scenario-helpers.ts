// --- src/lib/scenario-helpers.ts ---
// Helper functions extracted from scenarios.ts for reuse by individual runner files.

import type { PipelineEvent, PipelinePhase } from './prd-pipeline-types';

/** Sleep that bypasses playback speed multiplier. */
export function rawSleep(ms: number): Promise<void> {
  return new Promise(r => setTimeout(r, ms));
}

export { stripAnsi } from './strip-ansi';

/** Format current time as compact HH:MM:SS string. */
export function compactTime(): string {
  return new Date().toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

/** Monotonic sequence counter for pipeline events. */
let pipelineEventSeq = 0;

/** Create a PipelineEvent with auto-incrementing ID and current timestamp. */
export function pipelineEvent(
  phase: PipelinePhase,
  text: string,
  kind: PipelineEvent['kind'] = 'info',
): PipelineEvent {
  pipelineEventSeq += 1;
  return {
    id: `pipe-${Date.now()}-${pipelineEventSeq}`,
    ts: compactTime(),
    phase,
    text,
    kind,
  };
}
