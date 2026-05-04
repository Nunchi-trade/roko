/**
 * Scenario orchestration layer over useTerminal.
 *
 * Provides workspace management, command execution with typing animation,
 * output detection (gates, cost, tokens), and roko binary resolution.
 *
 * Reference: demo-web/demo.html showCmd/detectFromOutput
 */
import type { TerminalHandle } from '../hooks/useTerminal';
import type { PlaybackController } from './playback-controller';
import type { ScenarioContext } from './scenarios';
import { lookupCmdDesc } from './cmd-descriptions';

// ── ANSI stripping ──────────────────────────────────────────

export function stripAnsi(text: string): string {
  return text.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '');
}

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
  // Emit absolute paths so the binary remains reachable after `cd` into a workspace.
  const result = await handle.execCmd(
    'command -v roko >/dev/null 2>&1 && echo RP || { test -x ./target/release/roko && echo "RR:$PWD/target/release/roko" || { test -x ./target/debug/roko && echo "RD:$PWD/target/debug/roko" || echo RN; }; }',
    4000,
  );
  if (result.ok || result.exitCode >= 0) {
    const buf = handle.outputBuffer;
    if (buf.includes('RP')) {
      resolvedRoko = 'roko';
    } else if (buf.includes('RR:')) {
      const m = buf.match(/RR:(\S+)/);
      resolvedRoko = m ? m[1] : './target/release/roko';
    } else if (buf.includes('RD:')) {
      const m = buf.match(/RD:(\S+)/);
      resolvedRoko = m ? m[1] : './target/debug/roko';
    } else {
      resolvedRoko = 'roko';
    }
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

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

/**
 * Re-enter the expected workspace before hidden or visible scenario commands.
 *
 * Demo runs are long-lived terminal sessions; this keeps generated commands
 * anchored to the server-created workspace even if a previous command changed
 * directories or a shell integration restored a different working directory.
 */
export async function ensureWorkspaceCwd(
  handle: TerminalHandle,
  dir: string,
  timeout = 5000,
): Promise<boolean> {
  const cdResult = await handle.execCmd(`cd ${shellQuote(dir)}`, timeout);
  if (!cdResult.ok) {
    console.error('[ensureWorkspaceCwd] cd failed:', dir, cdResult);
    return false;
  }
  return true;
}

/**
 * Build a roko CLI command string, automatically injecting workspace and model context.
 * Every scenario runner should use this instead of `${ROKO} subcommand`.
 */
export function roko(ctx: ScenarioContext, subcommand: string): string {
  const bin = getRoko();
  const parts = [bin];
  if (ctx.workspaceDir) {
    parts.push('--repo', shellQuote(ctx.workspaceDir));
  }
  if (ctx.activeModel) {
    parts.push('--model', shellQuote(ctx.activeModel));
  }
  parts.push(subcommand);
  return parts.join(' ');
}

// ── Workspace entry ──────────────────────────────────────────

/**
 * Enter a workspace that was already created server-side via POST /api/workspaces.
 * Waits for WS + shell prompt, resolves roko binary, cd into dir.
 *
 * Throws on failure so scenario runners get a clear error instead of
 * silently proceeding against a broken terminal.
 */
export async function enterWorkspace(
  handle: TerminalHandle,
  dir: string,
): Promise<boolean> {
  // 1. Wait for WebSocket to be open
  const wsOk = await waitForOpen(handle, 8000);
  if (!wsOk) {
    console.error('[enterWorkspace] WebSocket never opened for', handle.sessionId);
    throw new Error(`Terminal WebSocket failed to connect (session: ${handle.sessionId})`);
  }

  // 2. Wait for shell prompt — the useTerminal hook now waits for this
  //    during connection, but we double-check here with a generous timeout.
  //    If the first check fails, send a blank line to nudge the shell.
  let promptOk = await handle.waitForPrompt(6000);
  if (!promptOk) {
    console.warn('[enterWorkspace] First prompt check failed, sending blank line to nudge shell');
    handle.sendRaw('\r');
    promptOk = await handle.waitForPrompt(5000);
  }
  if (!promptOk) {
    console.error('[enterWorkspace] Shell prompt never appeared for', handle.sessionId);
    throw new Error(`Shell prompt not detected (session: ${handle.sessionId}). Terminal may be hung.`);
  }

  // 3. Resolve roko binary location
  await resolveRoko(handle);

  // 4. cd into workspace
  const cdResult = await ensureWorkspaceCwd(handle, dir);
  if (!cdResult) {
    throw new Error(`Failed to cd into workspace: ${dir}`);
  }

  // 5. Clear screen so setup noise is invisible
  handle.clearTerminal();
  console.debug('[enterWorkspace] ready:', dir, 'roko:', getRoko());
  return true;
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

/**
 * Type a command character-by-character into the terminal with a typing
 * animation. Only types — does NOT send Enter or wait for output.
 * Returns false if the WebSocket closes mid-typing.
 */
async function typeChars(
  handle: TerminalHandle,
  cmd: string,
): Promise<boolean> {
  if (!handle?.ws || handle.ws.readyState !== WebSocket.OPEN) return false;

  for (const ch of cmd) {
    if (!handle.ws || handle.ws.readyState !== WebSocket.OPEN) return false;
    handle.ws.send(ch);
    await adjustedSleep(6 + Math.random() * 3);
  }

  return true;
}

/**
 * Type a command with animation into the terminal, press Enter, and wait
 * for the shell prompt to reappear (indicating the command finished).
 *
 * @param handle - Terminal handle
 * @param cmd - Shell command to type
 * @param opts - Options for command display and detection
 * @returns Command result with detected metrics
 */
export async function showCmd(
  handle: TerminalHandle,
  cmd: string,
  opts?: {
    timeout?: number;
    customDesc?: string;
    /** Called when the command is typed (before execution). */
    onLog?: (cmd: string, desc: string) => void;
    /** Called when the command finishes running. */
    onLogComplete?: (cmd: string, ok: boolean) => void;
    onGate?: (name: string, status: 'pass' | 'fail') => void;
    onCost?: (cost: string) => void;
    onTokens?: (tokens: string) => void;
    signal?: AbortSignal;
    /** Re-enter this workspace before typing the visible command. */
    workspaceDir?: string;
    /** Unused — kept for caller compatibility. */
    playback?: PlaybackController;
  },
): Promise<CommandResult> {
  const timeout = opts?.timeout ?? 60000;
  const desc = opts?.customDesc ?? lookupCmdDesc(cmd) ?? 'Executing command...';
  const startTime = Date.now();

  // Clear output buffer for fresh detection
  handle.outputBuffer = '';

  // Log to command panel (as pending — not yet complete)
  opts?.onLog?.(cmd, desc);

  if (opts?.signal?.aborted) {
    opts?.onLogComplete?.(cmd, false);
    return { ok: false, elapsed: 0, gates: [], cost: null, tokens: null };
  }

  if (opts?.workspaceDir) {
    const cwdOk = await ensureWorkspaceCwd(handle, opts.workspaceDir);
    if (!cwdOk) {
      opts?.onLogComplete?.(cmd, false);
      return { ok: false, elapsed: (Date.now() - startTime) / 1000, gates: [], cost: null, tokens: null };
    }
    handle.outputBuffer = '';
  }

  // Type the command character-by-character (visible animation)
  const typed = await typeChars(handle, cmd);
  if (!typed) {
    opts?.onLogComplete?.(cmd, false);
    return { ok: false, elapsed: 0, gates: [], cost: null, tokens: null };
  }

  // Press Enter and wait for prompt
  if (!handle.ws || handle.ws.readyState !== WebSocket.OPEN) {
    opts?.onLogComplete?.(cmd, false);
    return { ok: false, elapsed: (Date.now() - startTime) / 1000, gates: [], cost: null, tokens: null };
  }
  handle.ws.send('\r');
  const ok = await handle.waitForPrompt(timeout, opts?.signal);

  const elapsed = (Date.now() - startTime) / 1000;

  // Detect gates, cost, tokens from output
  const result = detectFromOutput(handle.outputBuffer, opts);

  // Print a visible separator line directly in the xterm display
  try {
    handle.terminal.write('\r\n\x1b[38;5;132m' + '\u2500'.repeat(60) + '\x1b[0m\r\n');
  } catch {
    // terminal may be disposed
  }

  // Mark the log entry as complete
  opts?.onLogComplete?.(cmd, ok);

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
  rawText: string,
  opts?: {
    onGate?: (name: string, status: 'pass' | 'fail') => void;
    onCost?: (cost: string) => void;
    onTokens?: (tokens: string) => void;
  },
): DetectionResult {
  const text = stripAnsi(rawText);
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
 * Deduplicates callbacks — only fires when values change.
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
  let lastCost: string | null = null;
  let lastTokens: string | null = null;
  const seenGates = new Set<string>();

  return setInterval(() => {
    const result = detectFromOutput(handle.outputBuffer);
    if (result.cost && result.cost !== lastCost) {
      lastCost = result.cost;
      opts.onCost?.(result.cost);
    }
    if (result.tokens && result.tokens !== lastTokens) {
      lastTokens = result.tokens;
      opts.onTokens?.(result.tokens);
    }
    for (const gate of result.gates) {
      const key = `${gate.name}:${gate.status}`;
      if (!seenGates.has(key)) {
        seenGates.add(key);
        opts.onGate?.(gate.name, gate.status);
      }
    }
  }, intervalMs);
}
