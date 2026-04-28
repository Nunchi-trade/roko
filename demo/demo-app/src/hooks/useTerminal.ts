import { useEffect, useRef, useCallback, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { rosedustTheme } from '../lib/rosedust-theme';
import { WS_BASE, SERVE_URL } from '../lib/serve-url';

export interface TerminalHandle {
  terminal: Terminal;
  fit: FitAddon;
  ws: WebSocket | null;
  sessionId: string;
  status: 'connecting' | 'connected' | 'disconnected';
}

/**
 * Manages a single xterm.js terminal + optional WebSocket PTY session.
 * Attach to a container div via the returned ref callback.
 * The terminal is always created — WebSocket is best-effort.
 */
export function useTerminal(sessionId?: string) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const handleRef = useRef<TerminalHandle | null>(null);
  const [status, setStatus] = useState<TerminalHandle['status']>('connecting');

  const attach = useCallback((el: HTMLDivElement | null) => {
    containerRef.current = el;
  }, []);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const id = sessionId ?? `t${Date.now()}`;
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

    // Delay fit to let layout settle
    requestAnimationFrame(() => fitAddon.fit());

    // Create handle immediately — terminal is ready for direct writes
    const handle: TerminalHandle = { terminal: term, fit: fitAddon, ws: null, sessionId: id, status: 'connecting' };
    handleRef.current = handle;

    // Try to connect to PTY backend (best-effort)
    const connect = async () => {
      try {
        // Create session via REST
        await fetch(`${SERVE_URL}/api/terminal/sessions`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ session_id: id, cols: term.cols, rows: term.rows }),
        });
      } catch {
        // No PTY server — that's fine, terminal still works for demo playback
        handle.status = 'connected';
        setStatus('connected');
        return;
      }

      try {
        const ws = new WebSocket(`${WS_BASE}/ws/terminal/${id}`);
        handle.ws = ws;

        ws.onopen = () => {
          handle.status = 'connected';
          setStatus('connected');
          ws.send(JSON.stringify({ type: 'resize', cols: term.cols, rows: term.rows }));
        };

        ws.onmessage = (e) => {
          if (typeof e.data === 'string') {
            try {
              const msg = JSON.parse(e.data);
              if (msg.type === 'output') {
                term.write(msg.data);
              }
            } catch {
              term.write(e.data);
            }
          }
        };

        ws.onclose = () => {
          handle.status = 'disconnected';
          setStatus('disconnected');
        };

        ws.onerror = () => {
          // WS failed — still usable for demo
          handle.status = 'connected';
          setStatus('connected');
        };

        // Terminal input → WS
        term.onData((data) => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: 'input', data }));
          }
        });

        // Terminal resize → WS
        term.onResize(({ cols, rows }) => {
          if (ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: 'resize', cols, rows }));
          }
        });
      } catch {
        // WS constructor failed — mark as connected for demo
        handle.status = 'connected';
        setStatus('connected');
      }
    };

    connect();

    // ResizeObserver for fit
    const ro = new ResizeObserver(() => fitAddon.fit());
    ro.observe(el);

    return () => {
      ro.disconnect();
      handle.ws?.close();
      term.dispose();
      handleRef.current = null;
    };
  }, [sessionId]);

  return { attach, status, handle: handleRef };
}
