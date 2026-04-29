import { useEffect, useState } from 'react';
import { SERVE_URL } from '../lib/serve-url';

export type ServerStatus = 'connected' | 'disconnected' | 'checking';

/**
 * Poll /health every `intervalMs` and track connection state.
 *
 * @deprecated Use `useServerStatus()` or `useServerConnected()` from
 *   `src/data/selectors.ts` instead. Health polling is now handled once
 *   in `src/app/bootstrap.ts` via `bootstrapTransport()`.
 */
export function useServerHealth(intervalMs = 5000) {
  const [status, setStatus] = useState<ServerStatus>('checking');

  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const res = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(2000) });
        if (!cancelled) setStatus(res.ok ? 'connected' : 'disconnected');
      } catch {
        if (!cancelled) setStatus('disconnected');
      }
    };
    check();
    const id = setInterval(check, intervalMs);
    return () => { cancelled = true; clearInterval(id); };
  }, [intervalMs]);

  return status;
}
