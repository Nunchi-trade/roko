import { useEffect, useRef, useState, useCallback } from 'react';
import { SERVE_URL } from '../lib/serve-url';
import type { BenchSSEEvent } from '../lib/bench-types';

interface UseBenchSSEOptions {
  benchId?: string;
  enabled?: boolean;
}

export function useBenchSSE({ benchId, enabled = true }: UseBenchSSEOptions = {}) {
  const [connected, setConnected] = useState(false);
  const [lastEvent, setLastEvent] = useState<BenchSSEEvent | null>(null);
  const [events, setEvents] = useState<BenchSSEEvent[]>([]);
  const esRef = useRef<EventSource | null>(null);

  const clear = useCallback(() => {
    setEvents([]);
    setLastEvent(null);
  }, []);

  useEffect(() => {
    if (!enabled) return;

    let reconnectTimer: ReturnType<typeof setTimeout>;
    const params = benchId ? `?bench_id=${encodeURIComponent(benchId)}` : '';

    function connect() {
      const es = new EventSource(`${SERVE_URL}/api/bench/events${params}`);
      esRef.current = es;

      es.onopen = () => setConnected(true);

      es.onmessage = (e) => {
        try {
          const parsed = JSON.parse(e.data) as BenchSSEEvent;
          setLastEvent(parsed);
          setEvents((prev) => [...prev, parsed]);
        } catch {
          // Ignore unparseable events
        }
      };

      es.onerror = () => {
        setConnected(false);
        es.close();
        reconnectTimer = setTimeout(connect, 3_000);
      };
    }

    connect();

    return () => {
      clearTimeout(reconnectTimer);
      esRef.current?.close();
    };
  }, [benchId, enabled]);

  return { connected, lastEvent, events, clear };
}
