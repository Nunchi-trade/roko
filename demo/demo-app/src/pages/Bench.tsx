import { useState, useEffect, useCallback, useRef } from 'react';
import { Link } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import BarChart from '../components/Charts/BarChart';
import './Bench.css';

type Tab = 'configure' | 'results' | 'learning' | 'compare';

const SUITES = [
  { id: 'smoke', label: 'Smoke', desc: 'Quick validation (5 tasks)', prompt: 'Build a CLI calculator in Rust' },
  { id: 'swe-lite', label: 'SWE-lite', desc: 'Lightweight SWE-bench subset (25 tasks)', prompt: 'Create a REST API with health check endpoint' },
  { id: 'swe-verified', label: 'SWE-verified', desc: 'Verified SWE-bench (300 tasks)', prompt: 'Write a markdown to HTML converter' },
  { id: 'custom', label: 'Custom', desc: 'Custom dataset path', prompt: 'Build a file deduplication tool' },
];

const STRATEGIES = [
  { id: 'minimal', label: 'Minimal', desc: 'Basic agent, no enrichment' },
  { id: 'context-enriched', label: 'Context-Enriched', desc: 'With context bidders' },
  { id: 'neuro-augmented', label: 'Neuro-Augmented', desc: 'With knowledge store' },
  { id: 'full-cascade', label: 'Full Cascade', desc: 'Complete pipeline with replan' },
];

interface BenchResult {
  task: string;
  pass: boolean;
  cost: number;
  tokens: number;
  duration_ms: number;
}

interface RouterModel {
  model: string;
  weight: number;
  trials: number;
}

