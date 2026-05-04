// --- src/lib/scenario-runners/chat.ts ---
import type { ClickableScenario, CommandDef, ScenarioContext } from '../scenarios';
import { rawSleep, stripAnsi } from '../scenario-helpers';
import { getRoko } from '../terminal-session';

export const CHAT_COMMANDS: CommandDef[] = [
  { id: 'start', command: 'roko', timeout: 30000, description: 'Start chat TUI' },
  {
    id: 'ask',
    command: 'explain what cascade routing does',
    timeout: 60000,
    description: 'Ask about cascade routing',
  },
  { id: 'slash-status', command: '/status', timeout: 10000, description: 'Inline workspace status' },
  { id: 'slash-model', command: '/model', timeout: 10000, description: 'Show/switch active model' },
];

export const chat: ClickableScenario = {
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
  commands: CHAT_COMMANDS,
  async runCommand(ctx: ScenarioContext, commandId: string) {
    const { entries } = ctx;

    switch (commandId) {
      case 'start': {
        const ROKO = getRoko();
        entries[0].outputBuffer = '';
        await entries[0].typeCmd(ROKO);
        const start = Date.now();
        while (Date.now() - start < 30000) {
          await rawSleep(300);
          const buf = stripAnsi(entries[0].outputBuffer);
          if (/❯|roko>|\/help|model|chat/i.test(buf)) break;
        }
        await rawSleep(800);
        return { ok: true };
      }

      case 'ask': {
        entries[0].outputBuffer = '';
        await entries[0].typeCmd('explain what cascade routing does', 20);
        const rStart = Date.now();
        while (Date.now() - rStart < 60000) {
          await rawSleep(500);
          const buf = stripAnsi(entries[0].outputBuffer);
          if (buf.length > 200 && /❯|roko>/i.test(buf.slice(-200))) break;
        }
        await rawSleep(500);
        return { ok: true };
      }

      case 'slash-status': {
        entries[0].outputBuffer = '';
        await entries[0].typeCmd('/status', 20);
        await rawSleep(3000);
        return { ok: true };
      }

      case 'slash-model': {
        entries[0].outputBuffer = '';
        await entries[0].typeCmd('/model', 20);
        await rawSleep(2000);
        return { ok: true };
      }

      default:
        return { ok: false, error: `Unknown command: ${commandId}` };
    }
  },
};
