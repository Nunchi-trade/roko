import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../lib/color';
import { shortModel } from '../lib/format';
import { useCanvasSetup } from '../hooks/useCanvasSetup';
import type { MatrixCell } from '../hooks/useMatrixBench';

interface MatrixRaceTrackProps {
  cells: MatrixCell[][];
  selectedModels: string[];
  presetLabels: string[];
  totalTasksPerLane: number;
  height?: number;
}

function getLaneColors(): string[] {
  return [
    getCssVar('--bone'), getCssVar('--success'), getCssVar('--rose'), getCssVar('--warning'),
    getCssVar('--status-blocked'), '#7FA8A4', '#B7918F', '#C49B6E', // TODO: add design tokens for last 3
  ];
}

/** Canvas-based animated horizontal race track for matrix lanes. */
export default function MatrixRaceTrack({
  cells,
  selectedModels,
  presetLabels,
  totalTasksPerLane,
  height: heightProp,
}: MatrixRaceTrackProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef<number | null>(null);
  const displayRef = useRef<Map<string, number>>(new Map());

  // Flatten cells into lanes
  const lanes = cells.flatMap((row, ri) =>
    row.map((cell, ci) => ({
      key: `${ri}-${ci}`,
      label: `${shortModel(selectedModels[ri] ?? '')} / ${presetLabels[ci] ?? ''}`,
      completed: cell.results.length,
      total: totalTasksPerLane,
      passRate: cell.passRate,
      costUsd: cell.costUsd,
      status: cell.status,
      colorIndex: ri * row.length + ci,
    })),
  );

  const computedHeight = heightProp ?? Math.max(200, lanes.length * 32 + 48);

  /** Core scene renderer — called by useCanvasSetup (DPR-adjusted) and by animation continuation. */
  const drawScene = (ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const muted = getCssVar('--text-ghost');
    const labelPad = Math.min(Math.max(w * 0.28, 100), 180);
    const valuePad = Math.min(Math.max(w * 0.22, 100), 160);
    const pad = { top: 32, right: valuePad, bottom: 12, left: labelPad };
    const plotW = Math.max(w - pad.left - pad.right, 8);

    ctx.clearRect(0, 0, w, h);

    const LANE_COLORS = getLaneColors();

    // Title
    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '11px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText('MATRIX RACE TRACK', pad.left, 16);

    if (lanes.length === 0) {
      ctx.fillStyle = muted;
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.fillText('no lanes configured', pad.left, 36);
      return;
    }

    const rowGap = Math.max(4, Math.min(8, Math.round(h * 0.02)));
    const rowH = Math.min(30, Math.max((h - pad.top - pad.bottom - rowGap * (lanes.length - 1)) / lanes.length, 20));
    const barH = Math.max(12, rowH - 6);
    const display = displayRef.current;
    let needsNextFrame = false;

    // Grid lines
    ctx.strokeStyle = 'rgba(255,255,255,0.04)';
    ctx.lineWidth = 1;
    ctx.fillStyle = muted;
    ctx.font = '8px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    for (let i = 0; i <= 4; i++) {
      const x = pad.left + (i / 4) * plotW;
      ctx.beginPath();
      ctx.moveTo(x, pad.top - 2);
      ctx.lineTo(x, h - pad.bottom + 4);
      ctx.stroke();
      const pct = (i / 4) * 100;
      ctx.fillText(`${pct}%`, x, pad.top - 6);
    }

    lanes.forEach((lane, index) => {
      const y = pad.top + index * (rowH + rowGap);
      const centerY = y + rowH / 2;
      const targetFrac = lane.total > 0 ? lane.completed / lane.total : 0;
      const current = display.get(lane.key) ?? 0;
      const next = current + (targetFrac - current) * 0.16;
      display.set(lane.key, next);

      if (Math.abs(next - targetFrac) > 0.001) needsNextFrame = true;

      const color = LANE_COLORS[lane.colorIndex % LANE_COLORS.length];
      const barW = Math.max(next * plotW, next > 0 ? 2 : 0);

      // Label
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.fillStyle = index === 0 ? getCssVar('--text-soft') : getCssVar('--text-dim');

      let label = lane.label;
      while (label.length > 4 && ctx.measureText(label).width > pad.left - 16) {
        label = label.slice(0, -1);
      }
      if (label !== lane.label) label += '..';
      ctx.fillText(label, pad.left - 8, centerY);

      // Track background
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y + 2, plotW, barH, 4);
      ctx.fill();

      // Bar
      if (barW > 0) {
        const grad = ctx.createLinearGradient(pad.left, 0, pad.left + barW, 0);
        grad.addColorStop(0, hexToRgba(color, 0.35));
        grad.addColorStop(1, hexToRgba(color, 0.85));
        ctx.fillStyle = grad;
        ctx.beginPath();
        ctx.roundRect(pad.left, y + 2, barW, barH, 4);
        ctx.fill();

        // Glow tip
        const glow = ctx.createRadialGradient(
          pad.left + barW, centerY, 0,
          pad.left + barW, centerY, Math.max(barH * 0.8, 10),
        );
        glow.addColorStop(0, hexToRgba(color, 0.25));
        glow.addColorStop(1, hexToRgba(color, 0));
        ctx.fillStyle = glow;
        ctx.beginPath();
        ctx.arc(pad.left + barW, centerY, Math.max(barH * 0.6, 6), 0, Math.PI * 2);
        ctx.fill();
      }

      // Values on right
      const valueX = pad.left + plotW + 10;
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';

      // Progress
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.fillStyle = color;
      ctx.fillText(`${lane.completed}/${lane.total}`, valueX, centerY - 6);

      // Pass rate + cost
      ctx.font = '8px "JetBrains Mono", monospace';
      ctx.fillStyle = muted;
      const rate = lane.passRate != null ? `${(lane.passRate * 100).toFixed(0)}%` : '-';
      const cost = lane.costUsd != null ? `$${lane.costUsd.toFixed(3)}` : '';
      ctx.fillText(`${rate}  ${cost}`, valueX, centerY + 6);
    });

    if (needsNextFrame) {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        const canvas = canvasRef.current;
        if (!canvas) return;
        const c = canvas.getContext('2d');
        if (!c) return;
        const dpr = window.devicePixelRatio || 1;
        const rect = canvas.getBoundingClientRect();
        canvas.width = Math.max(1, rect.width * dpr);
        canvas.height = Math.max(1, rect.height * dpr);
        c.setTransform(dpr, 0, 0, dpr, 0, 0);
        drawScene(c, rect.width, rect.height);
      });
    } else if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  };

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    // Cancel any pending animation frame — the hook is redrawing from scratch
    if (rafRef.current != null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    drawScene(ctx, w, h);
  }, [lanes]);

  return (
    <div
      style={{
        position: 'relative',
        width: '100%',
        height: computedHeight,
        overflow: 'hidden',
        borderRadius: 12,
        border: '1px solid var(--border-soft)',
        background: 'linear-gradient(180deg, rgba(255,255,255,0.02), rgba(255,255,255,0.01))',
      }}
    >
      <canvas
        ref={canvasRef}
        style={{ width: '100%', height: '100%', display: 'block' }}
      />
    </div>
  );
}
