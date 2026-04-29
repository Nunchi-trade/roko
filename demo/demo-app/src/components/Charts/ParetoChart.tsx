import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

export interface ParetoPoint {
  label: string;
  cost: number;
  passRate: number;
  color?: string;
}

interface ParetoChartProps {
  data: ParetoPoint[];
  height?: number;
}

/** Scatter plot with Pareto frontier line drawn. Canvas 2D. */
export default function ParetoChart({ data, height = 320 }: ParetoChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (data.length === 0) return;

    const pad = { top: 24, right: 24, bottom: 36, left: 56 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;

    ctx.clearRect(0, 0, w, h);

    const maxCost = Math.max(...data.map((d) => d.cost), 0.01) * 1.1;

    // Grid
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 4; i++) {
      const y = pad.top + plotH * (1 - i / 4);
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(pad.left + plotW, y);
      ctx.stroke();

      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.fillText(`${(i * 25)}%`, pad.left - 8, y + 3);
    }

    // X-axis labels
    for (let i = 0; i <= 4; i++) {
      const x = pad.left + (i / 4) * plotW;
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText(`$${((maxCost * i) / 4).toFixed(2)}`, x, pad.top + plotH + 16);
    }

    // Axis labels
    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText('Cost (USD)', pad.left + plotW / 2, h - 4);

    ctx.save();
    ctx.translate(12, pad.top + plotH / 2);
    ctx.rotate(-Math.PI / 2);
    ctx.fillText('Pass Rate', 0, 0);
    ctx.restore();

    // Compute Pareto frontier: sort by cost ascending, keep running max of passRate
    const sorted = [...data].sort((a, b) => a.cost - b.cost);
    const frontier: ParetoPoint[] = [];
    let bestRate = -1;
    for (const pt of sorted) {
      if (pt.passRate > bestRate) {
        frontier.push(pt);
        bestRate = pt.passRate;
      }
    }

    // Draw frontier line
    if (frontier.length > 1) {
      ctx.beginPath();
      ctx.strokeStyle = hexToRgba(getCssVar('--rose-glow'), 0.5);
      ctx.lineWidth = 1.5;
      ctx.setLineDash([4, 4]);
      for (let i = 0; i < frontier.length; i++) {
        const x = pad.left + (frontier[i].cost / maxCost) * plotW;
        const y = pad.top + plotH * (1 - frontier[i].passRate);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
      ctx.setLineDash([]);
    }

    // Draw points
    for (const pt of data) {
      const x = pad.left + (pt.cost / maxCost) * plotW;
      const y = pad.top + plotH * (1 - pt.passRate);
      const color = pt.color ?? getCssVar('--rose');

      // Glow
      ctx.beginPath();
      ctx.arc(x, y, 8, 0, Math.PI * 2);
      ctx.fillStyle = hexToRgba(color, 0.15);
      ctx.fill();

      // Dot
      ctx.beginPath();
      ctx.arc(x, y, 4, 0, Math.PI * 2);
      ctx.fillStyle = color;
      ctx.fill();

      // Label
      ctx.fillStyle = getCssVar('--text-soft');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(pt.label, x + 8, y + 3);
    }
  }, [data]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
