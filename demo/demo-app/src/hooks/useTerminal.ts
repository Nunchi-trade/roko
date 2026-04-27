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
 * Manages a single xterm.js terminal + WebSocket PTY session.
 * Attach to a container div via the returned ref callback.
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

    // Create PTY session then connect WS
    const handle: TerminalHandle = { terminal: term, fit: fitAddon, ws: null, sessionId: id, status: 'connecting' };
    handleRef.current = handle;
    setStatus('connecting');

    const connect = async () => {
      try {
        // Create session via REST
        await fetch(`${SERVE_URL}/api/terminal/sessions`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ session_id: id, cols: term.cols, rows: term.rows }),
        });
      } catch {
        // Session might already exist — that's fine
      }

      const ws = new WebSocket(`${WS_BASE}/ws/terminal/${id}`);
      handle.ws = ws;

      ws.onopen = () => {
        handle.status = 'connected';
        setStatus('connected');
        // Send initial resize
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
