import { useEffect, useState, useCallback, useRef } from 'react';
import { SERVE_URL } from '../lib/serve-url';

export type ServerStatus = 'connected' | 'disconnected' | 'checking';

/**
 * Poll /health and track connection state.
 * Returns status + a `checkNow()` for immediate re-check (e.g. on play press).
 */
export function useServerHealth(intervalMs = 3000) {
  const [status, setStatus] = useState<ServerStatus>('checking');
  const cancelledRef = useRef(false);

  const check = useCallback(async () => {
    try {
      const res = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(2000) });
      if (!cancelledRef.current) setStatus(res.ok ? 'connected' : 'disconnected');
    } catch {
      if (!cancelledRef.current) setStatus('disconnected');
    }
  }, []);

  useEffect(() => {
    cancelledRef.current = false;
    check();
    const id = setInterval(check, intervalMs);
    return () => { cancelledRef.current = true; clearInterval(id); };
  }, [intervalMs, check]);

  return { status, checkNow: check };
}
