import { useCallback, useMemo } from 'react';
import { SERVE_URL } from '../lib/serve-url';

/** Simple fetch wrapper with base URL resolution. */
export function useApi() {
  const get = useCallback(async <T = unknown>(path: string): Promise<T> => {
    const res = await fetch(`${SERVE_URL}${path}`);
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    return res.json() as Promise<T>;
  }, []);

  const post = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    const res = await fetch(`${SERVE_URL}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    return res.json() as Promise<T>;
  }, []);

  const put = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    const res = await fetch(`${SERVE_URL}${path}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
    return res.json() as Promise<T>;
  }, []);

  return useMemo(() => ({ get, post, put, baseUrl: SERVE_URL }), [get, post, put]);
}
