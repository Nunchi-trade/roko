import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

test.describe('Explorer page', () => {
  test('header with title "Explorer" is visible', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const title = page.locator('span.expl-title');
    await expect(title).toBeVisible({ timeout: 5000 });
    await expect(title).toContainText('Explorer');
  });

  test('live badge is visible', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const badge = page.locator('span.expl-live-badge');
    await expect(badge).toBeVisible({ timeout: 5000 });
    await expect(badge).toContainText('LIVE');
  });

  test('5 header pills are visible', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const pills = page.locator('div.expl-header-pills span.expl-pill');
    await expect(pills).toHaveCount(5);

    const labels = ['Status', 'Uptime', 'Version', 'Agents', 'Plans'];
    for (const label of labels) {
      await expect(
        pills.filter({ has: page.locator(`span.expl-pill-label:has-text("${label}")`) }),
      ).toBeVisible();
    }
  });

  test('stat strip with 5 metrics is visible', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const strip = page.locator('div.expl-stat-strip');
    await expect(strip).toBeVisible({ timeout: 5000 });

    const statPills = page.locator('div.expl-stat-pill');
    await expect(statPills).toHaveCount(5);

    const labels = ['EPISODES', 'COST', 'AGENTS', 'GATE PASS', 'AVG DURATION'];
    for (const label of labels) {
      await expect(
        statPills.filter({ has: page.locator(`span.expl-stat-label:has-text("${label}")`) }),
      ).toBeVisible();
    }
  });

  test('refresh button is clickable without JS errors', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const refreshBtn = page.locator('button.expl-refresh');
    await expect(refreshBtn).toBeVisible({ timeout: 5000 });

    await refreshBtn.click();
    await page.waitForTimeout(500);

    expectNoJsErrors(errors);
  });

  test('sparkline canvases are rendered', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    const sparklines = page.locator('canvas.expl-spark');
    await expect(sparklines).toHaveCount(5);

    // Each sparkline has an aria-label
    for (let i = 0; i < 5; i++) {
      const label = await sparklines.nth(i).getAttribute('aria-label');
      expect(label).toContain('sparkline');
    }
  });

  test('stat values are present', async ({ page }) => {
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });

    // Each stat pill should have a value element
    const values = page.locator('span.expl-stat-value');
    await expect(values).toHaveCount(5);

    // Values should exist (may be "0" or "--" if offline, but should be present)
    for (let i = 0; i < 5; i++) {
      await expect(values.nth(i)).toBeVisible();
    }
  });

  test('no JS errors on load', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/explorer', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(1000);
    expectNoJsErrors(errors);
  });
});
