import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApi } from './useApi';
import { SERVE_URL } from '../lib/serve-url';

// ── Module-level health probe singleton ─────────────────────────────
//
// @deprecated — This module-level singleton duplicates the health polling
// now handled by `bootstrapTransport()` in `src/app/bootstrap.ts`.
// New code should read `useServerConnected()` or `useServerStatus()`
// from `src/data/selectors.ts` instead.
//
// Shared server reachability state. It is intentionally re-probed because the
// demo UI is commonly opened before `roko serve` is ready.
let _serverLive: boolean | null = null; // null = unknown
let _healthProbeInFlight: Promise<void> | null = null;
const _healthListeners = new Set<() => void>();

function notifyHealthListeners(): void {
  for (const listener of _healthListeners) {
    listener();
  }
}

function probeServer(): Promise<void> {
  if (_healthProbeInFlight) return _healthProbeInFlight;
  _healthProbeInFlight = (async () => {
    let nextLive = false;
    try {
      const res = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(2000) });
      nextLive = res.ok;
    } catch {
      nextLive = false;
    } finally {
      if (_serverLive !== nextLive) {
        _serverLive = nextLive;
        notifyHealthListeners();
      } else {
        _serverLive = nextLive;
      }
      _healthProbeInFlight = null;
    }
  })();
  return _healthProbeInFlight;
}

/**
 * @deprecated Use `useServerConnected()` from `src/data/selectors.ts` for
 *   health status, and `api` from `src/transport/api.ts` for REST calls.
 *   The module-level `_serverLive` / `probeServer()` singleton is superseded
 *   by the single health poll in `bootstrapTransport()`.
 */
export function useLiveApi() {
  const api = useApi();
  const [isLive, setIsLive] = useState(_serverLive === true);

  useEffect(() => {
    const listener = () => {
      setIsLive(_serverLive === true);
    };

    _healthListeners.add(listener);
    void probeServer();
    listener();

    // Start polling while at least one subscriber exists.
    const interval = setInterval(() => {
      void probeServer();
    }, 5_000);

    return () => {
      _healthListeners.delete(listener);
      clearInterval(interval);
    };
  }, []);

  const get = useCallback(async <T = unknown>(path: string): Promise<T> => {
    try {
      const data = await api.get<T>(path);
      _serverLive = true;
      notifyHealthListeners();
      return data;
    } catch (error) {
      void probeServer();
      throw error;
    }
  }, [api]);

  const post = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    try {
      const data = await api.post<T>(path, body);
      _serverLive = true;
      notifyHealthListeners();
      return data;
    } catch (error) {
      void probeServer();
      throw error;
    }
  }, [api]);

  const put = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    try {
      const data = await api.put<T>(path, body);
      _serverLive = true;
      notifyHealthListeners();
      return data;
    } catch (error) {
      void probeServer();
      throw error;
    }
  }, [api]);

  return useMemo(
    () => ({ get, post, put, baseUrl: api.baseUrl, isLive }),
    [get, post, put, api.baseUrl, isLive],
  );
}
