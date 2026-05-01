import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

const NAV_LINKS = [
  { label: 'DEMO', route: '/demo' },
  { label: 'DASH', route: '/dashboard' },
  { label: 'BENCH', route: '/bench' },
  { label: 'EXPLORE', route: '/explorer' },
  { label: 'BUILD', route: '/builder' },
  { label: 'TERM', route: '/terminal' },
  { label: 'CONFIG', route: '/settings' },
] as const;

test.describe('Top navigation', () => {
  test('all 7 nav links are visible', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });
    await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });

    const links = page.locator('nav.topnav .links .nav-link');
    await expect(links).toHaveCount(7);

    for (const { label } of NAV_LINKS) {
      await expect(links.filter({ hasText: label })).toBeVisible();
    }
  });

  test('brand link navigates to landing page', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });
    await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });

    const brand = page.locator('a.brand');
    await expect(brand).toBeVisible();
    await expect(brand).toHaveAttribute('href', '/');

    await brand.click();
    await expect(page).toHaveURL(/\/$/);
  });

  test('status pill is visible', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const pill = page.locator('span.status-pill');
    await expect(pill).toBeVisible({ timeout: 5000 });
    // Should show one of: LIVE, SYNC, SEED
    const text = await pill.textContent();
    expect(text).toMatch(/LIVE|SYNC|SEED/);
  });

  for (const { label, route } of NAV_LINKS) {
    test(`clicking ${label} navigates to ${route}`, async ({ page }) => {
      const errors = collectConsoleErrors(page);
      await page.goto('/', { waitUntil: 'domcontentloaded' });
      await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });

      const link = page.locator('nav.topnav .links .nav-link').filter({ hasText: label });
      await link.click();
      await expect(page).toHaveURL(new RegExp(route));

      expectNoJsErrors(errors);
    });
  }

  test('active nav link has active styling', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });
    await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });

    const benchLink = page.locator('nav.topnav .links .nav-link').filter({ hasText: 'BENCH' });
    await expect(benchLink).toHaveClass(/active/);
  });

  test('config widget pill is visible on every page', async ({ page }) => {
    for (const path of ['/', '/bench', '/demo', '/settings', '/explorer']) {
      await page.goto(path, { waitUntil: 'domcontentloaded' });
      await expect(page.locator('.cw-pill')).toBeVisible({ timeout: 5000 });
    }
  });

  test('nav indicator element exists for sliding animation', async ({ page }) => {
    await page.goto('/bench', { waitUntil: 'domcontentloaded' });
    await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });

    const indicator = page.locator('nav.topnav .nav-indicator');
    await expect(indicator).toBeAttached();
  });
});
