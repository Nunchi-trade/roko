import { useRef, useEffect } from 'react';
import { getCssVar } from '../../lib/color';
import './NoiseBackground.css';

interface NoiseBackgroundProps {
  density?: number;
  opacity?: number;
  color?: string;
  speed?: number;
  className?: string;
}

const FPS = 10;
const FRAME_MS = 1000 / FPS;

/** Parse a hex or rgb color string into [r, g, b]. */
function parseColor(c: string): [number, number, number] {
  if (c.startsWith('#')) {
    let hex = c.slice(1);
    if (hex.length === 3) hex = hex.split('').map((ch) => ch + ch).join('');
    return [
      parseInt(hex.slice(0, 2), 16),
      parseInt(hex.slice(2, 4), 16),
      parseInt(hex.slice(4, 6), 16),
    ];
  }
  const m = c.match(/(\d+)/g);
  if (m && m.length >= 3) {
    return [Number(m[0]), Number(m[1]), Number(m[2])];
  }
  return [96, 80, 96]; // fallback ghost-like color
}

export default function NoiseBackground({
  density = 4,
  opacity = 0.15,
  color = '--text-ghost',
  speed = 1,
  className,
}: NoiseBackgroundProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef(0);
  const lastFrameRef = useRef(0);
  const reducedMotion = useRef(false);

  useEffect(() => {
    reducedMotion.current = window.matchMedia(
      '(prefers-reduced-motion: reduce)',
    ).matches;
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const resolved = getCssVar(color);
    const [cr, cg, cb] = parseColor(resolved);

    // Static noise for reduced motion
    if (reducedMotion.current) {
      renderNoise(canvas, density, opacity, cr, cg, cb);
      return;
    }

    function animate(now: number) {
      // Frame skip: effective FPS scales with speed
      const effectiveFrameMs = FRAME_MS / Math.max(speed, 0.1);
      if (now - lastFrameRef.current < effectiveFrameMs) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }
      lastFrameRef.current = now;

      const cvs = canvasRef.current;
      if (!cvs) return;
      renderNoise(cvs, density, opacity, cr, cg, cb);

      rafRef.current = requestAnimationFrame(animate);
    }

    rafRef.current = requestAnimationFrame(animate);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [density, opacity, color, speed]);

  return (
    <div className={`noise-background ${className ?? ''}`}>
      <canvas ref={canvasRef} role="presentation" aria-hidden="true" />
    </div>
  );
}

function renderNoise(
  canvas: HTMLCanvasElement,
  density: number,
  opacity: number,
  r: number,
  g: number,
  b: number,
) {
  const rect = canvas.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return;

  // Render at reduced resolution for performance
  const scale = 0.5;
  const w = Math.ceil(rect.width * scale);
  const h = Math.ceil(rect.height * scale);

  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
    // CSS will stretch to fill container
  }

  const ctx = canvas.getContext('2d');
  if (!ctx) return;

  const imageData = ctx.createImageData(w, h);
  const data = imageData.data;
  const step = Math.max(1, Math.round(density * scale));

  for (let y = 0; y < h; y += step) {
    for (let x = 0; x < w; x += step) {
      const alpha = Math.random() * opacity * 255;
      const idx = (y * w + x) * 4;
      data[idx] = r;
      data[idx + 1] = g;
      data[idx + 2] = b;
      data[idx + 3] = alpha;

      // Fill the density block
      for (let dy = 0; dy < step && y + dy < h; dy++) {
        for (let dx = 0; dx < step && x + dx < w; dx++) {
          if (dy === 0 && dx === 0) continue;
          const fi = ((y + dy) * w + (x + dx)) * 4;
          data[fi] = r;
          data[fi + 1] = g;
          data[fi + 2] = b;
          data[fi + 3] = alpha;
        }
      }
    }
  }

  ctx.putImageData(imageData, 0, 0);
}
