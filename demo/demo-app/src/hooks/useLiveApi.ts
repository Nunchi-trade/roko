import { useCallback, useMemo } from 'react';
import { useApi } from './useApi';
import { useServerConnected } from '../data/selectors';

/**
 * Provides REST helpers (`get`, `post`, `put`) alongside an `isLive` flag
 * derived from the DataHub SSE-backed server status.
 *
 * Previous implementation polled `/health` every 5 seconds via a module-level
 * singleton. That polling is now removed; `isLive` reads from the centralised
 * `useServerConnected()` selector which is driven by SSE + the 30 s bootstrap
 * health poll in `bootstrapTransport()`.
 *
 * @deprecated Prefer `useServerConnected()` for connectivity checks and `api`
 *   from `src/transport/api.ts` for REST calls. This hook is retained only for
 *   backward-compatible callsites that destructure `{ get, post, put, isLive }`.
 */
export function useLiveApi() {
  const api = useApi();
  const isLive = useServerConnected();

  const get = useCallback(
    async <T = unknown>(path: string): Promise<T> => {
      return api.get<T>(path);
    },
    [api],
  );

  const post = useCallback(
    async <T = unknown>(path: string, body?: unknown): Promise<T> => {
      return api.post<T>(path, body);
    },
    [api],
  );

  const put = useCallback(
    async <T = unknown>(path: string, body?: unknown): Promise<T> => {
      return api.put<T>(path, body);
    },
    [api],
  );

  return useMemo(
    () => ({ get, post, put, baseUrl: api.baseUrl, isLive }),
    [get, post, put, api.baseUrl, isLive],
  );
}
