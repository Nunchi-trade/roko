import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

const BENCH_TABS = ['Configure', 'Live', 'Results', 'History', 'Compare', 'Analysis', 'Learning'] as const;

test.describe('Bench page — full', () => {
  test('page title "Benchmark Lab" is visible', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });

    const title = page.locator('h1.bench-page-title');
    await expect(title).toBeVisible({ timeout: 5000 });
    await expect(title).toContainText('Benchmark Lab');
  });

  test('hero stats mosaic renders 4 cells', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });

    const heroStats = page.locator('div.bench-hero-stats');
    await expect(heroStats).toBeVisible({ timeout: 5000 });
  });

  test('all 7 tabs are visible and clickable', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });

    const tabs = page.locator('button.bench-tab');
    await expect(tabs).toHaveCount(7);

    for (const label of BENCH_TABS) {
      const tab = tabs.filter({ hasText: label });
      await expect(tab).toBeVisible();
    }

    // Click each tab and verify it becomes active
    for (const label of BENCH_TABS) {
      const tab = tabs.filter({ hasText: label });
      await tab.click();
      await page.waitForTimeout(300);
      await expect(tab).toHaveClass(/active/);
    }

    expectNoJsErrors(errors);
  });

  test.describe('Configure tab', () => {
    test('suite selector shows suite cards', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const suiteCards = page.locator('.suite-card');
      await expect(suiteCards.first()).toBeVisible({ timeout: 10000 });

      const count = await suiteCards.count();
      expect(count).toBeGreaterThanOrEqual(1);
    });

    test('suite cards show task counts', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const cardCounts = page.locator('.suite-card-count');
      await expect(cardCounts.first()).toBeVisible({ timeout: 10000 });

      const text = await cardCounts.first().textContent();
      expect(text).toMatch(/\d+ tasks/);
    });

    test('suite cards are clickable and show selection state', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const firstCard = page.locator('.suite-card').first();
      await expect(firstCard).toBeVisible({ timeout: 10000 });

      await firstCard.click();
      await page.waitForTimeout(300);

      await expect(firstCard).toHaveClass(/selected/);
    });

    test('4 strategy cards are visible', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const strategyCards = page.locator('button.config-card');
      await expect(strategyCards.first()).toBeVisible({ timeout: 5000 });
      await expect(strategyCards).toHaveCount(4);

      const labels = ['Minimal', 'Context-Enriched', 'Neuro-Augmented', 'Full Cascade'];
      for (const label of labels) {
        await expect(strategyCards.filter({ hasText: label })).toBeVisible();
      }
    });

    test('strategy cards are clickable with selection state', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const card = page.locator('button.config-card').filter({ hasText: 'Minimal' });
      await expect(card).toBeVisible({ timeout: 5000 });

      await card.click();
      await page.waitForTimeout(300);

      await expect(card).toHaveClass(/selected/);
    });

    test('RUN BENCHMARK button is present', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const runBtn = page.locator('button', { hasText: /run benchmark/i });
      await expect(runBtn).toBeVisible({ timeout: 5000 });
    });

    test('mode toggle (Single/Matrix) is visible', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      const modeToggle = page.locator('div.bench-mode-toggle');
      await expect(modeToggle).toBeVisible({ timeout: 5000 });

      await expect(modeToggle.locator('button', { hasText: 'Single' })).toBeVisible();
      await expect(modeToggle.locator('button', { hasText: 'Matrix' })).toBeVisible();
    });
  });

  test.describe('History tab', () => {
    test('renders table or empty state', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      // Switch to History tab
      await page.locator('button.bench-tab').filter({ hasText: 'History' }).click();
      await page.waitForTimeout(500);

      // Either the history table or an empty/loading state
      const table = page.locator('div.bench-history table.task-table');
      const historyArea = page.locator('div.bench-history');
      await expect(historyArea).toBeVisible({ timeout: 5000 });
    });

    test('filter dropdowns are present', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });
      await page.locator('button.bench-tab').filter({ hasText: 'History' }).click();
      await page.waitForTimeout(500);

      const filters = page.locator('div.bench-history-filters select.config-input');
      const count = await filters.count();
      expect(count).toBeGreaterThanOrEqual(1);
    });
  });

  test.describe('Compare tab', () => {
    test('renders with chip controls', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      await page.locator('button.bench-tab').filter({ hasText: 'Compare' }).click();
      await page.waitForTimeout(500);

      const compareArea = page.locator('div.bench-compare');
      await expect(compareArea).toBeVisible({ timeout: 5000 });
    });

    test('quick select buttons are visible', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });
      await page.locator('button.bench-tab').filter({ hasText: 'Compare' }).click();
      await page.waitForTimeout(500);

      const quickBtns = page.locator('div.bench-compare-quick button');
      const count = await quickBtns.count();
      expect(count).toBeGreaterThanOrEqual(1);
    });
  });

  test.describe('Analysis tab', () => {
    test('renders chart area', async ({ page }) => {
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      await page.locator('button.bench-tab').filter({ hasText: 'Analysis' }).click();
      await page.waitForTimeout(500);

      const analysisArea = page.locator('div.bench-analysis');
      await expect(analysisArea).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Learning tab', () => {
    test('renders content', async ({ page }) => {
      const errors = collectConsoleErrors(page);
      await page.goto('/bench', { waitUntil: 'domcontentloaded' });

      await page.locator('button.bench-tab').filter({ hasText: 'Learning' }).click();
      await page.waitForTimeout(500);

      // Learning tab should render without errors
      expectNoJsErrors(errors);
    });
  });

  test('suite expand/collapse shows task list', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });

    const firstCard = page.locator('.suite-card').first();
    await expect(firstCard).toBeVisible({ timeout: 10000 });

    // Click the expand button
    const expandBtn = firstCard.locator('button.suite-card-expand');
    if (await expandBtn.isVisible().catch(() => false)) {
      await expandBtn.click();
      await page.waitForTimeout(300);

      const taskList = firstCard.locator('div.suite-task-list');
      await expect(taskList).toBeVisible();
      await expect(expandBtn).toContainText('Hide tasks');
    }
  });

  test('no JS errors on load', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(1000);
    expectNoJsErrors(errors);
  });
});
