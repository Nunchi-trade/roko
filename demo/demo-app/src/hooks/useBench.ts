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
} from '../lib/bench-types';
import {
  DEMO_BENCH_SUITES,
  DEMO_BENCH_MODELS,
  DEMO_BENCH_RUNS,
} from '../lib/bench-demo-data';

export interface BenchConfig {
  model: string;
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
  type: 'pass' | 'fail' | 'info' | 'start';
  ts: string;
  cost?: number;
}

const DEFAULT_CONFIG: BenchConfig = {
  model: 'claude-sonnet-4-20250514',
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
  const { get, post } = useApiWithFallback();

  // Config
  const [config, setConfig] = useState<BenchConfig>(DEFAULT_CONFIG);
  const [selectedSuiteId, setSelectedSuiteId] = useState('smoke');

  // Data
  const [suites, setSuites] = useState<BenchSuite[]>([]);
  const [models, setModels] = useState<BenchModel[]>([]);
  const [history, setHistory] = useState<BenchRun[]>([]);

  // Active run
  const [activeRun, setActiveRun] = useState<ActiveRun | null>(null);
  const [feed, setFeed] = useState<FeedItem[]>([]);

  // Comparison
  const [compareIds, setCompareIds] = useState<string[]>([]);

  // SSE
  const { lastEvent, events: _sseEvents, clear: clearSSE } = useBenchSSE({
    benchId: activeRun?.id,
    enabled: activeRun?.status === 'running',
  });

  // Polling ref for active run
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Fetch suites + models + history on mount
  useEffect(() => {
    (async () => {
      try {
        const s = await get<BenchSuite[]>('/api/bench/suites');
        if (Array.isArray(s) && s.length > 0) setSuites(s);
        else setSuites(DEMO_BENCH_SUITES);
      } catch {
        setSuites(DEMO_BENCH_SUITES);
      }
    })();
  }, [get]);

  useEffect(() => {
    (async () => {
      try {
        const m = await get<BenchModel[]>('/api/bench/models');
        if (Array.isArray(m) && m.length > 0) setModels(m);
        else setModels(DEMO_BENCH_MODELS);
      } catch {
        setModels(DEMO_BENCH_MODELS);
      }
    })();
  }, [get]);

  useEffect(() => {
    (async () => {
      try {
        const h = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(h) && h.length > 0) setHistory(h);
        else setHistory(DEMO_BENCH_RUNS);
      } catch {
        setHistory(DEMO_BENCH_RUNS);
      }
    })();
  }, [get]);

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
    }
  }, [lastEvent, get]);

  // Cleanup polling on unmount
  useEffect(() => () => { if (pollRef.current) clearInterval(pollRef.current); }, []);

  const startRun = useCallback(async () => {
    if (pollRef.current) { clearInterval(pollRef.current); pollRef.current = null; }
    clearSSE();
    setFeed([]);

    const suite = suites.find((s) => s.id === selectedSuiteId) ?? suites[0];
    if (!suite) return;

    const ts = new Date().toLocaleTimeString();
    setFeed([{ text: `Starting ${suite.name} with ${config.model}...`, type: 'info' as const, ts }]);

    try {
      const res = await post<{ id: string }>('/api/bench/runs', {
        suite_id: suite.id,
        config: {
          model: config.model,
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

      const runId = res.id ?? `demo-${Date.now()}`;
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

    // Active run
    activeRun,
    activeRunSummary,
    feed,
    startRun,
    cancelRun,
    exportRun,
    importRun,

    // Results shortcut
    lastCompletedRun,

    // Comparison
    compareIds,
    setCompareIds,
  };
}
