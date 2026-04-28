import { useEffect, useRef, useState } from 'react';
import { SERVE_URL } from '../lib/serve-url';

export function useSSE(path: string) {
  const [connected, setConnected] = useState(false);
  const [lastEvent, setLastEvent] = useState<unknown>(null);
  const esRef = useRef<EventSource | null>(null);

  useEffect(() => {
    let reconnectTimer: ReturnType<typeof setTimeout>;

    function connect() {
      const es = new EventSource(`${SERVE_URL}${path}`);
      esRef.current = es;

      es.onopen = () => setConnected(true);

      es.onmessage = (e) => {
        try {
          setLastEvent(JSON.parse(e.data));
        } catch {
          setLastEvent(e.data);
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
  }, [path]);

  return { connected, lastEvent };
}
