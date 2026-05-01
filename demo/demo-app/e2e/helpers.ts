import { expect, type Page } from '@playwright/test';

/** Navigate to /demo and wait for the page to stabilise. */
export async function gotoDemo(page: Page) {
  await page.goto('/demo', { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(8000);
}

/** Wait for the roko-serve status indicator to show "serve live". */
export async function waitForServe(page: Page) {
  await expect(
    page.locator('.demo-serve-status').filter({ hasText: 'serve live' }),
  ).toBeVisible({ timeout: 15000 });
}

/** Click a demo scenario tab by index. */
export async function switchTab(page: Page, idx: number) {
  await page.locator('.demo-tab').nth(idx).click();
  await page.waitForTimeout(3000);
}

/** Wait for N terminal panes to reach "connected" status. */
export async function waitForTerminals(page: Page, count: number) {
  for (let i = 0; i < count; i++) {
    await expect(
      page.locator('.demo-term-status').nth(i),
    ).toHaveText('connected', { timeout: 20000 });
  }
}

/** Click the play button (overlay, bottom bar, or pipeline run). */
export async function clickPlay(page: Page, usePipelineBtn = false) {
  if (usePipelineBtn) {
    const btn = page.locator('.pp-run-btn');
    if (await btn.isVisible().catch(() => false) && !(await btn.isDisabled())) {
      await btn.click();
      return;
    }
  }
  const overlay = page.locator('.demo-intro-overlay .demo-play-btn');
  const bottom = page.locator('.demo-pb-btn.primary');
  if (await overlay.isVisible().catch(() => false)) await overlay.click();
  else if (await bottom.isVisible().catch(() => false)) await bottom.click();
}

/** Poll until the progress label changes from idle, returning the new text. */
export async function waitForProgress(page: Page, timeoutMs = 45000): Promise<string> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const text = await page.locator('.demo-pb-cmd-preview').textContent() ?? '';
    if (text !== 'press Play to begin' && text !== '') return text;
    await page.waitForTimeout(500);
  }
  return await page.locator('.demo-pb-cmd-preview').textContent() ?? '';
}

/**
 * Full scenario lifecycle: navigate → switch tab → connect terminals → play → verify progress.
 */
export async function testScenario(
  page: Page,
  tabIdx: number,
  panes: number,
  usePipelineBtn = false,
): Promise<string> {
  await gotoDemo(page);
  await waitForServe(page);
  if (tabIdx > 0) await switchTab(page, tabIdx);
  await waitForTerminals(page, panes);
  await clickPlay(page, usePipelineBtn);
  return waitForProgress(page, 30000);
}

/**
 * Filter console errors, ignoring network/fetch failures
 * that happen when the roko API server is not running.
 */
export function filterConsoleErrors(errors: string[]): string[] {
  return errors.filter(
    (e) =>
      !e.includes('fetch') &&
      !e.includes('Failed to') &&
      !e.includes('NetworkError') &&
      !e.includes('ERR_CONNECTION_REFUSED') &&
      !e.includes('net::ERR_'),
  );
}

/** Collect console errors from the page. Returns the error array. */
export function collectConsoleErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error') errors.push(msg.text());
  });
  return errors;
}

/** Navigate to a route and wait for the top nav to be visible. */
export async function waitForNavReady(page: Page, route = '/') {
  await page.goto(route, { waitUntil: 'domcontentloaded' });
  await expect(page.locator('nav.topnav')).toBeVisible({ timeout: 5000 });
}

/** Assert no JS errors occurred (ignoring network errors). */
export function expectNoJsErrors(errors: string[]) {
  const real = filterConsoleErrors(errors);
  expect(real).toEqual([]);
}
