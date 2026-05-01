// Playwright script to observe the PRD Pipeline demo and capture screenshots
import { chromium } from 'playwright';
import { mkdirSync } from 'fs';
import { join } from 'path';

const OUT = join(import.meta.dirname, 'pw-captures');
mkdirSync(OUT, { recursive: true });

let shotNum = 0;
function nextName(label) {
  return join(OUT, `${String(++shotNum).padStart(2, '0')}-${label}.png`);
}

async function main() {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    recordVideo: { dir: OUT, size: { width: 1440, height: 900 } },
  });
  const page = await context.newPage();

  // Collect console errors
  const consoleErrors = [];
  page.on('console', msg => {
    if (msg.type() === 'error') consoleErrors.push(msg.text());
  });

  console.log('Navigating to demo...');
  await page.goto('http://localhost:6677/demo', { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(5000);
  await page.screenshot({ path: nextName('initial-load'), fullPage: true });
  console.log(`${shotNum} - Initial load`);

  // Pre-run state
  await page.screenshot({ path: nextName('pre-run'), fullPage: true });
  console.log(`${shotNum} - Pre-run state`);

  // Force-click START LIVE RUN (overlays block normal clicks)
  const startBtn = page.locator('button:has-text("Start live run")').first();
  if (await startBtn.isVisible({ timeout: 5000 }).catch(() => false)) {
    console.log('Force-clicking START LIVE RUN...');
    await startBtn.click({ force: true });
  } else {
    console.log('No START LIVE RUN button found');
  }

  // Capture every 1.5s for 90 seconds to observe pacing, jankiness, readability
  const totalCaptures = 60;
  const interval = 1500;
  for (let i = 0; i < totalCaptures; i++) {
    await page.waitForTimeout(interval);
    await page.screenshot({ path: nextName(`t${Math.round(i * interval / 1000)}s`), fullPage: true });
    if (i % 10 === 0) console.log(`${shotNum} - t=${Math.round(i * interval / 1000)}s`);
  }

  // Final state
  await page.screenshot({ path: nextName('final'), fullPage: true });
  console.log(`${shotNum} - Final`);

  if (consoleErrors.length > 0) {
    console.log(`\n=== ${consoleErrors.length} console errors ===`);
    consoleErrors.slice(0, 20).forEach(e => console.log('  ERR:', e));
  }

  await context.close();
  await browser.close();
  console.log(`\nDone. ${shotNum} captures saved to ${OUT}`);
}

main().catch(e => { console.error(e); process.exit(1); });
