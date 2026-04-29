import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

interface ScatterPoint {
  x: number;
  y: number;
  label: string;
  color?: string;
  size?: number;
}

interface ScatterChartProps {
  points: ScatterPoint[];
  xLabel: string;
  yLabel: string;
  showTrendLine?: boolean;
  height?: number;
}

/** General purpose scatter plot using Canvas 2D. */
export default function ScatterChart({
  points,
  xLabel,
  yLabel,
  showTrendLine = false,
  height = 320,
}: ScatterChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (points.length === 0) return;

    const pad = { top: 16, right: 24, bottom: 36, left: 56 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;

    ctx.clearRect(0, 0, w, h);

    // Auto-scale axes
    const xs = points.map((p) => p.x);
    const ys = points.map((p) => p.y);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    const rangeX = Math.max(maxX - minX, 1e-9) * 1.1;
    const rangeY = Math.max(maxY - minY, 1e-9) * 1.1;
    const originX = minX - rangeX * 0.05;
    const originY = minY - rangeY * 0.05;

    const toCanvasX = (v: number) => pad.left + ((v - originX) / rangeX) * plotW;
    const toCanvasY = (v: number) => pad.top + plotH - ((v - originY) / rangeY) * plotH;

    // Grid lines
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    ctx.lineWidth = 1;
    ctx.font = '9px "JetBrains Mono", monospace';
    for (let i = 0; i <= 4; i++) {
      // Horizontal
      const yVal = originY + (i / 4) * rangeY;
      const yPx = toCanvasY(yVal);
      ctx.beginPath();
      ctx.moveTo(pad.left, yPx);
      ctx.lineTo(pad.left + plotW, yPx);
      ctx.stroke();
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.textAlign = 'right';
      ctx.fillText(yVal.toPrecision(3), pad.left - 8, yPx + 3);

      // Vertical
      const xVal = originX + (i / 4) * rangeX;
      const xPx = toCanvasX(xVal);
      ctx.beginPath();
      ctx.moveTo(xPx, pad.top);
      ctx.lineTo(xPx, pad.top + plotH);
      ctx.stroke();
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.textAlign = 'center';
      ctx.fillText(xVal.toPrecision(3), xPx, pad.top + plotH + 16);
    }

    // Axis labels
    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText(xLabel, pad.left + plotW / 2, h - 4);

    ctx.save();
    ctx.translate(12, pad.top + plotH / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText(yLabel, 0, 0);
    ctx.restore();

    // Trend line (linear regression)
    if (showTrendLine && points.length >= 2) {
      const n = points.length;
      let sumX = 0, sumY = 0, sumXX = 0, sumXY = 0;
      for (const p of points) {
        sumX += p.x;
        sumY += p.y;
        sumXX += p.x * p.x;
        sumXY += p.x * p.y;
      }
      const denom = n * sumXX - sumX * sumX;
      if (Math.abs(denom) > 1e-12) {
        const slope = (n * sumXY - sumX * sumY) / denom;
        const intercept = (sumY - slope * sumX) / n;

        const x0 = originX;
        const x1 = originX + rangeX;
        ctx.beginPath();
        ctx.strokeStyle = hexToRgba(getCssVar('--rose-glow'), 0.5);
        ctx.lineWidth = 1.5;
        ctx.setLineDash([4, 4]);
        ctx.moveTo(toCanvasX(x0), toCanvasY(slope * x0 + intercept));
        ctx.lineTo(toCanvasX(x1), toCanvasY(slope * x1 + intercept));
        ctx.stroke();
        ctx.setLineDash([]);
      }
    }

    // Points with glow
    for (const pt of points) {
      const cx = toCanvasX(pt.x);
      const cy = toCanvasY(pt.y);
      const color = pt.color ?? getCssVar('--rose');
      const r = pt.size ?? 4;

      // Glow
      ctx.beginPath();
      ctx.arc(cx, cy, r * 2, 0, Math.PI * 2);
      ctx.fillStyle = hexToRgba(color, 0.15);
      ctx.fill();

      // Dot
      ctx.beginPath();
      ctx.arc(cx, cy, r, 0, Math.PI * 2);
      ctx.fillStyle = color;
      ctx.fill();

      // Label
      ctx.fillStyle = getCssVar('--text-soft');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(pt.label, cx + r + 4, cy + 3);
    }
  }, [points, xLabel, yLabel, showTrendLine]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
