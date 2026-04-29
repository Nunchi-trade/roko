import { useState, useEffect, useCallback } from 'react';
import { useLiveApi } from './useLiveApi';
import { useBenchSSE } from './useBenchSSE';
import type {
  SweDataset,
  SweRun,
  SweInstance,
  BenchSSEEvent,
} from '../lib/bench-types';

export interface SweRunConfig {
  dataset: string;
  agent_mode: string;
  batch_size: number;
  offset: number;
  record_learning: boolean;
}

export function useSweBench() {
  const { get, post } = useLiveApi();

  const [datasets, setDatasets] = useState<SweDataset[]>([]);
  const [runs, setRuns] = useState<SweRun[]>([]);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [instances, setInstances] = useState<SweInstance[]>([]);
  const [status, setStatus] = useState<'idle' | 'running' | 'completed'>('idle');
  const [totalInstances, setTotalInstances] = useState(0);
  const [resolvedCount, setResolvedCount] = useState(0);

  // SSE for live updates
  const { lastEvent } = useBenchSSE({ enabled: status === 'running' });

  // Fetch datasets on mount
  useEffect(() => {
    (async () => {
      try {
        const data = await get<SweDataset[]>('/api/bench/swe/datasets');
        if (Array.isArray(data)) setDatasets(data);
      } catch { /* ok */ }
    })();
  }, [get]);

  // Fetch runs on mount
  useEffect(() => {
    (async () => {
      try {
        const data = await get<SweRun[]>('/api/bench/swe/runs');
        if (Array.isArray(data)) setRuns(data);
      } catch { /* ok */ }
    })();
  }, [get]);

  // Process SSE events
  useEffect(() => {
    if (!lastEvent || status !== 'running') return;
    const evt = lastEvent as BenchSSEEvent;

    switch (evt.type) {
      case 'SweInstanceCompleted': {
        if (evt.run_id !== activeRunId) break;
        setInstances((prev) => [
          ...prev,
          {
            instance_id: evt.instance_id,
            repo: '',
            resolved: evt.resolved,
            duration_ms: evt.duration_ms,
          },
        ]);
        if (evt.resolved) setResolvedCount((c) => c + 1);
        break;
      }
      case 'SweRunCompleted': {
        if (evt.run_id !== activeRunId) break;
        setStatus('completed');
        setResolvedCount(evt.resolved);
        // Refresh runs list
        get<SweRun[]>('/api/bench/swe/runs')
          .then((data) => { if (Array.isArray(data)) setRuns(data); })
          .catch(() => {});
        break;
      }
    }
  }, [lastEvent, status, activeRunId, get]);

  const startRun = useCallback(async (config: SweRunConfig) => {
    try {
      const res = await post<{ run_id: string; total_instances: number }>(
        '/api/bench/swe/run',
        config,
      );
      if (res.run_id) {
        setActiveRunId(res.run_id);
        setInstances([]);
        setResolvedCount(0);
        setTotalInstances(res.total_instances ?? config.batch_size);
        setStatus('running');
      }
    } catch { /* ok */ }
  }, [post]);

  const passRate = totalInstances > 0 ? resolvedCount / totalInstances : 0;

  return {
    datasets,
    runs,
    activeRunId,
    instances,
    status,
    totalInstances,
    resolvedCount,
    passRate,
    startRun,
  };
}
