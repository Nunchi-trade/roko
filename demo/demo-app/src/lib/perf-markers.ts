/**
 * Performance markers for animation frame profiling.
 *
 * Uses the Performance API (User Timing Level 2) to mark and measure
 * animation-related operations. All marks are prefixed with "roko:" to
 * avoid collisions. No-ops gracefully when the Performance API is unavailable.
 *
 * Usage:
 *   import { markStart, markEnd, measure } from '../lib/perf-markers';
 *
 *   markStart('draw-timeline');
 *   // ... draw operations ...
 *   markEnd('draw-timeline');
 *   const dur = measure('draw-timeline'); // ms or null
 */

const PREFIX = 'roko:';
const perf = typeof performance !== 'undefined' ? performance : null;

/** Place a start mark for the named operation. */
export function markStart(name: string): void {
  perf?.mark(`${PREFIX}${name}:start`);
}

/** Place an end mark for the named operation. */
export function markEnd(name: string): void {
  perf?.mark(`${PREFIX}${name}:end`);
}

/**
 * Measure the duration between start and end marks.
 * Returns duration in milliseconds, or null if marks are missing.
 */
export function measure(name: string): number | null {
  if (!perf) return null;
  const startMark = `${PREFIX}${name}:start`;
  const endMark = `${PREFIX}${name}:end`;
  try {
    const entry = perf.measure(`${PREFIX}${name}`, startMark, endMark);
    return entry.duration;
  } catch {
    return null;
  }
}

/**
 * Clear all roko-prefixed marks and measures.
 * Call periodically to prevent memory buildup in long sessions.
 */
export function clearMarks(): void {
  if (!perf) return;
  perf.getEntriesByType('mark').forEach((entry) => {
    if (entry.name.startsWith(PREFIX)) perf.clearMarks(entry.name);
  });
  perf.getEntriesByType('measure').forEach((entry) => {
    if (entry.name.startsWith(PREFIX)) perf.clearMeasures(entry.name);
  });
}

/**
 * Time a callback and return its result plus the duration in ms.
 * Useful for wrapping draw functions:
 *
 *   const [result, ms] = timeCall('draw-hero', () => drawTimeline());
 */
export function timeCall<T>(name: string, fn: () => T): [T, number | null] {
  markStart(name);
  const result = fn();
  markEnd(name);
  const duration = measure(name);
  return [result, duration];
}

/**
 * Log a performance warning if a frame exceeds the given budget.
 * Defaults to 16ms (60fps budget).
 */
export function warnIfSlow(name: string, durationMs: number | null, budgetMs = 16): void {
  if (durationMs !== null && durationMs > budgetMs) {
    console.warn(
      `[perf] ${name} took ${durationMs.toFixed(1)}ms (budget: ${budgetMs}ms)`,
    );
  }
}
