import { useEffect, useState, useCallback } from 'react';
import { useLiveApi } from './useLiveApi';

interface Agent {
  id: string;
  name: string;
  domain?: string;
  status: string;
  model?: string;
  capabilities?: string[];
  reputation?: number;
  stats?: {
    tasks?: number;
    cost?: number;
    tokens?: number;
  };
}

export function useAgents() {
  const { get } = useLiveApi();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const poll = useCallback(async () => {
    try {
      const data = await get<Agent[]>('/api/managed-agents');
      setAgents(Array.isArray(data) ? data : []);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch agents');
    } finally {
      setLoading(false);
    }
  }, [get]);

  useEffect(() => {
    poll();
    const id = setInterval(poll, 5_000);
    return () => clearInterval(id);
  }, [poll]);

  return { agents, loading, error };
}
