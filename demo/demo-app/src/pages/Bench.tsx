import { useState, useEffect, useCallback, useRef } from 'react';
import { useApi } from '../hooks/useApi';
import StatCard from '../components/StatCard';
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
  const { get, post } = useApi();

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
      // Submit run via POST /api/run
      const res = await post<{ id: string }>('/api/run', {
        prompt: `[bench:${suite}/${strategy}] ${prompt}`,
        workdir: `/tmp/roko-bench-${Date.now()}`,
      });

      const runId = res.id;
      setRunStatus(`running (${runId.slice(0, 8)}...)`);

      // Poll for completion
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
              cost: 0, // Real cost comes from efficiency data
              tokens: 0,
              duration_ms: elapsed,
            }]);

            setRunStatus(status.success ? 'completed' : `failed: ${status.error ?? 'unknown error'}`);
            setRunning(false);

            // Refresh efficiency data for real cost/token info
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
          // Poll error — will retry
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
          <StatCard label="Total Runs" value={results.length} color="bone" />
          <StatCard label="Pass Rate" value={`${(passRate * 100).toFixed(0)}%`} color="sage" />
          <StatCard label="Total Cost" value={`$${totalCost.toFixed(2)}`} color="warn" />
          <StatCard label="Episodes" value="—" color="rose" />
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
            <div className="config-section">
              <h3>Test Suite</h3>
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
            </div>
            <div className="config-section">
              <h3>Agent Strategy</h3>
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
            </div>
            <div className="config-section">
              <h3>Model</h3>
              <input
                className="config-input"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder="Model identifier"
              />
            </div>
            <div className="config-actions">
              <button className="btn-primary btn-lg" onClick={runBenchmark} disabled={running}>
                {running ? 'Running...' : 'Run Benchmark'}
              </button>
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
            <div className="results-stats">
              <StatCard label="Pass Rate" value={`${(passRate * 100).toFixed(0)}%`} color="sage" />
              <StatCard label="Total Cost" value={`$${totalCost.toFixed(2)}`} color="warn" />
              <StatCard label="Avg Time" value={results.length > 0 ? `${Math.round(results.reduce((s, r) => s + r.duration_ms, 0) / results.length / 1000)}s` : '—'} color="bone" />
            </div>
            {results.length > 0 ? (
              <BarChart
                title="Cost per Task"
                data={results.slice(-30).map((r) => ({
                  label: r.task.slice(0, 20),
                  value: r.cost,
                  color: r.pass ? '#70887A' : '#C36E55',
                }))}
                height={250}
              />
            ) : !running && (
              <div className="bench-empty">No results yet — run a benchmark from the Configure tab.</div>
            )}
          </div>
        )}

        {tab === 'learning' && (
          <div className="bench-learning">
            <div className="learning-grid">
              <StatCard label="Cascade Router" value={routerModels.length > 0 ? `${routerModels.length} models` : '—'} color="rose" />
              <StatCard label="Knowledge Store" value="—" color="bone" sub="query /api/neuro/stats" />
            </div>
            {routerModels.length > 0 && (
              <BarChart
                title="Model Routing Weights"
                data={routerModels.map((m) => ({
                  label: m.model.slice(0, 20),
                  value: m.weight,
                  color: '#AA7088',
                }))}
                height={200}
              />
            )}
          </div>
        )}

        {tab === 'compare' && (
          <div className="bench-compare">
            <div className="compare-placeholder">
              <p>Run benchmarks with different configurations to compare results here.</p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
