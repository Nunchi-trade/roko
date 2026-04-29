import { useState, useEffect, useMemo, useCallback, useRef, Fragment, type CSSProperties } from 'react';
import { useLiveApi } from '../hooks/useLiveApi';
import type { BenchRun } from '../lib/bench-types';
import { handleRowKeyDown } from '../lib/a11y';
import Pane from '../components/Pane';
import ConfigDiff from '../components/ConfigDiff';
import RadarChart from '../components/Charts/RadarChart';
import { ComponentErrorBoundary } from '../components/design';
import './Bench.css';

const RUN_COLORS = [
  'var(--rose)',          // --rose
  'var(--bone)',          // --bone
  'var(--success)',       // --success
  'var(--warning)',       // --warning
  'var(--dream-bright)',  // --dream-bright
  'var(--dream)',         // --dream (teal-ish)
];

const RADAR_AXES = ['Pass Rate', 'Speed', 'Cost Eff.', 'Token Eff.', 'Gate Pass'];

/* ── Animation helpers ── */

/** Hook that counts from 0 to `target` over `duration` ms with easeOutExpo. */
function useCountUp(target: number, duration = 800, active = true): number {
  const [value, setValue] = useState(0);
  const rafRef = useRef(0);

  useEffect(() => {
    if (!active || target === 0) { setValue(target); return; }
    const start = performance.now();
    const animate = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      // easeOutExpo
      const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
      setValue(target * eased);
      if (t < 1) rafRef.current = requestAnimationFrame(animate);
    };
    rafRef.current = requestAnimationFrame(animate);
    return () => { if (rafRef.current) cancelAnimationFrame(rafRef.current); };
  }, [target, duration, active]);

  return value;
}

/** Stagger-in wrapper: each child fades + slides in with delay. */
function StaggerSection({
  children,
  index,
  direction = 'up',
  className,
  style,
}: {
  children: React.ReactNode;
  index: number;
  direction?: 'up' | 'left' | 'right';
  className?: string;
  style?: CSSProperties;
}) {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const timer = setTimeout(() => setVisible(true), 80 * index);
    return () => clearTimeout(timer);
  }, [index]);

  const transform = !visible
    ? direction === 'left' ? 'translateX(-24px)' : direction === 'right' ? 'translateX(24px)' : 'translateY(16px)'
    : 'translate(0)';

  return (
    <div
      className={className}
      style={{
        opacity: visible ? 1 : 0,
        transform,
        transition: `opacity 500ms cubic-bezier(0.22,1,0.36,1), transform 500ms cubic-bezier(0.22,1,0.36,1)`,
        ...style,
      }}
    >
      {children}
    </div>
  );
}

/** Animated metric card with slide direction. */
function MetricCard({
  label,
  value,
  suffix,
  isWinner,
  slideFrom,
  index,
  className,
}: {
  label: string;
  value: number;
  suffix?: string;
  isWinner?: boolean;
  slideFrom: 'left' | 'right';
  index: number;
  className?: string;
}) {
  const displayed = useCountUp(value, 700, true);
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setVisible(true), 60 * index);
    return () => clearTimeout(t);
  }, [index]);

  const formatted = suffix === '%'
    ? `${displayed.toFixed(1)}%`
    : suffix === '$'
      ? `$${displayed.toFixed(3)}`
      : suffix === 's'
        ? `${displayed.toFixed(1)}s`
        : Math.round(displayed).toString();

  return (
    <div
      className={`mosaic-tile ${className ?? ''}`}
      style={{
        opacity: visible ? 1 : 0,
        transform: visible ? 'translateX(0)' : slideFrom === 'left' ? 'translateX(-20px)' : 'translateX(20px)',
        transition: 'opacity 400ms cubic-bezier(0.22,1,0.36,1), transform 400ms cubic-bezier(0.22,1,0.36,1)',
        position: 'relative',
      }}
    >
      <span className="detail-label">{label}</span>
      <span
        className={`mosaic-value ${isWinner ? 'gate-ok' : ''}`}
        style={isWinner ? {
          textShadow: '0 0 12px var(--status-success)',
          animation: 'winnerPulse 2s ease-in-out 1',
        } : undefined}
      >
        {formatted}
      </span>
      {isWinner && <span className="compare-winner-badge">BEST</span>}
    </div>
  );
}

