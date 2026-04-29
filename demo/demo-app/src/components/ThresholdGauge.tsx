import { useCallback, useEffect, useRef } from 'react';
import { getCssVar, hexToRgba } from '../lib/color';
import './Charts/Charts.css';

export interface RungThreshold {
  mean_pass_rate: number;
  ema_threshold: number;
  rung_count: number;
  consecutive_passes: number;
}

export interface AdaptiveThresholdsResponse {
  thresholds: Record<string, RungThreshold>;
}

export interface ThresholdGaugeProps {
  rung: string;
  data: RungThreshold;
  size?: number;
}

const THRESHOLD_RUNGS = ['compile', 'clippy', 'test', 'diff', 'fmt', 'custom', 'judge'] as const;

function getPalette() {
  return {
    success: getCssVar('--success'),
    warning: getCssVar('--warning'),
    rose: getCssVar('--rose-bright'),
    bone: getCssVar('--bone-bright'),
    dim: getCssVar('--text-dim'),
    ghost: getCssVar('--text-ghost'),
  };
}

function clamp01(value: number): number {
  if (Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(1, value));
}

function pickValueColor(meanPassRate: number, threshold: number, palette: ReturnType<typeof getPalette>): string {
  if (meanPassRate >= threshold) return palette.success;
  if (meanPassRate >= Math.max(0, threshold - 0.05)) return palette.warning;
  return palette.rose;
}

/**
 * Semi-circular gauge for a single gate rung.
 * Arc sweeps from 180deg (left) to 0deg (right).
 */
