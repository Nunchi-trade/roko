import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import { useLiveApi } from '../../hooks/useLiveApi';
import { getCssVar } from '../../lib/color';
import { roleColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
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

/* ── Table styles ────────────────────────────────────────── */

const thStyle: CSSProperties = {
  padding: '6px 10px',
  color: 'var(--text-dim)',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'var(--raised)',
  fontWeight: 600,
  textAlign: 'left',
  fontFamily: 'var(--mono)',
  fontSize: '0.6rem',
  letterSpacing: '.08em',
  textTransform: 'uppercase',
};

const tdStyle: CSSProperties = {
  padding: '5px 10px',
  color: 'var(--text)',
  borderBottom: '1px solid var(--glass-border)',
  verticalAlign: 'middle',
  fontFamily: 'var(--mono)',
  fontSize: '0.7rem',
};

/* ── Component ───────────────────────────────────────────── */

export default function CascadeRouter() {
  const { get } = useLiveApi();
  const [state, setState] = useState<CascadeState>({});
  const [loading, setLoading] = useState(true);

  const fetchState = useCallback(async () => {
    try {
      const data = await get<CascadeState>('/api/learn/cascade-router');
      setState(data ?? {});
    } catch {
      /* keep previous */
    } finally {
      setLoading(false);
    }
  }, [get]);

  // Initial fetch on mount
  useEffect(() => { fetchState(); }, [fetchState]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchState, 2000);
  useContextEventSubscription(
    ['inference_completed'],
    debouncedRefetch,
  );

  const rows = useMemo(
    () => Object.entries(state.confidence_stats ?? {}).sort(([a], [b]) => a.localeCompare(b)),
    [state.confidence_stats],
  );

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

  return (
    <div className="dash-page--full">
      {/* TOP MOSAIC */}
      <Mosaic columns={5}>
        <MosaicCell label="MODELS" value={rows.length} color="rose" mono />
        <MosaicCell label="OBSERVATIONS" value={state.total_observations ?? stats.totalTrials} color="bone" mono />
        <MosaicCell label="AVG CONFIDENCE" value={`${(stats.avgConfidence * 100).toFixed(1)}%`} color="success" mono />
        <MosaicCell label="BEST MODEL" value={stats.bestModel} color="dream" />
        <MosaicCell label="ROLES ASSIGNED" value={roleEntries.length} color="warning" mono />
      </Mosaic>

      {/* MAIN CONTENT: 3-column layout */}
      <div className="dash-grid-3col">
        {/* Model Confidence Chart */}
        <Pane
          title="MODEL CONFIDENCE"
          badge={<span className="dash-badge">success rate</span>}
        >
          <ModelConfidenceChart rows={rows} height={Math.max(80, rows.length * 30 + 16)} />
        </Pane>

        {/* Model Statistics Table */}
        <Pane
          title="MODEL STATISTICS"
          badge={<span className="dash-badge">detailed breakdown</span>}
          flat
        >
          <div className="dash-scroll">
            {loading ? (
              <div className="dash-placeholder">Loading cascade router...</div>
            ) : rows.length === 0 ? (
              <div className="dash-placeholder">No model stats found</div>
            ) : (
              <table style={{ width: '100%', borderCollapse: 'collapse' }}>
                <thead>
                  <tr>
                    <th style={thStyle}>Model</th>
                    <th style={thStyle}>Confidence</th>
                    <th style={thStyle}>Successes</th>
                    <th style={thStyle}>Trials</th>
                    <th style={thStyle}>Cost</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map(([model, stat]) => {
                    const conf = stat.trials > 0 ? stat.successes / stat.trials : 0;
                    return (
                      <tr
                        key={model}
                        className="dash-table-row"
                      >
                        <td style={tdStyle}>{model}</td>
                        <td style={tdStyle}>
                          <span className="dash-inline--8">
                            <span className="dash-minibar">
                              <span
                                className="dash-minibar__fill"
                                style={{
                                  width: `${conf * 100}%`,
                                  background: conf >= 0.9 ? 'var(--success)' : conf >= 0.7 ? 'var(--warning)' : 'var(--rose-bright)',
                                }}
                              />
                            </span>
                            {`${(conf * 100).toFixed(1)}%`}
                          </span>
                        </td>
                        <td style={tdStyle}>{stat.successes}</td>
                        <td style={tdStyle}>{stat.trials}</td>
                        <td style={tdStyle}>{stat.total_cost_usd != null ? `$${stat.total_cost_usd.toFixed(3)}` : '—'}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        </Pane>

        {/* Role Assignments */}
        <Pane
          title="ROLE ASSIGNMENTS"
          badge={<span className="dash-badge">{roleEntries.length} roles</span>}
        >
          {loading ? (
            <div className="dash-placeholder">Loading...</div>
          ) : roleEntries.length === 0 ? (
            <div className="dash-placeholder">No role assignments</div>
          ) : (
            <div className="dash-flex-col dash-scroll">
              {roleEntries.map(([role, model], i) => (
                <div
                  key={role}
                  className={`dash-role-row${i < roleEntries.length - 1 ? ' dash-row-sep' : ''}`}
                >
                  <span className="dash-inline">
                    <span
                      className="dash-dot--5"
                      style={{
                        background: roleColor(role),
                        boxShadow: `0 0 6px ${roleColor(role)}60`,
                      }}
                    />
                    <span className="dash-display-sm">{role}</span>
                  </span>
                  <span className="dash-tag--sm" style={{ flexShrink: 0 }}>
                    {model.replace(/^claude-/, '')}
                  </span>
                </div>
              ))}
            </div>
          )}
        </Pane>
      </div>
    </div>
  );
}
