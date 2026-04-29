import { useRef, useEffect, useCallback } from 'react';
import './Charts.css';

interface GateVerdict {
  gate: string;
  passed: boolean;
}

interface TimelineTask {
  name: string;
  startMs: number;
  durationMs: number;
  status: 'pass' | 'fail' | 'pending' | 'running' | 'skipped';
  gateVerdicts?: GateVerdict[];
}

interface TimelineChartProps {
  tasks: TimelineTask[];
  height?: number;
}

const STATUS_COLORS: Record<TimelineTask['status'], string> = {
  pass: '#5A9A6A',    // --success
  fail: '#AA7088',    // --rose-dim
  running: '#C8B890', // --bone
  pending: '#6a5a68', // dim
  skipped: '#6a5a68', // dim
};

/** Gantt-style waterfall chart using Canvas 2D. */
export default function TimelineChart({ tasks, height = 300 }: TimelineChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || tasks.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const pad = { top: 8, right: 16, bottom: 28, left: 100 };
    const plotW = w - pad.left - pad.right;
    const rowH = Math.min(24, (h - pad.top - pad.bottom) / tasks.length - 4);

    ctx.clearRect(0, 0, w, h);

    // Time range
    const minStart = Math.min(...tasks.map((t) => t.startMs));
    const maxEnd = Math.max(...tasks.map((t) => t.startMs + t.durationMs));
    const totalMs = Math.max(maxEnd - minStart, 1);

    // Time axis
    const ticks = 5;
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    for (let i = 0; i <= ticks; i++) {
      const x = pad.left + (i / ticks) * plotW;
      const ms = minStart + (i / ticks) * totalMs;

      // Grid line
      ctx.strokeStyle = 'rgba(255,255,255,0.05)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x, pad.top);
      ctx.lineTo(x, pad.top + tasks.length * (rowH + 4));
      ctx.stroke();

      // Label
      ctx.fillStyle = '#6a5a68';
      const label = ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms)}ms`;
      ctx.fillText(label, x, h - 8);
    }

    // Task rows
    for (let i = 0; i < tasks.length; i++) {
      const task = tasks[i];
      const y = pad.top + i * (rowH + 4);
      const barX = pad.left + ((task.startMs - minStart) / totalMs) * plotW;
      const barW = Math.max(2, (task.durationMs / totalMs) * plotW);
      const color = STATUS_COLORS[task.status];

      // Task name
      ctx.fillStyle = '#8a7a88';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      const nameText = task.name.length > 12 ? task.name.slice(0, 11) + '\u2026' : task.name;
      ctx.fillText(nameText, pad.left - 8, y + rowH / 2);

      // Bar background
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y, plotW, rowH, 3);
      ctx.fill();

      // Bar
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(barX, y, barW, rowH, 3);
      ctx.fill();

      // Gate verdict markers
      if (task.gateVerdicts && task.gateVerdicts.length > 0) {
        const spacing = barW / (task.gateVerdicts.length + 1);
        for (let g = 0; g < task.gateVerdicts.length; g++) {
          const gx = barX + spacing * (g + 1);
          const gy = y + rowH / 2;
          const size = 4;
          ctx.fillStyle = task.gateVerdicts[g].passed ? '#5A9A6A' : '#AA7088';
          ctx.fillRect(gx - size / 2, gy - size / 2, size, size);
        }
      }
    }
  }, [tasks]);

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
