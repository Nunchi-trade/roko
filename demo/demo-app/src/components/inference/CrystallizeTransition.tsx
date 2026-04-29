import { useEffect, useRef, useCallback, useState, type ReactNode } from 'react';
import { getCssVar } from '../../lib/color';
import './CrystallizeTransition.css';

interface CrystallizeTransitionProps {
  active: boolean;
  intensity?: number;
  duration?: number;
  children: ReactNode;
  className?: string;
}

/* ── Particle system ── */

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  size: number;
  color: string;
  life: number;
  decay: number;
}

/** Resolve rosedust token colors at runtime. */
function getParticleColors(): string[] {
  return [
    getCssVar('--rose-bright') || '#d89ab2',
    getCssVar('--bone-bright') || '#e4d8b0',
    getCssVar('--dream-bright') || '#a4a4c8',
  ];
}

/** Check if user prefers reduced motion. */
function prefersReducedMotion(): boolean {
  return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

function createParticles(
  count: number,
  width: number,
  height: number,
  colors: string[],
): Particle[] {
  const cx = width / 2;
  const cy = height / 2;
  const particles: Particle[] = [];

  for (let i = 0; i < count; i++) {
    const angle = (Math.PI * 2 * i) / count + (Math.random() - 0.5) * 0.8;
    const speed = 40 + Math.random() * 80;
    particles.push({
      x: cx + (Math.random() - 0.5) * width * 0.3,
      y: cy + (Math.random() - 0.5) * height * 0.3,
      vx: Math.cos(angle) * speed,
      vy: Math.sin(angle) * speed,
      size: 2 + Math.random() * 2,
      color: colors[Math.floor(Math.random() * colors.length)],
      life: 1,
      decay: 0.6 + Math.random() * 0.4,
    });
  }
  return particles;
}

/** Draw a single sparkle point (cross pattern). */
function drawSparkle(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  size: number,
  color: string,
  alpha: number,
) {
  ctx.globalAlpha = alpha;
  ctx.fillStyle = color;

  // Center dot
  ctx.beginPath();
  ctx.arc(x, y, size * 0.5, 0, Math.PI * 2);
  ctx.fill();

  // Cross arms
  const armLen = size * 1.2;
  const armWidth = size * 0.25;
  ctx.fillRect(x - armLen, y - armWidth, armLen * 2, armWidth * 2);
  ctx.fillRect(x - armWidth, y - armLen, armWidth * 2, armLen * 2);
}

export default function CrystallizeTransition({
  active,
  intensity = 0.6,
  duration = 1500,
  children,
  className,
}: CrystallizeTransitionProps) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rafRef = useRef<number>(0);
  const prevActiveRef = useRef(false);
  const shimmerRef = useRef<HTMLDivElement>(null);
  const ringRef = useRef<HTMLDivElement>(null);
  const [contentPhase, setContentPhase] = useState<'idle' | 'entering' | 'exiting'>('idle');

  const runEffect = useCallback(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;

    // Shimmer + ring (CSS-driven)
    if (shimmerRef.current) {
      shimmerRef.current.classList.remove('crystallize-transition__shimmer--active');
      // Force reflow to restart animation
      void shimmerRef.current.offsetWidth;
      shimmerRef.current.classList.add('crystallize-transition__shimmer--active');
    }
    if (ringRef.current) {
      ringRef.current.classList.remove('crystallize-transition__ring--active');
      void ringRef.current.offsetWidth;
      ringRef.current.classList.add('crystallize-transition__ring--active');
    }

    // Skip canvas particles for reduced motion
    if (prefersReducedMotion()) return;

    const rect = wrapper.getBoundingClientRect();
    const w = Math.round(rect.width);
    const h = Math.round(rect.height);
    if (w === 0 || h === 0) return;

    // Create or reuse canvas
    let canvas = canvasRef.current;
    if (!canvas) {
      canvas = document.createElement('canvas');
      canvas.className = 'crystallize-transition__canvas';
      canvasRef.current = canvas;
    }

    const dpr = window.devicePixelRatio || 1;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;

    if (!wrapper.contains(canvas)) {
      wrapper.appendChild(canvas);
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const colors = getParticleColors();
    const clampedIntensity = Math.max(0, Math.min(1, intensity));
    const count = Math.max(8, Math.min(40, Math.round(clampedIntensity * 30)));
    const particles = createParticles(count, w, h, colors);

    const durationSec = duration / 1000;
    let elapsed = 0;
    let lastTime = performance.now();

    // Cancel any prior animation
    if (rafRef.current) cancelAnimationFrame(rafRef.current);

    function tick(now: number) {
      const dt = Math.min((now - lastTime) / 1000, 0.05); // Cap delta at 50ms
      lastTime = now;
      elapsed += dt;

      if (elapsed > durationSec || !ctx || !canvas) {
        // Cleanup
        ctx?.clearRect(0, 0, w, h);
        if (canvas && wrapper?.contains(canvas)) {
          wrapper.removeChild(canvas);
        }
        canvasRef.current = null;
        return;
      }

      ctx.clearRect(0, 0, w, h);

      let alive = false;
      for (const p of particles) {
        if (p.life <= 0) continue;
        alive = true;

        // Physics: velocity with friction
        p.x += p.vx * dt;
        p.y += p.vy * dt;
        p.vx *= 0.97;
        p.vy *= 0.97;

        // Decay life
        p.life -= (dt / durationSec) * p.decay;

        // Draw
        const currentSize = p.size * Math.max(0, p.life);
        if (currentSize > 0.2) {
          drawSparkle(ctx, p.x, p.y, currentSize, p.color, p.life * 0.9);
        }
      }

      ctx.globalAlpha = 1;

      if (alive) {
        rafRef.current = requestAnimationFrame(tick);
      } else {
        ctx.clearRect(0, 0, w, h);
        if (canvas && wrapper && wrapper.contains(canvas)) {
          wrapper.removeChild(canvas);
        }
        canvasRef.current = null;
      }
    }

    rafRef.current = requestAnimationFrame(tick);
  }, [intensity, duration]);

  // Detect false->true and true->false transitions for content animation
  useEffect(() => {
    const wasActive = prevActiveRef.current;
    prevActiveRef.current = active;

    if (active && !wasActive) {
      // Entering: trigger crystallize-materialize + particle burst
      setContentPhase('entering');
      runEffect();
      const timer = setTimeout(() => setContentPhase('idle'), 450);
      return () => clearTimeout(timer);
    } else if (!active && wasActive) {
      // Exiting: trigger crystallize-dissolve (reverse shatter)
      setContentPhase('exiting');
      const timer = setTimeout(() => setContentPhase('idle'), 400);
      return () => clearTimeout(timer);
    }
  }, [active, runEffect]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      const canvas = canvasRef.current;
      const wrapper = wrapperRef.current;
      if (canvas && wrapper?.contains(canvas)) {
        wrapper.removeChild(canvas);
      }
    };
  }, []);

  const contentClass = contentPhase !== 'idle'
    ? `crystallize-transition__content crystallize-transition__content--${contentPhase}`
    : 'crystallize-transition__content';

  return (
    <div
      ref={wrapperRef}
      className={`crystallize-transition${className ? ` ${className}` : ''}`}
    >
      <div className={contentClass}>
        {children}
      </div>
      <div ref={shimmerRef} className="crystallize-transition__shimmer" />
      <div ref={ringRef} className="crystallize-transition__ring" />
    </div>
  );
}
