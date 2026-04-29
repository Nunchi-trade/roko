import { useRef } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './Charts.css';

interface TrendBucket {
  start: string;
  samples: number;
  avg: number;
  p50: number;
  p95: number;
}

interface CFactorSparklineProps {
  trend: TrendBucket[];
  woolley?: Record<string, number[]>;
  height?: number;
}

function getWoolleyKeys(): { key: string; label: string; color: string }[] {
  return [
    { key: 'turn_taking_equality', label: 'Turn-Taking', color: getCssVar('--bone') },
    { key: 'social_perceptiveness', label: 'Perception', color: getCssVar('--rose') },
    { key: 'citation_reciprocity', label: 'Reciprocity', color: getCssVar('--success') },
    { key: 'delivery_rate', label: 'Delivery', color: getCssVar('--warning') },
    { key: 'hdc_diversity', label: 'HDC Diversity', color: getCssVar('--status-blocked') },
  ];
}

function drawMiniSparkline(
  ctx: CanvasRenderingContext2D,
  values: number[],
  x: number,
  y: number,
  w: number,
  h: number,
  color: string,
  label: string,
) {
  if (values.length < 2) return;

  const min = Math.max(0, Math.min(...values) - 0.01);
  const max = Math.min(1, Math.max(...values) + 0.01);
  const range = max - min || 1;
  const lineTop = y + 12;
  const lineH = Math.max(8, h - 14);

  ctx.save();
  ctx.fillStyle = getCssVar('--text-ghost');
  ctx.font = '9px "JetBrains Mono", monospace';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'alphabetic';
  ctx.fillText(label, x, y + 8);

  ctx.textAlign = 'right';
  ctx.fillStyle = color;
  ctx.fillText(values[values.length - 1].toFixed(3), x + w, y + 8);

  ctx.beginPath();
  for (let i = 0; i < values.length; i += 1) {
    const px = x + (i / Math.max(values.length - 1, 1)) * w;
    const py = lineTop + lineH - ((values[i] - min) / range) * lineH;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.lineTo(x + w, lineTop + lineH);
  ctx.lineTo(x, lineTop + lineH);
  ctx.closePath();
  ctx.fillStyle = hexToRgba(color, 0.08);
  ctx.fill();

  ctx.beginPath();
  for (let i = 0; i < values.length; i += 1) {
    const px = x + (i / Math.max(values.length - 1, 1)) * w;
    const py = lineTop + lineH - ((values[i] - min) / range) * lineH;
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.lineJoin = 'round';
  ctx.stroke();

  const lastX = x + w;
  const lastY = lineTop + lineH - ((values[values.length - 1] - min) / range) * lineH;
  ctx.beginPath();
  ctx.arc(lastX, lastY, 1.8, 0, Math.PI * 2);
  ctx.fillStyle = color;
  ctx.fill();

  ctx.restore();
}

/** C-Factor trend chart with a main line and Woolley mini-sparklines. */
export default function CFactorSparkline({ trend, woolley, height = 320 }: CFactorSparklineProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useCanvasSetup(canvasRef, (ctx, w, h) => {
    const pad = { top: 28, right: 16, bottom: 12, left: 52 };
    const plotW = w - pad.left - pad.right;
    const availableH = h - pad.top - pad.bottom;

    ctx.clearRect(0, 0, w, h);

    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '11px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText('C-FACTOR TREND (24h)', pad.left, 16);

    if (trend.length === 0) {
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.fillText('No trend data available', pad.left, pad.top + 20);
      return;
    }

    const avgValues = trend.map((bucket) => bucket.avg);
    const p95Values = trend.map((bucket) => bucket.p95);
    const allValues = [...avgValues, ...p95Values];
    const minVal = Math.max(0, Math.min(...allValues) - 0.01);
    const maxVal = Math.min(1, Math.max(...allValues) + 0.01);
    const range = maxVal - minVal || 1;
    const mainH = Math.max(104, Math.min(136, Math.floor(availableH * 0.43)));
    const latest = trend[trend.length - 1];

    ctx.fillStyle = getCssVar('--text-ghost');
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.textAlign = 'right';
    ctx.fillText(`avg ${latest.avg.toFixed(3)} · p95 ${latest.p95.toFixed(3)} · n ${latest.samples}`, pad.left + plotW, 16);

    ctx.strokeStyle = 'rgba(255,255,255,0.05)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 3; i += 1) {
      const yy = pad.top + mainH * (1 - i / 3);
      ctx.beginPath();
      ctx.moveTo(pad.left, yy);
      ctx.lineTo(pad.left + plotW, yy);
      ctx.stroke();

      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.fillText((minVal + (range * i) / 3).toFixed(3), pad.left - 6, yy + 3);
    }

    ctx.beginPath();
    for (let i = 0; i < p95Values.length; i += 1) {
      const x = pad.left + (i / Math.max(p95Values.length - 1, 1)) * plotW;
      const y = pad.top + mainH * (1 - (p95Values[i] - minVal) / range);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    for (let i = avgValues.length - 1; i >= 0; i -= 1) {
      const x = pad.left + (i / Math.max(avgValues.length - 1, 1)) * plotW;
      const y = pad.top + mainH * (1 - (avgValues[i] - minVal) / range);
      ctx.lineTo(x, y);
    }
    ctx.closePath();
    ctx.fillStyle = hexToRgba(getCssVar('--rose-glow'), 0.08);
    ctx.fill();

    ctx.beginPath();
    ctx.strokeStyle = hexToRgba(getCssVar('--rose-glow'), 0.35);
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    for (let i = 0; i < p95Values.length; i += 1) {
      const x = pad.left + (i / Math.max(p95Values.length - 1, 1)) * plotW;
      const y = pad.top + mainH * (1 - (p95Values[i] - minVal) / range);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    ctx.setLineDash([]);

    ctx.beginPath();
    ctx.strokeStyle = getCssVar('--rose-glow');
    ctx.lineWidth = 2;
    ctx.lineJoin = 'round';
    for (let i = 0; i < avgValues.length; i += 1) {
      const x = pad.left + (i / Math.max(avgValues.length - 1, 1)) * plotW;
      const y = pad.top + mainH * (1 - (avgValues[i] - minVal) / range);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    const lastX = pad.left + plotW;
    const lastY = pad.top + mainH * (1 - (avgValues[avgValues.length - 1] - minVal) / range);
    ctx.beginPath();
    ctx.arc(lastX, lastY, 3, 0, Math.PI * 2);
    ctx.fillStyle = getCssVar('--rose-glow');
    ctx.shadowColor = hexToRgba(getCssVar('--rose-glow'), 0.45);
    ctx.shadowBlur = 8;
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';

    const woolleyKeys = getWoolleyKeys().filter((entry) => Array.isArray(woolley?.[entry.key]) && (woolley?.[entry.key]?.length ?? 0) > 1);
    if (woolleyKeys.length === 0) return;

    const sparkTop = pad.top + mainH + 18;
    const sparkAvailable = h - sparkTop - pad.bottom;
    const sparkGap = 6;
    const rowH = (sparkAvailable - sparkGap * (woolleyKeys.length - 1)) / woolleyKeys.length;

    woolleyKeys.forEach((entry, index) => {
      const values = woolley?.[entry.key] ?? [];
      const rowY = sparkTop + index * (rowH + sparkGap);
      drawMiniSparkline(ctx, values, pad.left, rowY, plotW, rowH, entry.color, entry.label);
    });
  }, [trend, woolley]);

  return (
    <div className="chart-container" style={{ height }}>
      <canvas ref={canvasRef} className="chart-canvas" />
    </div>
  );
}
