import { useRef, useEffect, useCallback } from 'react';

interface OscilloscopeProps {
  data: number[];
  color?: string;
  height?: number;
  className?: string;
}

/** Resolve a CSS variable like 'var(--rose-bright)' to an actual color string. */
function resolveColor(raw: string, el: HTMLElement): string {
  const m = raw.match(/^var\(\s*(--[^)]+)\s*\)$/);
  if (!m) return raw;
  return getComputedStyle(el).getPropertyValue(m[1]).trim() || raw;
}

export default function Oscilloscope({
  data,
  color = 'var(--rose-bright)',
  height = 120,
  className,
}: OscilloscopeProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef(0);
  const phaseRef = useRef(0);
  const resolvedColorRef = useRef('#cc90a8');

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    if (w === 0 || h === 0) {
      rafRef.current = requestAnimationFrame(draw);
      return;
    }

    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    // Phosphor decay: semi-transparent black overlay instead of clearRect
    ctx.fillStyle = 'rgba(0,0,0,0.06)';
    ctx.fillRect(0, 0, w, h);

    // Resolve CSS color on first frame
    resolvedColorRef.current = resolveColor(color, canvas);
    const c = resolvedColorRef.current;

    const phase = phaseRef.current;
    phaseRef.current += 0.03;

    const hasData = data.length > 0;
    const midY = h * 0.5;
    const amplitude = h * 0.35;

    // Build path points
    const points: [number, number][] = [];
    const step = 2;
    for (let x = 0; x <= w; x += step) {
      const t = x / w;
      // Carrier wave scrolling left
      const carrier = Math.sin(t * Math.PI * 4 - phase * 2);

      let envelope: number;
      if (hasData) {
        // Map x position to data index
        const di = t * (data.length - 1);
        const lo = Math.floor(di);
        const hi = Math.min(lo + 1, data.length - 1);
        const frac = di - lo;
        const val = data[lo] + (data[hi] - data[lo]) * frac;
        // Normalize: assume data is bps values, scale to 0..1 range
        const maxVal = Math.max(...data, 1);
        envelope = (val / maxVal) * 0.6 + 0.15;
      } else {
        // Subtle breathing when idle
        envelope = 0.1 + 0.05 * Math.sin(phase * 0.5);
      }

      const y = midY - carrier * amplitude * envelope;
      points.push([x, y]);
    }

    // Gradient fill below line
    const grad = ctx.createLinearGradient(0, midY - amplitude, 0, h);
    grad.addColorStop(0, c + '1a'); // ~10% opacity
    grad.addColorStop(1, 'transparent');
    ctx.beginPath();
    ctx.moveTo(points[0][0], points[0][1]);
    for (let i = 1; i < points.length; i++) ctx.lineTo(points[i][0], points[i][1]);
    ctx.lineTo(w, h);
    ctx.lineTo(0, h);
    ctx.closePath();
    ctx.fillStyle = grad;
    ctx.fill();

    // Echo shadow line (1px, 30% opacity, offset down 2px)
    ctx.beginPath();
    ctx.moveTo(points[0][0], points[0][1] + 2);
    for (let i = 1; i < points.length; i++) ctx.lineTo(points[i][0], points[i][1] + 2);
    ctx.strokeStyle = c + '4d'; // ~30%
    ctx.lineWidth = 1;
    ctx.stroke();

    // Main signal line (2px, full color)
    ctx.beginPath();
    ctx.moveTo(points[0][0], points[0][1]);
    for (let i = 1; i < points.length; i++) ctx.lineTo(points[i][0], points[i][1]);
    ctx.strokeStyle = c;
    ctx.lineWidth = 2;
    ctx.stroke();

    rafRef.current = requestAnimationFrame(draw);
  }, [data, color]);

  useEffect(() => {
    rafRef.current = requestAnimationFrame(draw);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [draw]);

  return (
    <canvas
      ref={canvasRef}
      className={className}
      style={{ width: '100%', height, display: 'block' }}
    />
  );
}
