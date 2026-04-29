import { useRef, useEffect, useCallback } from 'react';
import './Charts.css';

interface RadarDataset {
  label: string;
  values: number[];
  color: string;
}

interface RadarChartProps {
  axes: string[];
  datasets: RadarDataset[];
  height?: number;
}

/** Spider/radar chart using Canvas 2D. */
export default function RadarChart({ axes, datasets, height = 320 }: RadarChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || axes.length < 3) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const legendH = datasets.length > 0 ? 28 : 0;
    const cx = w / 2;
    const cy = (h - legendH) / 2 + 8;
    const radius = Math.min(w / 2, (h - legendH) / 2) - 40;
    const n = axes.length;
    const angleStep = (Math.PI * 2) / n;

    ctx.clearRect(0, 0, w, h);

    const vertex = (i: number, r: number): [number, number] => {
      const angle = -Math.PI / 2 + i * angleStep;
      return [cx + Math.cos(angle) * r, cy + Math.sin(angle) * r];
    };

    // Concentric grid polygons (5 levels: 0%, 25%, 50%, 75%, 100%)
    for (let level = 1; level <= 5; level++) {
      const r = (level / 5) * radius;
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const [x, y] = vertex(i, r);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.closePath();
      ctx.strokeStyle = 'rgba(255,255,255,0.08)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // Axis lines + labels
    for (let i = 0; i < n; i++) {
      const [x, y] = vertex(i, radius);
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(x, y);
      ctx.strokeStyle = 'rgba(255,255,255,0.08)';
      ctx.lineWidth = 1;
      ctx.stroke();

      // Label
      const [lx, ly] = vertex(i, radius + 16);
      ctx.fillStyle = '#8a7a88';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(axes[i], lx, ly);
    }

    // Dataset polygons
    for (const ds of datasets) {
      if (ds.values.length !== n) continue;

      // Fill
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const val = Math.max(0, Math.min(1, ds.values[i]));
        const [x, y] = vertex(i, val * radius);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.closePath();
      ctx.fillStyle = ds.color + '4D'; // 30% alpha
      ctx.fill();

      // Stroke
      ctx.beginPath();
      for (let i = 0; i < n; i++) {
        const val = Math.max(0, Math.min(1, ds.values[i]));
        const [x, y] = vertex(i, val * radius);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.closePath();
      ctx.strokeStyle = ds.color;
      ctx.lineWidth = 2;
      ctx.stroke();
    }

    // Legend at bottom
    if (datasets.length > 0) {
      const legendY = h - 12;
      let lx = 16;
      ctx.font = '10px "JetBrains Mono", monospace';
      for (const ds of datasets) {
        ctx.fillStyle = ds.color;
        ctx.fillRect(lx, legendY - 8, 10, 10);
        ctx.fillStyle = '#8a7a88';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'middle';
        ctx.fillText(ds.label, lx + 14, legendY - 3);
        lx += ctx.measureText(ds.label).width + 30;
      }
    }
  }, [axes, datasets]);

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
