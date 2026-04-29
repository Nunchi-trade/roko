import { useState, useEffect, useMemo, useCallback, Fragment } from 'react';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import type { BenchRun } from '../lib/bench-types';
import Pane from '../components/Pane';
import ConfigDiff from '../components/ConfigDiff';
import RadarChart from '../components/Charts/RadarChart';
import './Bench.css';

const RUN_COLORS = [
  '#AA7088', '#C8B890', '#8A9C86', '#D8A878', '#9A8AB8', '#6BA0A0',
];

const RADAR_AXES = ['Pass Rate', 'Speed', 'Cost Eff.', 'Token Eff.', 'Gate Pass'];

function formatDuration(ms: number): string {
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatCost(costUsd: number): string {
  return `$${costUsd.toFixed(3)}`;
}

function runLabel(r: BenchRun): string {
  return `${r.id.slice(0, 8)} - ${r.suite_name} (${r.config.model.split('-').slice(0, 2).join('-')})`;
}

/** Read `ids` param from URL. */
function getIdsFromUrl(): string[] {
  const params = new URLSearchParams(window.location.search);
  const raw = params.get('ids');
  if (!raw) return [];
  return raw.split(',').filter(Boolean).slice(0, 6);
}

/** Write `ids` param to URL (replace state). */
function setIdsInUrl(ids: string[]) {
  const params = new URLSearchParams(window.location.search);
  if (ids.length > 0) {
    params.set('ids', ids.join(','));
  } else {
    params.delete('ids');
  }
  const query = params.toString();
  const next = `${window.location.pathname}${query ? `?${query}` : ''}`;
  window.history.replaceState(null, '', next);
}

export default function BenchCompare() {
  const { get } = useApiWithFallback();
  const [allRuns, setAllRuns] = useState<BenchRun[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>(getIdsFromUrl);
  const [fullRuns, setFullRuns] = useState<BenchRun[]>([]);

  // Fetch all runs on mount
  useEffect(() => {
    (async () => {
      try {
        const data = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(data) && data.length > 0) {
          setAllRuns(data);
          return;
        }
      } catch { /* no fallback */ }
      setAllRuns([]);
    })();
  }, [get]);

  // Auto-select first two runs if nothing from URL
  useEffect(() => {
    if (selectedIds.length > 0 || allRuns.length < 2) return;
    setSelectedIds([allRuns[0].id, allRuns[1].id]);
  }, [allRuns]); // eslint-disable-line react-hooks/exhaustive-deps

  // Sync selectedIds -> URL
  useEffect(() => {
    setIdsInUrl(selectedIds);
  }, [selectedIds]);

  // Fetch full run data when selection changes
  useEffect(() => {
    if (selectedIds.length < 2) {
      setFullRuns([]);
      return;
    }

    let cancelled = false;
    setFullRuns([]);

    (async () => {
      try {
        const data = await get<{ runs: BenchRun[] }>(
          `/api/bench/runs/compare?ids=${selectedIds.join(',')}`
        );
        if (!cancelled && data?.runs?.length >= 2) {
          // Preserve selection order
          const ordered = selectedIds
            .map((id) => data.runs.find((r) => r.id === id))
            .filter((r): r is BenchRun => !!r);
          setFullRuns(ordered);
          return;
        }
      } catch { /* fallback */ }

      if (cancelled) return;
      const found = selectedIds
        .map((id) => allRuns.find((r) => r.id === id))
        .filter((r): r is BenchRun => !!r);
      setFullRuns(found);
    })();

    return () => { cancelled = true; };
  }, [selectedIds, get, allRuns]);

  // ── Selection actions ──

  const addRun = useCallback((id: string) => {
    if (!id || selectedIds.includes(id)) return;
    setSelectedIds((prev) => [...prev, id].slice(0, 6));
  }, [selectedIds]);

  const removeRun = useCallback((id: string) => {
    setSelectedIds((prev) => prev.filter((x) => x !== id));
  }, []);

  const quickSelectLast2 = useCallback(() => {
    if (allRuns.length >= 2) setSelectedIds([allRuns[0].id, allRuns[1].id]);
  }, [allRuns]);

  const quickSelectSameSuite = useCallback(() => {
    if (selectedIds.length === 0 || allRuns.length < 2) return;
    const first = allRuns.find((r) => r.id === selectedIds[0]);
    if (!first) return;
    const same = allRuns.filter((r) => r.suite_id === first.suite_id).slice(0, 6);
    if (same.length >= 2) setSelectedIds(same.map((r) => r.id));
  }, [allRuns, selectedIds]);

  const quickSelectSameModel = useCallback(() => {
    if (selectedIds.length === 0 || allRuns.length < 2) return;
    const first = allRuns.find((r) => r.id === selectedIds[0]);
    if (!first) return;
    const same = allRuns.filter((r) => r.config.model === first.config.model).slice(0, 6);
    if (same.length >= 2) setSelectedIds(same.map((r) => r.id));
  }, [allRuns, selectedIds]);

  // ── Radar datasets ──

  const radarDatasets = useMemo(() => {
    return fullRuns.map((run, i) => {
      const s = run.summary;
      if (!s) return null;
      const gatePass = run.results.reduce((acc, r) => {
        const total = r.gate_verdicts.length;
        const passed = r.gate_verdicts.filter((g) => g.passed).length;
        return total > 0 ? acc + passed / total : acc;
      }, 0) / (run.results.length || 1);
      return {
        label: run.config.model.split('-').slice(0, 2).join('-'),
        values: [
          s.pass_rate,
          1 - Math.min(s.avg_duration_ms / 60000, 1),
          1 - Math.min(s.total_cost_usd / 1, 1),
          Math.min(s.passed / Math.max(s.total_tokens / 1000, 0.001), 1),
          gatePass,
        ],
        color: RUN_COLORS[i % RUN_COLORS.length],
      };
    }).filter((d): d is NonNullable<typeof d> => d !== null);
  }, [fullRuns]);

  // ── Summary metrics mosaic ──

  const summaryMetrics = useMemo(() => {
    if (fullRuns.length < 2) return null;
    const totalTasks = fullRuns.reduce((s, r) => s + r.results.length, 0);
    const totalPassed = fullRuns.reduce(
      (s, r) => s + r.results.filter((t) => t.status === 'pass').length, 0
    );
    const totalCost = fullRuns.reduce(
      (s, r) => s + r.results.reduce((a, t) => a + t.cost_usd, 0), 0
    );
    const avgDuration = fullRuns.reduce(
      (s, r) => s + (r.summary?.avg_duration_ms ?? 0), 0
    ) / fullRuns.length;
    const bestPassRate = Math.max(...fullRuns.map((r) => r.summary?.pass_rate ?? 0));
    const cheapestRun = fullRuns.reduce((best, r) =>
      (r.summary?.total_cost_usd ?? Infinity) < (best.summary?.total_cost_usd ?? Infinity) ? r : best
    );
    return { totalTasks, totalPassed, totalCost, avgDuration, bestPassRate, cheapestRun };
  }, [fullRuns]);

  // ── Task-by-task N-way matrix ──

  const taskMatrix = useMemo(() => {
    if (fullRuns.length < 2) return [];
    const taskIds = new Set<string>();
    for (const run of fullRuns) {
      for (const r of run.results) taskIds.add(r.task_id);
    }
    return Array.from(taskIds).map((taskId) => {
      const results = fullRuns.map((run) =>
        run.results.find((r) => r.task_id === taskId) ?? null
      );
      const name = results.find((r) => r !== null)?.task_name ?? taskId;

      // Find cheapest passing run for this task (winner = green)
      let bestIdx = -1;
      let bestCost = Infinity;
      results.forEach((r, i) => {
        if (r && r.status === 'pass' && r.cost_usd < bestCost) {
          bestCost = r.cost_usd;
          bestIdx = i;
        }
      });

      return { taskId, name, results, bestIdx };
    });
  }, [fullRuns]);

  const availableToAdd = allRuns.filter((r) => !selectedIds.includes(r.id));

  return (
    <div className="bench-page">
      <div className="bench-hero">
        <div className="bench-hero-header">
          <h1 className="bench-page-title">Compare Runs</h1>
          <p className="bench-page-sub">
            Multi-run comparison of benchmark configurations and results (2-6 runs)
          </p>
        </div>
      </div>

      <div className="bench-body">
        {/* ── Multi-select chip input ── */}
        <Pane title="SELECT RUNS">
          <div className="compare-chips-wrap">
            <div className="compare-chips">
              {selectedIds.map((id, i) => {
                const run = allRuns.find((r) => r.id === id);
                return (
                  <span
                    key={id}
                    className="compare-chip"
                    style={{ borderColor: RUN_COLORS[i % RUN_COLORS.length] }}
                  >
                    <span
                      className="compare-chip-dot"
                      style={{ background: RUN_COLORS[i % RUN_COLORS.length] }}
                    />
                    <span className="compare-chip-label">
                      {run ? runLabel(run) : id.slice(0, 8)}
                    </span>
                    <button
                      className="compare-chip-x"
                      onClick={() => removeRun(id)}
                      title="Remove"
                    >
                      x
                    </button>
                  </span>
                );
              })}
              {selectedIds.length < 6 && availableToAdd.length > 0 && (
                <select
                  className="config-input compare-add-select"
                  value=""
                  onChange={(e) => addRun(e.target.value)}
                >
                  <option value="">+ Add run...</option>
                  {availableToAdd.map((r) => (
                    <option key={r.id} value={r.id}>
                      {runLabel(r)}
                    </option>
                  ))}
                </select>
              )}
            </div>
            <div className="compare-quick-btns">
              <button className="btn btn-sm" onClick={quickSelectLast2}>Last 2</button>
              <button className="btn btn-sm" onClick={quickSelectSameSuite}>Same Suite</button>
              <button className="btn btn-sm" onClick={quickSelectSameModel}>Same Model</button>
            </div>
          </div>
        </Pane>

        {fullRuns.length >= 2 ? (
          <>
            {/* ── Config Diff (new N-way component) ── */}
            <Pane title="CONFIG COMPARISON">
              <ConfigDiff runs={fullRuns} />
            </Pane>

            {/* ── Summary Metrics Mosaic ── */}
            {summaryMetrics && (
              <Pane title="AGGREGATE SUMMARY">
                <div className="compare-mosaic">
                  <div className="mosaic-tile">
                    <span className="detail-label">Runs Compared</span>
                    <span className="mosaic-value">{fullRuns.length}</span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Total Tasks</span>
                    <span className="mosaic-value">{summaryMetrics.totalTasks}</span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Total Passed</span>
                    <span className="mosaic-value gate-ok">{summaryMetrics.totalPassed}</span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Total Cost</span>
                    <span className="mosaic-value">{formatCost(summaryMetrics.totalCost)}</span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Best Pass Rate</span>
                    <span className="mosaic-value gate-ok">
                      {(summaryMetrics.bestPassRate * 100).toFixed(1)}%
                    </span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Avg Duration</span>
                    <span className="mosaic-value">
                      {formatDuration(summaryMetrics.avgDuration)}
                    </span>
                  </div>
                  <div className="mosaic-tile">
                    <span className="detail-label">Cheapest Run</span>
                    <span className="mosaic-value">
                      {summaryMetrics.cheapestRun.id.slice(0, 8)}
                    </span>
                  </div>
                </div>
              </Pane>
            )}

            {/* ── Radar Chart Overlay ── */}
            {radarDatasets.length >= 2 && (
              <Pane title="RADAR OVERLAY">
                <RadarChart axes={RADAR_AXES} datasets={radarDatasets} height={360} />
              </Pane>
            )}

            {/* ── Task-by-Task N-way Matrix ── */}
            <Pane title="TASK-BY-TASK MATRIX">
              <div className="task-table-wrap">
                <table className="task-table">
                  <thead>
                    <tr>
                      <th rowSpan={2} style={{ cursor: 'default' }}>Task</th>
                      {fullRuns.map((run, i) => (
                        <th
                          key={run.id}
                          colSpan={2}
                          style={{
                            cursor: 'default',
                            borderBottom: `2px solid ${RUN_COLORS[i % RUN_COLORS.length]}`,
                          }}
                        >
                          Run {run.id.slice(0, 8)}
                        </th>
                      ))}
                    </tr>
                    <tr>
                      {fullRuns.map((run) => (
                        <Fragment key={run.id}>
                          <th style={{ cursor: 'default' }}>Status</th>
                          <th style={{ cursor: 'default' }}>Cost</th>
                        </Fragment>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {taskMatrix.map((row) => (
                      <tr key={row.taskId}>
                        <td className="task-name" title={row.taskId}>{row.name}</td>
                        {row.results.map((r, i) => (
                          <Fragment key={fullRuns[i].id}>
                            <td>
                              {r ? (
                                <span className={`status-badge status-${r.status}`}>
                                  {r.status.toUpperCase()}
                                </span>
                              ) : (
                                <span className="mono">-</span>
                              )}
                            </td>
                            <td
                              className={`mono${row.bestIdx === i ? ' gate-ok' : ''}`}
                              style={row.bestIdx === i ? { fontWeight: 700 } : undefined}
                            >
                              {r ? formatCost(r.cost_usd) : '-'}
                            </td>
                          </Fragment>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                  <tfoot>
                    <tr>
                      <td className="detail-label">Totals</td>
                      {fullRuns.map((run) => {
                        const passed = run.results.filter((r) => r.status === 'pass').length;
                        const total = run.results.length;
                        const cost = run.results.reduce((s, r) => s + r.cost_usd, 0);
                        return (
                          <Fragment key={run.id}>
                            <td className="mono" style={{ fontWeight: 700 }}>
                              {passed}/{total} ({total > 0 ? ((passed / total) * 100).toFixed(1) : 0}%)
                            </td>
                            <td className="mono" style={{ fontWeight: 700 }}>
                              {formatCost(cost)}
                            </td>
                          </Fragment>
                        );
                      })}
                    </tr>
                  </tfoot>
                </table>
              </div>
            </Pane>
          </>
        ) : (
          <div className="bench-empty">
            <p className="bench-empty-text">
              {selectedIds.length < 2
                ? 'Select at least two runs above to compare.'
                : 'Loading comparison data...'}
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
