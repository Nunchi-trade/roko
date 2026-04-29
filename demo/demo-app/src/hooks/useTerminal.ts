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

const PROMPT_RE = /[❯\$%>#]\s*$/;

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;]*[A-Za-z]/g, '');
}

export interface TerminalHandle {
  terminal: Terminal;
  fit: FitAddon;
  ws: WebSocket | null;
  sessionId: string;
  status: 'connecting' | 'connected' | 'disconnected';
  /** Bounded output buffer for prompt detection and output scraping */
  outputBuffer: string;
  /** Send a command instantly and wait for shell prompt */
  execCmd(cmd: string, timeout?: number): Promise<boolean>;
  /** Type a command char-by-char then wait for prompt */
  typeCmd(cmd: string, speed?: number, timeout?: number): Promise<boolean>;
  /** Wait for a shell prompt to appear in output buffer */
  waitForPrompt(timeout?: number): Promise<boolean>;
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
      smoothScrollDuration: 80,
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

    term.open(el);

    // GPU-accelerated WebGL renderer with DOM fallback
    try {
      const webglAddon = new WebglAddon();
      webglAddon.onContextLoss(() => { if (!disposed) webglAddon.dispose(); });
      term.loadAddon(webglAddon);
    } catch {
      // DOM renderer fallback — fine for low-end GPUs
    }

    // Push cursor to bottom so text anchors at the bottom
    requestAnimationFrame(() => {
      if (!disposed) {
        try {
          fitAddon.fit();
          const rows = term.rows;
          if (rows > 1) {
            term.write('\n'.repeat(rows - 1));
          }
        } catch { /* disposed */ }
      }
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

    handle.waitForPrompt = async (timeout = 60000): Promise<boolean> => {
      const start = Date.now();
      await sleep(80);
      while (Date.now() - start < timeout) {
        const tail = stripAnsi(outBuf).slice(-300);
        if (PROMPT_RE.test(tail)) {
          const snapshot = outBuf.length;
          await sleep(120);
          if (outBuf.length === snapshot) {
            const recheck = stripAnsi(outBuf).slice(-300);
            if (PROMPT_RE.test(recheck)) return true;
          }
        }
        await sleep(30);
      }
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

    handle.execCmd = async (cmd: string, timeout = 30000): Promise<boolean> => {
      const marker = `__RK${(++execSeq).toString(36)}${Date.now().toString(36)}__`;
      outBuf = '';
      handle.sendRaw(`${cmd}; echo ${marker}\r`);
      return handle.waitForMarker(marker, timeout);
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
      const ws = new WebSocket(`${WS_BASE}/ws/terminal/${id}`);
      ws.binaryType = 'arraybuffer';

      ws.onopen = () => {
        if (disposed) { ws.close(); return; }
        handle.ws = ws;
        handle.status = 'connected';
        if (!disposed) setStatus('connected');
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
      };

      ws.onmessage = (e: MessageEvent) => {
        if (disposed) return;
        if (e.data instanceof ArrayBuffer) {
          const text = new TextDecoder().decode(e.data);
          term.write(new Uint8Array(e.data));
          appendOutput(text);
        } else if (typeof e.data === 'string') {
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

    connectWs();

    const ro = new ResizeObserver(() => { if (!disposed) { try { fitAddon.fit(); } catch { /* disposed */ } } });
    ro.observe(el);

    return () => {
      disposed = true;
      mountedRef.current = false;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ro.disconnect();
      onDataDisposable.dispose();
      onResizeDisposable.dispose();
      handle.ws?.close();
      term.dispose();
      handleRef.current = null;
    };
  }, [sessionId]);

  return { attach, status, handle: handleRef };
}
