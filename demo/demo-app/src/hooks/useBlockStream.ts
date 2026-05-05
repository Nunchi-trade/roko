import { useState, useEffect, useRef, useCallback } from 'react';
import { MIRAGE_WS_URL, SERVE_URL } from '../lib/serve-url';

export interface BlockInfo {
  number: number;
  hash: string;
  timestamp: number;
}

interface UseBlockStreamResult {
  blocks: BlockInfo[];
  connected: boolean;
  latestBlock: BlockInfo | null;
}

const MAX_BLOCKS = 20;

/**
 * Connects to mirage-rs via WebSocket and subscribes to newHeads.
 * Only attempts connection after a successful HTTP pre-flight check
 * to avoid spamming console errors when mirage-rs isn't running.
 */
export function useBlockStream(enabled = true): UseBlockStreamResult {
  const [blocks, setBlocks] = useState<BlockInfo[]>([]);
  const [connected, setConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const pollIntervalRef = useRef<ReturnType<typeof setInterval>>(undefined);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const pendingBlocks = useRef<BlockInfo[]>([]);
  const rafId = useRef<number>(0);

  // Batch new blocks with requestAnimationFrame to avoid jank at 20 blocks/sec
  const flushPending = useCallback(() => {
    rafId.current = 0;
    if (pendingBlocks.current.length === 0) return;
    const batch = pendingBlocks.current.splice(0);
    setBlocks(prev => {
      const next = [...prev, ...batch];
      return next.slice(-MAX_BLOCKS);
    });
  }, []);

  useEffect(() => {
    if (!enabled) return;

    let disposed = false;
    let attempt = 0;
    const MAX_ATTEMPTS = 2; // Don't spam — 2 retries max
    const directMirageWs = MIRAGE_WS_URL;

    async function pollViaServe() {
      try {
        const res = await fetch(`${SERVE_URL}/api/chain/status`, {
          signal: AbortSignal.timeout(2000),
        });
        if (!res.ok) {
          setConnected(false);
          return;
        }
        const status = await res.json() as { block_number?: number };
        const number = Number(status.block_number ?? 0);
        if (!Number.isFinite(number) || number <= 0) {
          setConnected(false);
          return;
        }
        setConnected(true);
        const block: BlockInfo = {
          number,
          hash: '',
          timestamp: Date.now(),
        };
        setBlocks(prev => {
          if (prev[prev.length - 1]?.number === number) return prev;
          return [...prev, block].slice(-MAX_BLOCKS);
        });
      } catch {
        setConnected(false);
      }
    }

    if (!directMirageWs) {
      pollViaServe();
      reconnectTimer.current = setInterval(pollViaServe, 2000) as unknown as ReturnType<typeof setTimeout>;
      return () => {
        disposed = true;
        clearInterval(reconnectTimer.current);
      };
    }

    const wsUrl: string = directMirageWs;

    // Derive HTTP URL from WS URL for pre-flight check
    const httpUrl = wsUrl.replace(/^ws:/, 'http:').replace(/^wss:/, 'https:');

    async function preflight(): Promise<boolean> {
      try {
        const res = await fetch(httpUrl, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_blockNumber', params: [] }),
          signal: AbortSignal.timeout(2000),
        });
        return res.ok;
      } catch {
        return false;
      }
    }

    async function tryConnect() {
      if (disposed) return;

      // Pre-flight: check if mirage-rs is reachable via HTTP first
      const reachable = await preflight();
      if (!reachable) {
        // Mirage not available — don't attempt WS connection at all
        return;
      }

      connect();
    }

    function connect() {
      if (disposed) return;
      try {
        const ws = new WebSocket(wsUrl);
        wsRef.current = ws;

        ws.onopen = () => {
          attempt = 0;
          ws.send(JSON.stringify({
            jsonrpc: '2.0',
            id: 1,
            method: 'eth_subscribe',
            params: ['newHeads'],
          }));
        };

        ws.onmessage = (evt) => {
          try {
            const msg = JSON.parse(evt.data);
            if (msg.id === 1 && msg.result) {
              setConnected(true);
              return;
            }
            if (msg.method === 'eth_subscription' && msg.params?.result) {
              const head = msg.params.result;
              const block: BlockInfo = {
                number: parseInt(head.number, 16),
                hash: head.hash,
                timestamp: parseInt(head.timestamp, 16),
              };
              pendingBlocks.current.push(block);
              if (!rafId.current) {
                rafId.current = requestAnimationFrame(flushPending);
              }
            }
          } catch { /* ignore parse errors */ }
        };

        ws.onclose = () => {
          setConnected(false);
          wsRef.current = null;
          if (!disposed && attempt < MAX_ATTEMPTS) {
            const delay = Math.min(2000 * 2 ** attempt, 10000);
            attempt++;
            reconnectTimer.current = setTimeout(tryConnect, delay);
          }
        };

        ws.onerror = () => {
          ws.close();
        };
      } catch {
        setConnected(false);
      }
    }

    tryConnect();

    return () => {
      disposed = true;
      clearTimeout(reconnectTimer.current);
      if (rafId.current) cancelAnimationFrame(rafId.current);
      wsRef.current?.close();
      wsRef.current = null;
      setConnected(false);
    };
  }, [enabled, flushPending]);

  const latestBlock = blocks.length > 0 ? blocks[blocks.length - 1] : null;

  return { blocks, connected, latestBlock };
}
