import { useRef, useEffect, useCallback } from 'react';
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

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || data.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const pad = { top: 24, right: 16, bottom: 8, left: 80 };
    const plotW = w - pad.left - pad.right;
    const barH = Math.min(24, (h - pad.top - pad.bottom) / data.length - 4);
    const maxVal = Math.max(...data.map((d) => d.value), 1);

    ctx.clearRect(0, 0, w, h);

    if (title) {
      ctx.fillStyle = '#706070';
      ctx.font = '11px "General Sans", sans-serif';
      ctx.fillText(title, pad.left, 16);
    }

    data.forEach((d, i) => {
      const y = pad.top + i * (barH + 4);
      const barW = (d.value / maxVal) * plotW;
      const color = d.color ?? '#AA7088';

      // Label
      ctx.fillStyle = '#706070';
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
      ctx.fillStyle = '#B0A0B0';
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(d.value.toFixed(1), pad.left + barW + 6, y + barH / 2 + 3);
    });
  }, [data, title]);

  useEffect(() => {
    draw();
    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
