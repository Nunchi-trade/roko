import { useEffect, useRef, useCallback, type RefObject } from 'react';

/**
 * Shared canvas setup hook that handles DPR scaling, resize observation,
 * and requestAnimationFrame batching.
 *
 * Usage:
 *   const canvasRef = useRef<HTMLCanvasElement>(null);
 *   useCanvasSetup(canvasRef, (ctx, width, height) => {
 *     // draw here — width/height are CSS pixels, ctx is already DPR-scaled
 *   }, [dependency1, dependency2]);
 */
export function useCanvasSetup(
  canvasRef: RefObject<HTMLCanvasElement | null>,
  draw: (ctx: CanvasRenderingContext2D, width: number, height: number) => void,
  deps: unknown[] = [],
) {
  const rafRef = useRef<number>(0);
  const drawRef = useRef(draw);
  drawRef.current = draw;

  const scheduleDraw = useCallback(() => {
    if (rafRef.current) return; // already scheduled
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0;
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      const dpr = window.devicePixelRatio || 1;
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawRef.current(ctx, rect.width, rect.height);
    });
  }, [canvasRef]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    // Initial draw
    scheduleDraw();

    // Observe resize
    const ro = new ResizeObserver(() => scheduleDraw());
    ro.observe(canvas);

    return () => {
      ro.disconnect();
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = 0;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [canvasRef, scheduleDraw, ...deps]);
}
