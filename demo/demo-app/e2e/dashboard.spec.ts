import { test, expect, type Page } from '@playwright/test';

/**
 * Smoke tests for the 7 dashboard tabs.
 * Verifies each tab renders without JS errors and produces the expected
 * structural elements (nav links, Mosaic cells, Pane components).
 * Resilient to empty data — the API may not be running.
 */

const TABS = [
  { name: 'Cost', route: '/dashboard' },
  { name: 'Fleet', route: '/dashboard/fleet' },
  { name: 'Knowledge', route: '/dashboard/knowledge' },
  { name: 'Entries', route: '/dashboard/entries' },
  { name: 'Routing', route: '/dashboard/routing' },
  { name: 'Integrity', route: '/dashboard/integrity' },
  { name: 'Dreams', route: '/dashboard/dreams' },
] as const;

/** Navigate to a dashboard route, wait for it to stabilise, and collect console errors. */
async function loadDashboardTab(page: Page, route: string): Promise<string[]> {
  const errors: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text());
  });

  await page.goto(route, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(500);

  return errors;
}

test.describe('Dashboard Tabs', () => {
  for (const tab of TABS) {
    test(`${tab.name} tab renders at ${tab.route}`, async ({ page }) => {
      const errors = await loadDashboardTab(page, tab.route);

      // Dashboard nav should be visible with all 7 links
      const nav = page.locator('nav[aria-label="Dashboard sections"]');
      await expect(nav).toBeVisible({ timeout: 5000 });

      const navLinks = nav.locator('a');
      await expect(navLinks).toHaveCount(7);

      // At least one Mosaic cell should be rendered
      const cells = page.locator('.mosaic .cell');
      await expect(cells.first()).toBeVisible({ timeout: 5000 });

      // At least one Pane should be rendered
      const panes = page.locator('.pane');
      await expect(panes.first()).toBeVisible({ timeout: 5000 });

      // Filter out network/fetch errors from missing API server
      const jsErrors = errors.filter(
        (e) => !e.includes('fetch') && !e.includes('Failed to') && !e.includes('NetworkError'),
      );
      expect(jsErrors).toEqual([]);
    });
  }
});
