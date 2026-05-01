import { useState, useEffect, useRef, useCallback } from 'react';
import { MIRAGE_WS_URL } from '../lib/serve-url';

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

export function useBlockStream(enabled = true): UseBlockStreamResult {
  const [blocks, setBlocks] = useState<BlockInfo[]>([]);
  const [connected, setConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
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

    let attempt = 0;
    let disposed = false;

    function connect() {
      if (disposed) return;
      try {
        const ws = new WebSocket(MIRAGE_WS_URL);
        wsRef.current = ws;

        ws.onopen = () => {
          attempt = 0;
          // Send eth_subscribe for newHeads
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
            // Subscription confirmation
            if (msg.id === 1 && msg.result) {
              setConnected(true);
              return;
            }
            // Subscription event
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
          if (!disposed && attempt < 5) {
            const delay = Math.min(1000 * 2 ** attempt, 15000);
            attempt++;
            reconnectTimer.current = setTimeout(connect, delay);
          }
        };

        ws.onerror = () => {
          ws.close();
        };
      } catch {
        // WebSocket constructor can throw if URL is invalid
        setConnected(false);
      }
    }

    connect();

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