/** Animated horizontal bar for per-run metric comparison. */
function CompareBar({
  value,
  maxValue,
  color,
  label,
  index,
}: {
  value: number;
  maxValue: number;
  color: string;
  label: string;
  index: number;
}) {
  const [grown, setGrown] = useState(false);
  useEffect(() => {
    const t = setTimeout(() => setGrown(true), 100 + index * 60);
    return () => clearTimeout(t);
  }, [index]);

  const pct = maxValue > 0 ? (value / maxValue) * 100 : 0;

  return (
    <div className="compare-bar-row">
      <span className="compare-bar-label">{label}</span>
      <div className="compare-bar-track">
        <div
          className="compare-bar-fill"
          style={{
            width: grown ? `${pct}%` : '0%',
            background: color,
            transition: 'width 600ms cubic-bezier(0.22,1,0.36,1)',
            transitionDelay: `${index * 60}ms`,
          }}
        />
      </div>
      <span className="compare-bar-value mono">{(value * 100).toFixed(1)}%</span>
    </div>
  );
}

/* ── View type for toggle crossfade ── */
type CompareView = 'overview' | 'matrix';

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

/** Determine the "winner" run index (highest pass rate, then lowest cost). */
function findWinnerIdx(runs: BenchRun[]): number {
  if (runs.length < 2) return -1;
  let bestIdx = 0;
  let bestRate = runs[0].summary?.pass_rate ?? 0;
  let bestCost = runs[0].summary?.total_cost_usd ?? Infinity;
  for (let i = 1; i < runs.length; i++) {
    const rate = runs[i].summary?.pass_rate ?? 0;
    const cost = runs[i].summary?.total_cost_usd ?? Infinity;
    if (rate > bestRate || (rate === bestRate && cost < bestCost)) {
      bestIdx = i;
      bestRate = rate;
      bestCost = cost;
    }
  }
  return bestIdx;
}

