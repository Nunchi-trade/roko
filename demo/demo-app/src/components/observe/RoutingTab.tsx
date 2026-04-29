import { useRef, useEffect, useCallback } from 'react';
import Mosaic, { MosaicCell } from '../Mosaic';
import Pane from '../Pane';
import { getCssVar } from '../../lib/color';
import './RoutingTab.css';

/* ── Types ────────────────────────────────────────────── */

export interface ModelStats {
  model: string;
  count: number;
  avg_cost: number;
  avg_quality: number;
}

export interface RoutingStats {
  total_decisions: number;
  models: ModelStats[];
  cascade_hits: number;
  fallback_rate: number;
}

export interface RoutingTabProps {
  stats: RoutingStats | null;
  loading?: boolean;
}

/* ── Constants ────────────────────────────────────────── */

const BAR_COLORS = ['#8a9c86', '#b4a0c8', '#d4c89c', '#d89ab2', '#9a8a98'];

/* ── Component ────────────────────────────────────────── */

/**
 * Routing tab: mosaic stats + horizontal bar chart + model detail table.
 * Displays CascadeRouter metrics.
 */
export function RoutingTab({ stats, loading }: RoutingTabProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Distribution chart (horizontal bar chart on canvas)
  const drawChart = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container || !stats || stats.models.length === 0) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = container.getBoundingClientRect();
    const chartH = Math.min(200, 10 + stats.models.length * 32 + 10);
    canvas.width = rect.width * dpr;
    canvas.height = chartH * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${chartH}px`;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    ctx.fillStyle = getCssVar('--bg-void') || '#08080c';
    ctx.fillRect(0, 0, rect.width, chartH);

    const total = stats.models.reduce((s, m) => s + m.count, 0) || 1;
    const barHeight = 24;
    const gap = 8;
    const labelWidth = 120;
    const barMax = rect.width - labelWidth - 60;

    stats.models.forEach((m, i) => {
      const y = 10 + i * (barHeight + gap);
      const w = (m.count / total) * barMax;

      // Label
      ctx.fillStyle = getCssVar('--text-primary') || '#e8dce8';
      ctx.font = '12px JetBrains Mono, monospace';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      const label = m.model.length > 16 ? m.model.slice(0, 15) + '\u2026' : m.model;
      ctx.fillText(label, 8, y + barHeight / 2);

      // Bar
      ctx.fillStyle = BAR_COLORS[i % BAR_COLORS.length];
      ctx.globalAlpha = 0.8;
      ctx.beginPath();
      ctx.roundRect(labelWidth, y, Math.max(w, 4), barHeight, 3);
      ctx.fill();
      ctx.globalAlpha = 1;

      // Count
      ctx.fillStyle = getCssVar('--text-dim') || '#9a8a98';
      ctx.textAlign = 'left';
      ctx.fillText(String(m.count), labelWidth + w + 8, y + barHeight / 2);
    });
  }, [stats]);

  useEffect(() => {
    drawChart();
    const ro = new ResizeObserver(() => drawChart());
    const container = containerRef.current;
    if (container) ro.observe(container);
    return () => ro.disconnect();
  }, [drawChart]);

  if (loading) {
    return <div className="routing-tab__loading">Loading routing data...</div>;
  }
  if (!stats) {
    return <div className="routing-tab__loading">No routing data available</div>;
  }

  return (
    <div className="routing-tab">
      <Mosaic columns={3}>
        <MosaicCell label="Decisions" value={String(stats.total_decisions)} color="dream" mono />
        <MosaicCell label="Cascade Hits" value={String(stats.cascade_hits)} color="success" mono />
        <MosaicCell
          label="Fallback Rate"
          value={`${(stats.fallback_rate * 100).toFixed(1)}%`}
          color="rose"
          mono
        />
      </Mosaic>

      <Pane title="Model Distribution" flat>
        <div ref={containerRef} className="routing-tab__chart">
          <canvas ref={canvasRef} />
        </div>
      </Pane>

      <Pane title="Model Details" flat>
        <table className="routing-tab__table">
          <thead>
            <tr>
              <th>Model</th>
              <th>Calls</th>
              <th>Avg Cost</th>
              <th>Avg Quality</th>
            </tr>
          </thead>
          <tbody>
            {stats.models.map((m) => (
              <tr key={m.model}>
                <td className="routing-tab__model-name">{m.model}</td>
                <td>{m.count}</td>
                <td>${m.avg_cost.toFixed(4)}</td>
                <td>{(m.avg_quality * 100).toFixed(1)}%</td>
              </tr>
            ))}
            {stats.models.length === 0 && (
              <tr>
                <td colSpan={4} className="routing-tab__empty">No model data</td>
              </tr>
            )}
          </tbody>
        </table>
      </Pane>
    </div>
  );
}
