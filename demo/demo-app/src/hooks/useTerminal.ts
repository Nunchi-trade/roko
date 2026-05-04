import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { ClipboardAddon } from '@xterm/addon-clipboard';
import { ImageAddon } from '@xterm/addon-image';
import { rosedustTheme } from '../lib/rosedust-theme';
import { WS_BASE } from '../lib/serve-url';
import { stripAnsi } from '../lib/strip-ansi';

// Match common shell prompts. The key chars are: $ % # > ❯ → ➜ ➤ ›
// Also matches bash's default `bash-5.2$ ` and zsh's `user@host% ` patterns.
// Tested against the tail of a stripped output buffer (multiline).
const PROMPT_RE = /[^\n]*[❯%#$>→➜➤›]\s*$/m;

export interface ExecResult {
  ok: boolean;
  exitCode: number;
}

export interface TerminalHandle {
  terminal: Terminal;
  fit: FitAddon;
  ws: WebSocket | null;
  sessionId: string;
  status: 'connecting' | 'connected' | 'disconnected';
  /** Bounded output buffer for prompt detection and output scraping */
  outputBuffer: string;
  /** Send a command and wait for completion via invisible OSC sideband.
   *  When `silent` is true, suppresses terminal echo so the wrapper text
   *  is not visible. Use for hidden helper commands (cd, exit-check). */
  execCmd(cmd: string, timeout?: number, opts?: { silent?: boolean }): Promise<ExecResult>;
  /** Type a command char-by-char then wait for prompt */
  typeCmd(cmd: string, speed?: number, timeout?: number): Promise<boolean>;
  /** Wait for a shell prompt to appear in output buffer */
  waitForPrompt(timeout?: number, signal?: AbortSignal): Promise<boolean>;
  /** Wait for a specific marker string in output */
  waitForMarker(marker: string, timeout?: number): Promise<boolean>;
  /** Get current output buffer content */
  getOutputBuffer(): string;
  /** Clear terminal screen and output buffer */
  clearTerminal(): void;
  /** Send raw text to PTY */
  sendRaw(data: string): void;
}

let execSeq = 0;

function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Manages a single xterm.js terminal + WebSocket PTY session.
 * Attach to a container div via the returned ref callback.
 *
 * Protocol:
 * - WS endpoint auto-creates sessions (no REST POST needed)
 * - Server sends Binary(data) for PTY output
 * - Client sends raw text for input
 * - Client sends JSON only for resize: { type: 'resize', cols, rows }
 */
export function useTerminal(sessionId?: string) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const handleRef = useRef<TerminalHandle | null>(null);
  const mountedRef = useRef(true);
  const [status, setStatus] = useState<TerminalHandle['status']>('connecting');

  const attach = useCallback((el: HTMLDivElement | null) => {
    containerRef.current = el;
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    const el = containerRef.current;
    if (!el) return;

    const id = sessionId ?? `t${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
    const term = new Terminal({
      theme: rosedustTheme,
      fontFamily: "'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', 'SF Mono', monospace",
      fontSize: 12,
      lineHeight: 1.1,
      letterSpacing: 0,
      cursorBlink: true,
      cursorStyle: 'bar',
      cursorWidth: 2,
      cursorInactiveStyle: 'outline',
      scrollback: 5000,
      scrollSensitivity: 2,
      fastScrollSensitivity: 8,
      fastScrollModifier: 'alt',
      drawBoldTextInBrightColors: false,
      fontWeight: '400',
      fontWeightBold: '600',
      minimumContrastRatio: 1,
      smoothScrollDuration: 0,
      allowProposedApi: true,
      allowTransparency: true,
      overviewRulerWidth: 8,
      customGlyphs: true,
      rescaleOverlappingGlyphs: true,
      macOptionIsMeta: true,
      macOptionClickForcesSelection: true,
      rightClickSelectsWord: true,
      altClickMovesCursor: true,
    });

    // Core addons
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.loadAddon(new WebLinksAddon(undefined, {
      hover: (event, uri) => {
        const target = event.target as HTMLElement;
        if (target) target.title = uri;
      },
    }));
    term.loadAddon(new ClipboardAddon());

    // Unicode 11 for better character width handling
    const unicode11 = new Unicode11Addon();
    term.loadAddon(unicode11);
    term.unicode.activeVersion = '11';

    // Inline image support (Sixel + iTerm2 protocol)
    term.loadAddon(new ImageAddon());

    // Track disposal state for async callbacks.
    let disposed = false;
    let ready = false;
    const pendingMessages: Uint8Array[] = [];

    term.open(el);

    // ── OSC 7777 sideband: invisible command completion signaling ──
    // Commands wrapped by execCmd emit OSC 7777;D;<exitCode>;<marker> which
    // xterm.js intercepts here and never renders. This replaces the old
    // __RKxxx__ echo-based markers that leaked visible junk into the terminal.
    type OscListener = (exitCode: number, marker: string) => void;
    const oscListeners = new Set<OscListener>();
    const oscDisposable = term.parser.registerOscHandler(7777, (data) => {
      // Format: "D;<exitCode>;<marker>"
      const parts = data.split(';');
      if (parts[0] === 'D' && parts.length >= 3) {
        const exitCode = parseInt(parts[1], 10);
        const marker = parts.slice(2).join(';');
        for (const listener of oscListeners) {
          listener(exitCode, marker);
        }
      }
      return true; // swallow — never display
    });

    // GPU-accelerated WebGL renderer with DOM fallback
    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => { if (!disposed) webglAddon.dispose(); });
      term.loadAddon(webglAddon);
    } catch {
      // DOM renderer fallback — fine for low-end GPUs
    }

    // Fit terminal then connect WS so it has correct dimensions.
    requestAnimationFrame(() => {
      if (disposed) return;
      try {
        fitAddon.fit();
        // Flush any messages buffered before terminal was ready
        ready = true;
        for (const buf of pendingMessages) {
          term.write(buf);
          appendOutput(new TextDecoder().decode(buf));
        }
        pendingMessages.length = 0;
        // Now connect WS — terminal is fitted and ready for data
        connectWs();
      } catch { /* disposed */ }
    });

    // Internal output buffer, capped high enough for artifact scraping.
    let outBuf = '';

    function appendOutput(text: string) {
      outBuf += text;
      if (outBuf.length > 60000) outBuf = outBuf.slice(-40000);
    }

    // Build handle using defineProperties for outputBuffer getter/setter
    const handle = {} as TerminalHandle;

    Object.defineProperties(handle, {
      terminal: { value: term, enumerable: true },
      fit: { value: fitAddon, enumerable: true },
      ws: { value: null, writable: true, enumerable: true },
      sessionId: { value: id, enumerable: true },
      status: { value: 'connecting' as TerminalHandle['status'], writable: true, enumerable: true },
      outputBuffer: {
        get() { return outBuf; },
        set(val: string) { outBuf = val; },
        enumerable: true,
      },
    });

    handle.getOutputBuffer = () => outBuf;

    handle.clearTerminal = () => {
      term.clear();
      term.write('\x1b[2J\x1b[3J\x1b[H');
      outBuf = '';
    };

    handle.sendRaw = (data: string) => {
      if (handle.ws && handle.ws.readyState === WebSocket.OPEN) {
        handle.ws.send(data);
      }
    };

    handle.waitForPrompt = async (timeout = 60000, signal?: AbortSignal): Promise<boolean> => {
      const start = Date.now();
      await sleep(50);
      while (Date.now() - start < timeout) {
        if (signal?.aborted) return false;
        // Check a much larger window — long command output shouldn't hide the prompt
        const tail = stripAnsi(outBuf).slice(-2000);
        if (PROMPT_RE.test(tail)) {
          // Stability check: wait a short time and re-check.
          // Allow up to 50 bytes of noise (partial PTY writes) — only require
          // that the prompt still appears, not that the buffer stopped growing.
          const snapshot = outBuf.length;
          await sleep(60);
          if (signal?.aborted) return false;
          const growth = outBuf.length - snapshot;
          if (growth <= 50) {
            const recheck = stripAnsi(outBuf).slice(-2000);
            if (PROMPT_RE.test(recheck)) return true;
          }
        }
        await sleep(20);
      }
      console.warn('[useTerminal] waitForPrompt timed out after', timeout, 'ms. Buffer tail:', JSON.stringify(stripAnsi(outBuf).slice(-200)));
      return false;
    };

    handle.waitForMarker = async (marker: string, timeout = 30000): Promise<boolean> => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        if (outBuf.includes(marker)) return true;
        await sleep(30);
      }
      return false;
    };

    handle.execCmd = async (cmd: string, timeout = 30000, opts?: { silent?: boolean }): Promise<ExecResult> => {
      const marker = `rk${(++execSeq).toString(36)}${Date.now().toString(36)}`;
      outBuf = '';
      // Wrap command: run it, capture exit code, emit invisible OSC 7777
      // sequence, then propagate the original exit code to the shell.
      // The printf produces an OSC escape that xterm.js intercepts and
      // swallows — nothing visible appears in the terminal.
      const wrapped = `${cmd}; __rk_ec=$?; printf '\\033]7777;D;%d;${marker}\\033\\\\' "$__rk_ec"; (exit $__rk_ec)`;
      if (wrapped.length > 3000) {
        console.warn(`[useTerminal] execCmd sending large command (${wrapped.length} chars): ${cmd.slice(0, 60)}...`);
      }
      // Record cursor row before sending so we can erase the echoed text later.
      const preRow = opts?.silent ? term.buffer.active.cursorY : -1;
      handle.sendRaw(wrapped + '\r');
      const result = await new Promise<ExecResult>((resolve) => {
        let settled = false;
        const listener: OscListener = (exitCode, m) => {
          if (m === marker && !settled) {
            settled = true;
            oscListeners.delete(listener);
            resolve({ ok: exitCode === 0, exitCode });
          }
        };
        oscListeners.add(listener);
        setTimeout(() => {
          if (!settled) {
            settled = true;
            oscListeners.delete(listener);
            console.warn(`[useTerminal] execCmd timed out after ${timeout}ms: ${cmd.slice(0, 80)}`);
            resolve({ ok: false, exitCode: -1 });
          }
        }, timeout);
      });

      // For silent commands, erase the echoed wrapper text from the terminal.
      // We cursor-up from the current position back to where we started and
      // clear each line. A short sleep lets the PTY prompt arrive first.
      if (preRow >= 0) {
        await sleep(20);
        try {
          const postRow = term.buffer.active.cursorY;
          const lines = Math.max(0, postRow - preRow) + 1;
          // Move to start of the echoed area and clear each line
          term.write('\x1b[2K' + '\x1b[1A\x1b[2K'.repeat(Math.min(lines, 8)) + '\r');
        } catch { /* terminal disposed */ }
      }
      return result;
    };

    handle.typeCmd = async (cmd: string, charDelay = 12, timeout = 60000): Promise<boolean> => {
      if (!handle.ws || handle.ws.readyState !== WebSocket.OPEN) return false;
      for (const ch of cmd) {
        handle.ws.send(ch);
        await sleep(charDelay + Math.random() * 6);
      }
      await sleep(40);
      handle.ws.send('\r');
      return handle.waitForPrompt(timeout);
    };

    handleRef.current = handle;

    // --- WebSocket connection with auto-reconnect ---

    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    // Register terminal I/O handlers once — they read from the handle's
    // current `ws` so they survive reconnects without stacking.
    const onDataDisposable = term.onData((data) => {
      if (handle.ws && handle.ws.readyState === WebSocket.OPEN) {
        handle.ws.send(data);
      }
    });

    const onResizeDisposable = term.onResize(({ cols, rows }) => {
      if (!disposed && handle.ws && handle.ws.readyState === WebSocket.OPEN) {
        handle.ws.send(JSON.stringify({ type: 'resize', cols, rows }));
      }
    });

    function connectWs() {
      if (disposed) return;
      const url = `${WS_BASE}/ws/terminal/${id}`;
      console.log(`[useTerminal:${id}] connecting to ${url}`);
      const ws = new WebSocket(url);
      ws.binaryType = 'arraybuffer';

      ws.onopen = () => {
        if (disposed) { ws.close(); return; }
        handle.ws = ws;
        // Mark as 'connecting' until we detect a shell prompt — 'connected'
        // means the PTY shell is actually ready, not just that the WS is open.
        handle.status = 'connecting';
        if (!disposed) setStatus('connecting');
        try {
          if (!disposed) {
            const dims = fitAddon.proposeDimensions();
            if (dims) {
              ws.send(JSON.stringify({ type: 'resize', cols: dims.cols, rows: dims.rows }));
            }
          }
        } catch {
          // Terminal may have been disposed between open and this callback
        }
        // Wait for the shell to actually print its first prompt before
        // declaring "connected". This prevents scenarios from starting
        // before the PTY shell has finished loading .bashrc/.zshrc.
        (async () => {
          const shellReady = await handle.waitForPrompt(8000);
          if (disposed) return;
          if (shellReady) {
            handle.status = 'connected';
            if (!disposed) setStatus('connected');
            console.log(`[useTerminal:${id}] shell ready (prompt detected)`);
          } else {
            // Prompt not found but WS is open — mark connected anyway
            // so scenarios can attempt to proceed (they have their own checks).
            handle.status = 'connected';
            if (!disposed) setStatus('connected');
            console.warn(`[useTerminal:${id}] shell prompt not detected within 8s, proceeding anyway. Buffer tail: ${JSON.stringify(stripAnsi(outBuf).slice(-200))}`);
          }
        })();
      };

      ws.onmessage = (e: MessageEvent) => {
        if (disposed) return;
        if (e.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(e.data);
          if (!ready) {
            pendingMessages.push(bytes);
            return;
          }
          term.write(bytes);
          appendOutput(new TextDecoder().decode(e.data));
        } else if (typeof e.data === 'string') {
          if (!ready) {
            pendingMessages.push(new TextEncoder().encode(e.data));
            return;
          }
          term.write(e.data);
          appendOutput(e.data);
        }
      };

      ws.onclose = () => {
        handle.ws = null;
        handle.status = 'disconnected';
        if (!disposed) setStatus('disconnected');
        if (!disposed && mountedRef.current) {
          if (reconnectTimer) clearTimeout(reconnectTimer);
          reconnectTimer = setTimeout(connectWs, 500);
        }
      };

      ws.onerror = () => {
        handle.status = 'disconnected';
        if (!disposed) setStatus('disconnected');
      };
    }

    // connectWs() is called from the requestAnimationFrame callback above
    // after fit completes, not here — avoids the WS race condition.

    const ro = new ResizeObserver(() => { if (!disposed) { try { fitAddon.fit(); } catch { /* disposed */ } } });
    ro.observe(el);

    return () => {
      disposed = true;
      mountedRef.current = false;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ro.disconnect();
      onDataDisposable.dispose();
      onResizeDisposable.dispose();
      oscDisposable.dispose();
      oscListeners.clear();
      handle.ws?.close();
      term.dispose();
      handleRef.current = null;
    };
  }, [sessionId]);

  return { attach, status, handle: handleRef };
}
