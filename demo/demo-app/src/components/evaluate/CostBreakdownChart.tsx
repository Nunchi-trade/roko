/**
 * CostBreakdownChart — horizontal stacked-bar cost breakdown by task/model/difficulty.
 * Extracted from BenchRunDetail.tsx (inline canvas component).
 */
import { useState, useMemo, useRef, useEffect, useCallback } from 'react';
import { getCssVar } from '../../lib/color';
import type { BenchTaskResult } from '../../lib/bench-types';
import './CostBreakdownChart.css';

/* ── Types ── */

type CostGroupBy = 'task' | 'model' | 'difficulty';

type CostSegment = {
  label: string;
  inputCost: number;
  outputCost: number;
};

/* ── Helpers ── */

function tokenCost(model: string, tokensIn: number, tokensOut: number): [number, number] {
  const rates: Record<string, [number, number]> = {
    'claude-haiku':   [0.00025, 0.00125],
    'claude-sonnet':  [0.003, 0.015],
    'claude-opus':    [0.015, 0.075],
    'gpt-5.4-mini':   [0.00015, 0.0006],
    'gpt-5.4':        [0.005, 0.015],
    'gpt-5.3-codex':  [0.003, 0.012],
    'o3-mini':         [0.0011, 0.0044],
    'gemini':          [0.00125, 0.01],
  };
  const key = Object.keys(rates).find((k) => model.startsWith(k)) ?? 'claude-sonnet';
  const [inRate, outRate] = rates[key];
  return [(Math.max(tokensIn, 0) * inRate) / 1000, (Math.max(tokensOut, 0) * outRate) / 1000];
}

function fitLabel(ctx: CanvasRenderingContext2D, text: string, maxWidth: number) {
  if (maxWidth <= 0) return '';
  if (ctx.measureText(text).width <= maxWidth) return text;

  let value = text;
  while (value.length > 1 && ctx.measureText(`${value}...`).width > maxWidth) {
    value = value.slice(0, -1);
  }
  return `${value}...`;
}

function truncateLabel(text: string, maxLength: number) {
  if (text.length <= maxLength) return text;
  if (maxLength <= 3) return text.slice(0, maxLength);
  return `${text.slice(0, maxLength - 3)}...`;
}

function buildCostSegments(results: BenchTaskResult[], groupBy: CostGroupBy): CostSegment[] {
  if (results.length === 0) return [];

  const addCost = (segment: CostSegment, result: BenchTaskResult) => {
    const [inputCost, outputCost] = tokenCost(result.model, result.tokens_in, result.tokens_out);
    segment.inputCost += inputCost;
    segment.outputCost += outputCost;
  };

  if (groupBy === 'task') {
    return results.map((result) => {
      const [inputCost, outputCost] = tokenCost(result.model, result.tokens_in, result.tokens_out);
      return {
        label: truncateLabel(result.task_name, 25),
        inputCost,
        outputCost,
      };
    });
  }

  if (groupBy === 'model') {
    const grouped = new Map<string, CostSegment>();
    for (const result of results) {
      const label = result.model || 'unknown';
      const segment = grouped.get(label) ?? { label, inputCost: 0, outputCost: 0 };
      addCost(segment, result);
      grouped.set(label, segment);
    }
    return Array.from(grouped.values());
  }

  const ordered = [...results].sort(
    (a, b) => a.duration_ms - b.duration_ms || a.task_name.localeCompare(b.task_name),
  );
  const buckets = new Map<'Easy' | 'Medium' | 'Hard', CostSegment>([
    ['Easy', { label: 'Easy', inputCost: 0, outputCost: 0 }],
    ['Medium', { label: 'Medium', inputCost: 0, outputCost: 0 }],
    ['Hard', { label: 'Hard', inputCost: 0, outputCost: 0 }],
  ]);
  const easyCut = Math.ceil(ordered.length / 3);
  const mediumCut = Math.ceil((ordered.length * 2) / 3);

  ordered.forEach((result, index) => {
    const label = index < easyCut ? 'Easy' : index < mediumCut ? 'Medium' : 'Hard';
    const segment = buckets.get(label);
    if (segment) addCost(segment, result);
  });

  return (['Easy', 'Medium', 'Hard'] as const)
    .map((label) => buckets.get(label))
    .filter((segment): segment is CostSegment => Boolean(segment));
}

/* ── Props ── */

export interface CostBreakdownChartProps {
  results: BenchTaskResult[];
  height?: number;
}

/* ── Component ── */

