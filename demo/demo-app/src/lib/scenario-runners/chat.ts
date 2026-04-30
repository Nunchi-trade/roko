// --- src/lib/scenario-runners/chat.ts ---
import type { Scenario } from '../scenarios';
import { rawSleep, stripAnsi } from '../scenario-helpers';
import { enterWorkspace, getRoko } from '../terminal-session';

export const chat: Scenario = {
  id: 'chat',
  title: 'Chat',
  subtitle:
    'The bare command is the product. Just type roko — auto-detect, auto-init, drop into chat.',
  panes: 1,
  labels: ['roko chat'],
  panel: true,
  promptBar: false,
  category: 'exploration',
  features: ['Auto-detect workspace', 'Auto-init', 'Interactive REPL'],
  durationHint: '~30s',
  accent: 'teal',
  icon: 'chat',
  steps: [
    { label: 'Start TUI', sublabel: 'roko' },
    { label: 'Send message', sublabel: 'explain cascade routing' },
    { label: 'Slash commands', sublabel: '/status, /model' },
  ],
  async run({ entries, playback, timeline, logCommand, workspaceDir }) {
    const e = entries[0];
    await enterWorkspace(e, workspaceDir);
    const ROKO = getRoko();

    timeline.init(this.steps);

    // Phase 1: start roko
    await playback.waitForStep();
    playback.setProgress(1, 3, ROKO);
    timeline.setActive(0);
    logCommand(
      ROKO,
      'Starts the unified chat TUI — auto-detects auth, auto-creates .roko/ if missing, starts serve in-process, drops into interactive chat.',
    );

    e.outputBuffer = '';
    await e.typeCmd(ROKO);

    // Wait for chat prompt to appear
    const start = Date.now();
    while (Date.now() - start < 30000) {
      await rawSleep(300);
      const buf = stripAnsi(e.outputBuffer);
      if (/❯|roko>|\/help|model|chat/i.test(buf)) break;
    }
    await rawSleep(800);

    // Phase 2: send a message
    await playback.waitForStep();
    playback.setProgress(2, 3, 'explain what cascade routing does');
    timeline.setActive(1);
    logCommand(
      'explain what cascade routing does',
      'Sends a natural-language question to the active agent. The agent uses context from the knowledge store and responds inline with streaming markdown.',
    );
    e.outputBuffer = '';
    await e.typeCmd('explain what cascade routing does', 20);

    // Wait for response to complete
    const rStart = Date.now();
    while (Date.now() - rStart < 60000) {
      await rawSleep(500);
      const buf = stripAnsi(e.outputBuffer);
      if (buf.length > 200 && /❯|roko>/i.test(buf.slice(-200))) break;
    }
    await rawSleep(500);

    // Phase 3: slash commands
    await playback.waitForStep();
    playback.setProgress(3, 3, '/status');
    timeline.setActive(2);
    logCommand(
      '/status',
      'Runs workspace status inline from the chat TUI. Slash commands give quick access to all roko features without leaving the conversation.',
    );
    e.outputBuffer = '';
    await e.typeCmd('/status', 20);
    await rawSleep(3000);

    logCommand(
      '/model',
      'Shows or switches the active model. Supports all configured providers — Anthropic, OpenAI, Zhipu, Google, Moonshot, Ollama.',
    );
    e.outputBuffer = '';
    await e.typeCmd('/model', 20);
    await rawSleep(2000);

    timeline.markAllComplete();
  },
};
