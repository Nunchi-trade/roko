import { useRef, useEffect, useCallback } from 'react';
import { getCssVar } from '../../lib/color';
import { useCanvasSetup } from '../../hooks/useCanvasSetup';
import './HeartbeatLine.css';

interface HeartbeatLineProps {
  speed: 'fast' | 'medium' | 'slow';
  amplitude?: number;
  color?: string;
  height?: number;
  className?: string;
}

const CYCLE_MS: Record<HeartbeatLineProps['speed'], number> = {
  fast: 700,
  medium: 3000,
  slow: 8000,
};

const TARGET_FPS = 30;
const FRAME_MS = 1000 / TARGET_FPS;

export default function HeartbeatLine({
  speed,
  amplitude = 0.5,
  color = '--rose-bright',
  height = 40,
  className,
}: HeartbeatLineProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const timeRef = useRef(0);
  const lastFrameRef = useRef(0);
  const rafRef = useRef(0);
  const reducedMotion = useRef(false);

  useEffect(() => {
    reducedMotion.current = window.matchMedia(
      '(prefers-reduced-motion: reduce)',
    ).matches;
  }, []);

  // Use useCanvasSetup for DPR-aware sizing on resize
  const drawStatic = useCallback(
    (ctx: CanvasRenderingContext2D, w: number, h: number) => {
      const resolved = getCssVar(color);
      ctx.clearRect(0, 0, w, h);
      ctx.strokeStyle = resolved;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(0, h / 2);
      ctx.lineTo(w, h / 2);
      ctx.stroke();
    },
    [color],
  );

  useCanvasSetup(canvasRef, drawStatic, [color]);

  useEffect(() => {
    if (reducedMotion.current) return;
    const canvas = canvasRef.current;
    if (!canvas) return;

    const cycleMs = CYCLE_MS[speed];

    function animate(now: number) {
      if (now - lastFrameRef.current < FRAME_MS) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }
      lastFrameRef.current = now;
      timeRef.current = now;

      const cvs = canvasRef.current;
      if (!cvs) return;
      const rect = cvs.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }

      const ctx = cvs.getContext('2d');
      if (!ctx) return;

      const dpr = window.devicePixelRatio || 1;
      const w = rect.width;
      const h = rect.height;

      // Ensure canvas buffer matches
      if (cvs.width !== Math.round(w * dpr) || cvs.height !== Math.round(h * dpr)) {
        cvs.width = w * dpr;
        cvs.height = h * dpr;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const resolved = getCssVar(color);
      ctx.clearRect(0, 0, w, h);

      const midY = h / 2;
      const amp = amplitude * midY * 0.8;
      const phase = (now % cycleMs) / cycleMs;

      // Draw glow layer
      ctx.save();
      ctx.shadowBlur = 6;
      ctx.shadowColor = resolved;
      ctx.strokeStyle = resolved;
      ctx.lineWidth = 1.5;
      ctx.globalAlpha = 0.4;
      ctx.beginPath();
      drawWave(ctx, w, midY, amp, phase, speed);
      ctx.stroke();
      ctx.restore();

      // Draw main line
      ctx.strokeStyle = resolved;
      ctx.lineWidth = 1.5;
      ctx.globalAlpha = 1;
      ctx.beginPath();
      drawWave(ctx, w, midY, amp, phase, speed);
      ctx.stroke();

      rafRef.current = requestAnimationFrame(animate);
    }

    rafRef.current = requestAnimationFrame(animate);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [speed, amplitude, color]);

  return (
    <div
      className={`heartbeat-line ${className ?? ''}`}
      style={{ height: `${height}px` }}
    >
      <canvas ref={canvasRef} />
    </div>
  );
}

/** Draw EKG-style waveform across width. */
function drawWave(
  ctx: CanvasRenderingContext2D,
  w: number,
  midY: number,
  amp: number,
  phase: number,
  speed: HeartbeatLineProps['speed'],
) {
  const steps = Math.max(Math.floor(w / 2), 60);
  const TAU = Math.PI * 2;

  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const x = t * w;

    // Scrolling phase offset
    const p = (t + phase) * TAU;

    let y: number;
    if (speed === 'fast') {
      // Sharp EKG peaks
      const base = Math.sin(p * 3);
      const spike = Math.pow(Math.abs(Math.sin(p * 3)), 8) * Math.sign(Math.sin(p * 3));
      y = midY - amp * (base * 0.3 + spike * 0.7);
    } else if (speed === 'medium') {
      // Gentle wave
      y = midY - amp * (Math.sin(p * 2) * 0.6 + Math.sin(p * 5) * 0.15);
    } else {
      // Slow drift
      y = midY - amp * (Math.sin(p) * 0.4 + Math.sin(p * 0.5) * 0.2);
    }

    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
}