export function CostBreakdownChart({ results, height = 280 }: CostBreakdownChartProps) {
  const [groupBy, setGroupBy] = useState<CostGroupBy>('task');
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const segments = useMemo(() => buildCostSegments(results, groupBy), [results, groupBy]);
  const [animProgress, setAnimProgress] = useState(0);

  // Animate bars from 0 to full on mount / groupBy change
  useEffect(() => {
    setAnimProgress(0);
    const start = performance.now();
    const duration = 700;
    let raf = 0;
    const tick = (now: number) => {
      const t = Math.min((now - start) / duration, 1);
      const eased = 1 - Math.pow(1 - t, 3);
      setAnimProgress(eased);
      if (t < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [segments]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || segments.length === 0) return;

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
    const pad = { top: 12, right: 16, bottom: 24, left: 120 };
    const legendSpace = 20;
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom - legendSpace;
    if (plotW <= 0 || plotH <= 0) return;

    ctx.clearRect(0, 0, w, h);

    const maxCost = Math.max(...segments.map((segment) => segment.inputCost + segment.outputCost), 0.001);
    const rowGap = 4;
    const barHeight = Math.max(4, Math.min(24, plotH / segments.length - rowGap));
    const bone = getCssVar('--bone');
    const roseDim = getCssVar('--rose-bright');
    const labelColor = getCssVar('--text-soft');
    const mutedColor = getCssVar('--text-dim');
    const costColor = getCssVar('--text-strong');

    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    for (let i = 0; i <= 4; i += 1) {
      const x = pad.left + (plotW * i) / 4;
      ctx.beginPath();
      ctx.moveTo(x, pad.top - 2);
      ctx.lineTo(x, pad.top + plotH);
      ctx.stroke();
    }

    segments.forEach((segment, index) => {
      // Per-bar stagger: each bar starts slightly later
      const barDelay = index * 0.04;
      const barProgress = Math.max(0, Math.min(1, (animProgress - barDelay) / (1 - barDelay)));

      const y = pad.top + index * (barHeight + rowGap);
      const centerY = y + barHeight / 2;
      const total = segment.inputCost + segment.outputCost;
      const inputWidth = (segment.inputCost / maxCost) * plotW * barProgress;
      const outputWidth = (segment.outputCost / maxCost) * plotW * barProgress;
      const barEnd = pad.left + inputWidth + outputWidth;

      ctx.fillStyle = 'rgba(255,255,255,0.04)';
      ctx.fillRect(pad.left, y, plotW, barHeight);

      if (inputWidth > 0) {
        ctx.fillStyle = bone;
        ctx.fillRect(pad.left, y, inputWidth, barHeight);
      }

      if (outputWidth > 0) {
        ctx.fillStyle = roseDim;
        ctx.fillRect(pad.left + inputWidth, y, outputWidth, barHeight);
      }

      ctx.font = '11px "General Sans", sans-serif';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillStyle = labelColor;
      ctx.globalAlpha = barProgress;
      ctx.fillText(fitLabel(ctx, segment.label, pad.left - 12), pad.left - 10, centerY);

      const costText = `$${(total * barProgress).toFixed(3)}`;
      ctx.font = '11px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillStyle = costColor;
      const costTextWidth = ctx.measureText(costText).width;
      const costX = Math.max(pad.left + 4, Math.min(barEnd + 8, pad.left + plotW - costTextWidth));
      ctx.fillText(costText, costX, centerY);
      ctx.globalAlpha = 1;
    });

    const legendY = h - 9;
    ctx.font = '10px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const legendItems = [
      { color: bone, label: 'Input tokens' },
      { color: roseDim, label: 'Output tokens' },
    ];
    let legendX = pad.left;
    legendItems.forEach((item) => {
      ctx.fillStyle = item.color;
      ctx.fillRect(legendX, legendY - 5, 10, 10);
      ctx.fillStyle = mutedColor;
      ctx.fillText(item.label, legendX + 14, legendY);
      legendX += ctx.measureText(item.label).width + 34;
    });
  }, [segments, animProgress]);

  useEffect(() => {
    draw();
    const canvas = canvasRef.current;
    if (!canvas || typeof ResizeObserver === 'undefined') return undefined;

    const ro = new ResizeObserver(() => {
      draw();
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div className="cost-breakdown-chart" style={{ height }}>
      <div className="cost-breakdown-chart__controls">
        {(['task', 'model', 'difficulty'] as const).map((mode) => (
          <label key={mode} className="gate-toggle">
            <input
              type="radio"
              name="costGroup"
              checked={groupBy === mode}
              onChange={() => setGroupBy(mode)}
            />
            <span>{mode.charAt(0).toUpperCase() + mode.slice(1)}</span>
          </label>
        ))}
      </div>
      <div className="cost-breakdown-chart__canvas-wrap">
        {segments.length === 0 ? (
          <div className="cost-breakdown-chart__empty">
            <p className="bench-empty-text">No cost data available.</p>
          </div>
        ) : (
          <canvas ref={canvasRef} className="chart-canvas" role="img" aria-label="Task cost breakdown chart" />
        )}
      </div>
    </div>
  );
}
