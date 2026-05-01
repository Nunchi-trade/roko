import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

test.describe('Settings page', () => {
  test('page title "Settings" is visible', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    const title = page.locator('span.settings-title');
    await expect(title).toBeVisible({ timeout: 5000 });
    await expect(title).toHaveText('Settings');
  });

  test('subtitle is visible', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    const subtitle = page.locator('span.settings-subtitle');
    await expect(subtitle).toBeVisible();
    await expect(subtitle).toContainText('manage providers, models, and defaults');
  });

  test('4 collapsible sections are visible', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    const toggles = page.locator('button.settings-section-toggle');
    await expect(toggles).toHaveCount(4);

    const sectionNames = ['Providers', 'Models', 'Agent Defaults', 'Gates'];
    for (const name of sectionNames) {
      await expect(toggles.filter({ hasText: name })).toBeVisible();
    }
  });

  test('clicking section toggle collapses and expands body', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    // Find the "Agent Defaults" section toggle
    const toggle = page.locator('button.settings-section-toggle').filter({ hasText: 'Agent Defaults' });
    await expect(toggle).toBeVisible({ timeout: 5000 });

    const sectionBody = page.locator('#section-agent-defaults');

    // Click to toggle — check if it collapses
    const wasExpanded = await toggle.getAttribute('aria-expanded');
    await toggle.click();
    await page.waitForTimeout(300);

    if (wasExpanded === 'true') {
      await expect(sectionBody).toHaveClass(/collapsed/);
    } else {
      await expect(sectionBody).not.toHaveClass(/collapsed/);
    }

    // Click again to restore
    await toggle.click();
    await page.waitForTimeout(300);
  });

  test('dropdowns are present in Agent Defaults section', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    // Expand Agent Defaults if collapsed
    const toggle = page.locator('button.settings-section-toggle').filter({ hasText: 'Agent Defaults' });
    const expanded = await toggle.getAttribute('aria-expanded');
    if (expanded !== 'true') {
      await toggle.click();
      await page.waitForTimeout(300);
    }

    const section = page.locator('#section-agent-defaults');
    const selects = section.locator('select.select-animate');
    // Should have 3 selects: Default Model, Default Backend, Effort
    await expect(selects).toHaveCount(3);
  });

  test('toggle switches are present', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    // Expand Agent Defaults section for Bare Mode toggle
    const agentToggle = page.locator('button.settings-section-toggle').filter({ hasText: 'Agent Defaults' });
    if (await agentToggle.getAttribute('aria-expanded') !== 'true') {
      await agentToggle.click();
      await page.waitForTimeout(300);
    }

    // Expand Gates section for Clippy and Skip Tests toggles
    const gatesToggle = page.locator('button.settings-section-toggle').filter({ hasText: 'Gates' });
    if (await gatesToggle.getAttribute('aria-expanded') !== 'true') {
      await gatesToggle.click();
      await page.waitForTimeout(300);
    }

    // All toggle switches across sections
    const toggleWraps = page.locator('label.toggle-wrap');
    const count = await toggleWraps.count();
    // Bare Mode + Clippy + Skip Tests = 3
    expect(count).toBeGreaterThanOrEqual(3);
  });

  test('Max Iterations input is present in Gates section', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    // Expand Gates section
    const gatesToggle = page.locator('button.settings-section-toggle').filter({ hasText: 'Gates' });
    if (await gatesToggle.getAttribute('aria-expanded') !== 'true') {
      await gatesToggle.click();
      await page.waitForTimeout(300);
    }

    const section = page.locator('#section-gates');
    const input = section.locator('input[type="text"].input-narrow');
    await expect(input).toBeVisible();
  });

  test('Save and Reset buttons are visible', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    const saveBtn = page.locator('div.settings-actions button.primary');
    await expect(saveBtn).toBeVisible({ timeout: 5000 });
    await expect(saveBtn).toContainText('Save');

    const resetBtn = page.locator('button.reset-btn');
    await expect(resetBtn).toBeVisible();
    await expect(resetBtn).toContainText('Reset');
  });

  test('offline banner shown when server not running', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(1000);

    // Check status indicator — if offline, banner should show
    const status = page.locator('div.settings-status');
    const statusText = await status.textContent();

    if (statusText?.includes('offline')) {
      const banner = page.locator('div.settings-offline-banner');
      await expect(banner).toBeVisible();
      await expect(banner).toContainText('Server offline');

      // Page should have offline class
      await expect(page.locator('div.settings-page')).toHaveClass(/settings-offline/);
    }
  });

  test('connection status indicator is present', async ({ page }) => {
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });

    const status = page.locator('div.settings-status');
    await expect(status).toBeVisible({ timeout: 5000 });

    const text = await status.textContent();
    expect(text).toMatch(/connected|offline/);
  });

  test('no JS errors on load', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/settings', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(1000);
    expectNoJsErrors(errors);
  });
});
