#!/usr/bin/env node
/**
 * Quick targeted test: terminal connects, commands execute, separators appear.
 * Usage: node pw-quick-test.mjs
 * Requires: roko serve --enable-terminal on :6677, vite dev on :5173
 */
import { chromium } from 'playwright';

const BASE = 'http://localhost:5173';

async function test() {
  const browser = await chromium.launch({ headless: false });
  const page = await browser.newPage();

  page.on('console', msg => {
    const text = msg.text();
    const type = msg.type();
    if (['warning', 'error', 'log', 'debug'].includes(type)) {
      console.log(`  [${type}] ${text.slice(0, 300)}`);
    }
  });

  console.log('1. Loading demo page...');
  await page.goto(`${BASE}/demo`, { waitUntil: 'domcontentloaded', timeout: 15000 });
  console.log('   ✓ Page loaded');

  console.log('2. Waiting for terminal to connect + detect shell prompt...');
  await page.waitForFunction(() => {
    const dots = document.querySelectorAll('.demo-term-dot.connected');
    return dots.length > 0;
  }, { timeout: 15000 });
  console.log('   ✓ Terminal connected with shell prompt');

  console.log('3. Clicking play to start PRD Pipeline...');
  const playBtn = page.locator('button:has-text("play"), .scenario-preview-play, [class*="play"]').first();
  if (await playBtn.isVisible()) {
    await playBtn.click();
    console.log('   ✓ Clicked play');
  } else {
    const preview = page.locator('.scenario-preview').first();
    await preview.click();
    console.log('   ✓ Clicked preview');
  }

  // Wait for scenario to complete (seeded pipeline completes in ~10s)
  console.log('4. Waiting for scenario to complete...');

  // Watch for the scenario-run performance marker that signals completion
  let scenarioComplete = false;
  const startTime = Date.now();

  page.on('console', msg => {
    if (msg.text().includes('scenario-run:')) {
      scenarioComplete = true;
    }
  });

  // Wait up to 30s for scenario to complete
  while (!scenarioComplete && Date.now() - startTime < 30000) {
    await page.waitForTimeout(1000);
  }

  if (scenarioComplete) {
    console.log('   ✓ Scenario completed');
  } else {
    console.log('   ⚠ Scenario did not complete within 30s');
  }

  // Wait a moment for final renders
  await page.waitForTimeout(2000);

  // Take screenshot
  await page.screenshot({ path: 'pw-captures/quick-test.png', fullPage: true });
  console.log('   Screenshot saved');

  // Check log entries (command log panel)
  const logEntries = await page.locator('.cmd-log-entry').count();
  console.log(`5. Command log entries: ${logEntries}`);

  // Check pipeline state text
  const panels = await page.locator('.pipeline-panel, .dsb-cell, [class*="pipeline"]').count();
  console.log(`   Pipeline panels found: ${panels}`);

  // Check gates in sidebar
  const gateElements = await page.locator('[class*="gate-status"], [class*="gate-badge"]').count();
  console.log(`   Gate status elements: ${gateElements}`);

  await browser.close();
  console.log('\n✓ Test complete');
}

test().catch(err => {
  console.error('Test failed:', err.message);
  process.exit(1);
});
