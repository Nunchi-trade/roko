import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../lib/color';
import { useCanvasSetup } from '../hooks/useCanvasSetup';
import './Charts/Charts.css';

export interface GateRung {
  name: string;
  rung: number;
  status: 'passed' | 'failed' | 'skipped';
  duration_ms: number;
}

export interface GateRun {
  task_id: string;
  timestamp: string;
  rungs: GateRung[];
}

interface GateWaterfallProps {
  runs: GateRun[];
  height?: number;
}

function getStatusColors() {
  return {
    passed: getCssVar('--success'),
    failed: getCssVar('--rose-bright'),
    skipped: 'rgba(255,255,255,0.06)',
  } as const;
}

function getStatusBorder() {
  return {
    passed: hexToRgba(getCssVar('--success'), 0.6),
    failed: hexToRgba(getCssVar('--rose-bright'), 0.6),
    skipped: 'rgba(255,255,255,0.08)',
  } as const;
}

function formatDuration(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${Math.round(ms)}ms`;
}

function drawRoundedRect(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  ctx.beginPath();
  if (typeof ctx.roundRect === 'function') {
    ctx.roundRect(x, y, w, h, r);
  } else {
    // Fallback for older browsers
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.arcTo(x + w, y, x + w, y + r, r);
    ctx.lineTo(x + w, y + h - r);
    ctx.arcTo(x + w, y + h, x + w - r, y + h, r);
    ctx.lineTo(x + r, y + h);
    ctx.arcTo(x, y + h, x, y + h - r, r);
    ctx.lineTo(x, y + r);
    ctx.arcTo(x, y, x + r, y, r);
    ctx.closePath();
  }
}

/** Gate waterfall timeline using Canvas 2D. */
export default function GateWaterfall({ runs, height = 360 }: GateWaterfallProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (runs.length === 0) {
      ctx.clearRect(0, 0, w, h);
      return;
    }

    const pad = { top: 30, right: 20, bottom: 30, left: 160 };
    const plotW = Math.max(w - pad.left - pad.right, 1);
    const rowH = Math.min(28, Math.max((h - pad.top - pad.bottom - 18) / runs.length - 4, 18));

    ctx.clearRect(0, 0, w, h);

    const maxTotal = Math.max(
      ...runs.map((run) => run.rungs.reduce((sum, rung) => sum + rung.duration_ms, 0)),
      1,
    );

    const STATUS_COLORS = getStatusColors();
    const STATUS_BORDER = getStatusBorder();

    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '11px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.fillText('GATE WATERFALL', pad.left, 16);

    ctx.strokeStyle = getCssVar('--border-soft');
    ctx.lineWidth = 1;
    ctx.fillStyle = getCssVar('--text-ghost');
    ctx.font = '8px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    for (let i = 0; i <= 4; i++) {
      const x = pad.left + (i / 4) * plotW;
      const ms = (maxTotal * i) / 4;
      ctx.fillText(formatDuration(ms), x, pad.top - 7);

      ctx.beginPath();
      ctx.moveTo(x, pad.top);
      ctx.lineTo(x, Math.min(h - pad.bottom, pad.top + runs.length * (rowH + 4)));
      ctx.stroke();
    }

    runs.forEach((run, runIndex) => {
      const y = pad.top + runIndex * (rowH + 4);
      const totalMs = run.rungs.reduce((sum, rung) => sum + rung.duration_ms, 0);

      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      const label = run.task_id.length > 20 ? `${run.task_id.slice(0, 20)}...` : run.task_id;
      ctx.fillText(label, pad.left - 10, y + rowH / 2 + 3);

      ctx.fillStyle = 'rgba(255,255,255,0.02)';
      drawRoundedRect(ctx, pad.left, y, plotW, rowH, 3);
      ctx.fill();

      let offsetMs = 0;
      run.rungs.forEach((rung) => {
        const width = (rung.duration_ms / maxTotal) * plotW;
        const drawWidth = rung.status === 'skipped' ? 0 : Math.max(width, 3);
        const x = pad.left + (offsetMs / maxTotal) * plotW;

        if (drawWidth > 0) {
          ctx.fillStyle = STATUS_COLORS[rung.status];
          drawRoundedRect(ctx, x, y + 1, drawWidth, rowH - 2, 2);
          ctx.fill();

          ctx.strokeStyle = STATUS_BORDER[rung.status];
          ctx.lineWidth = 0.5;
          ctx.stroke();

          if (drawWidth > 28) {
            ctx.fillStyle = rung.status === 'skipped' ? getCssVar('--text-ghost') : '#1a1418'; // #1a1418: dark text on colored bar
            ctx.font = '8px "JetBrains Mono", monospace';
            ctx.textAlign = 'left';
            ctx.fillText(rung.name, x + 4, y + rowH / 2 + 3);
          }
        }

        offsetMs += rung.duration_ms;
      });

      const totalLabel = formatDuration(totalMs);
      const totalX = pad.left + (totalMs / maxTotal) * plotW + 8;
      if (totalX + 48 < w) {
        ctx.fillStyle = getCssVar('--text-ghost');
        ctx.font = '9px "JetBrains Mono", monospace';
        ctx.textAlign = 'left';
        ctx.fillText(totalLabel, totalX, y + rowH / 2 + 3);
      }
    });

    const legendY = Math.min(h - pad.bottom + 2, pad.top + runs.length * (rowH + 4) + 12);
    const legendItems = [
      { label: 'passed', color: STATUS_COLORS.passed },
      { label: 'failed', color: STATUS_COLORS.failed },
      { label: 'skipped', color: STATUS_COLORS.skipped },
    ];

    let legendX = pad.left;
    legendItems.forEach((item) => {
      ctx.fillStyle = item.color;
      ctx.fillRect(legendX, legendY, 10, 10);
      ctx.strokeStyle = getCssVar('--border');
      ctx.lineWidth = 0.5;
      ctx.strokeRect(legendX, legendY, 10, 10);
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(item.label, legendX + 14, legendY + 9);
      legendX += 72;
    });
  }, [runs]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
