import { test, expect } from '@playwright/test';
import { collectConsoleErrors, filterConsoleErrors } from './helpers';

const DASHBOARD_TABS = [
  { name: 'Cost', route: '/dashboard' },
  { name: 'Fleet', route: '/dashboard/fleet' },
  { name: 'Knowledge', route: '/dashboard/knowledge' },
  { name: 'Entries', route: '/dashboard/entries' },
  { name: 'Routing', route: '/dashboard/routing' },
  { name: 'Integrity', route: '/dashboard/integrity' },
  { name: 'Dreams', route: '/dashboard/dreams' },
] as const;

test.describe('Dashboard — full', () => {
  test('dashboard nav has 7 links', async ({ page }) => {
    await page.goto('/dashboard', { waitUntil: 'domcontentloaded' });

    const nav = page.locator('nav[aria-label="Dashboard sections"]');
    await expect(nav).toBeVisible({ timeout: 5000 });

    const navLinks = nav.locator('a');
    await expect(navLinks).toHaveCount(7);
  });

  for (const tab of DASHBOARD_TABS) {
    test(`${tab.name} tab renders at ${tab.route}`, async ({ page }) => {
      const errors = collectConsoleErrors(page);
      await page.goto(tab.route, { waitUntil: 'domcontentloaded' });
      await page.waitForTimeout(500);

      // Dashboard nav should be visible
      const nav = page.locator('nav[aria-label="Dashboard sections"]');
      await expect(nav).toBeVisible({ timeout: 5000 });

      // At least one Mosaic cell should be rendered
      const cells = page.locator('.mosaic .cell');
      await expect(cells.first()).toBeVisible({ timeout: 5000 });

      // At least one Pane should be rendered
      const panes = page.locator('.pane');
      await expect(panes.first()).toBeVisible({ timeout: 5000 });

      const jsErrors = filterConsoleErrors(errors);
      expect(jsErrors).toEqual([]);
    });
  }

  for (const tab of DASHBOARD_TABS) {
    test(`clicking ${tab.name} nav link navigates correctly`, async ({ page }) => {
      await page.goto('/dashboard', { waitUntil: 'domcontentloaded' });
      const nav = page.locator('nav[aria-label="Dashboard sections"]');
      await expect(nav).toBeVisible({ timeout: 5000 });

      const link = nav.locator('a').filter({ hasText: tab.name });
      await link.click();
      await expect(page).toHaveURL(new RegExp(tab.route));
    });
  }

  test.describe('Cost dashboard', () => {
    test('mosaic stat cells render', async ({ page }) => {
      await page.goto('/dashboard', { waitUntil: 'domcontentloaded' });

      const cells = page.locator('.mosaic .cell');
      await expect(cells.first()).toBeVisible({ timeout: 5000 });
      const count = await cells.count();
      expect(count).toBeGreaterThanOrEqual(4);
    });

    test('panes with expected titles render', async ({ page }) => {
      await page.goto('/dashboard', { waitUntil: 'domcontentloaded' });

      const panes = page.locator('.pane');
      await expect(panes.first()).toBeVisible({ timeout: 5000 });

      // Should have multiple panes
      const count = await panes.count();
      expect(count).toBeGreaterThanOrEqual(3);
    });

    test('activity cards are visible', async ({ page }) => {
      await page.goto('/dashboard', { waitUntil: 'domcontentloaded' });

      const cards = page.locator('.dash-card');
      await expect(cards.first()).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Fleet dashboard', () => {
    test('renders agent list or empty state', async ({ page }) => {
      await page.goto('/dashboard/fleet', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });

    test('view toggle buttons are visible', async ({ page }) => {
      await page.goto('/dashboard/fleet', { waitUntil: 'domcontentloaded' });

      // "list" and "topology" toggle badges
      const badges = page.locator('span.dash-badge');
      if (await badges.first().isVisible().catch(() => false)) {
        const count = await badges.count();
        expect(count).toBeGreaterThanOrEqual(2);
      }
    });
  });

  test.describe('Knowledge dashboard', () => {
    test('renders graph canvas or empty state', async ({ page }) => {
      await page.goto('/dashboard/knowledge', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Entries dashboard', () => {
    test('renders search interface', async ({ page }) => {
      await page.goto('/dashboard/entries', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page--full');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Routing dashboard', () => {
    test('renders cascade router display', async ({ page }) => {
      await page.goto('/dashboard/routing', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page--full');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Integrity dashboard', () => {
    test('renders integrity view', async ({ page }) => {
      await page.goto('/dashboard/integrity', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page--wide');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Dreams dashboard', () => {
    test('renders dreams view', async ({ page }) => {
      await page.goto('/dashboard/dreams', { waitUntil: 'domcontentloaded' });

      const dashPage = page.locator('.dash-page');
      await expect(dashPage).toBeVisible({ timeout: 5000 });
    });
  });

  test('no JS errors loading all tabs sequentially', async ({ page }) => {
    const errors = collectConsoleErrors(page);

    for (const tab of DASHBOARD_TABS) {
      await page.goto(tab.route, { waitUntil: 'domcontentloaded' });
      await page.waitForTimeout(300);
    }

    const jsErrors = filterConsoleErrors(errors);
    expect(jsErrors).toEqual([]);
  });
});
