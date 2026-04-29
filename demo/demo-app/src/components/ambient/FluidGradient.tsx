import { useRef, useEffect } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import './FluidGradient.css';

interface FluidGradientProps {
  colors?: string[];
  speed?: number;
  opacity?: number;
  className?: string;
}

const FPS = 15;
const FRAME_MS = 1000 / FPS;
const NUM_BLOBS = 5;

interface Blob {
  /** Lissajous frequency ratios */
  fx: number;
  fy: number;
  /** Phase offsets */
  px: number;
  py: number;
  /** Radius as fraction of min(w, h) */
  radius: number;
  /** Index into resolved colors */
  colorIdx: number;
}

function createBlobs(numColors: number): Blob[] {
  const blobs: Blob[] = [];
  for (let i = 0; i < NUM_BLOBS; i++) {
    blobs.push({
      fx: 0.3 + Math.random() * 0.7,
      fy: 0.2 + Math.random() * 0.8,
      px: Math.random() * Math.PI * 2,
      py: Math.random() * Math.PI * 2,
      radius: 0.25 + Math.random() * 0.3,
      colorIdx: i % numColors,
    });
  }
  return blobs;
}

export default function FluidGradient({
  colors = ['--rose-dim', '--dream-deep', '--bg-void'],
  speed = 1,
  opacity = 0.3,
  className,
}: FluidGradientProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef(0);
  const lastFrameRef = useRef(0);
  const blobsRef = useRef<Blob[]>(createBlobs(colors.length));
  const reducedMotion = useRef(false);

  useEffect(() => {
    reducedMotion.current = window.matchMedia(
      '(prefers-reduced-motion: reduce)',
    ).matches;
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const resolvedColors = colors.map((c) => getCssVar(c));

    // Render at 1/4 resolution for performance; CSS stretches to fill
    function getSize() {
      const rect = canvas!.getBoundingClientRect();
      return {
        cssW: rect.width,
        cssH: rect.height,
        w: Math.ceil(rect.width / 4),
        h: Math.ceil(rect.height / 4),
      };
    }

    function drawFrame(time: number) {
      const cvs = canvasRef.current;
      if (!cvs) return;
      const { w, h } = getSize();
      if (w === 0 || h === 0) return;

      if (cvs.width !== w || cvs.height !== h) {
        cvs.width = w;
        cvs.height = h;
      }

      const ctx = cvs.getContext('2d');
      if (!ctx) return;

      ctx.clearRect(0, 0, w, h);
      ctx.globalAlpha = opacity;

      const minDim = Math.min(w, h);
      const t = time * 0.001 * speed;

      for (const blob of blobsRef.current) {
        // Lissajous position
        const cx = w * 0.5 + w * 0.35 * Math.sin(t * blob.fx + blob.px);
        const cy = h * 0.5 + h * 0.35 * Math.cos(t * blob.fy + blob.py);
        const r = blob.radius * minDim;

        const blobColor = resolvedColors[blob.colorIdx % resolvedColors.length];
        const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
        grad.addColorStop(0, hexToRgba(blobColor, 0.6));
        grad.addColorStop(0.5, hexToRgba(blobColor, 0.2));
        grad.addColorStop(1, hexToRgba(blobColor, 0));

        ctx.fillStyle = grad;
        ctx.fillRect(cx - r, cy - r, r * 2, r * 2);
      }

      ctx.globalAlpha = 1;
    }

    // Static render for reduced motion
    if (reducedMotion.current) {
      drawFrame(0);
      return;
    }

    function animate(now: number) {
      if (now - lastFrameRef.current < FRAME_MS) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }
      lastFrameRef.current = now;
      drawFrame(now);
      rafRef.current = requestAnimationFrame(animate);
    }

    rafRef.current = requestAnimationFrame(animate);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [colors, speed, opacity]);

  return (
    <div className={`fluid-gradient ${className ?? ''}`}>
      <canvas ref={canvasRef} />
    </div>
  );
}
