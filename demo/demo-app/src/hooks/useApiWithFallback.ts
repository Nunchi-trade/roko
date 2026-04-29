import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApi } from './useApi';
import { SERVE_URL } from '../lib/serve-url';
import * as Demo from '../lib/demo-data';
import * as BenchDemo from '../lib/bench-demo-data';

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
  if (path.includes('/bench/suites')) return BenchDemo.DEMO_BENCH_SUITES;
  if (path.includes('/bench/models')) return BenchDemo.DEMO_BENCH_MODELS;
  if (path.includes('/bench/runs')) return BenchDemo.DEMO_BENCH_RUNS;
  if (path.includes('/dream/journal')) return Demo.DEMO_DREAM_JOURNAL;
  if (path.includes('/share/')) return null;
  return {};
}

// Probe: is the server reachable at all? That's the only question.
let _serverLive: boolean | null = null; // null = unknown
let _probePromise: Promise<void> | null = null;

// Tally of seed vs non-seed records observed across all API responses.
let _seedCount = 0;
let _nonSeedCount = 0;
let _dataModeListeners = new Set<() => void>();

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
  return 'unknown';
}

function probeServer(): Promise<void> {
  if (_probePromise) return _probePromise;
  _probePromise = (async () => {
    try {
      const res = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(2000) });
      _serverLive = res.ok;
    } catch {
      _serverLive = false;
    }
  })();
  return _probePromise;
}

export function useApiWithFallback() {
  const api = useApi();
  const [isLive, setIsLive] = useState(_serverLive === true);
  const [dataMode, setDataMode] = useState<'seed' | 'live' | 'unknown'>('unknown');

  useEffect(() => {
    probeServer().then(() => {
      setIsLive(_serverLive === true);
    });
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
      tallySourceField(result);
      setDataMode(deriveDataMode());
      notifyDataModeListeners();
      return result;
    }

    // Server is live (or probe hasn't finished) — always try the real API
    try {
      const data = await api.get<T>(path);
      tallySourceField(data);
      setDataMode(deriveDataMode());
      notifyDataModeListeners();
      return data;
    } catch {
      // Network error or 4xx/5xx → fallback for this endpoint
      const result = getFallback(path) as T;
      tallySourceField(result);
      setDataMode(deriveDataMode());
      notifyDataModeListeners();
      return result;
    }
  }, [api]);

  const post = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    if (_serverLive === false) return {} as T;
    try {
      return await api.post<T>(path, body);
    } catch {
      return {} as T;
    }
  }, [api]);

  return useMemo(
    () => ({ get, post, baseUrl: api.baseUrl, isLive, dataMode }),
    [get, post, api.baseUrl, isLive, dataMode],
  );
}
