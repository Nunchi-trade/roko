import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

interface HeatmapChartProps {
  rows: string[];
  columns: string[];
  values: (boolean | null)[][];
  height?: number;
}

/** Grid heatmap for gate pass rates using Canvas 2D. */
export default function HeatmapChart({ rows, columns, values, height = 280 }: HeatmapChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    if (rows.length === 0 || columns.length === 0) return;

    const pad = { top: 32, right: 16, bottom: 8, left: 80 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;
    const cellW = plotW / columns.length;
    const cellH = Math.min(plotH / rows.length, 28);

    ctx.clearRect(0, 0, w, h);

    // Column headers
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.fillStyle = getCssVar('--text-dim');
    ctx.textAlign = 'center';
    ctx.textBaseline = 'bottom';
    for (let c = 0; c < columns.length; c++) {
      const x = pad.left + c * cellW + cellW / 2;
      const label = columns[c].length > 8 ? columns[c].slice(0, 7) + '\u2026' : columns[c];
      ctx.fillText(label, x, pad.top - 4);
    }

    // Rows
    for (let r = 0; r < rows.length; r++) {
      const y = pad.top + r * cellH;

      // Row label
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      const rowLabel = rows[r].length > 10 ? rows[r].slice(0, 9) + '\u2026' : rows[r];
      ctx.fillText(rowLabel, pad.left - 8, y + cellH / 2);

      // Cells
      const rowValues = values[r] ?? [];
      for (let c = 0; c < columns.length; c++) {
        const x = pad.left + c * cellW;
        const val = c < rowValues.length ? rowValues[c] : null;

        // Cell fill
        if (val === true) {
          ctx.fillStyle = hexToRgba(getCssVar('--success'), 0.4);
        } else if (val === false) {
          ctx.fillStyle = hexToRgba(getCssVar('--rose-dim'), 0.4);
        } else {
          ctx.fillStyle = 'rgba(255,255,255,0.03)'; // null/missing
        }
        ctx.fillRect(x + 1, y + 1, cellW - 2, cellH - 2);

        // Cell border
        ctx.strokeStyle = 'rgba(255,255,255,0.06)';
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 1, y + 1, cellW - 2, cellH - 2);
      }
    }
  }, [rows, columns, values]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Heatmap data visualization" />
    </div>
  );
}
