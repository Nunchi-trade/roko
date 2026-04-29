import { useRef, useEffect, useCallback } from 'react';

export interface TokenVelocityPoint {
  taskId: string;
  tokensPerSecond: number;
}

export interface TokenVelocitySparklineProps {
  points: TokenVelocityPoint[];
  height?: number;
}

function hexToRgba(hex: string, alpha: number): string {
  const normalized = hex.trim().replace(/^#/, '');
  if (!/^[0-9a-fA-F]{6}$/.test(normalized)) return hex;
  const r = Number.parseInt(normalized.slice(0, 2), 16);
  const g = Number.parseInt(normalized.slice(2, 4), 16);
  const b = Number.parseInt(normalized.slice(4, 6), 16);
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Canvas-based sparkline showing token velocity per task. */
export default function TokenVelocitySparkline({ points, height = 120 }: TokenVelocitySparklineProps) {
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
    const lineColor = '#8A9C86';
    const titleColor = '#8a7a88';
    const muted = '#6a5a68';
    const pad = { top: 36, right: 16, bottom: 16, left: 16 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;

    ctx.clearRect(0, 0, w, h);

    // Title
    ctx.fillStyle = titleColor;
    ctx.font = '11px "General Sans", sans-serif';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText('TOKEN VELOCITY', pad.left, 16);

    if (points.length === 0) {
      ctx.fillStyle = muted;
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.fillText('No velocity data', pad.left, pad.top + 20);
      return;
    }

    const values = points.map((p) => p.tokensPerSecond);
    const maxVal = Math.max(...values, 1);
    const avg = values.reduce((s, v) => s + v, 0) / values.length;
    const current = values[values.length - 1];

    // Current value
    ctx.fillStyle = lineColor;
    ctx.font = 'bold 14px "JetBrains Mono", monospace';
    ctx.textAlign = 'right';
    ctx.fillText(`${current.toFixed(1)} tok/s`, w - pad.right, 16);

    // Average line
    const avgY = pad.top + plotH - (avg / maxVal) * plotH;
    ctx.strokeStyle = hexToRgba(lineColor, 0.25);
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 3]);
    ctx.beginPath();
    ctx.moveTo(pad.left, avgY);
    ctx.lineTo(pad.left + plotW, avgY);
    ctx.stroke();
    ctx.setLineDash([]);

    // Avg label
    ctx.fillStyle = muted;
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.textAlign = 'right';
    ctx.fillText(`avg ${avg.toFixed(1)}`, w - pad.right, avgY - 4);

    // Gradient fill under line
    const gradient = ctx.createLinearGradient(0, pad.top, 0, pad.top + plotH);
    gradient.addColorStop(0, hexToRgba(lineColor, 0.15));
    gradient.addColorStop(1, hexToRgba(lineColor, 0.02));

    // Build path
    const getX = (i: number) => pad.left + (i / Math.max(values.length - 1, 1)) * plotW;
    const getY = (v: number) => pad.top + plotH - (v / maxVal) * plotH;

    // Fill area
    ctx.beginPath();
    for (let i = 0; i < values.length; i++) {
      const x = getX(i);
      const y = getY(values[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(getX(values.length - 1), pad.top + plotH);
    ctx.lineTo(getX(0), pad.top + plotH);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Stroke line
    ctx.beginPath();
    for (let i = 0; i < values.length; i++) {
      const x = getX(i);
      const y = getY(values[i]);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = lineColor;
    ctx.lineWidth = 2;
    ctx.lineJoin = 'round';
    ctx.stroke();

    // Dot on last point
    const lastX = getX(values.length - 1);
    const lastY = getY(current);
    ctx.beginPath();
    ctx.arc(lastX, lastY, 3, 0, Math.PI * 2);
    ctx.fillStyle = lineColor;
    ctx.shadowColor = hexToRgba(lineColor, 0.45);
    ctx.shadowBlur = 8;
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.shadowColor = 'transparent';
  }, [points]);

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
