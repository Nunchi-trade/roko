import { useEffect, useRef, useState, useCallback } from 'react';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface InsightEvent {
  type: 'posted' | 'confirmed' | 'stateTransition';
  id: string;
  kind?: string;        // heuristic | strategy | causal | warning
  content?: string;
  author?: string;
  createdAt?: number;
  by?: string;          // confirmer
  at?: number;
  from?: string;        // state transition
  to?: string;
}

export interface PheromoneEvent {
  id: number;
  kind: string;
  intensity: number;
  depositedAt: number;
}

export interface ChainWsStats {
  insights: number;
  confirms: number;
  pheromones: number;
}

export interface ChainWsState {
  connected: boolean;
  insights: InsightEvent[];
  pheromones: PheromoneEvent[];
  stats: ChainWsStats;
  error: string | null;
  refresh: () => void;
}

// ---------------------------------------------------------------------------
// WebSocket message types from mirage-rs
// ---------------------------------------------------------------------------

interface WsConnectedMsg {
  type: 'connected';
  pheromones: boolean;
  insights: boolean;
  agents: boolean;
  predictions: boolean;
}

interface WsChannelMsg {
  channel: 'insight' | 'pheromone' | 'agent';
  data: InsightEvent | PheromoneEvent;
  type?: 'lagged';
  missed?: number;
}

type WsMessage = WsConnectedMsg | WsChannelMsg;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

const MIRAGE_HOST = 'localhost:8545';
const WS_URL = `ws://${MIRAGE_HOST}/api/ws?insights=true&pheromones=true&agents=true`;
const MAX_INSIGHTS = 200;
const MAX_PHEROMONES = 200;

/** Exponential backoff: 1s, 2s, 4s, 8s, … capped at 30s */
const INITIAL_BACKOFF_MS = 1_000;
const MAX_BACKOFF_MS = 30_000;
const MAX_RETRIES = 5;

// ---------------------------------------------------------------------------
// useChainWs — real WebSocket hook
// ---------------------------------------------------------------------------

export function useChainWs(enabled = true): ChainWsState {
  const [connected, setConnected] = useState(false);
  const [insights, setInsights] = useState<InsightEvent[]>([]);
  const [pheromones, setPheromones] = useState<PheromoneEvent[]>([]);
  const [stats, setStats] = useState<ChainWsStats>({ insights: 0, confirms: 0, pheromones: 0 });
  const [error, setError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const backoffRef = useRef(INITIAL_BACKOFF_MS);
  const retriesRef = useRef(0);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const mountedRef = useRef(true);

  const connect = useCallback(() => {
    // Clean up any existing connection
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }

    const ws = new WebSocket(WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      if (!mountedRef.current) return;
      backoffRef.current = INITIAL_BACKOFF_MS;
      retriesRef.current = 0;
      setError(null);
      // connected state is set when we receive the server confirmation message
    };

    ws.onmessage = (e: MessageEvent) => {
      if (!mountedRef.current) return;
      let msg: WsMessage;
      try {
        msg = JSON.parse(e.data as string);
      } catch {
        return;
      }

      // Connection confirmation from mirage
      if ('type' in msg && msg.type === 'connected') {
        setConnected(true);
        return;
      }

      const channelMsg = msg as WsChannelMsg;

      // Backpressure warning
      if (channelMsg.type === 'lagged') {
        console.warn(`[useChainWs] backpressure: missed ${channelMsg.missed} events on ${channelMsg.channel}`);
        return;
      }

      if (channelMsg.channel === 'insight') {
        const data = channelMsg.data as InsightEvent;
        setInsights(prev => {
          const next = [...prev, data];
          return next.length > MAX_INSIGHTS ? next.slice(-MAX_INSIGHTS) : next;
        });
        setStats(prev => ({
          ...prev,
          insights: data.type === 'posted' ? prev.insights + 1 : prev.insights,
          confirms: data.type === 'confirmed' ? prev.confirms + 1 : prev.confirms,
        }));
      } else if (channelMsg.channel === 'pheromone') {
        const data = channelMsg.data as PheromoneEvent;
        setPheromones(prev => {
          const next = [...prev, data];
          return next.length > MAX_PHEROMONES ? next.slice(-MAX_PHEROMONES) : next;
        });
        setStats(prev => ({ ...prev, pheromones: prev.pheromones + 1 }));
      }
    };

    ws.onclose = () => {
      if (!mountedRef.current) return;
      wsRef.current = null;
      setConnected(false);

      retriesRef.current += 1;
      if (retriesRef.current > MAX_RETRIES) {
        setError('mirage-rs unreachable — chain features disabled');
        return;
      }

      // Schedule reconnect with exponential backoff
      const delay = backoffRef.current;
      backoffRef.current = Math.min(delay * 2, MAX_BACKOFF_MS);
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      reconnectTimer.current = setTimeout(() => {
        if (mountedRef.current) connect();
      }, delay);
    };

    ws.onerror = () => {
      if (!mountedRef.current) return;
      if (retriesRef.current >= MAX_RETRIES) {
        setError('mirage-rs unreachable — chain features disabled');
      } else {
        setError('WebSocket error — will retry');
      }
      // onclose fires after onerror, so reconnect is handled there
    };
  }, []);

  const refresh = useCallback(() => {
    backoffRef.current = INITIAL_BACKOFF_MS;
    retriesRef.current = 0;
    if (reconnectTimer.current) {
      clearTimeout(reconnectTimer.current);
      reconnectTimer.current = null;
    }
    setError(null);
    connect();
  }, [connect]);

  useEffect(() => {
    mountedRef.current = true;
    if (enabled) {
      connect();
    }

    return () => {
      mountedRef.current = false;
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
      wsRef.current = null;
    };
  }, [connect, enabled]);

  return { connected, insights, pheromones, stats, error, refresh };
}
