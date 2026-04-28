import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApi } from './useApi';
import { SERVE_URL } from '../lib/serve-url';
import * as Demo from '../lib/demo-data';

/**
 * Checks if API response data looks "empty" — i.e. has no real business data.
 * The server may be UP (health returns ok) but have zero episodes, agents, etc.
 * In that case we should still show demo data for investor demos.
 */
function isEmptyData(path: string, data: unknown): boolean {
  if (data == null) return true;

  // Arrays: empty means fallback
  if (Array.isArray(data)) return data.length === 0;

  // Objects: check for known empty patterns
  if (typeof data === 'object') {
    const obj = data as Record<string, unknown>;

    // Health: check if statehub has real data
    if (path.includes('/health') && obj.status === 'ok') {
      const snap = (obj.statehub as Record<string, unknown>)?.snapshot as Record<string, unknown> | undefined;
      // If episodes_total is 0 or missing, it's effectively empty
      if (!snap || (snap.episodes_total === 0 && snap.cost_usd_total === 0)) return true;
      return false;
    }

    // Efficiency: no tasks = empty
    if (path.includes('/efficiency') && (!obj.tasks || (Array.isArray(obj.tasks) && obj.tasks.length === 0))) {
      return obj.total_cost === 0 || obj.total_cost === undefined;
    }

    // C-factor: composite 0 = empty
    if (path.includes('/c_factor')) {
      const comp = obj.composite as Record<string, unknown> | undefined;
      if (!comp || comp.overall === 0) return true;
      return false;
    }

    // Dashboard: all zeros = empty
    if (path.includes('/dashboard') && obj.total_cost === 0 && obj.gate_pass_rate === 0) return true;

    // Cascade router: no models = empty
    if (path.includes('/cascade-router')) {
      if (!obj.models || (Array.isArray(obj.models) && obj.models.length === 0)) return true;
      return false;
    }

    // Gates summary: 0 pass rate = empty
    if (path.includes('/gates/summary') && (obj.pass_rate === 0 || obj.pass_rate == null)) return true;

    // Status: all zeros
    if (path.includes('/status') && obj.episodes === 0 && obj.agents === 0) return true;
  }

  return false;
}

// Map API paths to demo data
function getFallback(path: string): unknown {
  if (path.includes('/health')) return Demo.DEMO_HEALTH;
  if (path.includes('/managed-agents')) return Demo.DEMO_AGENTS;
  if (path.includes('/knowledge/entries')) return Demo.DEMO_KNOWLEDGE_ENTRIES;
  if (path.includes('/knowledge/edges')) return Demo.DEMO_KNOWLEDGE_EDGES;
  if (path.includes('/episodes')) return Demo.DEMO_EPISODES;
  if (path.includes('/learn/efficiency')) return Demo.DEMO_EFFICIENCY;
  if (path.includes('/metrics/c_factor')) return Demo.DEMO_CFACTOR;
  if (path.includes('/learn/cascade-router')) return Demo.DEMO_ROUTER_MODELS;
  if (path.includes('/gates/summary')) return Demo.DEMO_GATES_SUMMARY;
  if (path.includes('/status')) return Demo.DEMO_STATUS;
  if (path.includes('/statehub/events')) return Demo.DEMO_EVENTS;
  if (path.includes('/dashboard')) return Demo.DEMO_DASHBOARD;
  if (path.includes('/share/')) return null; // no fallback for specific share tokens
  return {};
}

// Probe server once — but also check if it has real data
let _serverStatus: 'unknown' | 'live-with-data' | 'live-empty' | 'offline' = 'unknown';
let _probePromise: Promise<void> | null = null;

function probeServer(): Promise<void> {
  if (_probePromise) return _probePromise;
  _probePromise = (async () => {
    try {
      const res = await fetch(`${SERVE_URL}/health`, { signal: AbortSignal.timeout(2000) });
      if (!res.ok) { _serverStatus = 'offline'; return; }
      const data = await res.json();
      // Check if server has real business data
      const snap = data?.statehub?.snapshot;
      if (snap && (snap.episodes_total > 0 || snap.cost_usd_total > 0)) {
        _serverStatus = 'live-with-data';
      } else {
        _serverStatus = 'live-empty';
      }
    } catch {
      _serverStatus = 'offline';
    }
  })();
  return _probePromise;
}

export function useApiWithFallback() {
  const api = useApi();
  const [isLive, setIsLive] = useState(_serverStatus !== 'offline' && _serverStatus !== 'unknown');

  useEffect(() => {
    probeServer().then(() => {
      setIsLive(_serverStatus === 'live-with-data');
    });
  }, []);

  const get = useCallback(async <T = unknown>(path: string): Promise<T> => {
    // If server is offline or has no data, go straight to fallback
    if (_serverStatus === 'offline' || _serverStatus === 'live-empty') {
      return getFallback(path) as T;
    }

    try {
      const data = await api.get<T>(path);
      // Even if fetch succeeds, check if data is empty
      if (isEmptyData(path, data)) {
        return getFallback(path) as T;
      }
      return data;
    } catch {
      return getFallback(path) as T;
    }
  }, [api]);

  const post = useCallback(async <T = unknown>(path: string, body?: unknown): Promise<T> => {
    if (_serverStatus === 'offline') return {} as T;
    try {
      return await api.post<T>(path, body);
    } catch {
      return {} as T;
    }
  }, [api]);

  return useMemo(() => ({ get, post, baseUrl: api.baseUrl, isLive }), [get, post, api.baseUrl, isLive]);
}
