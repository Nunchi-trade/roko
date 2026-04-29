import { test, expect, type Page } from '@playwright/test';

/**
 * Verify every demo scenario can start and make progress.
 * Each scenario gets its own fresh page to avoid terminal session contention.
 */

async function gotoDemo(page: Page) {
  await page.goto('/demo', { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(8000);
}

async function waitForServe(page: Page) {
  await expect(
    page.locator('.demo-serve-status').filter({ hasText: 'serve live' }),
  ).toBeVisible({ timeout: 15000 });
}

async function switchTab(page: Page, idx: number) {
  await page.locator('.demo-tab').nth(idx).click();
  await page.waitForTimeout(3000);
}

async function waitForTerminals(page: Page, count: number) {
  for (let i = 0; i < count; i++) {
    await expect(page.locator('.demo-term-status').nth(i)).toHaveText('connected', { timeout: 20000 });
  }
}

async function clickPlay(page: Page, usePipelineBtn = false) {
  if (usePipelineBtn) {
    const btn = page.locator('.pp-run-btn');
    if (await btn.isVisible().catch(() => false) && !(await btn.isDisabled())) {
      await btn.click();
      return;
    }
  }
  const overlay = page.locator('.demo-intro-overlay .demo-play-btn');
  const bottom = page.locator('.demo-pb-btn.primary');
  if (await overlay.isVisible().catch(() => false)) await overlay.click();
  else if (await bottom.isVisible().catch(() => false)) await bottom.click();
}

async function waitForProgress(page: Page, timeoutMs = 45000): Promise<string> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const text = await page.locator('.demo-pb-cmd-preview').textContent() ?? '';
    if (text !== 'press Play to begin' && text !== '') return text;
    await page.waitForTimeout(500);
  }
  return await page.locator('.demo-pb-cmd-preview').textContent() ?? '';
}

/**
 * Test a single scenario: navigate, switch tab, connect terminals, play, verify progress.
 */
async function testScenario(
  page: Page,
  tabIdx: number,
  panes: number,
  usePipelineBtn = false,
): Promise<string> {
  await gotoDemo(page);
  await waitForServe(page);
  if (tabIdx > 0) await switchTab(page, tabIdx);
  await waitForTerminals(page, panes);
  await clickPlay(page, usePipelineBtn);
  return waitForProgress(page, 30000);
}

// Each scenario gets its own test with a fresh page context.
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
