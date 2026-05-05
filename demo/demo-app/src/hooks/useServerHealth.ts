import { useEffect, useState, useCallback } from 'react';
import { SERVE_URL, TIMEOUTS } from '../lib/serve-url';
import { useEventStreamContext } from '../contexts/EventStreamContext';

export type ServerStatus = 'connected' | 'disconnected' | 'checking';

/**
 * Track server connection state via SSE push + one-shot fetch fallback.
 * Returns status + a `checkNow()` for immediate re-check (e.g. on play press).
 *
 * Replaces the previous setInterval polling pattern with:
 * 1. Initial fetch on mount
 * 2. SSE subscription for push health updates
 * 3. Fallback timeout if no SSE health event arrives within 10s
 */
export function useServerHealth() {
  const [status, setStatus] = useState<ServerStatus>('checking');
  const { subscribe, connected: sseConnected } = useEventStreamContext();

  // When SSE itself connects/disconnects, that's a strong signal about server health
  useEffect(() => {
    if (sseConnected) {
      setStatus('connected');
    }
  }, [sseConnected]);

  useEffect(() => {
    // Initial fetch
    const ac = new AbortController();
    fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) })
      .then(r => { if (!ac.signal.aborted) setStatus(r.ok ? 'connected' : 'disconnected'); })
      .catch(() => { if (!ac.signal.aborted) setStatus('disconnected'); });

    // SSE subscription for push updates (server:health or health event types)
    const unsub = subscribe(['server:health', 'health'], (event) => {
      const ok = (event as Record<string, unknown>)?.ok ??
        (event as Record<string, unknown>)?.status === 'ok';
      setStatus(ok ? 'connected' : 'disconnected');
    });

    // Fallback: if no SSE health event within 10s, do a single re-check
    const fallback = setTimeout(() => {
      fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) })
        .then(r => setStatus(r.ok ? 'connected' : 'disconnected'))
        .catch(() => setStatus('disconnected'));
    }, 10_000);

    return () => {
      ac.abort();
      unsub();
      clearTimeout(fallback);
    };
  }, [subscribe]);

  const checkNow = useCallback(async () => {
    try {
      const r = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(TIMEOUTS.health) });
      setStatus(r.ok ? 'connected' : 'disconnected');
    } catch {
      setStatus('disconnected');
    }
  }, []);

  return { status, checkNow };
}
