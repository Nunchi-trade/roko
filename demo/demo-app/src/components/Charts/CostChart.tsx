import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

interface DataPoint {
  label: string;
  value: number;
}

interface CostChartProps {
  data: DataPoint[];
  title?: string;
  color?: string;
  height?: number;
}

/** Cumulative cost chart using Canvas 2D. */
export default function CostChart({ data, title, color: colorProp, height = 200 }: CostChartProps) {
  const color = colorProp ?? getCssVar('--bone');
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (data.length === 0) return;

    const pad = { top: 24, right: 16, bottom: 28, left: 52 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;

    // Clear
    ctx.clearRect(0, 0, w, h);

    // Title
    if (title) {
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '11px "General Sans", sans-serif';
      ctx.fillText(title, pad.left, 16);
    }

    // Cumulative values
    const cumulative: number[] = [];
    let sum = 0;
    for (const d of data) {
      sum += d.value;
      cumulative.push(sum);
    }
    const maxVal = Math.max(...cumulative, 1);

    // Grid lines
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
      ctx.fillText(`$${((maxVal * i) / 4).toFixed(2)}`, pad.left - 6, y + 3);
    }

    // Line
    ctx.beginPath();
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.lineJoin = 'round';
    for (let i = 0; i < cumulative.length; i++) {
      const x = pad.left + (i / Math.max(cumulative.length - 1, 1)) * plotW;
      const y = pad.top + plotH * (1 - cumulative[i] / maxVal);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    // Fill area under curve
    if (cumulative.length > 0) {
      const lastX = pad.left + ((cumulative.length - 1) / Math.max(cumulative.length - 1, 1)) * plotW;
      ctx.lineTo(lastX, pad.top + plotH);
      ctx.lineTo(pad.left, pad.top + plotH);
      ctx.closePath();
      ctx.fillStyle = hexToRgba(color, 0.08);
      ctx.fill();
    }
  }, [data, title, color]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Cost over time chart" />
    </div>
  );
}
