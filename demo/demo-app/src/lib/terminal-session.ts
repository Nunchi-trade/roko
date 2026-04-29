/**
 * Scenario orchestration layer over useTerminal.
 *
 * Provides workspace management, command execution with typing animation,
 * output detection (gates, cost, tokens), and roko binary resolution.
 *
 * Reference: demo-web/demo.html setupWorkspace/joinWorkspace/showCmd/detectFromOutput
 */
import type { TerminalHandle } from '../hooks/useTerminal';
import { lookupCmdDesc } from './cmd-descriptions';
import { ABSOLUTE_SERVE_URL } from './serve-url';

// ── Speed multiplier ─────────────────────────────────────────

let speedMultiplier = 1;

export function setSpeedMultiplier(m: number) {
  speedMultiplier = m;
}

// ── Helpers ──────────────────────────────────────────────────

function rawSleep(ms: number): Promise<void> {
  return new Promise(r => setTimeout(r, ms));
}

function adjustedSleep(ms: number): Promise<void> {
  return rawSleep(ms / speedMultiplier);
}

// ── Roko binary resolution ───────────────────────────────────

let resolvedRoko = 'roko';
let rokoResolved = false;

/**
 * Detect whether `roko` is on PATH, in ./target/release, or ./target/debug.
 * Caches the result globally. Uses execCmd (marker-based) to avoid leaving
 * visible shell garbage in the terminal.
 */
export async function resolveRoko(handle: TerminalHandle): Promise<string> {
  if (rokoResolved) return resolvedRoko;

  handle.outputBuffer = '';
  const ok = await handle.execCmd(
    'command -v roko >/dev/null 2>&1 && echo __ROKO_PATH__ || { test -x ./target/release/roko && echo __ROKO_REL__ || { test -x ./target/debug/roko && echo __ROKO_DBG__ || echo __ROKO_NONE__; }; }',
    4000,
  );
  if (ok) {
    const buf = handle.outputBuffer;
    if (buf.includes('__ROKO_PATH__')) resolvedRoko = 'roko';
    else if (buf.includes('__ROKO_REL__')) resolvedRoko = './target/release/roko';
    else if (buf.includes('__ROKO_DBG__')) resolvedRoko = './target/debug/roko';
    else resolvedRoko = 'roko';
  }
  rokoResolved = true;
  return resolvedRoko;
}

/** Reset roko resolution (e.g. when switching scenarios). */
export function resetRokoResolution() {
  rokoResolved = false;
  resolvedRoko = 'roko';
}

/** Get the resolved roko command. */
export function getRoko(): string {
  return resolvedRoko;
}

// ── Workspace management ─────────────────────────────────────

/**
 * Create an ephemeral workspace: mktemp, cd, roko init, fetch live config, clear terminal.
 * Returns the workspace directory path.
 *
 * Optimized to use a SINGLE PTY round-trip for all setup steps (mkdir, cd,
 * roko init, config copy) instead of 5 sequential commands.
 */
export async function setupWorkspace(
  handle: TerminalHandle,
  dirPrefix: string,
): Promise<string> {
  // Wait for WS connection + initial prompt
  const wsOk = await waitForOpen(handle);
  if (!wsOk) return '/tmp/roko-unavailable';
  await handle.waitForPrompt(5000);

  const dir = `/tmp/${dirPrefix}-${Date.now()}`;
  const ROKO = rokoResolved ? resolvedRoko : 'roko';

  // Single atomic command: resolve roko path + create workspace + init + copy config.
  // This replaces 5 sequential PTY round-trips with 1.
  const setupCmd = [
    `mkdir -p ${dir}`,
    `cd ${dir}`,
    `${ROKO} init`,
    `curl -sf --connect-timeout 2 --max-time 5 ${ABSOLUTE_SERVE_URL}/api/config/toml -o roko.toml 2>/dev/null; true`,
  ].join(' && ');

  await handle.execCmd(setupCmd, 30000);

  if (!rokoResolved) {
    resolvedRoko = ROKO;
    rokoResolved = true;
  }

  // Clear only the output buffer, not the visible terminal.
  // The terminal now shows the setup commands which is better than a blank screen.
  handle.outputBuffer = '';
  return dir;
}

/**
 * Join an existing workspace: cd into it.
 */
export async function joinWorkspace(
  handle: TerminalHandle,
  dir: string,
): Promise<void> {
  const wsOk = await waitForOpen(handle);
  if (!wsOk) return;
  await handle.waitForPrompt(5000);
  if (!rokoResolved) {
    resolvedRoko = 'roko';
    rokoResolved = true;
  }
  await handle.execCmd(`cd ${dir}`, 3000);
  handle.outputBuffer = '';
}

// ── Fast workspace entry (server-created) ────────────────────

/**
 * Enter a workspace that was already created server-side via POST /api/workspaces.
 * Much faster than setupWorkspace() since it only needs to `cd` into the directory.
 *
 * Note: Does NOT clear the terminal afterwards — the scenario should control when
 * clearing is appropriate. Aggressive clearing caused blank terminal panes.
 */
export async function enterWorkspace(
  handle: TerminalHandle,
  dir: string,
): Promise<void> {
  const wsOk = await waitForOpen(handle);
  if (!wsOk) return;
  await handle.waitForPrompt(5000);
  await resolveRoko(handle);
  await handle.execCmd(`cd ${dir}`, 3000);
  // Only clear the output buffer (for prompt detection), not the visible terminal.
  // Scenarios can call handle.clearTerminal() explicitly when they want a clean slate.
  handle.outputBuffer = '';
}

// ── Command execution with logging ──────────────────────────

