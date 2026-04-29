import { useEffect, useRef, useState } from 'react';
import { SERVE_URL } from '../lib/serve-url';

export function useSSE(path: string) {
  const [connected, setConnected] = useState(false);
  const [lastEvent, setLastEvent] = useState<unknown>(null);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    let cancelled = false;
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    setConnected(false);

    function connect() {
      if (cancelled) return;

      esRef.current?.close();

      const es = new EventSource(`${SERVE_URL}${path}`);
      esRef.current = es;

      es.onopen = () => {
        if (cancelled || esRef.current !== es) return;
        setConnected(true);
      };

      es.onmessage = (e) => {
        if (cancelled || esRef.current !== es) return;

        try {
          setLastEvent(JSON.parse(e.data));
        } catch {
          setLastEvent(e.data);
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
  }, [path]);

  return { connected, lastEvent };
}
