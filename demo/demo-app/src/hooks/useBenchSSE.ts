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
    if (!enabled) {
      setConnected(false);
      esRef.current?.close();
      esRef.current = null;
      return;
    }

    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    const params = benchId ? `?bench_id=${encodeURIComponent(benchId)}` : '';
    setConnected(false);

    function connect() {
      if (cancelled) return;

      esRef.current?.close();

      const es = new EventSource(`${SERVE_URL}/api/bench/events${params}`);
      esRef.current = es;

      es.onopen = () => {
        if (cancelled || esRef.current !== es) return;
        setConnected(true);
      };

      es.onmessage = (e) => {
        if (cancelled || esRef.current !== es) return;

        try {
          const parsed = JSON.parse(e.data) as BenchSSEEvent;
          const eventBenchId = 'bench_id' in parsed ? parsed.bench_id : undefined;
          if (benchId && eventBenchId && eventBenchId !== benchId) return;
          setLastEvent(parsed);
          setEvents((prev) => [...prev, parsed]);
        } catch {
          // Ignore unparseable events
        }
      };

      es.onerror = () => {
        if (cancelled || esRef.current !== es) {
          es.close();
          return;
        }

        setConnected(false);
        es.close();
        esRef.current = null;
        clearTimeout(reconnectTimer);
        reconnectTimer = setTimeout(connect, 3_000);
      };
    }

    connect();

    return () => {
      cancelled = true;
      clearTimeout(reconnectTimer);
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }
    };
  }, [benchId, enabled]);

  return { connected, lastEvent, events, clear };
}