export interface CommandResult {
  ok: boolean;
  elapsed: number;
  gates: GateResult[];
  cost: string | null;
  tokens: string | null;
}

export interface GateResult {
  name: string;
  status: 'pass' | 'fail';
}

async function typeVisibleCommandAndWait(
  handle: TerminalHandle,
  cmd: string,
  timeout: number,
): Promise<boolean> {
  if (!handle?.ws || handle.ws.readyState !== WebSocket.OPEN) return false;

  for (const ch of cmd) {
    if (!handle.ws || handle.ws.readyState !== WebSocket.OPEN) return false;
    handle.ws.send(ch);
    await adjustedSleep(6 + Math.random() * 3);
  }
  await adjustedSleep(20);
  if (!handle.ws || handle.ws.readyState !== WebSocket.OPEN) return false;
  handle.ws.send('\r');
  return handle.waitForPrompt(timeout);
}

/**
 * Type a command with animation, wait for prompt, and detect output.
 *
 * @param handle - Terminal handle
 * @param cmd - Shell command to execute
 * @param opts - Options for command execution
 * @returns Command result with detected metrics
 */
export async function showCmd(
  handle: TerminalHandle,
  cmd: string,
  opts?: {
    timeout?: number;
    customDesc?: string;
    onLog?: (cmd: string, desc: string) => void;
    onGate?: (name: string, status: 'pass' | 'fail') => void;
    onCost?: (cost: string) => void;
    onTokens?: (tokens: string) => void;
  },
): Promise<CommandResult> {
  const timeout = opts?.timeout ?? 60000;
  const desc = opts?.customDesc ?? lookupCmdDesc(cmd) ?? 'Executing command...';
  const startTime = Date.now();

  // Clear output buffer for fresh detection
  handle.outputBuffer = '';

  // Log to command panel
  opts?.onLog?.(cmd, desc);

  // Type and execute. Waiting on an explicit marker is more reliable than
  // trying to parse arbitrary themed shell prompts.
  const ok = await typeVisibleCommandAndWait(handle, cmd, timeout);

  const elapsed = (Date.now() - startTime) / 1000;

  // Detect gates, cost, tokens from output
  const result = detectFromOutput(handle.outputBuffer, opts);

  return {
    ok,
    elapsed,
    gates: result.gates,
    cost: result.cost,
    tokens: result.tokens,
  };
}

// ── Output detection ─────────────────────────────────────────

interface DetectionResult {
  gates: GateResult[];
  cost: string | null;
  tokens: string | null;
}

/**
 * Detect gates, cost, and token counts from command output.
 * Matching patterns from demo-web/demo.html detectFromOutput().
 */
function detectFromOutput(
  text: string,
  opts?: {
    onGate?: (name: string, status: 'pass' | 'fail') => void;
    onCost?: (cost: string) => void;
    onTokens?: (tokens: string) => void;
  },
): DetectionResult {
  const gates: GateResult[] = [];

  // Gate detection (✔ = pass, ✖ = fail)
  const gatePatterns: [string, RegExp, RegExp][] = [
    ['compile', /compile.*[✔✓]|[✔✓].*compile|compile.*\bpass\b|compile.*\bok\b/i, /compile.*[✖✗]|[✖✗].*compile/i],
    ['test', /\btest\b.*[✔✓]|[✔✓].*\btest\b|\btest\b.*\bpass\b|\btest\b.*\bok\b/i, /\btest\b.*[✖✗]|[✖✗].*\btest\b/i],
    ['clippy', /clippy.*[✔✓]|[✔✓].*clippy|clippy.*\bpass\b|clippy.*\bok\b/i, /clippy.*[✖✗]|[✖✗].*clippy/i],
    ['diff', /diff.*[✔✓]|[✔✓].*diff/i, /diff.*[✖✗]|[✖✗].*diff/i],
    ['coverage', /coverage.*[✔✓]|[✔✓].*coverage/i, /coverage.*[✖✗]|[✖✗].*coverage/i],
  ];

  for (const [name, passRe, failRe] of gatePatterns) {
    if (passRe.test(text)) {
      gates.push({ name, status: 'pass' });
      opts?.onGate?.(name, 'pass');
    } else if (failRe.test(text)) {
      gates.push({ name, status: 'fail' });
      opts?.onGate?.(name, 'fail');
    }
  }

  // Cost detection
  const costMatch = text.match(/\$(\d+\.\d+)/);
  const cost = costMatch ? `$${costMatch[1]}` : null;
  if (cost) opts?.onCost?.(cost);

  // Token detection
  const tokenMatch = text.match(/(\d[\d,]*)\s*(?:tokens?|tok)/i);
  const tokens = tokenMatch ? tokenMatch[1] : null;
  if (tokens) opts?.onTokens?.(tokens);

  return { gates, cost, tokens };
}

// ── Utilities ────────────────────────────────────────────────

/**
 * Wait for WebSocket to be open (max 8s).
 */
async function waitForOpen(handle: TerminalHandle, timeout = 5000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeout) {
    if (handle.ws && handle.ws.readyState === WebSocket.OPEN) return true;
    await rawSleep(30);
  }
  return false;
}

/**
 * Continuously detect metrics from a handle's output buffer.
 * Returns an interval ID that should be cleared when done.
 */
export function trackMetrics(
  handle: TerminalHandle,
  opts: {
    onCost?: (cost: string) => void;
    onTokens?: (tokens: string) => void;
    onGate?: (name: string, status: 'pass' | 'fail') => void;
  },
  intervalMs = 500,
): ReturnType<typeof setInterval> {
  return setInterval(() => {
    detectFromOutput(handle.outputBuffer, opts);
  }, intervalMs);
}
