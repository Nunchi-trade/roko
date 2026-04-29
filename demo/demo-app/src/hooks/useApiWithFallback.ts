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
  if (path.includes('/share/')) return null;
  return {};
}

// Probe: is the server reachable at all? That's the only question.
let _serverLive: boolean | null = null; // null = unknown
let _probePromise: Promise<void> | null = null;

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

  useEffect(() => {
    probeServer().then(() => {
      setIsLive(_serverLive === true);
    });
  }, []);

  const get = useCallback(async <T = unknown>(path: string): Promise<T> => {
    // Offline → straight to fallback
    if (_serverLive === false) {
      return getFallback(path) as T;
    }

    // Server is live (or probe hasn't finished) — always try the real API
    try {
      const data = await api.get<T>(path);
      return data;
    } catch {
      // Network error or 4xx/5xx → fallback for this endpoint
      return getFallback(path) as T;
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

  return useMemo(() => ({ get, post, baseUrl: api.baseUrl, isLive }), [get, post, api.baseUrl, isLive]);
}
