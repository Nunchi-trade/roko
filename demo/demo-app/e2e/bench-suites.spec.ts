import { test, expect } from '@playwright/test';

test.describe('Bench suites loading', () => {
  test('bench page loads and displays suites', async ({ page }) => {
    await page.goto('/bench');

    // The page should render the bench hero title
    await expect(page.locator('.bench-page-title')).toBeVisible({ timeout: 5000 });

    // Suite cards should appear (we have 6 built-in suites)
    const suiteCards = page.locator('.suite-card');
    await expect(suiteCards.first()).toBeVisible({ timeout: 10000 });
    const count = await suiteCards.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test('suite cards show task count', async ({ page }) => {
    await page.goto('/bench');

    // Wait for suites to load
    const suiteCards = page.locator('.suite-card');
    await expect(suiteCards.first()).toBeVisible({ timeout: 10000 });

    // Each card should have a task count
    const firstCount = page.locator('.suite-card-count').first();
    await expect(firstCount).toBeVisible();
    const text = await firstCount.textContent();
    expect(text).toMatch(/\d+ tasks/);
  });

  test('"No suites" message is NOT shown when server is running', async ({ page }) => {
    await page.goto('/bench');

    // Wait for data to load
    await page.waitForTimeout(3000);

    // The "No suites" fallback text should not appear
    const pageContent = await page.textContent('body');
    expect(pageContent).not.toContain('No suites');
  });

  test('bench models load from API', async ({ page }) => {
    await page.goto('/bench');

    // The model display should show a model name (not empty or "--")
    const modelValue = page.locator('.bench-model-display .param-value').first();
    await expect(modelValue).toBeVisible({ timeout: 5000 });
    const text = await modelValue.textContent();
    expect(text).toBeTruthy();
    expect(text).not.toBe('--');
  });

  test('RUN BENCHMARK button is present', async ({ page }) => {
    await page.goto('/bench');

    // Wait for suites to load
    await page.locator('.suite-card').first().waitFor({ timeout: 10000 });

    // The run button should be visible
    const runButton = page.locator('button', { hasText: /run benchmark/i });
    await expect(runButton).toBeVisible();
  });

  test('bench API returns correct suite data', async ({ page }) => {
    // Test the API directly
    const response = await page.request.get('http://localhost:6677/api/bench/suites');
    expect(response.ok()).toBeTruthy();

    const data = await response.json();
    // Backend wraps in { suites: [...] }
    const suites = data.suites ?? data;
    expect(Array.isArray(suites)).toBe(true);
    expect(suites.length).toBeGreaterThanOrEqual(1);

    // Each suite should have id, name, task_count
    for (const suite of suites) {
      expect(suite.id).toBeTruthy();
      expect(suite.name).toBeTruthy();
      expect(suite.task_count).toBeGreaterThan(0);
    }
  });

  test('bench API returns models', async ({ page }) => {
    const response = await page.request.get('http://localhost:6677/api/bench/models');
    expect(response.ok()).toBeTruthy();

    const data = await response.json();
    const models = data.models ?? data;
    expect(Array.isArray(models)).toBe(true);
    expect(models.length).toBeGreaterThan(0);
  });
});
