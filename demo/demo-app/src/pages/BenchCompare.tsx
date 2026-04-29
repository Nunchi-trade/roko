import { useState, useEffect, useMemo } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import type { BenchRun, BenchTaskResult } from '../lib/bench-types';
import Pane from '../components/Pane';
import ConfigDiff from '../components/ConfigDiff';
import './Bench.css';

type ComparisonRow = {
  taskId: string;
  name: string;
  a?: BenchTaskResult;
  b?: BenchTaskResult;
  aFaster: boolean;
  bFaster: boolean;
  aCheaper: boolean;
  bCheaper: boolean;
};

type RunSummary = {
  passed: number;
  totalTasks: number;
  totalDurationMs: number;
  totalCostUsd: number;
  passRate: number;
};

function formatDuration(ms: number): string {
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatCost(costUsd: number): string {
  return `$${costUsd.toFixed(3)}`;
}

export default function BenchCompare() {
  const { get } = useApiWithFallback();
  const [runs, setRuns] = useState<BenchRun[]>([]);
  const [selectedA, setSelectedA] = useState<string>('');
  const [selectedB, setSelectedB] = useState<string>('');
  const [fullRunA, setFullRunA] = useState<BenchRun | null>(null);
  const [fullRunB, setFullRunB] = useState<BenchRun | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const data = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(data) && data.length > 0) {
          setRuns(data);
          return;
        }
      } catch { /* no fallback */ }
      setRuns([]);
    })();
  }, [get]);

  // Auto-select first two runs
  useEffect(() => {
    if (runs.length >= 2 && !selectedA && !selectedB) {
      setSelectedA(runs[0].id);
      setSelectedB(runs[1].id);
    } else if (runs.length === 1 && !selectedA) {
      setSelectedA(runs[0].id);
    }
  }, [runs, selectedA, selectedB]);

  useEffect(() => {
    if (!selectedA || !selectedB) {
      setFullRunA(null);
      setFullRunB(null);
      return;
    }

    let cancelled = false;
    setFullRunA(null);
    setFullRunB(null);

    (async () => {
      try {
        const data = await get<{ runs: BenchRun[] }>(
          `/api/bench/runs/compare?ids=${selectedA},${selectedB}`
        );
        if (!cancelled && data?.runs?.length >= 2) {
          setFullRunA(data.runs.find((r) => r.id === selectedA) ?? data.runs[0] ?? null);
          setFullRunB(data.runs.find((r) => r.id === selectedB) ?? data.runs[1] ?? null);
          return;
        }
      } catch { /* fallback */ }

      if (cancelled) return;
      setFullRunA(runs.find((r) => r.id === selectedA) ?? null);
      setFullRunB(runs.find((r) => r.id === selectedB) ?? null);
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedA, selectedB, get, runs]);

  const runA = runs.find((r) => r.id === selectedA);
  const runB = runs.find((r) => r.id === selectedB);
  const comparisonRunA = fullRunA ?? runA ?? null;
  const comparisonRunB = fullRunB ?? runB ?? null;

  const taskMatrix = useMemo<ComparisonRow[]>(() => {
    if (!fullRunA || !fullRunB) return [];
    const taskIds = new Set<string>();
    for (const r of fullRunA.results) taskIds.add(r.task_id);
    for (const r of fullRunB.results) taskIds.add(r.task_id);
    return Array.from(taskIds).map((taskId) => {
      const a = fullRunA.results.find((r) => r.task_id === taskId);
      const b = fullRunB.results.find((r) => r.task_id === taskId);
      const name = a?.task_name ?? b?.task_name ?? taskId;
      return {
        taskId,
        name,
        a,
        b,
        aFaster: !!(a && b && a.duration_ms < b.duration_ms),
        bFaster: !!(a && b && b.duration_ms < a.duration_ms),
        aCheaper: !!(a && b && a.cost_usd < b.cost_usd),
        bCheaper: !!(a && b && b.cost_usd < a.cost_usd),
      };
    });
  }, [fullRunA, fullRunB]);

  const comparisonSummary = useMemo<{
    a: RunSummary;
    b: RunSummary;
  } | null>(() => {
    if (!fullRunA || !fullRunB) return null;

    const summarize = (run: BenchRun): RunSummary => {
      const passed = run.results.filter((r) => r.status === 'pass').length;
      const totalTasks = run.results.length;
      const totalDurationMs = run.results.reduce((sum, r) => sum + r.duration_ms, 0);
      const totalCostUsd = run.results.reduce((sum, r) => sum + r.cost_usd, 0);
      return {
        passed,
        totalTasks,
        totalDurationMs,
        totalCostUsd,
        passRate: totalTasks > 0 ? passed / totalTasks : 0,
      };
    };

    return {
      a: summarize(fullRunA),
      b: summarize(fullRunB),
    };
  }, [fullRunA, fullRunB]);

  const betterPassRate = comparisonSummary
    ? comparisonSummary.a.passRate > comparisonSummary.b.passRate
      ? 'a'
      : comparisonSummary.b.passRate > comparisonSummary.a.passRate
        ? 'b'
        : null
    : null;

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Compare Runs</h1>
          <p className="bench-page-sub">Side-by-side comparison of benchmark configurations and results</p>
        </div>
      </div>

      <div className="bench-body">
        <Pane title="SELECT RUNS">
          <div className="compare-selectors">
            <div className="compare-select">
              <label className="param-label">Run A</label>
              <select
                className="config-input"
                value={selectedA}
                onChange={(e) => setSelectedA(e.target.value)}
              >
                <option value="">Select run...</option>
                {runs.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.id.slice(0, 8)} - {r.suite_name} ({r.config.model.split('-').slice(0, 2).join('-')})
                  </option>
                ))}
              </select>
            </div>
            <div className="compare-select">
              <label className="param-label">Run B</label>
              <select
                className="config-input"
                value={selectedB}
                onChange={(e) => setSelectedB(e.target.value)}
              >
                <option value="">Select run...</option>
                {runs.map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.id.slice(0, 8)} - {r.suite_name} ({r.config.model.split('-').slice(0, 2).join('-')})
                  </option>
                ))}
              </select>
            </div>
          </div>
        </Pane>

        {runA && runB ? (
          <>
            <Pane title="COMPARISON">
              <ConfigDiff runs={[comparisonRunA ?? runA, comparisonRunB ?? runB]} />
            </Pane>

            <Pane title="TASK-BY-TASK COMPARISON">
              {comparisonSummary ? (
                <div className="task-table-wrap">
                  <table className="task-table">
                    <thead>
                      <tr>
                        <th rowSpan={2} style={{ cursor: 'default' }}>
                          Task
                        </th>
                        <th colSpan={3} style={{ cursor: 'default' }}>
                          Run A
                        </th>
                        <th colSpan={3} style={{ cursor: 'default' }}>
                          Run B
                        </th>
                      </tr>
                      <tr>
                        <th style={{ cursor: 'default' }}>Result</th>
                        <th style={{ cursor: 'default' }}>Duration</th>
                        <th style={{ cursor: 'default' }}>Cost</th>
                        <th style={{ cursor: 'default' }}>Result</th>
                        <th style={{ cursor: 'default' }}>Duration</th>
                        <th style={{ cursor: 'default' }}>Cost</th>
                      </tr>
                    </thead>
                    <tbody>
                      {taskMatrix.map((row) => (
                        <tr key={row.taskId}>
                          <td className="task-name" title={row.taskId}>
                            {row.name}
                          </td>
                          <td>
                            {row.a ? (
                              <span className={`status-badge status-${row.a.status}`}>
                                {row.a.status.toUpperCase()}
                              </span>
                            ) : (
                              <span className="mono">-</span>
                            )}
                          </td>
                          <td
                            className={row.aFaster ? 'mono gate-ok' : 'mono'}
                            style={row.aFaster ? { fontWeight: 700 } : undefined}
                          >
                            {row.a ? formatDuration(row.a.duration_ms) : '-'}
                          </td>
                          <td
                            className={row.aCheaper ? 'mono gate-ok' : 'mono'}
                            style={row.aCheaper ? { fontWeight: 700 } : undefined}
                          >
                            {row.a ? formatCost(row.a.cost_usd) : '-'}
                          </td>
                          <td>
                            {row.b ? (
                              <span className={`status-badge status-${row.b.status}`}>
                                {row.b.status.toUpperCase()}
                              </span>
                            ) : (
                              <span className="mono">-</span>
                            )}
                          </td>
                          <td
                            className={row.bFaster ? 'mono gate-ok' : 'mono'}
                            style={row.bFaster ? { fontWeight: 700 } : undefined}
                          >
                            {row.b ? formatDuration(row.b.duration_ms) : '-'}
                          </td>
                          <td
                            className={row.bCheaper ? 'mono gate-ok' : 'mono'}
                            style={row.bCheaper ? { fontWeight: 700 } : undefined}
                          >
                            {row.b ? formatCost(row.b.cost_usd) : '-'}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                    <tfoot>
                      <tr>
                        <td className="detail-label">Totals</td>
                        <td
                          className={betterPassRate === 'a' ? 'mono gate-ok' : 'mono'}
                          style={{ fontWeight: 700 }}
                        >
                          {comparisonSummary.a.passed} passed ({(comparisonSummary.a.passRate * 100).toFixed(1)}%)
                        </td>
                        <td className="mono" style={{ fontWeight: 700 }}>
                          {formatDuration(comparisonSummary.a.totalDurationMs)}
                        </td>
                        <td className="mono" style={{ fontWeight: 700 }}>
                          {formatCost(comparisonSummary.a.totalCostUsd)}
                        </td>
                        <td
                          className={betterPassRate === 'b' ? 'mono gate-ok' : 'mono'}
                          style={{ fontWeight: 700 }}
                        >
                          {comparisonSummary.b.passed} passed ({(comparisonSummary.b.passRate * 100).toFixed(1)}%)
                        </td>
                        <td className="mono" style={{ fontWeight: 700 }}>
                          {formatDuration(comparisonSummary.b.totalDurationMs)}
                        </td>
                        <td className="mono" style={{ fontWeight: 700 }}>
                          {formatCost(comparisonSummary.b.totalCostUsd)}
                        </td>
                      </tr>
                    </tfoot>
                  </table>
                </div>
              ) : (
                <div className="bench-empty">
                  <p className="bench-empty-text">Loading task comparison data...</p>
                </div>
              )}
            </Pane>
          </>
        ) : (
          <div className="bench-empty">
            <p className="bench-empty-text">
              Select two runs above to compare their configurations and results.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
