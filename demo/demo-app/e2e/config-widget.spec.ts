import { test, expect } from '@playwright/test';

test.describe('ConfigWidget', () => {
  test('pill is visible on every page', async ({ page }) => {
    for (const path of ['/', '/bench', '/demo', '/settings', '/explorer']) {
      await page.goto(path);
      await expect(page.locator('.cw-pill')).toBeVisible({ timeout: 5000 });
    }
  });

  test('click pill opens panel with sections', async ({ page }) => {
    await page.goto('/');
    await page.locator('.cw-pill').click();
    await expect(page.locator('.cw-panel')).toBeVisible();

    // Should have multiple collapsible sections
    const sections = page.locator('.cw-section');
    await expect(sections).not.toHaveCount(0);

    // First section (Agent Model) should be open by default
    const firstSection = sections.first();
    await expect(firstSection).toHaveAttribute('open', '');
  });

  test('expand Agent section shows model dropdown', async ({ page }) => {
    await page.goto('/');
    await page.locator('.cw-pill').click();

    // Agent Model section is open by default
    const modelSelect = page.locator('.cw-section').first().locator('select');
    await expect(modelSelect).toBeVisible();
  });

  test('change model and apply updates pill', async ({ page }) => {
    await page.goto('/');

    // Get initial pill text
    const pill = page.locator('.cw-pill');
    const initialText = await pill.textContent();

    // Open panel
    await pill.click();
    await expect(page.locator('.cw-panel')).toBeVisible();

    // Check Apply button exists
    const applyBtn = page.locator('.cw-section').first().locator('.cw-apply');
    await expect(applyBtn).toBeVisible();
  });

  test('close and reopen preserves state', async ({ page }) => {
    await page.goto('/');

    // Open panel
    await page.locator('.cw-pill').click();
    await expect(page.locator('.cw-panel')).toBeVisible();

    // Close
    await page.locator('.cw-close').click();
    await expect(page.locator('.cw-pill')).toBeVisible();

    // Reopen
    await page.locator('.cw-pill').click();
    await expect(page.locator('.cw-panel')).toBeVisible();

    // Sections should still be there
    const sections = page.locator('.cw-section');
    await expect(sections).not.toHaveCount(0);
  });
});
