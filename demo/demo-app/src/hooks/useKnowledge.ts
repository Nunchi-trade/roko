import { useEffect, useState, useCallback } from 'react';
import { useLiveApi } from './useLiveApi';

interface KnowledgeEntry {
  id: string;
  domain?: string;
  citations?: number;
  label?: string;
}

interface KnowledgeEdge {
  source: string;
  target: string;
  frequency?: number;
}

export function useKnowledge() {
  const { get } = useLiveApi();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [edges, setEdges] = useState<KnowledgeEdge[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const poll = useCallback(async () => {
    try {
      const [eData, edData] = await Promise.all([
        get<{ items?: KnowledgeEntry[] } | KnowledgeEntry[]>('/api/knowledge/entries'),
        get<{ items?: KnowledgeEdge[] } | KnowledgeEdge[]>('/api/knowledge/edges'),
      ]);
      const e = Array.isArray(eData) ? eData : (eData.items ?? []);
      const ed = Array.isArray(edData) ? edData : (edData.items ?? []);
      setEntries(e);
      setEdges(ed);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch knowledge');
    } finally {
      setLoading(false);
    }
  }, [get]);

  useEffect(() => {
    poll();
    const id = setInterval(poll, 30_000);
    return () => clearInterval(id);
  }, [poll]);

  return { entries, edges, loading, error };
}
