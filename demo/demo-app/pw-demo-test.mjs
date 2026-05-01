#!/usr/bin/env node
/**
 * Quick Playwright E2E test for the demo page.
 * Tests: page loads, terminal connects, PRD pipeline starts, commands execute.
 *
 * Usage: node pw-demo-test.mjs
 * Requires: roko serve on :6677, vite dev on :5173
 */
import { chromium } from 'playwright';

const BASE = 'http://localhost:5173';
const TIMEOUT = 30000;

async function test() {
  const browser = await chromium.launch({ headless: false });
  const page = await browser.newPage();
  const errors = [];

  page.on('console', msg => {
    const text = msg.text();
    if (msg.type() === 'error' && !text.includes('8545') && !text.includes('favicon')) {
      errors.push(text);
    }
    // Show ALL console output for debugging
    const type = msg.type();
    if (type === 'warning' || type === 'error' || type === 'log' || type === 'debug' || type === 'info') {
      console.log(`  [${type}] ${text.slice(0, 200)}`);
    }
  });

  console.log('1. Loading demo page...');
  await page.goto(`${BASE}/demo`, { waitUntil: 'domcontentloaded', timeout: TIMEOUT });
  console.log('   ✓ Page loaded');

  // Check server health indicator
  console.log('2. Checking server health...');
  const serverLabel = page.locator('.dsb-server-label');
  await serverLabel.waitFor({ timeout: 5000 });
  const serverText = await serverLabel.textContent();
  console.log(`   Server status: ${serverText}`);

  // Check terminal status
  console.log('3. Checking terminal connection...');
  const termDot = page.locator('.demo-term-dot').first();
  await termDot.waitFor({ timeout: 5000 });

  // Wait for the terminal to show "connected" (our fix waits for shell prompt)
  console.log('   Waiting for terminal to detect shell prompt...');
  await page.waitForFunction(() => {
    const dots = document.querySelectorAll('.demo-term-dot.connected');
    return dots.length > 0;
  }, { timeout: 15000 }).catch(() => {
    console.log('   ⚠ Terminal did not reach "connected" state within 15s');
  });

  const connectedDots = await page.locator('.demo-term-dot.connected').count();
  const termStatus = await page.locator('.demo-term-status').first().textContent();
  console.log(`   Terminal status: ${termStatus}, connected dots: ${connectedDots}`);

  // Click Play button
  console.log('4. Starting PRD Pipeline...');
  const playBtn = page.locator('button:has-text("play"), .scenario-preview-play, [class*="play"]').first();
  if (await playBtn.isVisible()) {
    await playBtn.click();
    console.log('   ✓ Clicked play');
  } else {
    console.log('   ⚠ Play button not found, trying direct method...');
    // Try clicking the preview overlay
    const preview = page.locator('.scenario-preview').first();
    if (await preview.isVisible()) {
      await preview.click();
    }
  }

  // Wait for countdown to finish and scenario to start
  console.log('5. Waiting for scenario to start...');
  await page.waitForTimeout(4000); // 3-2-1 countdown + transition

  // Check if terminal has content (not blank)
  console.log('6. Checking terminal content...');
  const termBody = page.locator('.demo-term-body').first();
  const termHtml = await termBody.innerHTML();
  const hasContent = termHtml.length > 100;
  console.log(`   Terminal body HTML length: ${termHtml.length} (has content: ${hasContent})`);

  // Wait for seeding to complete and first visible command to run
  console.log('7. Waiting for pipeline to start executing...');
  await page.waitForTimeout(25000);

  // Take mid-run screenshot
  await page.screenshot({ path: 'pw-captures/demo-midrun.png', fullPage: true });
  console.log('   Mid-run screenshot saved');

  // Check log entries
  const logEntries = await page.locator('.cmd-log-entry, [class*="log-entry"]').count();
  console.log(`   Log entries: ${logEntries}`);

  // Check for separator lines in terminal
  const termContent = await termBody.textContent() || '';
  const hasSeparator = termContent.includes('─') || termContent.includes('━');
  console.log(`   Terminal has separator lines: ${hasSeparator}`);

  // Check pipeline progress
  const progressText = await page.locator('[class*="progress"]').first().textContent().catch(() => 'n/a');
  console.log(`   Progress: ${progressText}`);

  // Summary
  console.log('\n── Summary ──');
  console.log(`  Page loads:       ✓`);
  console.log(`  Server health:    ${serverText?.includes('SERVE') ? '✓' : '⚠'} ${serverText}`);
  console.log(`  Terminal connect:  ${connectedDots > 0 ? '✓' : '✗'} (${connectedDots} connected)`);
  console.log(`  Terminal content:  ${hasContent ? '✓' : '✗'}`);
  console.log(`  Command separator: ${hasSeparator ? '✓' : '✗'}`);
  console.log(`  Console errors:    ${errors.length} (excluding mirage-rs WS)`);
  if (errors.length > 0) {
    errors.slice(0, 5).forEach(e => console.log(`    - ${e.slice(0, 120)}`));
  }

  // Take screenshot
  await page.screenshot({ path: 'pw-captures/demo-test.png', fullPage: true });
  console.log('\n  Screenshot saved to pw-captures/demo-test.png');

  await browser.close();
  process.exit(errors.length > 3 ? 1 : 0);
}

test().catch(err => {
  console.error('Test failed:', err);
  process.exit(1);
});
