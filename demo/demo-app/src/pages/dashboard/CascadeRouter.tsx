import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import {
  AnimatedRow,
  AnimatedHeaderCell,
  TableEmptyState,
} from '../../components/AnimatedTable';
import { useLiveApi } from '../../hooks/useLiveApi';
import { getCssVar } from '../../lib/color';
import { roleColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import DataSurface from '../../components/design/DataSurface';
import '../../styles/table.css';
import './CascadeRouter.css';
import './dashboard.css';

/* ── Types ────────────────────────────────────────────────── */

interface ConfidenceStat {
  successes: number;
  trials: number;
  total_cost_usd?: number;
}

interface CascadeState {
  model_slugs?: string[];
  role_table?: Record<string, string>;
  confidence_stats?: Record<string, ConfidenceStat>;
  total_observations?: number;
}

/* ── Confidence bar chart canvas ─────────────────────────── */

function ModelConfidenceChart({ rows, height = 200 }: { rows: [string, ConfidenceStat][]; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    ctx.clearRect(0, 0, w, h);

    if (rows.length === 0) {
      ctx.fillStyle = 'rgba(194,184,201,0.5)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText('No model stats', w / 2, h / 2);
      return;
    }

    const pad = { left: 120, right: 60, top: 8, bottom: 8 };
    const plotW = w - pad.left - pad.right;
    const barH = Math.min(24, (h - pad.top - pad.bottom) / rows.length - 5);

    rows.forEach(([model, stat], i) => {
      const y = pad.top + i * (barH + 5);
      const conf = stat.trials > 0 ? stat.successes / stat.trials : 0;
      const barW = conf * plotW;

      // Color based on confidence
      const color = conf >= 0.9 ? getCssVar('--success') : conf >= 0.7 ? getCssVar('--warning') : getCssVar('--rose-bright');

      // Label
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      const label = model.replace(/^claude-/, '').slice(0, 16);
      ctx.fillText(label, pad.left - 10, y + barH / 2);

      // Bar track
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y, plotW, barH, 3);
      ctx.fill();

      // Bar fill
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.7;
      ctx.beginPath();
      ctx.roundRect(pad.left, y, Math.max(barW, 3), barH, 3);
      ctx.fill();
      ctx.globalAlpha = 1;

      // Glow on the bar
      ctx.shadowColor = `${color}60`;
      ctx.shadowBlur = 8;
      ctx.beginPath();
      ctx.roundRect(pad.left, y, Math.max(barW, 3), barH, 3);
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.shadowColor = 'transparent';

      // Value
      ctx.fillStyle = getCssVar('--text-soft');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(`${(conf * 100).toFixed(1)}%`, pad.left + barW + 8, y + barH / 2);
    });
  }, [rows]);

  useEffect(() => {
    draw();
    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div className="dash-canvas-wrap" style={{ height }}>
      <canvas ref={canvasRef} role="img" aria-label="Cascade router model distribution" className="dash-canvas" />
    </div>
  );
}

/* ── Component ───────────────────────────────────────────── */

type CRSortKey = 'model' | 'confidence' | 'successes' | 'trials' | 'cost';

