import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

test.describe('Terminal page', () => {
  test('page title "Terminal" is visible', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const title = page.locator('span.terminal-page-title');
    await expect(title).toBeVisible({ timeout: 5000 });
    await expect(title).toHaveText('Terminal');
  });

  test('empty state shown when no terminals', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // If no terminals are open, empty state should show
    const empty = page.locator('div.terminal-empty');
    const grid = page.locator('div.term-grid');

    // Either empty state or grid should be visible
    const hasEmpty = await empty.isVisible().catch(() => false);
    const hasGrid = await grid.isVisible().catch(() => false);
    expect(hasEmpty || hasGrid).toBe(true);

    if (hasEmpty) {
      await expect(page.locator('span.terminal-empty-title')).toHaveText('No terminals open');
      await expect(page.locator('span.terminal-empty-sub')).toContainText('Click + to add one');
    }
  });

  test('add terminal button (+) is visible', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const addBtn = page.locator('button.term-btn-add');
    await expect(addBtn).toBeVisible({ timeout: 5000 });
    await expect(addBtn).toHaveText('+');
  });

  test('clicking add terminal spawns a pane', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const addBtn = page.locator('button.term-btn-add');
    await expect(addBtn).toBeVisible({ timeout: 5000 });

    await addBtn.click();
    await page.waitForTimeout(1000);

    // A terminal pane should now exist
    const panes = page.locator('div.term-pane-real');
    await expect(panes.first()).toBeVisible({ timeout: 10000 });

    expectNoJsErrors(errors);
  });

  test('multiple terminals can be opened', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const addBtn = page.locator('button.term-btn-add');
    await expect(addBtn).toBeVisible({ timeout: 5000 });

    // Open two terminals
    await addBtn.click();
    await page.waitForTimeout(500);
    await addBtn.click();
    await page.waitForTimeout(1000);

    const panes = page.locator('div.term-pane-real');
    const count = await panes.count();
    expect(count).toBeGreaterThanOrEqual(2);
  });

  test('column layout buttons (1/2/4) are visible', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // Column buttons have aria-labels "1 column", "2 columns", "4 columns"
    await expect(page.locator('button[aria-label="1 column"]')).toBeVisible({ timeout: 5000 });
    await expect(page.locator('button[aria-label="2 columns"]')).toBeVisible();
    await expect(page.locator('button[aria-label="4 columns"]')).toBeVisible();
  });

  test('column layout buttons change grid class', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // Add a terminal first
    await page.locator('button.term-btn-add').click();
    await page.waitForTimeout(1000);

    const grid = page.locator('div.term-grid');

    // Click "2 columns"
    await page.locator('button[aria-label="2 columns"]').click();
    await expect(grid).toHaveClass(/cols-2/);

    // Click "4 columns"
    await page.locator('button[aria-label="4 columns"]').click();
    await expect(grid).toHaveClass(/cols-4/);

    // Click "1 column"
    await page.locator('button[aria-label="1 column"]').click();
    await expect(grid).toHaveClass(/cols-1/);
  });

  test('close button removes a pane', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // Add a terminal
    await page.locator('button.term-btn-add').click();
    await page.waitForTimeout(1000);

    const panes = page.locator('div.term-pane-real');
    const initialCount = await panes.count();
    expect(initialCount).toBeGreaterThanOrEqual(1);

    // Close it
    const closeBtn = page.locator('button.term-close-btn').first();
    await closeBtn.click();
    await page.waitForTimeout(1000);

    const afterCount = await panes.count();
    expect(afterCount).toBeLessThan(initialCount);
  });

  test('init workspace button is present', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const initBtn = page.locator('button.term-init-workspace-btn');
    await expect(initBtn).toBeVisible({ timeout: 5000 });
    await expect(initBtn).toContainText(/Init workspace|Creating|ws ready/);
  });

  test('tab bar shows terminal tabs when terminals exist', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // Add a terminal
    await page.locator('button.term-btn-add').click();
    await page.waitForTimeout(1000);

    const tabBar = page.locator('div.term-tab-bar');
    await expect(tabBar).toBeVisible({ timeout: 5000 });

    const tabs = page.locator('button.term-tab');
    await expect(tabs.first()).toBeVisible();
  });

  test('active tab highlighting works', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    // Add two terminals
    await page.locator('button.term-btn-add').click();
    await page.waitForTimeout(500);
    await page.locator('button.term-btn-add').click();
    await page.waitForTimeout(1000);

    // One tab should be active
    const activeTab = page.locator('button.term-tab.term-tab-active');
    await expect(activeTab).toHaveCount(1);

    // Click the other tab
    const tabs = page.locator('button.term-tab');
    const firstTab = tabs.first();
    await firstTab.click();
    await page.waitForTimeout(300);

    // First tab should now be active
    await expect(firstTab).toHaveClass(/term-tab-active/);
  });

  test('clear button is visible', async ({ page }) => {
    await page.goto('/terminal', { waitUntil: 'domcontentloaded' });

    const clearBtn = page.locator('button.term-btn-clear');
    await expect(clearBtn).toBeVisible({ timeout: 5000 });
    await expect(clearBtn).toHaveText('Clear');
  });
});
