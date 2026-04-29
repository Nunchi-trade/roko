import { useState, useEffect, useCallback, useRef } from 'react';
import { useApiWithFallback } from './useApiWithFallback';
import { useBenchSSE } from './useBenchSSE';
import type {
  AgentStrategy,
  BenchSuite,
  BenchModel,
  BenchRun,
  BenchTaskResult,
  BenchRunSummary,
  BenchGateConfig,
  BenchLearningEvent,
  ParetoFrontierResponse,
} from '../lib/bench-types';

export interface BenchConfig {
  strategy: AgentStrategy;
  temperature: number;
  maxTokens: number;
  timeoutSecs: number;
  retries: number;
  gates: BenchGateConfig;
  maxCostUsd: number;
  parallel: boolean;
}

export interface ActiveRun {
  id: string;
  progress: number;
  total: number;
  costSoFar: number;
  results: BenchTaskResult[];
  status: 'running' | 'completed' | 'cancelled';
  startedAt: number;
}

interface FeedItem {
  text: string;
  type: 'pass' | 'fail' | 'info' | 'start' | 'learning';
  ts: string;
  cost?: number;
}

export type ConnectionState = 'connecting' | 'connected' | 'offline';

const DEFAULT_CONFIG: BenchConfig = {
  strategy: 'full_cascade',
  temperature: 0.1,
  maxTokens: 8192,
  timeoutSecs: 120,
  retries: 1,
  gates: { compile: true, test: true, clippy: true, diff: false },
  maxCostUsd: 5.0,
  parallel: false,
};

