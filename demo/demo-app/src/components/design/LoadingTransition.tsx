import { useRef, useEffect, useCallback, type ReactNode } from 'react';
import { getCssVar } from '../../lib/color';
import { Skeleton } from './Skeleton';
import './LoadingTransition.css';

type DitherPattern = 'random' | 'scanline' | 'radial';

interface LoadingTransitionProps {
  loading: boolean;
  children: ReactNode;
  pattern?: DitherPattern;
  duration?: number;
  grainSize?: number;
  color?: string;
  skeleton?: ReactNode;
  className?: string;
}

/** Parse a CSS color string to [r, g, b, a]. Falls back to opaque black. */
function parseColor(raw: string): [number, number, number, number] {
  const hex = raw.replace(/\s/g, '');
  if (hex.startsWith('#')) {
    let h = hex.slice(1);
    if (h.length === 3) h = h.split('').map(c => c + c).join('');
    const n = parseInt(h, 16);
    return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff, 255];
  }
  const rgba = raw.match(/rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)(?:\s*,\s*([\d.]+))?\)/);
  if (rgba) {
    return [
      parseInt(rgba[1]),
      parseInt(rgba[2]),
      parseInt(rgba[3]),
      rgba[4] !== undefined ? Math.round(parseFloat(rgba[4]) * 255) : 255,
    ];
  }
  return [8, 8, 12, 255]; // --bg-void fallback
}

/** Fisher-Yates shuffle (in-place). */
function shuffle(arr: Uint32Array): Uint32Array {
  for (let i = arr.length - 1; i > 0; i--) {
    const j = (Math.random() * (i + 1)) | 0;
    const tmp = arr[i];
    arr[i] = arr[j];
    arr[j] = tmp;
  }
  return arr;
}

/** Build ordered indices for a given pattern. */
function buildOrder(
  cols: number,
  rows: number,
  pattern: DitherPattern,
): Uint32Array {
  const total = cols * rows;
  const order = new Uint32Array(total);

  if (pattern === 'random') {
    for (let i = 0; i < total; i++) order[i] = i;
    shuffle(order);
  } else if (pattern === 'scanline') {
    for (let i = 0; i < total; i++) order[i] = i;
  } else {
    // radial: sort by distance from center
    const cx = cols / 2;
    const cy = rows / 2;
    const indices = Array.from({ length: total }, (_, i) => i);
    indices.sort((a, b) => {
      const ax = a % cols, ay = (a / cols) | 0;
      const bx = b % cols, by = (b / cols) | 0;
      const da = (ax - cx) ** 2 + (ay - cy) ** 2;
      const db = (bx - cx) ** 2 + (by - cy) ** 2;
      return da - db;
    });
    for (let i = 0; i < total; i++) order[i] = indices[i];
  }
  return order;
}

export default function LoadingTransition({
  loading,
  children,
  pattern = 'random',
  duration = 600,
  grainSize = 4,
  color = '--bg-void',
  skeleton,
  className,
}: LoadingTransitionProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rafRef = useRef<number>(0);
  const prevLoading = useRef(loading);

  const runReveal = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;

    // Respect prefers-reduced-motion
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    const w = container.offsetWidth;
    const h = container.offsetHeight;
    if (w === 0 || h === 0) return;

    const canvas = document.createElement('canvas');
    canvas.width = w;
    canvas.height = h;
    canvas.style.width = '100%';
    canvas.style.height = '100%';
    container.appendChild(canvas);
    canvasRef.current = canvas;

    const ctx = canvas.getContext('2d');
    if (!ctx) { canvas.remove(); return; }

    // Fill with dither color
    const rawColor = getCssVar(color) || '#08080c';
    const [r, g, b, a] = parseColor(rawColor);
    ctx.fillStyle = `rgba(${r},${g},${b},${a / 255})`;
    ctx.fillRect(0, 0, w, h);

    const cols = Math.ceil(w / grainSize);
    const rows = Math.ceil(h / grainSize);
    const order = buildOrder(cols, rows, pattern);
    const total = order.length;

    const startTime = performance.now();

    function frame(now: number) {
      const elapsed = now - startTime;
      const progress = Math.min(elapsed / duration, 1);
      const target = (progress * total) | 0;

      // Clear cells in batch
      const batchStart = Math.max(0, target - Math.ceil(total / (duration / 16)));
      for (let i = batchStart; i < target && i < total; i++) {
        const idx = order[i];
        const cx = (idx % cols) * grainSize;
        const cy = ((idx / cols) | 0) * grainSize;
        ctx!.clearRect(cx, cy, grainSize, grainSize);
      }

      if (progress < 1) {
        rafRef.current = requestAnimationFrame(frame);
      } else {
        canvas.remove();
        canvasRef.current = null;
      }
    }

    rafRef.current = requestAnimationFrame(frame);
  }, [color, duration, grainSize, pattern]);

  useEffect(() => {
    const wasLoading = prevLoading.current;
    prevLoading.current = loading;

    // Trigger on true -> false transition
    if (wasLoading && !loading) {
      runReveal();
    }

    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      if (canvasRef.current) {
        canvasRef.current.remove();
        canvasRef.current = null;
      }
    };
  }, [loading, runReveal]);

  return (
    <div ref={containerRef} className={`loading-transition${className ? ` ${className}` : ''}`}>
      {loading
        ? (skeleton ?? <Skeleton variant="pane" />)
        : <div className="loading-transition__content">{children}</div>
      }
    </div>
  );
}
