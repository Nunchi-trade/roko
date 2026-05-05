import { useEffect, useRef } from 'react';

const TAU = Math.PI * 2;

/* ── configurable particle field ── */

export interface ParticleFieldConfig {
  /** Number of particles (default 30) */
  count?: number;
  /** Base velocity magnitude (default 0.00012) */
  speed?: number;
  /** Particle colors as [r,g,b] tuples (default: rosedust rose + bone) */
  colors?: [number, number, number][];
  /** Min particle radius in CSS px (default 0.3) */
  minSize?: number;
  /** Max particle radius in CSS px (default 1.7) */
  maxSize?: number;
  /** Base alpha (default 0.18) */
  baseAlpha?: number;
  /** Alpha oscillation amplitude (default 0.12) */
  alphaSwing?: number;
  /** Shadow blur radius multiplied by dpr (default 8) */
  glowRadius?: number;
  /** Animation speed factor (1.0 = default, 2.0 = double speed) */
  animSpeed?: number;
  /** Reactivity: 0-1 value that modulates speed + glow in real-time */
  reactivity?: number;
}

export interface AmbientParticlesProps {
  config?: ParticleFieldConfig;
  className?: string;
}

const DEFAULTS: Required<ParticleFieldConfig> = {
  count: 30,
  speed: 0.00012,
  colors: [[220, 165, 189], [200, 184, 144]],
  minSize: 0.3,
  maxSize: 1.7,
  baseAlpha: 0.18,
  alphaSwing: 0.12,
  glowRadius: 8,
  animSpeed: 1.0,
  reactivity: 0,
};

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  sz: number;
  phase: number;
  hue: [number, number, number];
}

export default function AmbientParticles({ config, className }: AmbientParticlesProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // Store config in a ref so the animation loop always reads the latest
  // values without restarting the effect.
  const cfgRef = useRef({ ...DEFAULTS, ...config });
  cfgRef.current = { ...DEFAULTS, ...config };

  useEffect(() => {
    const can = canvasRef.current;
    if (!can) return;
    const ctx = can.getContext('2d')!;
    const dpr = Math.min(devicePixelRatio, 2);
    const cfg = cfgRef.current;

    const particles: Particle[] = [];
    for (let i = 0; i < cfg.count; i++) {
      particles.push({
        x: Math.random(),
        y: Math.random(),
        vx: (Math.random() - 0.5) * cfg.speed,
        vy: (Math.random() - 0.5) * cfg.speed,
        sz: cfg.minSize + Math.random() * (cfg.maxSize - cfg.minSize),
        phase: Math.random() * TAU,
        hue: cfg.colors[i % cfg.colors.length],
      });
    }

    function resize() {
      const w = innerWidth, h = innerHeight;
      can!.width = w * dpr;
      can!.height = h * dpr;
      can!.style.width = w + 'px';
      can!.style.height = h + 'px';
    }
    resize();
    window.addEventListener('resize', resize);

    let t = 0;
    let raf: number;
    function tick() {
      const c = cfgRef.current;
      t += 0.005 * c.animSpeed;
      const w = can!.width, h = can!.height;
      const reactMult = 1 + c.reactivity * 2;
      ctx.clearRect(0, 0, w, h);
      for (const p of particles) {
        p.x += p.vx * reactMult;
        p.y += p.vy * reactMult;
        if (p.x < 0 || p.x > 1) p.vx *= -1;
        if (p.y < 0 || p.y > 1) p.vy *= -1;
        const a = c.baseAlpha + Math.sin(t * 1.5 + p.phase) * c.alphaSwing;
        ctx.fillStyle = `rgba(${p.hue[0]},${p.hue[1]},${p.hue[2]},${a})`;
        ctx.shadowBlur = c.glowRadius * dpr * reactMult;
        ctx.shadowColor = `rgba(${p.hue[0]},${p.hue[1]},${p.hue[2]},.5)`;
        ctx.beginPath();
        ctx.arc(p.x * w, p.y * h, p.sz * dpr, 0, TAU);
        ctx.fill();
      }
      ctx.shadowBlur = 0;
      raf = requestAnimationFrame(tick);
    }
    tick();

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', resize);
    };
  }, []);

  return (
    <div
      className={className}
      style={{ position: 'fixed', inset: 0, pointerEvents: 'none', zIndex: 0 }}
    >
      <canvas
        ref={canvasRef}
        role="img"
        aria-label="Ambient particle field background animation"
      />
    </div>
  );
}
