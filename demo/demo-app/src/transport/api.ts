import { SERVE_URL } from '../lib/serve-url';

export interface ApiError {
  status: number;
  statusText: string;
  body: string | null;
}

/** Result type -- never throws. Callers check `.ok` and branch. */
export type ApiResult<T> = { ok: true; data: T } | { ok: false; error: ApiError };

/** Health probe result cached with TTL. */
export interface HealthSnapshot {
  reachable: boolean;
  checkedAt: number; // Date.now() ms
}

export class RokoApi {
  readonly baseUrl: string;
  private healthCache: HealthSnapshot | null;
  private healthInflight: Promise<HealthSnapshot> | null;
  private static readonly HEALTH_TTL_MS = 30_000;

  constructor(baseUrl?: string) {
    this.baseUrl = baseUrl ?? SERVE_URL;
    this.healthCache = null;
    this.healthInflight = null;
  }

  /** Internal fetch helper -- never throws. */
  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    signal?: AbortSignal,
  ): Promise<ApiResult<T>> {
    const url = this.baseUrl + path;
    const headers: Record<string, string> = {};
    if (body !== undefined) {
      headers['Content-Type'] = 'application/json';
    }
    try {
      const res = await fetch(url, {
        method,
        headers,
        body: body !== undefined ? JSON.stringify(body) : undefined,
        signal,
      });
      if (!res.ok) {
        const text = await res.text().catch(() => null);
        return { ok: false, error: { status: res.status, statusText: res.statusText, body: text } };
      }
      const data = (await res.json()) as T;
      return { ok: true, data };
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      return { ok: false, error: { status: 0, statusText: message, body: null } };
    }
  }

  /** GET with JSON parse. Returns ApiResult -- never throws. */
  get<T = unknown>(path: string, signal?: AbortSignal): Promise<ApiResult<T>> {
    return this.request<T>('GET', path, undefined, signal);
  }

  /** POST with JSON body. Returns ApiResult -- never throws. */
  post<T = unknown>(path: string, body?: unknown, signal?: AbortSignal): Promise<ApiResult<T>> {
    return this.request<T>('POST', path, body, signal);
  }

  /** PUT with JSON body. Returns ApiResult -- never throws. */
  put<T = unknown>(path: string, body?: unknown, signal?: AbortSignal): Promise<ApiResult<T>> {
    return this.request<T>('PUT', path, body, signal);
  }

  /** DELETE. Returns ApiResult -- never throws. */
  delete<T = unknown>(path: string, signal?: AbortSignal): Promise<ApiResult<T>> {
    return this.request<T>('DELETE', path, undefined, signal);
  }

  /** Probe /health with 30s TTL cache + 2s timeout. Deduplicated -- only one in-flight. */
  probe(force?: boolean): Promise<HealthSnapshot> {
    if (
      !force &&
      this.healthCache &&
      Date.now() - this.healthCache.checkedAt < RokoApi.HEALTH_TTL_MS
    ) {
      return Promise.resolve(this.healthCache);
    }
    if (this.healthInflight) {
      return this.healthInflight;
    }
    this.healthInflight = (async () => {
      let snapshot: HealthSnapshot;
      try {
        const res = await fetch(this.baseUrl + '/health', {
          signal: AbortSignal.timeout(2000),
        });
        snapshot = { reachable: res.ok, checkedAt: Date.now() };
      } catch {
        snapshot = { reachable: false, checkedAt: Date.now() };
      }
      this.healthCache = snapshot;
      this.healthInflight = null;
      return snapshot;
    })();
    return this.healthInflight;
  }
}

/** Singleton instance. Import this everywhere instead of constructing. */
export const api = new RokoApi();
