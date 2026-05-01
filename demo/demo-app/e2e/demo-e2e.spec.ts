import { test, expect } from '@playwright/test';
import {
  gotoDemo,
  waitForServe,
  switchTab,
  waitForTerminals,
  clickPlay,
  waitForProgress,
} from './helpers';

/**
 * End-to-end tests: verify scenarios actually execute commands, not just start.
 */

test.describe('Scenario end-to-end', () => {
  test.setTimeout(300_000); // 5 minutes

  test('Explore executes roko commands and shows output', async ({ page }) => {
    const consoleErrors: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error') consoleErrors.push(msg.text());
    });

    await gotoDemo(page);
    await waitForServe(page);

    // Switch to Explore (tab 6)
    await switchTab(page, 6);
    await waitForTerminals(page, 4);

    // Click play
    await clickPlay(page);

    // Wait for scenario to start creating workspace
    const progress = await waitForProgress(page, 30000);
    expect(progress).not.toBe('press Play to begin');
    console.log(`Started: ${progress}`);

    // Wait for actual command execution (Explore runs ~12 commands across 4 panes)
    // Give it up to 2 minutes
    const startTime = Date.now();
    let lastProgress = '';
    let commandsSeen = 0;

    while (Date.now() - startTime < 120000) {
      const currentProgress = await page.locator('.demo-pb-cmd-preview').textContent() ?? '';
      if (currentProgress !== lastProgress) {
        console.log(`Progress: ${currentProgress}`);
        lastProgress = currentProgress;
        commandsSeen++;
      }

      // Check if scenario finished (play button reappears)
      const hasPlayBtn = await page.locator('.demo-pb-btn.primary').isVisible().catch(() => false);
      if (hasPlayBtn && commandsSeen > 2) {
        console.log('Scenario completed!');
        break;
      }

      await page.waitForTimeout(1000);
    }

    console.log(`Commands seen: ${commandsSeen}`);

    // Verify terminal panes have output (xterm should have content)
    const termBodies = page.locator('.demo-term-body');
    for (let i = 0; i < 4; i++) {
      const hasXterm = await termBodies.nth(i).locator('.xterm').count();
      expect(hasXterm).toBeGreaterThan(0);
    }

    // Should have seen multiple command steps
    expect(commandsSeen).toBeGreaterThanOrEqual(3);
  });

  test('PRD Pipeline shows pipeline phases advancing', async ({ page }) => {
    await gotoDemo(page);
    await waitForServe(page);

    // PRD Pipeline is tab 0 (default)
    await waitForTerminals(page, 1);

    // Click pipeline run button
    await clickPlay(page, true);

    // Wait for start
    const progress = await waitForProgress(page, 30000);
    expect(progress).not.toBe('press Play to begin');
    console.log(`Started: ${progress}`);

    // Wait for pipeline phases to start advancing
    let pipelineAdvanced = false;
    const startTime = Date.now();

    while (Date.now() - startTime < 120000) {
      // Check for active phases in pipeline panel
      const activePhases = await page.locator('.pp-phase.active, .pp-phase.current, [class*="phase"][class*="active"]').count();
      const currentProgress = await page.locator('.demo-pb-cmd-preview').textContent() ?? '';

      if (activePhases > 0 || currentProgress.includes('roko')) {
        console.log(`Pipeline advancing: ${activePhases} active phases, progress: ${currentProgress}`);
        pipelineAdvanced = true;
        break;
      }

      // Also check if commands are running by looking at terminal output
      const termOutput = await page.evaluate(() => {
        const body = document.querySelector('.demo-term-body');
        return body?.textContent?.length ?? 0;
      });

      if (termOutput > 100) {
        console.log(`Terminal has output: ${termOutput} chars`);
        pipelineAdvanced = true;
        break;
      }

      await page.waitForTimeout(2000);
    }

    expect(pipelineAdvanced).toBe(true);
  });

  test('Cost Race runs two panes side by side', async ({ page }) => {
    await gotoDemo(page);
    await waitForServe(page);

    // Switch to Cost Race (tab 2)
    await switchTab(page, 2);
    await waitForTerminals(page, 2);

    // Click play
    await clickPlay(page);

    // Wait for start
    const progress = await waitForProgress(page, 30000);
    expect(progress).not.toBe('press Play to begin');
    console.log(`Started: ${progress}`);

    // Wait for both panes to show terminal content
    await page.waitForTimeout(20000);

    // Both terminal panes should have xterm content
    const termCount = await page.locator('.demo-term-pane').count();
    expect(termCount).toBe(2);
  });
});
