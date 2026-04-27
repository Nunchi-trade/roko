import { useRef, useEffect, useCallback } from 'react';
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
export default function CostChart({ data, title, color = '#C8B890', height = 200 }: CostChartProps) {
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
    const pad = { top: 24, right: 16, bottom: 28, left: 52 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;

    // Clear
    ctx.clearRect(0, 0, w, h);

    // Title
    if (title) {
      ctx.fillStyle = '#706070';
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

      ctx.fillStyle = '#504050';
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
      ctx.fillStyle = color.replace(')', ',0.08)').replace('rgb', 'rgba');
      ctx.fill();
    }
  }, [data, title, color]);

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
