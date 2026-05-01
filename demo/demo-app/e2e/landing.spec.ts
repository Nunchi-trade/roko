import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

test.describe('Landing page', () => {
  test('hero renders with title and subtitle', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const title = page.locator('h1.landing-title-gradient');
    await expect(title).toBeVisible({ timeout: 5000 });

    // Title is assembled from individual letter spans spelling "nunchi"
    const letters = title.locator('span.landing-letter');
    await expect(letters).toHaveCount(6);
    await expect(title).toContainText('nunchi');

    const subtitle = page.locator('p.landing-subtitle');
    await expect(subtitle).toBeVisible();
    await expect(subtitle).toContainText('agent coordination plane');

    expectNoJsErrors(errors);
  });

  test('CTA "start" link navigates to /demo', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const cta = page.locator('a.landing-cta');
    await expect(cta).toBeVisible();
    await expect(cta).toHaveText('start');
    await expect(cta).toHaveAttribute('href', '/demo');

    await cta.click();
    await expect(page).toHaveURL(/\/demo/);
  });

  test('loop ticker cycles through 8 phases', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const ticker = page.locator('div.landing-loop-ticker');
    await expect(ticker).toBeVisible();

    const phases = ticker.locator('span.phase');
    await expect(phases).toHaveCount(8);

    // Verify the 8 phase names
    const expectedPhases = ['query', 'score', 'route', 'compose', 'act', 'verify', 'write', 'react'];
    for (let i = 0; i < expectedPhases.length; i++) {
      await expect(phases.nth(i)).toHaveText(expectedPhases[i]);
    }

    // At least one phase should be active
    const activePhase = ticker.locator('span.phase.active');
    await expect(activePhase).toHaveCount(1);
  });

  test('corner decorations are visible', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    for (const corner of ['tl', 'tr', 'bl', 'br']) {
      await expect(page.locator(`.landing-corner--${corner}`)).toBeVisible();
    }
  });

  test('footer mark text is visible', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const footer = page.locator('div.landing-footer-mark');
    await expect(footer).toBeVisible();
    await expect(footer).toContainText('18 crates');
  });

  test('hero rules (dividers) are visible', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });

    const rules = page.locator('div.landing-rule');
    await expect(rules).toHaveCount(2);
    await expect(rules.first()).toBeVisible();
  });
});
