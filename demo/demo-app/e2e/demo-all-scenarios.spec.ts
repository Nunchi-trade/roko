import { test, expect } from '@playwright/test';
import { testScenario } from './helpers';

/**
 * Verify every demo scenario can start and make progress.
 * Each scenario gets its own fresh page to avoid terminal session contention.
 *
 * The demo has been collapsed from 14 scenarios to 5:
 *   Cost, Pipeline, Memory, ISFR, Oracle
 */

const SCENARIOS = [
  { idx: 0, name: 'Cost', panes: 2 },
  { idx: 1, name: 'Pipeline', panes: 1 },
  { idx: 2, name: 'Memory', panes: 2 },
  { idx: 3, name: 'ISFR', panes: 4 },
  { idx: 4, name: 'Oracle', panes: 2 },
];

test.describe('All scenarios', () => {
  for (const scenario of SCENARIOS) {
    test(`${scenario.name} starts and makes progress`, async ({ page }) => {
      test.setTimeout(120_000);
      const progress = await testScenario(page, scenario.idx, scenario.panes);
      console.log(`${scenario.name}: ${progress}`);
      expect(progress).not.toBe('press Play to begin');
    });
  }
});