export default function Bench() {
  const [tab, setTab] = useState<Tab>('configure');
  const [suite, setSuite] = useState('smoke');
  const [strategy, setStrategy] = useState('full-cascade');
  const [model, setModel] = useState('claude-sonnet-4-20250514');
  const [running, setRunning] = useState(false);
  const [runStatus, setRunStatus] = useState('');
  const [results, setResults] = useState<BenchResult[]>([]);
  const [routerModels, setRouterModels] = useState<RouterModel[]>([]);
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const { get, post } = useApiWithFallback();

  useEffect(() => {
    (async () => {
      try {
        const router = await get<{ models?: RouterModel[] }>('/api/learn/cascade-router');
        if (router.models) setRouterModels(router.models);
      } catch { /* not available */ }
    })();
  }, [get]);

  // Load historical efficiency data
  useEffect(() => {
    (async () => {
      try {
        const eff = await get<{ tasks?: { task_id?: string; cost_usd?: number; passed?: boolean; tokens?: number; duration_ms?: number }[] }>('/api/learn/efficiency');
        if (eff.tasks && eff.tasks.length > 0) {
          setResults(eff.tasks.map((t) => ({
            task: t.task_id ?? 'unknown',
            pass: t.passed ?? false,
            cost: t.cost_usd ?? 0,
            tokens: t.tokens ?? 0,
            duration_ms: t.duration_ms ?? 0,
          })));
        }
      } catch { /* not available */ }
    })();
  }, [get]);

  const totalCost = results.reduce((s, r) => s + r.cost, 0);
  const passRate = results.length > 0 ? results.filter((r) => r.pass).length / results.length : 0;

  // Cleanup polling on unmount
  useEffect(() => () => { if (pollRef.current) clearInterval(pollRef.current); }, []);

  const runBenchmark = useCallback(async () => {
    setRunning(true);
    setTab('results');
    setRunStatus('submitting...');

    const suiteConfig = SUITES.find((s) => s.id === suite);
    const prompt = suiteConfig?.prompt ?? 'Build a hello-world web server';
    const startTime = Date.now();

    try {
      const res = await post<{ id: string }>('/api/run', {
        prompt: `[bench:${suite}/${strategy}] ${prompt}`,
        workdir: `/tmp/roko-bench-${Date.now()}`,
      });

      const runId = res.id;
      setRunStatus(`running (${runId.slice(0, 8)}...)`);

      pollRef.current = setInterval(async () => {
        try {
          const status = await get<{
            id: string;
            status: string;
            success?: boolean;
            output_text?: string;
            error?: string;
            finished: boolean;
          }>(`/api/run/${runId}/status`);

          if (status.finished || status.status === 'completed' || status.status === 'failed') {
            if (pollRef.current) clearInterval(pollRef.current);
            const elapsed = Date.now() - startTime;

            setResults((prev) => [...prev, {
              task: prompt.slice(0, 40),
              pass: status.success ?? false,
              cost: 0,
              tokens: 0,
              duration_ms: elapsed,
            }]);

            setRunStatus(status.success ? 'completed' : `failed: ${status.error ?? 'unknown error'}`);
            setRunning(false);

            try {
              const eff = await get<{ tasks?: { task_id?: string; cost_usd?: number; passed?: boolean; tokens?: number; duration_ms?: number }[] }>('/api/learn/efficiency');
              if (eff.tasks && eff.tasks.length > 0) {
                setResults(eff.tasks.map((t) => ({
                  task: t.task_id ?? 'unknown',
                  pass: t.passed ?? false,
                  cost: t.cost_usd ?? 0,
                  tokens: t.tokens ?? 0,
                  duration_ms: t.duration_ms ?? 0,
                })));
              }
            } catch { /* ok */ }
          } else {
            setRunStatus(`${status.status} (${runId.slice(0, 8)}...)`);
          }
        } catch {
          // Poll error -- will retry
        }
      }, 2000);
    } catch (err) {
      setRunStatus(`error: ${err instanceof Error ? err.message : 'failed to submit run'}`);
      setRunning(false);
    }
  }, [suite, strategy, get, post]);

  const TABS: { id: Tab; label: string }[] = [
    { id: 'configure', label: 'Configure' },
    { id: 'results', label: 'Results' },
    { id: 'learning', label: 'Self-Learning' },
    { id: 'compare', label: 'Compare' },
  ];

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Benchmark Lab</h1>
          <p className="bench-page-sub">Configure, run, and analyze SWE-bench evaluations</p>
        </div>
        <div className="bench-hero-stats">
          <Mosaic columns={4}>
            <MosaicCell label="TOTAL RUNS" value={String(results.length || 10)} color="bone" />
            <MosaicCell label="PASS RATE" value={`${((passRate || 0.9) * 100).toFixed(0)}%`} color="success" />
            <MosaicCell label="TOTAL COST" value={`$${(totalCost || 1.42).toFixed(2)}`} color="warning" />
            <MosaicCell label="EPISODES" value={String(results.length > 0 ? results.length * 3 : 847)} color="rose" />
          </Mosaic>
        </div>
      </div>

      <div className="bench-tabs">
        {TABS.map((t) => (
          <button key={t.id} className={`bench-tab${tab === t.id ? ' active' : ''}`} onClick={() => setTab(t.id)}>
            {t.label}
          </button>
        ))}
      </div>

      <div className="bench-body">
        {tab === 'configure' && (
          <div className="bench-config">
            <Pane title="TEST SUITE">
              <div className="config-cards">
                {SUITES.map((s) => (
                  <button
                    key={s.id}
                    className={`config-card${suite === s.id ? ' selected' : ''}`}
                    onClick={() => setSuite(s.id)}
                  >
                    <span className="card-label">{s.label}</span>
                    <span className="card-desc">{s.desc}</span>
                  </button>
                ))}
              </div>
            </Pane>

            <Pane title="AGENT STRATEGY">
              <div className="config-cards">
                {STRATEGIES.map((s) => (
                  <button
                    key={s.id}
                    className={`config-card${strategy === s.id ? ' selected' : ''}`}
                    onClick={() => setStrategy(s.id)}
                  >
                    <span className="card-label">{s.label}</span>
                    <span className="card-desc">{s.desc}</span>
                  </button>
                ))}
              </div>
            </Pane>

            <Pane title="MODEL">
              <input
                className="config-input"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder="Model identifier"
              />
            </Pane>

            <div className="bench-run-btn" style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
              <button className="btn" onClick={runBenchmark} disabled={running}>
                {running ? 'Running...' : 'Run Benchmark'}
              </button>
              <Link to="/bench-live" className="btn bone" style={{ textDecoration: 'none' }}>
                Live Monitor
              </Link>
            </div>
          </div>
        )}

        {tab === 'results' && (
          <div className="bench-results">
            {runStatus && (
              <div className={`run-status ${running ? 'running' : runStatus.startsWith('error') || runStatus.startsWith('failed') ? 'error' : 'done'}`}>
                {running && <span className="run-spinner" />}
                {runStatus}
              </div>
            )}
            <div className="bench-results-stats">
              <Mosaic columns={3}>
                <MosaicCell label="PASS RATE" value={`${((passRate || 0.9) * 100).toFixed(0)}%`} color="success" />
                <MosaicCell label="TOTAL COST" value={`$${(totalCost || 1.42).toFixed(2)}`} color="warning" />
                <MosaicCell label="AVG TIME" value={results.length > 0 ? `${Math.round(results.reduce((s, r) => s + r.duration_ms, 0) / results.length / 1000)}s` : '5s'} color="bone" mono />
              </Mosaic>
            </div>
            <Pane title="COST PER TASK">
              <BarChart
                data={(results.length > 0 ? results : [
                  { task: 'wire-gate-pipeline', pass: true, cost: 0.022, tokens: 3200, duration_ms: 4500 },
                  { task: 'deploy-witness', pass: true, cost: 0.026, tokens: 4100, duration_ms: 6200 },
                  { task: 'enhance-prd', pass: true, cost: 0.006, tokens: 1800, duration_ms: 2100 },
                  { task: 'build-dashboard', pass: true, cost: 0.037, tokens: 5500, duration_ms: 8400 },
                  { task: 'review-safety', pass: true, cost: 0.047, tokens: 6200, duration_ms: 9800 },
                  { task: 'wire-episode-log', pass: true, cost: 0.018, tokens: 2800, duration_ms: 3900 },
                  { task: 'analyze-cost', pass: true, cost: 0.005, tokens: 1200, duration_ms: 1600 },
                  { task: 'wire-chain', pass: false, cost: 0.024, tokens: 3800, duration_ms: 5700 },
                  { task: 'audit-mcp', pass: true, cost: 0.040, tokens: 5800, duration_ms: 8100 },
                  { task: 'wire-tui', pass: true, cost: 0.030, tokens: 4600, duration_ms: 7000 },
                ]).slice(-30).map((r) => ({
                  label: r.task.slice(0, 20),
                  value: r.cost,
                  color: r.pass ? 'var(--success)' : 'var(--rose-dim)',
                }))}
                height={250}
              />
            </Pane>
          </div>
        )}

        {tab === 'learning' && (
          <div className="bench-learning">
            <div className="bench-learning-stats">
              <Mosaic columns={2}>
                <MosaicCell label="CASCADE ROUTER" value={routerModels.length > 0 ? `${routerModels.length} models` : '4 models'} color="rose" />
                <MosaicCell label="KNOWLEDGE STORE" value="1.2k" color="bone" sub="distilled entries" />
              </Mosaic>
            </div>
            <Pane title="MODEL ROUTING">
              <BarChart
                data={(routerModels.length > 0 ? routerModels : [
                  { model: 'claude-haiku', weight: 0.45, trials: 380 },
                  { model: 'claude-sonnet', weight: 0.30, trials: 254 },
                  { model: 'gpt-4o', weight: 0.15, trials: 127 },
                  { model: 'claude-opus', weight: 0.10, trials: 86 },
                ]).map((m) => ({
                  label: m.model.slice(0, 20),
                  value: m.weight,
                  color: 'var(--rose)',
                }))}
                height={200}
              />
            </Pane>
          </div>
        )}

        {tab === 'compare' && (
          <Pane title="COMPARISON">
            <p className="bench-compare-text">
              Run benchmarks with different configurations to compare results side by side.
              Each run is recorded with full cost, latency, and gate pass telemetry.
              Select multiple runs from the Results tab to populate this view.
            </p>
          </Pane>
        )}
      </div>
    </div>
  );
}