export function useBench() {
  const { get, post, isLive } = useApiWithFallback();

  // Config
  const [config, setConfig] = useState<BenchConfig>(DEFAULT_CONFIG);
  const [selectedSuiteId, setSelectedSuiteId] = useState('smoke');

  // Data
  const [suites, setSuites] = useState<BenchSuite[]>([]);
  const [models, setModels] = useState<BenchModel[]>([]);
  const [history, setHistory] = useState<BenchRun[]>([]);

  // Loading states
  const [suitesLoading, setSuitesLoading] = useState(true);
  const [modelsLoading, setModelsLoading] = useState(true);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [connectionState, setConnectionState] = useState<ConnectionState>('connecting');

  // Active run
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null);
  const [feed, setFeed] = useState<FeedItem[]>([]);
  const [activeRunLearning, setActiveRunLearning] = useState<BenchLearningEvent[]>([]);

  // Comparison
  const [compareIds, setCompareIds] = useState<string[]>([]);

  // Pareto
  const [pareto, setPareto] = useState<ParetoFrontierResponse | null>(null);

  // SSE
  const { lastEvent, events: _sseEvents, clear: clearSSE } = useBenchSSE({
    benchId: activeRun?.id,
    enabled: activeRun?.status === 'running',
  });

  // Polling ref for active run
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Update connection state from isLive
  useEffect(() => {
    setConnectionState(isLive ? 'connected' : 'offline');
  }, [isLive]);

  // Fetch suites on mount
  useEffect(() => {
    setSuitesLoading(true);
    (async () => {
      try {
        const s = await get<BenchSuite[]>('/api/bench/suites');
        if (Array.isArray(s) && s.length > 0) setSuites(s);
        else setSuites([]);
      } catch {
        setSuites([]);
      } finally {
        setSuitesLoading(false);
      }
    })();
  }, [get]);

  // Fetch models on mount
  useEffect(() => {
    setModelsLoading(true);
    (async () => {
      try {
        const m = await get<BenchModel[]>('/api/bench/models');
        if (Array.isArray(m) && m.length > 0) setModels(m);
        else setModels([]);
      } catch {
        setModels([]);
      } finally {
        setModelsLoading(false);
      }
    })();
  }, [get]);

  // Fetch history on mount
  useEffect(() => {
    setHistoryLoading(true);
    (async () => {
      try {
        const h = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(h) && h.length > 0) setHistory(h);
        else setHistory([]);
      } catch {
        setHistory([]);
      } finally {
        setHistoryLoading(false);
      }
    })();
  }, [get]);

  // ETA computation
  const eta = (() => {
    if (!activeRun || activeRun.status !== 'running' || activeRun.results.length === 0) return null;
    const elapsed = Date.now() - activeRun.startedAt;
    const avgPerTask = elapsed / activeRun.results.length;
    const remaining = activeRun.total - activeRun.results.length;
    return Math.round(avgPerTask * remaining);
  })();

  // Process SSE events
  useEffect(() => {
    if (!lastEvent) return;
    const ts = new Date().toLocaleTimeString();

    switch (lastEvent.type) {
      case 'BenchTaskStarted':
        setFeed((f): FeedItem[] => [
          { text: `Started: ${lastEvent.task_name}`, type: 'start' as const, ts },
          ...f,
        ].slice(0, 100));
        break;

      case 'BenchTaskCompleted':
        setActiveRun((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            results: [...prev.results, lastEvent.result],
          };
        });
        setFeed((f): FeedItem[] => [
          {
            text: `${lastEvent.result.task_name}: ${lastEvent.result.status === 'pass' ? 'PASS' : 'FAIL'}`,
            type: (lastEvent.result.status === 'pass' ? 'pass' : 'fail') as FeedItem['type'],
            ts,
            cost: lastEvent.result.cost_usd,
          },
          ...f,
        ].slice(0, 100));
        break;

      case 'BenchProgress':
        setActiveRun((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            progress: lastEvent.completed,
            total: lastEvent.total,
            costSoFar: lastEvent.cost_so_far,
          };
        });
        break;

      case 'BenchRunCompleted':
        setActiveRun((prev) => {
          if (!prev) return prev;
          return { ...prev, status: 'completed' };
        });
        setFeed((f): FeedItem[] => [
          {
            text: `Run completed: ${lastEvent.summary.passed}/${lastEvent.summary.total_tasks} passed ($${lastEvent.summary.total_cost_usd.toFixed(3)})`,
            type: 'info' as const,
            ts,
          },
          ...f,
        ].slice(0, 100));
        // Refresh history
        get<BenchRun[]>('/api/bench/runs')
          .then((h) => { if (Array.isArray(h) && h.length > 0) setHistory(h); })
          .catch(() => {});
        break;

      case 'BenchLearning':
        setActiveRunLearning((prev) => [...prev, lastEvent as BenchLearningEvent]);
        setFeed((f): FeedItem[] => [
          { text: `Learning: ${(lastEvent as BenchLearningEvent).insight}`, type: 'learning' as const, ts },
          ...f,
        ].slice(0, 100));
        break;
    }
  }, [lastEvent, get]);

  // Cleanup polling on unmount
  useEffect(() => () => { if (pollRef.current) clearInterval(pollRef.current); }, []);

  const fetchPareto = useCallback(async () => {
    try {
      const data = await get<ParetoFrontierResponse>('/api/bench/pareto');
      if (data && Array.isArray(data.points)) setPareto(data);
    } catch {
      setPareto(null);
    }
  }, [get]);

  const startRun = useCallback(async (model: string, provider: string) => {
    if (pollRef.current) { clearInterval(pollRef.current); pollRef.current = null; }
    clearSSE();
    setFeed([]);
    setActiveRunLearning([]);

    const suite = suites.find((s) => s.id === selectedSuiteId) ?? suites[0];
    if (!suite) return;

    const ts = new Date().toLocaleTimeString();
    setFeed([{ text: `Starting ${suite.name} with ${model}...`, type: 'info' as const, ts }]);

    try {
      const res = await post<{ id: string }>('/api/bench/runs', {
        suite_id: suite.id,
        config: {
          model,
          provider,
          temperature: config.temperature,
          max_tokens: config.maxTokens,
          timeout_secs: config.timeoutSecs,
          strategy: config.strategy,
          retries: config.retries,
          gates: config.gates,
          max_cost_usd: config.maxCostUsd,
          parallel: config.parallel,
        },
      });

      const runId = res.id;
      if (!runId) {
        setFeed((f): FeedItem[] => [
          { text: 'Server unavailable — run bench locally with `roko bench`', type: 'fail' as const, ts },
          ...f,
        ]);
        return;
      }

      setActiveRun({
        id: runId,
        progress: 0,
        total: suite.tasks.length,
        costSoFar: 0,
        results: [],
        status: 'running',
        startedAt: Date.now(),
      });

      // Poll for status as fallback to SSE
      pollRef.current = setInterval(async () => {
        try {
          const run = await get<BenchRun>(`/api/bench/runs/${runId}`);
          if (run.status === 'completed' || run.status === 'cancelled') {
            if (pollRef.current) clearInterval(pollRef.current);
            setActiveRun((prev) => {
              if (!prev) return prev;
              return {
                ...prev,
                status: run.status as 'completed' | 'cancelled',
                results: run.results,
                progress: run.results.length,
                total: run.summary?.total_tasks ?? prev.total,
                costSoFar: run.summary?.total_cost_usd ?? prev.costSoFar,
              };
            });
            // Refresh history
            const h = await get<BenchRun[]>('/api/bench/runs');
            if (Array.isArray(h) && h.length > 0) setHistory(h);
          }
        } catch {
          // poll error, will retry
        }
      }, 3000);
    } catch {
      // Server not available — no active run
      setFeed((f): FeedItem[] => [
        { text: 'Server unavailable — run bench locally with `roko bench`', type: 'fail' as const, ts },
        ...f,
      ]);
    }
  }, [config, selectedSuiteId, suites, get, post, clearSSE]);

  const cancelRun = useCallback(async () => {
    if (!activeRun) return;
    try {
      await post(`/api/bench/runs/${activeRun.id}/cancel`, {});
    } catch { /* ok */ }
    if (pollRef.current) clearInterval(pollRef.current);
    setActiveRun((prev) => prev ? { ...prev, status: 'cancelled' } : null);
  }, [activeRun, post]);

  const exportRun = useCallback(async (runId: string) => {
    let data: BenchRun | null = null;
    try {
      data = await get<BenchRun>(`/api/bench/export/${runId}`);
    } catch {
      // Fallback below.
    }

    if (!data?.id) data = history.find((r) => r.id === runId) ?? null;
    if (!data) return;

    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `bench_${runId.slice(0, 8)}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, [get, history]);

  const importRun = useCallback((file: File) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const text = e.target?.result;
        if (typeof text !== 'string') return;

        const data = JSON.parse(text) as Partial<BenchRun>;
        if (
          !data ||
          typeof data !== 'object' ||
          !data.id ||
          !data.suite_id ||
          !Array.isArray(data.results)
        ) {
          console.warn('Invalid bench run file: missing required fields');
          return;
        }

        const imported: BenchRun = { ...(data as BenchRun), kind: 'comparison' };
        setHistory((prev) => prev.some((r) => r.id === imported.id) ? prev : [imported, ...prev]);
      } catch (err) {
        console.warn('Failed to parse bench run file:', err);
      }
    };
    reader.readAsText(file);
  }, []);

  const selectedSuite = suites.find((s) => s.id === selectedSuiteId);

  // Last completed run for results tab
  const lastCompletedRun = activeRun?.status === 'completed'
    ? activeRun
    : history.find((r) => r.status === 'completed');

  // Build summary from active run results
  const activeRunSummary: BenchRunSummary | undefined = activeRun ? (() => {
    const results = activeRun.results;
    if (results.length === 0) return undefined;
    const passed = results.filter((r) => r.status === 'pass').length;
    const failed = results.filter((r) => r.status === 'fail').length;
    const skipped = results.filter((r) => r.status === 'skipped').length;
    const totalCost = results.reduce((s, r) => s + r.cost_usd, 0);
    const totalTokens = results.reduce((s, r) => s + r.tokens_in + r.tokens_out, 0);
    const totalDuration = results.reduce((s, r) => s + r.duration_ms, 0);
    return {
      total_tasks: activeRun.total,
      passed,
      failed,
      skipped,
      total_cost_usd: totalCost,
      total_tokens: totalTokens,
      total_duration_ms: totalDuration,
      pass_rate: passed / results.length,
      cost_per_success_usd: passed > 0 ? totalCost / passed : 0,
      avg_duration_ms: totalDuration / results.length,
    };
  })() : undefined;

  return {
    // Config
    config,
    setConfig,
    selectedSuiteId,
    setSelectedSuiteId,
    selectedSuite,

    // Data
    suites,
    models,
    history,

    // Loading
    suitesLoading,
    modelsLoading,
    historyLoading,
    connectionState,

    // Active run
    activeRun,
    activeRunSummary,
    activeRunLearning,
    feed,
    eta,
    startRun,
    cancelRun,
    exportRun,
    importRun,

    // Results shortcut
    lastCompletedRun,

    // Comparison
    compareIds,
    setCompareIds,

    // Pareto
    pareto,
    fetchPareto,
  };
}
