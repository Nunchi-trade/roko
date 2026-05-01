import { test, expect } from '@playwright/test';
import { testScenario } from './helpers';

/**
 * Verify every demo scenario can start and make progress.
 * Each scenario gets its own fresh page to avoid terminal session contention.
 */

const SCENARIOS = [
  { idx: 0, name: 'PRD Pipeline', panes: 1, pipeline: true },
  { idx: 1, name: 'Research Loop', panes: 1 },
  { idx: 2, name: 'Cost Race', panes: 2 },
  { idx: 3, name: 'Gate Retry', panes: 2 },
  { idx: 4, name: 'Providers', panes: 4 },
  { idx: 5, name: 'Provider Race', panes: 4 },
  { idx: 6, name: 'Explore', panes: 4 },
  { idx: 7, name: 'Knowledge Growth', panes: 2 },
  { idx: 8, name: 'Dream Cycle', panes: 2 },
  { idx: 9, name: 'Chat', panes: 1 },
  { idx: 10, name: 'Knowledge Transfer', panes: 2 },
  { idx: 11, name: 'Chain Intelligence', panes: 2 },
  { idx: 12, name: 'Mirage', panes: 1 },
  { idx: 13, name: 'ISFR Agents', panes: 2 },
];

test.describe('All scenarios', () => {
  for (const scenario of SCENARIOS) {
    test(`${scenario.name} starts and makes progress`, async ({ page }) => {
      test.setTimeout(120_000);
      const progress = await testScenario(page, scenario.idx, scenario.panes, scenario.pipeline);
      console.log(`${scenario.name}: ${progress}`);
      expect(progress).not.toBe('press Play to begin');
    });
  }
});
