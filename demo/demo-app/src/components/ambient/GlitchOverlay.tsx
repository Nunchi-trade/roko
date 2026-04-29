import { useRef, useEffect } from 'react';
import { getCssVar, hexToRgba } from '../../lib/color';
import './GlitchOverlay.css';

interface GlitchOverlayProps {
  intensity: number;
  active?: boolean;
  color?: string;
  className?: string;
}

const MAX_FPS = 30;
const FRAME_MS = 1000 / MAX_FPS;

export default function GlitchOverlay({
  intensity,
  active = true,
  color = '--rose-dim',
  className,
}: GlitchOverlayProps) {
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
    if (!active || intensity <= 0 || reducedMotion.current) {
      // Clear canvas when inactive
      const cvs = canvasRef.current;
      if (cvs) {
        const ctx = cvs.getContext('2d');
        if (ctx) ctx.clearRect(0, 0, cvs.width, cvs.height);
      }
      return;
    }

    const canvas = canvasRef.current;
    if (!canvas) return;

    function animate(now: number) {
      if (now - lastFrameRef.current < FRAME_MS) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }
      lastFrameRef.current = now;

      const cvs = canvasRef.current;
      if (!cvs) return;
      const rect = cvs.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) {
        rafRef.current = requestAnimationFrame(animate);
        return;
      }

      const dpr = window.devicePixelRatio || 1;
      const w = rect.width;
      const h = rect.height;

      if (cvs.width !== Math.round(w * dpr) || cvs.height !== Math.round(h * dpr)) {
        cvs.width = w * dpr;
        cvs.height = h * dpr;
      }

      const ctx = cvs.getContext('2d');
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const resolved = getCssVar(color);

      // Probability of a glitch event this frame scales with intensity
      const glitchChance = intensity * 0.4;

      if (Math.random() < glitchChance) {
        // Scanline noise: horizontal bars
        const numLines = Math.floor(2 + intensity * 8);
        for (let i = 0; i < numLines; i++) {
          const y = Math.random() * h;
          const lineH = 1 + Math.random() * 3 * intensity;
          ctx.fillStyle = hexToRgba(resolved, 0.05 + intensity * 0.12);
          ctx.fillRect(0, y, w, lineH);
        }

        // Horizontal slice displacement
        if (intensity > 0.2) {
          const slices = Math.floor(1 + intensity * 4);
          for (let i = 0; i < slices; i++) {
            const sy = Math.random() * h;
            const sh = 2 + Math.random() * 20 * intensity;
            const offset = (Math.random() - 0.5) * 12 * intensity;
            ctx.fillStyle = hexToRgba(resolved, 0.03 + intensity * 0.06);
            ctx.fillRect(offset, sy, w, sh);
          }
        }

        // Color channel offset (RGB shift)
        if (intensity > 0.4) {
          const shift = intensity * 4;
          // Red channel
          ctx.fillStyle = `rgba(255, 60, 80, ${0.02 * intensity})`;
          ctx.fillRect(-shift, 0, w, h);
          // Cyan channel
          ctx.fillStyle = `rgba(60, 200, 255, ${0.02 * intensity})`;
          ctx.fillRect(shift, 0, w, h);
        }
      }

      // Persistent subtle scanlines at any intensity
      ctx.fillStyle = hexToRgba(resolved, 0.015 * intensity);
      for (let y = 0; y < h; y += 4) {
        ctx.fillRect(0, y, w, 1);
      }

      rafRef.current = requestAnimationFrame(animate);
    }

    rafRef.current = requestAnimationFrame(animate);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [active, intensity, color]);

  return (
    <div className={`glitch-overlay ${className ?? ''}`}>
      <canvas ref={canvasRef} role="presentation" aria-hidden="true" />
    </div>
  );
}