export default function BenchCompare() {
  const { get } = useLiveApi();
  const [allRuns, setAllRuns] = useState<BenchRun[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>(getIdsFromUrl);
  const [fullRuns, setFullRuns] = useState<BenchRun[]>([]);
  const didAutoSelectRef = useRef(selectedIds.length > 0);
  const [view, setView] = useState<CompareView>('overview');
  const [hoveredCol, setHoveredCol] = useState<number>(-1);
  const [hoveredRow, setHoveredRow] = useState<string>('');

  // Fetch all runs on mount
  useEffect(() => {
    (async () => {
      try {
        const data = await get<BenchRun[]>('/api/bench/runs');
        if (Array.isArray(data) && data.length > 0) {
          setAllRuns(data);
          return;
        }
      } catch { /* show empty state */ }
      setAllRuns([]);
    })();
  }, [get]);

  // Auto-select first two runs if nothing from URL
  useEffect(() => {
    if (didAutoSelectRef.current || selectedIds.length > 0 || allRuns.length < 2) return;
    didAutoSelectRef.current = true;
    setSelectedIds([allRuns[0].id, allRuns[1].id]);
  }, [allRuns, selectedIds.length]);

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
      } catch { /* use already-loaded live run summaries */ }

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
    setSelectedIds((prev) => {
      if (!id || prev.includes(id)) return prev;
      return [...prev, id].slice(0, 6);
    });
  }, []);

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

  // ── Winner index ──
  const winnerIdx = useMemo(() => findWinnerIdx(fullRuns), [fullRuns]);

  // ── Summary metrics ──

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

  // ── Per-run pass rate bars ──

  const passRateBars = useMemo(() => {
    return fullRuns.map((run, i) => ({
      label: run.config.model.split('-').slice(0, 2).join('-'),
      value: run.summary?.pass_rate ?? 0,
      color: RUN_COLORS[i % RUN_COLORS.length],
    }));
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

      // Diff highlighting: find best/worst cost among passing
      const passingCosts = results
        .map((r, i) => r && r.status === 'pass' ? { i, cost: r.cost_usd } : null)
        .filter((x): x is { i: number; cost: number } => x !== null);

      let worstIdx = -1;
      if (passingCosts.length >= 2) {
        let worst = -Infinity;
        for (const { i, cost } of passingCosts) {
          if (cost > worst) { worst = cost; worstIdx = i; }
        }
      }

      return { taskId, name, results, bestIdx, worstIdx };
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
        <StaggerSection index={0}>
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
        </StaggerSection>

        {fullRuns.length >= 2 ? (
          <>
            {/* ── View toggle with crossfade ── */}
            <StaggerSection index={1}>
              <div className="compare-view-toggle">
                <button
                  className={`compare-view-btn ${view === 'overview' ? 'active' : ''}`}
                  onClick={() => setView('overview')}
                >
                  Overview
                </button>
                <button
                  className={`compare-view-btn ${view === 'matrix' ? 'active' : ''}`}
                  onClick={() => setView('matrix')}
                >
                  Task Matrix
                </button>
              </div>
            </StaggerSection>

            {/* ── Crossfade container ── */}
            <div className="compare-crossfade-container">
              {/* ── Overview view ── */}
              <div
                className="compare-view-panel"
                style={{
                  opacity: view === 'overview' ? 1 : 0,
                  transform: view === 'overview' ? 'scale(1)' : 'scale(0.97)',
                  pointerEvents: view === 'overview' ? 'auto' : 'none',
                  position: view === 'overview' ? 'relative' : 'absolute',
                  transition: 'opacity 350ms cubic-bezier(0.22,1,0.36,1), transform 350ms cubic-bezier(0.22,1,0.36,1)',
                  width: '100%',
                }}
              >
                {/* ── Winner announcement ── */}
                {winnerIdx >= 0 && (
                  <StaggerSection index={2}>
                    <div className="compare-winner-banner">
                      <span className="compare-winner-crown">&#9733;</span>
                      <span className="compare-winner-text">
                        <strong>{fullRuns[winnerIdx].config.model}</strong> wins with{' '}
                        {((fullRuns[winnerIdx].summary?.pass_rate ?? 0) * 100).toFixed(1)}% pass rate
                        at {formatCost(fullRuns[winnerIdx].summary?.total_cost_usd ?? 0)}
                      </span>
                    </div>
                  </StaggerSection>
                )}

                {/* ── Config Diff ── */}
                <StaggerSection index={3}>
                  <Pane title="CONFIG COMPARISON">
                    <ConfigDiff runs={fullRuns} />
                  </Pane>
                </StaggerSection>

                {/* ── Summary Metrics Mosaic (animated cards) ── */}
                {summaryMetrics && (
                  <StaggerSection index={4}>
                    <Pane title="AGGREGATE SUMMARY">
                      <div className="compare-mosaic">
                        <MetricCard label="Runs Compared" value={fullRuns.length} index={0} slideFrom="left" />
                        <MetricCard label="Total Tasks" value={summaryMetrics.totalTasks} index={1} slideFrom="right" />
                        <MetricCard
                          label="Total Passed"
                          value={summaryMetrics.totalPassed}
                          index={2}
                          slideFrom="left"
                          isWinner
                          className="gate-ok-bg"
                        />
                        <MetricCard label="Total Cost" value={summaryMetrics.totalCost} suffix="$" index={3} slideFrom="right" />
                        <MetricCard
                          label="Best Pass Rate"
                          value={summaryMetrics.bestPassRate * 100}
                          suffix="%"
                          index={4}
                          slideFrom="left"
                          isWinner
                        />
                        <MetricCard
                          label="Avg Duration"
                          value={summaryMetrics.avgDuration / 1000}
                          suffix="s"
                          index={5}
                          slideFrom="right"
                        />
                        <MetricCard
                          label="Cheapest Run"
                          value={summaryMetrics.cheapestRun.summary?.total_cost_usd ?? 0}
                          suffix="$"
                          index={6}
                          slideFrom="left"
                        />
                      </div>
                    </Pane>
                  </StaggerSection>
                )}

                {/* ── Pass Rate Bar Chart ── */}
                {passRateBars.length >= 2 && (
                  <StaggerSection index={5}>
                    <Pane title="PASS RATE COMPARISON">
                      <div className="compare-bars">
                        {passRateBars.map((bar, i) => (
                          <CompareBar
                            key={i}
                            value={bar.value}
                            maxValue={1}
                            color={bar.color}
                            label={bar.label}
                            index={i}
                          />
                        ))}
                      </div>
                    </Pane>
                  </StaggerSection>
                )}

                {/* ── Radar Chart Overlay ── */}
                {radarDatasets.length >= 2 && (
                  <StaggerSection index={6}>
                    <ComponentErrorBoundary name="RadarOverlay">
                      <Pane title="RADAR OVERLAY">
                        <RadarChart axes={RADAR_AXES} datasets={radarDatasets} height={360} />
                      </Pane>
                    </ComponentErrorBoundary>
                  </StaggerSection>
                )}
              </div>

              {/* ── Matrix view ── */}
              <div
                className="compare-view-panel"
                style={{
                  opacity: view === 'matrix' ? 1 : 0,
                  transform: view === 'matrix' ? 'scale(1)' : 'scale(0.97)',
                  pointerEvents: view === 'matrix' ? 'auto' : 'none',
                  position: view === 'matrix' ? 'relative' : 'absolute',
                  transition: 'opacity 350ms cubic-bezier(0.22,1,0.36,1), transform 350ms cubic-bezier(0.22,1,0.36,1)',
                  width: '100%',
                }}
              >
                {/* ── Task-by-Task N-way Matrix ── */}
                <ComponentErrorBoundary name="TaskMatrix">
                  <Pane title="TASK-BY-TASK MATRIX">
                    <div className="task-table-wrap">
                      <table className="task-table" role="table" tabIndex={0}>
                        <thead>
                          <tr>
                            <th rowSpan={2} role="columnheader" style={{ cursor: 'default' }}>Task</th>
                            {fullRuns.map((run, i) => (
                              <th
                                key={run.id}
                                colSpan={2}
                                role="columnheader"
                                className={hoveredCol === i ? 'compare-col-highlight' : ''}
                                style={{
                                  cursor: 'default',
                                  borderBottom: `2px solid ${RUN_COLORS[i % RUN_COLORS.length]}`,
                                }}
                              >
                                Run {run.id.slice(0, 8)}
                                {winnerIdx === i && <span className="compare-th-winner"> &#9733;</span>}
                              </th>
                            ))}
                          </tr>
                          <tr>
                            {fullRuns.map((run, i) => (
                              <Fragment key={run.id}>
                                <th
                                  role="columnheader"
                                  className={hoveredCol === i ? 'compare-col-highlight' : ''}
                                  style={{ cursor: 'default' }}
                                >
                                  Status
                                </th>
                                <th
                                  role="columnheader"
                                  className={hoveredCol === i ? 'compare-col-highlight' : ''}
                                  style={{ cursor: 'default' }}
                                >
                                  Cost
                                </th>
                              </Fragment>
                            ))}
                          </tr>
                        </thead>
                        <tbody>
                          {taskMatrix.map((row, rowIdx) => (
                            <tr
                              key={row.taskId}
                              tabIndex={0}
                              role="row"
                              className={hoveredRow === row.taskId ? 'compare-row-highlight' : ''}
                              onKeyDown={(e) => handleRowKeyDown(e, () => {})}
                              onMouseEnter={() => setHoveredRow(row.taskId)}
                              onMouseLeave={() => setHoveredRow('')}
                              style={{
                                animation: `compareRowFadeIn 400ms cubic-bezier(0.22,1,0.36,1) ${rowIdx * 30}ms both`,
                              }}
                            >
                              <td className="task-name" title={row.taskId}>{row.name}</td>
                              {row.results.map((r, i) => (
                                <Fragment key={fullRuns[i].id}>
                                  <td
                                    className={hoveredCol === i ? 'compare-col-highlight' : ''}
                                    onMouseEnter={() => setHoveredCol(i)}
                                    onMouseLeave={() => setHoveredCol(-1)}
                                  >
                                    {r ? (
                                      <span className={`status-badge status-${r.status}`}>
                                        {r.status.toUpperCase()}
                                      </span>
                                    ) : (
                                      <span className="mono">-</span>
                                    )}
                                  </td>
                                  <td
                                    className={`mono${row.bestIdx === i ? ' compare-diff-better' : ''}${row.worstIdx === i ? ' compare-diff-worse' : ''}${hoveredCol === i ? ' compare-col-highlight' : ''}`}
                                    style={row.bestIdx === i ? { fontWeight: 700 } : undefined}
                                    onMouseEnter={() => setHoveredCol(i)}
                                    onMouseLeave={() => setHoveredCol(-1)}
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
                </ComponentErrorBoundary>
              </div>
            </div>
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