export default function CascadeRouter() {
  const { get } = useLiveApi();
  const [state, setState] = useState<CascadeState>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<CRSortKey>('model');
  const [sortAsc, setSortAsc] = useState(true);

  function handleSort(key: string) {
    const k = key as CRSortKey;
    if (sortKey === k) setSortAsc(!sortAsc);
    else { setSortKey(k); setSortAsc(true); }
  }

  const fetchState = useCallback(async () => {
    try {
      const data = await get<CascadeState>('/api/learn/cascade-router');
      setState(data ?? {});
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load cascade router data');
    } finally {
      setLoading(false);
    }
  }, [get]);

  // Initial fetch + 30s fallback poll
  useEffect(() => {
    fetchState();
    const id = setInterval(fetchState, 30_000);
    return () => clearInterval(id);
  }, [fetchState]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchState, 2000);
  useContextEventSubscription(
    ['inference_completed'],
    debouncedRefetch,
  );

  const rows = useMemo(() => {
    const entries = Object.entries(state.confidence_stats ?? {});
    return entries.sort(([aModel, aStat], [bModel, bStat]) => {
      let cmp = 0;
      switch (sortKey) {
        case 'model': cmp = aModel.localeCompare(bModel); break;
        case 'confidence': {
          const ac = aStat.trials > 0 ? aStat.successes / aStat.trials : 0;
          const bc = bStat.trials > 0 ? bStat.successes / bStat.trials : 0;
          cmp = ac - bc;
          break;
        }
        case 'successes': cmp = aStat.successes - bStat.successes; break;
        case 'trials': cmp = aStat.trials - bStat.trials; break;
        case 'cost': cmp = (aStat.total_cost_usd ?? 0) - (bStat.total_cost_usd ?? 0); break;
      }
      return sortAsc ? cmp : -cmp;
    });
  }, [state.confidence_stats, sortKey, sortAsc]);

  const roleEntries = useMemo(
    () => Object.entries(state.role_table ?? {}).sort(([a], [b]) => a.localeCompare(b)),
    [state.role_table],
  );

  const stats = useMemo(() => {
    const totalTrials = rows.reduce((sum, [, s]) => sum + s.trials, 0);
    const totalSuccesses = rows.reduce((sum, [, s]) => sum + s.successes, 0);
    const totalCost = rows.reduce((sum, [, s]) => sum + (s.total_cost_usd ?? 0), 0);
    const bestModel = rows.length > 0
      ? rows.reduce((best, curr) => {
          const bestConf = best[1].trials > 0 ? best[1].successes / best[1].trials : 0;
          const currConf = curr[1].trials > 0 ? curr[1].successes / curr[1].trials : 0;
          return currConf > bestConf ? curr : best;
        })
      : null;

    return {
      totalTrials,
      avgConfidence: totalTrials > 0 ? totalSuccesses / totalTrials : 0,
      totalCost,
      bestModel: bestModel ? bestModel[0].replace(/^claude-/, '') : '—',
    };
  }, [rows]);

  if (loading) {
    return (
      <div className="dash-page progressive-reveal">
        <div className="skeleton" style={{ height: 32, borderRadius: 6 }} />
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, marginTop: 8 }}>
          <div className="skeleton" style={{ height: 200, borderRadius: 6 }} />
          <div className="skeleton" style={{ height: 200, borderRadius: 6 }} />
        </div>
      </div>
    );
  }

  return (
    <DataSurface
      loading={false}
      empty={rows.length === 0 && roleEntries.length === 0}
      emptyLabel="No cascade router data. Run a plan to populate model statistics."
    >
    <div className="dash-page--full" style={{ animation: 'fadeInUp 0.35s var(--ease) both' }}>
      {error && (
        <div style={{ padding: '8px 12px', background: 'var(--rose-deep)', border: '1px solid var(--rose-dim)', borderRadius: 'var(--radius-md)', fontFamily: 'var(--mono)', fontSize: 'var(--text-xs)', color: 'var(--rose-bright)', marginBottom: 8 }}>
          {error}
        </div>
      )}
      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={5}>
          <MosaicCell label="MODELS" value={rows.length} color="rose" mono />
          <MosaicCell label="OBSERVATIONS" value={state.total_observations ?? stats.totalTrials} color="bone" mono />
          <MosaicCell label="AVG CONFIDENCE" value={`${(stats.avgConfidence * 100).toFixed(1)}%`} color="success" mono />
          <MosaicCell label="BEST MODEL" value={stats.bestModel} color="dream" />
          <MosaicCell label="ROLES ASSIGNED" value={roleEntries.length} color="warning" mono />
        </Mosaic>
      </div>

      {/* MAIN CONTENT: 3-column layout */}
      <div className="dash-grid-3col">
        {/* Model Confidence Chart */}
        <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
          <Pane
            title="MODEL CONFIDENCE"
            badge={<span className="dash-badge">success rate</span>}
          >
            <div className="dash-chart-enter">
              <ModelConfidenceChart rows={rows} height={Math.max(80, rows.length * 30 + 16)} />
            </div>
          </Pane>
        </div>

        {/* Model Statistics Table */}
        <div className="dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane
            title="MODEL STATISTICS"
            badge={<span className="dash-badge">detailed breakdown</span>}
            flat
          >
            <div className="dash-scroll dash-crossfade-enter">
              {loading ? (
                <div className="dash-placeholder">Loading cascade router...</div>
              ) : (
                <table className="tbl-container">
                  <thead>
                    <tr>
                      <AnimatedHeaderCell sortKey="model" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Model</AnimatedHeaderCell>
                      <AnimatedHeaderCell sortKey="confidence" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Confidence</AnimatedHeaderCell>
                      <AnimatedHeaderCell sortKey="successes" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Successes</AnimatedHeaderCell>
                      <AnimatedHeaderCell sortKey="trials" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Trials</AnimatedHeaderCell>
                      <AnimatedHeaderCell sortKey="cost" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Cost</AnimatedHeaderCell>
                    </tr>
                  </thead>
                  <tbody>
                    {rows.length === 0 ? (
                      <TableEmptyState colSpan={5} message="No model stats found" />
                    ) : (
                      rows.map(([model, stat], rowIdx) => {
                        const conf = stat.trials > 0 ? stat.successes / stat.trials : 0;
                        return (
                          <AnimatedRow key={model} index={rowIdx}>
                            <td className="tbl-cell">{model}</td>
                            <td className="tbl-cell">
                              <span className="dash-inline--8">
                                <span className="dash-minibar">
                                  <span
                                    className="dash-minibar__fill dash-bar-animate"
                                    style={{
                                      width: `${conf * 100}%`,
                                      background: conf >= 0.9 ? 'var(--success)' : conf >= 0.7 ? 'var(--warning)' : 'var(--rose-bright)',
                                      animationDelay: `${rowIdx * 60 + 200}ms`,
                                    }}
                                  />
                                </span>
                                {`${(conf * 100).toFixed(1)}%`}
                              </span>
                            </td>
                            <td className="tbl-cell">{stat.successes}</td>
                            <td className="tbl-cell">{stat.trials}</td>
                            <td className="tbl-cell">{stat.total_cost_usd != null ? `$${stat.total_cost_usd.toFixed(3)}` : '---'}</td>
                          </AnimatedRow>
                        );
                      })
                    )}
                  </tbody>
                </table>
              )}
            </div>
          </Pane>
        </div>

        {/* Role Assignments */}
        <div className="dash-stagger" style={{ '--stagger-i': 3 } as React.CSSProperties}>
          <Pane
            title="ROLE ASSIGNMENTS"
            badge={<span className="dash-badge">{roleEntries.length} roles</span>}
          >
            {loading ? (
              <div className="dash-placeholder">Loading...</div>
            ) : roleEntries.length === 0 ? (
              <div className="dash-empty">
                <span className="dash-empty__label">No role assignments</span>
                <code className="dash-empty__cmd">roko plan run plans/</code>
              </div>
            ) : (
              <div className="dash-flex-col dash-scroll">
                {roleEntries.map(([role, model], i) => (
                  <div
                    key={role}
                    className={`dash-role-row dash-stagger${i < roleEntries.length - 1 ? ' dash-row-sep' : ''}`}
                    style={{ '--stagger-i': i } as React.CSSProperties}
                  >
                    <span className="dash-inline">
                      <span
                        className="dash-dot--5 dash-status-breathe"
                        style={{
                          background: roleColor(role),
                          color: roleColor(role),
                          boxShadow: `0 0 6px ${roleColor(role)}60`,
                        }}
                      />
                      <span className="dash-display-sm">{role}</span>
                    </span>
                    <span className="dash-tag--sm cr-role-model">
                      {model.replace(/^claude-/, '')}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </Pane>
        </div>
      </div>
    </div>
    </DataSurface>
  );
}
