import { useState, useEffect } from 'react';
import { useParams, Link } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import type { BenchRun } from '../lib/bench-types';
import { DEMO_BENCH_RUNS } from '../lib/bench-demo-data';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import BarChart from '../components/Charts/BarChart';
import TaskTable from '../components/TaskTable';
import './Bench.css';

export default function BenchRunDetail() {
  const { id } = useParams<{ id: string }>();
  const { get } = useApiWithFallback();
  const [run, setRun] = useState<BenchRun | null>(null);

  useEffect(() => {
    if (!id) return;
    (async () => {
      try {
        const data = await get<BenchRun>(`/api/bench/runs/${id}`);
        if (data && data.id) {
          setRun(data);
          return;
        }
      } catch { /* fallback */ }
      // Fallback to demo data
      const demo = DEMO_BENCH_RUNS.find((r) => r.id === id);
      if (demo) setRun(demo);
    })();
  }, [id, get]);

  if (!run) {
    return (
      <div className="bench-page">
        <div className="bench-body">
          <div className="bench-empty">
            <p className="bench-empty-text">Loading run {id}...</p>
          </div>
        </div>
      </div>
    );
  }

  const summary = run.summary;

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <Link to="/bench" className="bench-back">&larr; Back</Link>
            <h1 className="bench-page-title">Run {run.id.slice(0, 8)}</h1>
          </div>
          <p className="bench-page-sub">
            {run.suite_name} &middot; {run.config.model} &middot; {run.config.strategy.replace(/_/g, ' ')}
          </p>
        </div>
        {summary && (
          <div className="bench-hero-stats">
            <Mosaic columns={5}>
              <MosaicCell label="PASS RATE" value={`${(summary.pass_rate * 100).toFixed(0)}%`} color="success" />
              <MosaicCell label="TOTAL COST" value={`$${summary.total_cost_usd.toFixed(3)}`} color="warning" />
              <MosaicCell label="USD/SUCCESS" value={`$${summary.cost_per_success_usd.toFixed(3)}`} color="bone" mono />
              <MosaicCell label="TASKS" value={`${summary.passed}/${summary.total_tasks}`} color="rose" />
              <MosaicCell label="DURATION" value={`${(summary.total_duration_ms / 1000).toFixed(1)}s`} color="dream" mono />
            </Mosaic>
          </div>
        )}
      </div>

      <div className="bench-body">
        <Pane title="TASK RESULTS">
          <TaskTable results={run.results} />
        </Pane>

        <Pane title="COST PER TASK">
          <BarChart
            data={run.results.map((r) => ({
              label: r.task_name.slice(0, 20),
              value: r.cost_usd,
              color: r.status === 'pass' ? 'var(--success)' : 'var(--rose-dim)',
            }))}
            height={250}
          />
        </Pane>

        <Pane title="GATE WATERFALL">
          <div className="gate-waterfall">
            {run.results.map((r) => (
              <div key={r.task_id} className="gate-waterfall-row">
                <span className="gate-waterfall-task">{r.task_name}</span>
                <div className="gate-waterfall-cells">
                  {r.gate_verdicts.map((g) => (
                    <div
                      key={g.gate}
                      className={`gate-waterfall-cell gate-${g.passed ? 'pass' : 'fail'}`}
                      title={`${g.gate}: ${g.passed ? 'passed' : 'failed'}${g.duration_ms ? ` (${g.duration_ms}ms)` : ''}`}
                    >
                      {g.gate[0].toUpperCase()}
                    </div>
                  ))}
                </div>
                <span className={`gate-waterfall-status status-badge status-${r.status}`}>
                  {r.status.toUpperCase()}
                </span>
              </div>
            ))}
          </div>
        </Pane>

        <Pane title="CONFIGURATION">
          <div className="config-detail-grid">
            <div><span className="detail-label">Model:</span> {run.config.model}</div>
            <div><span className="detail-label">Provider:</span> {run.config.provider ?? '-'}</div>
            <div><span className="detail-label">Strategy:</span> {run.config.strategy}</div>
            <div><span className="detail-label">Temperature:</span> {run.config.temperature ?? '-'}</div>
            <div><span className="detail-label">Max Tokens:</span> {run.config.max_tokens ?? '-'}</div>
            <div><span className="detail-label">Timeout:</span> {run.config.timeout_secs}s</div>
            <div><span className="detail-label">Retries:</span> {run.config.retries}</div>
            <div><span className="detail-label">Started:</span> {new Date(run.started_at).toLocaleString()}</div>
            {run.finished_at && (
              <div><span className="detail-label">Finished:</span> {new Date(run.finished_at).toLocaleString()}</div>
            )}
          </div>
        </Pane>
      </div>
    </div>
  );
}
