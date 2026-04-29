/**
 * TokenFlowChart — horizontal stacked bars showing token in/out per task.
 * Extracted from BenchRunDetail.tsx (inline canvas component).
 */
import { useRef, useEffect, useCallback } from 'react';
import { getCssVar } from '../../lib/color';
import type { BenchTaskResult } from '../../lib/bench-types';
import './TokenFlowChart.css';

/* ── Props ── */

export interface TokenFlowChartProps {
  results: BenchTaskResult[];
  height?: number;
}

/* ── Component ── */

export function TokenFlowChart({ results, height = 280 }: TokenFlowChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || results.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const pad = { top: 8, right: 16, bottom: 24, left: 100 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;
    if (plotW <= 0 || plotH <= 0) return;

    ctx.clearRect(0, 0, w, h);

    const maxTokens = Math.max(...results.map((r) => r.tokens_in + r.tokens_out), 1);
    const rowGap = 4;
    const barHeight = Math.max(4, Math.min(22, plotH / results.length - rowGap));
    const inColor = getCssVar('--success');
    const outColor = getCssVar('--bone');

    // Grid lines
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    for (let i = 0; i <= 4; i++) {
      const x = pad.left + (plotW * i) / 4;
      ctx.beginPath();
      ctx.moveTo(x, pad.top);
      ctx.lineTo(x, pad.top + plotH);
      ctx.stroke();
    }

    results.forEach((r, index) => {
      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const inW = (r.tokens_in / maxTokens) * plotW;
      const outW = (r.tokens_out / maxTokens) * plotW;

      // Background
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.fillRect(pad.left, y, plotW, barHeight);

      // Input bar
      if (inW > 0) {
        ctx.fillStyle = inColor;
        ctx.fillRect(pad.left, y, inW, barHeight);
      }

      // Output bar
      if (outW > 0) {
        ctx.fillStyle = outColor;
        ctx.fillRect(pad.left + inW, y, outW, barHeight);
      }

      // Label
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = getCssVar('--text-dim');
      const nameText = r.task_name.length > 12 ? r.task_name.slice(0, 11) + '\u2026' : r.task_name;
      ctx.fillText(nameText, pad.left - 8, centerY);

      // Token count
      const total = r.tokens_in + r.tokens_out;
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = getCssVar('--text-soft');
      const barEnd = pad.left + inW + outW;
      ctx.fillText(total.toLocaleString(), Math.min(barEnd + 6, pad.left + plotW - 40), centerY);
    });

    // Legend
    const legendY = h - 8;
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const legendItems = [
      { color: inColor, label: 'Tokens in' },
      { color: outColor, label: 'Tokens out' },
    ];
    let legendX = pad.left;
    legendItems.forEach((item) => {
      ctx.fillStyle = item.color;
      ctx.fillRect(legendX, legendY - 5, 10, 10);
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.fillText(item.label, legendX + 14, legendY);
      legendX += ctx.measureText(item.label).width + 34;
    });
  }, [results]);

  useEffect(() => {
    draw();
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === 'undefined') return undefined;

    const ro = new ResizeObserver(draw);
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [draw]);

  if (results.length === 0) {
    return (
      <div className="token-flow-chart token-flow-chart--empty" style={{ height }}>
        <p className="bench-empty-text">No token data.</p>
      </div>
    );
  }

  return (
    <div className="token-flow-chart" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
