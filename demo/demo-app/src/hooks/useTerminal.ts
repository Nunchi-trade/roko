import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
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
      fontFamily: "'JetBrains Mono', 'SF Mono', monospace",
      fontSize: 13,
      cursorBlink: true,
      allowProposedApi: true,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(el);

    requestAnimationFrame(() => fitAddon.fit());

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
      await sleep(500);
      while (Date.now() - start < timeout) {
        const tail = stripAnsi(outBuf).slice(-300);
        if (PROMPT_RE.test(tail)) {
          const snapshot = outBuf.length;
          await sleep(400);
          if (outBuf.length === snapshot) {
            const recheck = stripAnsi(outBuf).slice(-300);
            if (PROMPT_RE.test(recheck)) return true;
          }
        }
        await sleep(100);
      }
      return false;
    };

    handle.waitForMarker = async (marker: string, timeout = 30000): Promise<boolean> => {
      const start = Date.now();
      while (Date.now() - start < timeout) {
        if (outBuf.includes(marker)) return true;
        await sleep(100);
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

    function connectWs() {
      const ws = new WebSocket(`${WS_BASE}/ws/terminal/${id}`);
      ws.binaryType = 'arraybuffer';

      ws.onopen = () => {
        handle.ws = ws;
        handle.status = 'connected';
        setStatus('connected');
        const dims = fitAddon.proposeDimensions();
        if (dims) {
          ws.send(JSON.stringify({ type: 'resize', cols: dims.cols, rows: dims.rows }));
        }
      };

      ws.onmessage = (e: MessageEvent) => {
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
        setStatus('disconnected');
        if (mountedRef.current) {
          reconnectTimer = setTimeout(connectWs, 2000);
        }
      };

      ws.onerror = () => {
        // Don't change status on error — onclose will handle disconnection
      };

      term.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        }
      });

      term.onResize(({ cols, rows }) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: 'resize', cols, rows }));
        }
      });
    }

    connectWs();

    const ro = new ResizeObserver(() => fitAddon.fit());
    ro.observe(el);

    return () => {
      mountedRef.current = false;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      ro.disconnect();
      handle.ws?.close();
      term.dispose();
      handleRef.current = null;
    };
  }, [sessionId]);

  return { attach, status, handle: handleRef };
}