export default function ThresholdGauge({ rung, data, size = 120 }: ThresholdGaugeProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const cx = w / 2;
    const cy = h * 0.58;
    const radius = Math.min(w / 2 - 12, h * 0.52);
    const lineW = Math.max(7, Math.min(10, w * 0.065));

    const PALETTE = getPalette();
    const meanPassRate = clamp01(data.mean_pass_rate);
    const threshold = clamp01(data.ema_threshold);
    const amberFloor = clamp01(threshold - 0.05);
    const valueColor = pickValueColor(meanPassRate, threshold, PALETTE);

    const startAngle = Math.PI;
    const endAngle = 0;
    const totalSweep = Math.PI;
    const thresholdAngle = startAngle - threshold * totalSweep;
    const amberAngle = startAngle - amberFloor * totalSweep;
    const valueAngle = startAngle - meanPassRate * totalSweep;

    ctx.clearRect(0, 0, w, h);

    // Base track.
    ctx.beginPath();
    ctx.arc(cx, cy, radius, startAngle, endAngle, false);
    ctx.strokeStyle = 'rgba(255,255,255,0.06)';
    ctx.lineWidth = lineW;
    ctx.lineCap = 'round';
    ctx.stroke();

    // Red / amber / green zones.
    ctx.lineCap = 'butt';
    ctx.beginPath();
    ctx.arc(cx, cy, radius, startAngle, amberAngle, false);
    ctx.strokeStyle = hexToRgba(PALETTE.rose, 0.22);
    ctx.lineWidth = lineW;
    ctx.stroke();

    ctx.beginPath();
    ctx.arc(cx, cy, radius, amberAngle, thresholdAngle, false);
    ctx.strokeStyle = hexToRgba(PALETTE.warning, 0.26);
    ctx.lineWidth = lineW;
    ctx.stroke();

    ctx.beginPath();
    ctx.arc(cx, cy, radius, thresholdAngle, endAngle, false);
    ctx.strokeStyle = hexToRgba(PALETTE.success, 0.24);
    ctx.lineWidth = lineW;
    ctx.stroke();

    // Value overlay.
    ctx.beginPath();
    ctx.arc(cx, cy, radius, startAngle, valueAngle, false);
    ctx.strokeStyle = valueColor;
    ctx.lineWidth = lineW + 1;
    ctx.lineCap = 'round';
    ctx.shadowColor = hexToRgba(valueColor, 0.28);
    ctx.shadowBlur = 8;
    ctx.stroke();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';

    // Endpoint glow.
    const endX = cx + Math.cos(valueAngle) * radius;
    const endY = cy + Math.sin(valueAngle) * radius;
    ctx.beginPath();
    ctx.arc(endX, endY, lineW / 2 + 2, 0, Math.PI * 2);
    ctx.fillStyle = hexToRgba(valueColor, 0.30);
    ctx.fill();

    // Threshold marker.
    const markerInset = lineW / 2 + 2;
    const markerOuter = radius + markerInset;
    const markerInner = radius - markerInset;
    const thX1 = cx + Math.cos(thresholdAngle) * markerInner;
    const thY1 = cy + Math.sin(thresholdAngle) * markerInner;
    const thX2 = cx + Math.cos(thresholdAngle) * markerOuter;
    const thY2 = cy + Math.sin(thresholdAngle) * markerOuter;
    ctx.beginPath();
    ctx.moveTo(thX1, thY1);
    ctx.lineTo(thX2, thY2);
    ctx.strokeStyle = hexToRgba(PALETTE.bone, 0.74);
    ctx.lineWidth = 1.5;
    ctx.stroke();

    const pctText = `${(meanPassRate * 100).toFixed(1)}%`;
    const thresholdText = `thr: ${(threshold * 100).toFixed(0)}%`;

    // Text stack inside the gauge.
    ctx.fillStyle = valueColor;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.font = `700 ${Math.max(13, Math.round(size * 0.13))}px "JetBrains Mono", monospace`;
    ctx.fillText(pctText, cx, cy - radius * 0.16);

    ctx.fillStyle = PALETTE.dim;
    ctx.font = `600 ${Math.max(9, Math.round(size * 0.08))}px "JetBrains Mono", monospace`;
    ctx.fillText(rung.toUpperCase(), cx, cy + radius * 0.07);

    ctx.fillStyle = PALETTE.ghost;
    ctx.font = `600 ${Math.max(8, Math.round(size * 0.065))}px "JetBrains Mono", monospace`;
    ctx.fillText(thresholdText, cx, cy + radius * 0.23);
  }, [data.consecutive_passes, data.ema_threshold, data.mean_pass_rate, data.rung_count, rung, size]);

  useEffect(() => {
    draw();
    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);
    return () => ro.disconnect();
  }, [draw]);

  const canvasHeight = Math.round(size * 0.65);

  return (
    <div style={{
      display: 'flex',
      flexDirection: 'column',
      alignItems: 'center',
      gap: 4,
      width: size,
      flex: '0 0 auto',
    }}>
      <div className="chart-container" style={{ width: size, height: canvasHeight }}>
        <canvas
          ref={canvasRef}
          className="chart-canvas"
          style={{ width: '100%', height: '100%', display: 'block' }}
          aria-label={`${rung} adaptive threshold gauge`}
        />
      </div>
      <div style={{
        display: 'flex',
        gap: 8,
        justifyContent: 'center',
      }}>
        <span style={{
          fontFamily: 'var(--mono)',
          fontSize: '0.55rem',
          color: 'var(--text-ghost)',
        }}>
          {data.rung_count} runs
        </span>
        <span style={{
          fontFamily: 'var(--mono)',
          fontSize: '0.55rem',
          color: data.consecutive_passes >= 10 ? 'var(--success)' : 'var(--text-ghost)',
        }}>
          {data.consecutive_passes} streak
        </span>
      </div>
    </div>
  );
}

export function ThresholdGaugeRow({ thresholds }: {
  thresholds: Record<string, RungThreshold>;
}) {
  return (
    <div style={{
      display: 'flex',
      flexWrap: 'wrap',
      gap: 8,
      justifyContent: 'center',
      alignItems: 'flex-start',
    }}>
      {THRESHOLD_RUNGS.map((rung) => {
        const data = thresholds[rung];
        if (!data) return null;
        return <ThresholdGauge key={rung} rung={rung} data={data} size={110} />;
      })}
    </div>
  );
}
