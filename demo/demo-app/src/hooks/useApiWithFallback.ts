import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApi } from './useApi';
import { SERVE_URL } from '../lib/serve-url';
import * as Demo from '../lib/demo-data';

// Map API paths to demo fallback data (used only when offline or endpoint returns nothing)
function getFallback(path: string): unknown {
  if (path.includes('/health') && !path.includes('/providers/health')) return Demo.DEMO_HEALTH;
  if (path.includes('/managed-agents')) return Demo.DEMO_AGENTS;
  if (path.includes('/agents/topology')) return Demo.DEMO_AGENT_TOPOLOGY;
  if (path.includes('/knowledge/entries')) return Demo.DEMO_KNOWLEDGE_ENTRIES;
  if (path.includes('/knowledge/edges')) return Demo.DEMO_KNOWLEDGE_EDGES;
  if (path.includes('/episodes')) return Demo.DEMO_EPISODES;
  if (path.includes('/learn/efficiency')) return Demo.DEMO_EFFICIENCY;
  if (path.includes('/metrics/c_factor')) return Demo.DEMO_CFACTOR;
  if (path.includes('/c-factor/trend')) return Demo.DEMO_CFACTOR_TREND;
  if (path.includes('/learn/cascade-router')) return Demo.DEMO_ROUTER_MODELS;
  if (path.includes('/gates/summary')) return Demo.DEMO_GATES_SUMMARY;
  if (path.includes('/gates/history')) return Demo.DEMO_GATE_HISTORY;
  if (path.includes('/learn/adaptive-thresholds')) return Demo.DEMO_ADAPTIVE_THRESHOLDS;
  if (path.includes('/status')) return Demo.DEMO_STATUS;
  if (path.includes('/statehub/events')) return Demo.DEMO_EVENTS;
  if (path.includes('/dashboard')) return Demo.DEMO_DASHBOARD;
  if (path.includes('/learn/provider-outcomes') || path.includes('/providers/health')) return Demo.DEMO_PROVIDER_HEALTH;
  if (path.includes('/cost-race') || path.includes('/bench/cost-summary')) return Demo.DEMO_COST_RACE;
  if (path.includes('/dream/journal')) return Demo.DEMO_DREAM_JOURNAL;
  if (path.includes('/config') && !path.includes('/config/')) return Demo.DEMO_CONFIG;
  if (path.includes('/share/')) return null;
  return {};
}

// Shared server reachability state. It is intentionally re-probed because the
// demo UI is commonly opened before `roko serve` is ready.
let _serverLive: boolean | null = null; // null = unknown
let _healthPoll: ReturnType<typeof setInterval> | null = null;
let _healthProbeInFlight: Promise<void> | null = null;
const _healthListeners = new Set<() => void>();

// Tally of fallback vs non-fallback records observed across all API responses.
let _seedCount = 0;
let _nonSeedCount = 0;
let _fallbackCount = 0;
const _dataModeListeners = new Set<() => void>();

function notifyHealthListeners(): void {
  for (const listener of _healthListeners) {
    listener();
  }
}

function notifyDataModeListeners(): void {
  for (const listener of _dataModeListeners) {
    listener();
  }
}

function tallySourceField(data: unknown): void {
  if (!Array.isArray(data)) return;
  for (const item of data) {
    if (item && typeof item === 'object') {
      if ((item as Record<string, unknown>).source === 'seed') {
        _seedCount += 1;
      } else {
        _nonSeedCount += 1;
      }
    } else {
      _nonSeedCount += 1;
    }
  }
}

function deriveDataMode(): 'seed' | 'live' | 'unknown' {
  if (_nonSeedCount === 0 && _seedCount > 0) return 'seed';
  if (_nonSeedCount > 0) return 'live';
  if (_fallbackCount > 0) return 'seed';
  return 'unknown';
}

function recordLiveData(data: unknown): void {
  tallySourceField(data);
  notifyDataModeListeners();
}

function recordFallbackData(data: unknown): void {
  _fallbackCount += 1;
  tallySourceField(data);
  notifyDataModeListeners();
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

function ensureHealthPolling(): void {
  void probeServer();
  if (_healthPoll) return;
  _healthPoll = setInterval(() => {
    void probeServer();
  }, 5_000);
}

export function useApiWithFallback() {
  const api = useApi();
  const [isLive, setIsLive] = useState(_serverLive === true);
  const [dataMode, setDataMode] = useState<'seed' | 'live' | 'unknown'>('unknown');

  useEffect(() => {
    const listener = () => {
      setIsLive(_serverLive === true);
    };

    _healthListeners.add(listener);
    ensureHealthPolling();
    listener();

    return () => {
      _healthListeners.delete(listener);
    };
  }, []);

  useEffect(() => {
    const listener = () => {
      setDataMode(deriveDataMode());
    };

    _dataModeListeners.add(listener);
    listener();

    return () => {
      _dataModeListeners.delete(listener);
    };
  }, []);

  const get = useCallback(async <T = unknown>(path: string): Promise<T> => {
    // Offline → straight to fallback
    if (_serverLive === false) {
      const result = getFallback(path) as T;
      recordFallbackData(result);
      setDataMode(deriveDataMode());
      return result;
    }

    // Server is live (or probe hasn't finished) — always try the real API
    try {
      const data = await api.get<T>(path);
      recordLiveData(data);
      setDataMode(deriveDataMode());
      return data;
    } catch {
      // Network error or 4xx/5xx → fallback for this endpoint
      void probeServer();
      const result = getFallback(path) as T;
      recordFallbackData(result);
      setDataMode(deriveDataMode());
      return result;
    }
  }, [api]);

  const post = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    if (_serverLive === false) {
      throw new Error('roko serve is offline');
    }
    return api.post<T>(path, body);
  }, [api]);

  const put = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    if (_serverLive === false) {
      throw new Error('roko serve is offline');
    }
    return api.put<T>(path, body);
  }, [api]);

  return useMemo(
    () => ({ get, post, put, baseUrl: api.baseUrl, isLive, dataMode }),
    [get, post, put, api.baseUrl, isLive, dataMode],
  );
}
