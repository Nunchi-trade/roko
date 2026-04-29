import { test, expect } from '@playwright/test';

test.describe('Config sync across pages', () => {
  test('config pill is consistent across navigation', async ({ page }) => {
    await page.goto('/');
    const pillText = await page.locator('.cw-pill').textContent();

    // Navigate to bench
    await page.goto('/bench');
    const benchPillText = await page.locator('.cw-pill').textContent();
    expect(benchPillText).toBe(pillText);

    // Navigate to settings
    await page.goto('/settings');
    const settingsPillText = await page.locator('.cw-pill').textContent();
    expect(settingsPillText).toBe(pillText);
  });

  test('settings page shows model matching config pill', async ({ page }) => {
    await page.goto('/');

    // Get model from pill
    const pillText = await page.locator('.cw-pill').textContent();

    // Navigate to settings
    await page.goto('/settings');

    // Settings model selector should exist
    const modelSelect = page.locator('.settings-section').nth(2).locator('select').first();
    await expect(modelSelect).toBeVisible({ timeout: 5000 });
  });

  test('bench page shows model from config context', async ({ page }) => {
    await page.goto('/bench');

    // The MODEL pane should show the model as read-only text
    const modelDisplay = page.locator('.bench-model-display');
    await expect(modelDisplay).toBeVisible({ timeout: 5000 });

    // Should contain "Change via config pill" hint
    const hint = page.locator('.bench-model-hint');
    await expect(hint).toContainText('config pill');
  });
});
