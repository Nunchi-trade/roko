import { useRef } from 'react';
import { getCssVar } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

interface BarData {
  label: string;
  value: number;
  color?: string;
}

interface BarChartProps {
  data: BarData[];
  title?: string;
  height?: number;
}

/** Simple horizontal bar chart using Canvas 2D. */
export default function BarChart({ data, title, height = 200 }: BarChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (data.length === 0) return;

    const pad = { top: 24, right: 16, bottom: 8, left: 80 };
    const plotW = w - pad.left - pad.right;
    const barH = Math.min(24, (h - pad.top - pad.bottom) / data.length - 4);
    const maxVal = Math.max(...data.map((d) => d.value), 1);

    ctx.clearRect(0, 0, w, h);

    if (title) {
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '11px "General Sans", sans-serif';
      ctx.fillText(title, pad.left, 16);
    }

    data.forEach((d, i) => {
      const y = pad.top + i * (barH + 4);
      const barW = (d.value / maxVal) * plotW;
      const color = d.color ?? getCssVar('--rose');

      // Label
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.fillText(d.label, pad.left - 8, y + barH / 2 + 3);

      // Bar background
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y, plotW, barH, 3);
      ctx.fill();

      // Bar fill
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(pad.left, y, barW, barH, 3);
      ctx.fill();

      // Value
      ctx.fillStyle = getCssVar('--text-soft');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(d.value.toFixed(1), pad.left + barW + 6, y + barH / 2 + 3);
    });
  }, [data, title]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Bar chart visualization" />
    </div>
  );
}
