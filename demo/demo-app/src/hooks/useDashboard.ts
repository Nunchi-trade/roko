import { useEffect, useState, useCallback } from 'react';
import { useLiveApi } from './useLiveApi';

interface DashboardData {
  total_cost?: number;
  cache_hit_rate?: number;
  routing_distribution?: Record<string, number>;
  gate_pass_rate?: number;
}

export function useDashboard() {
  const { get } = useLiveApi();
  const [data, setData] = useState<DashboardData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const poll = useCallback(async () => {
    try {
      const d = await get<DashboardData>('/api/dashboard');
      setData(d);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch dashboard');
    } finally {
      setLoading(false);
    }
  }, [get]);

  useEffect(() => {
    poll();
    const id = setInterval(poll, 10_000);
    return () => clearInterval(id);
  }, [poll]);

  return { data, loading, error };
}
