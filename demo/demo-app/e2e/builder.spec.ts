import { test, expect } from '@playwright/test';
import { collectConsoleErrors, expectNoJsErrors } from './helpers';

test.describe('Builder page', () => {
  test('page title "Builder" is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const title = page.locator('span.builder-title');
    await expect(title).toBeVisible({ timeout: 5000 });
    await expect(title).toHaveText('Builder');
  });

  test('info subtitle is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const info = page.locator('span.builder-info');
    await expect(info).toBeVisible();
    await expect(info).toContainText('type a request');
  });

  test('input field with placeholder is present', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const input = page.locator('input[placeholder="describe what to build..."]');
    await expect(input).toBeVisible({ timeout: 5000 });
  });

  test('build button is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const buildBtn = page.locator('button.btn-build');
    await expect(buildBtn).toBeVisible({ timeout: 5000 });
    await expect(buildBtn).toContainText('Build');
  });

  test('preset buttons are visible', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const presets = page.locator('button.preset-btn');
    await expect(presets.first()).toBeVisible({ timeout: 5000 });

    const count = await presets.count();
    expect(count).toBeGreaterThanOrEqual(10);

    // Verify some known preset labels
    const labels = ['calculator', 'REST API', 'dedup', 'commitgen', 'web scraper'];
    for (const label of labels) {
      await expect(presets.filter({ hasText: label })).toBeVisible();
    }

    expectNoJsErrors(errors);
  });

  test('clicking preset fills input field', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const input = page.locator('input[placeholder="describe what to build..."]');
    const preset = page.locator('button.preset-btn').first();

    await expect(preset).toBeVisible({ timeout: 5000 });
    const presetText = await preset.textContent();

    await preset.click();
    await page.waitForTimeout(300);

    // Input should now contain something (the preset fills the input)
    const value = await input.inputValue();
    expect(value.length).toBeGreaterThan(0);
  });

  test('model selector dropdown is present', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const modelBtn = page.locator('button.model-select-btn');
    await expect(modelBtn).toBeVisible({ timeout: 5000 });
  });

  test('model selector opens dropdown on click', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const modelBtn = page.locator('button.model-select-btn');
    await expect(modelBtn).toBeVisible({ timeout: 5000 });

    await modelBtn.click();
    await page.waitForTimeout(300);

    const dropdown = page.locator('div.model-dropdown');
    await expect(dropdown).toBeVisible();
  });

  test('files sidebar shows "no project yet"', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const sidebar = page.locator('div.builder-sidebar');
    await expect(sidebar).toBeVisible({ timeout: 5000 });

    const placeholder = page.locator('div.file-placeholder');
    await expect(placeholder).toContainText('no project yet');
  });

  test('gate bar is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const gateBar = page.locator('div.builder-gate-bar');
    await expect(gateBar).toBeVisible({ timeout: 5000 });
  });

  test('status bar is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const statusBar = page.locator('div.builder-status-bar');
    await expect(statusBar).toBeVisible({ timeout: 5000 });
  });

  test('prompt marker is visible', async ({ page }) => {
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });

    const marker = page.locator('span.prompt-marker');
    await expect(marker).toBeVisible({ timeout: 5000 });
    await expect(marker).toHaveText('▸');
  });

  test('no JS errors on load', async ({ page }) => {
    const errors = collectConsoleErrors(page);
    await page.goto('/builder', { waitUntil: 'domcontentloaded' });
    await page.waitForTimeout(1000);
    expectNoJsErrors(errors);
  });
});
