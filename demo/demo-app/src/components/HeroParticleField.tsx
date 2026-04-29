import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  useCallback,
} from 'react';

/* ── constants ── */
const TAU = Math.PI * 2;
const MAX_PARTICLES = 120;
const CONNECTION_DIST = 80;
const MOUSE_RADIUS = 150;
const MOUSE_FORCE = 0.008;
const LIFETIME_MIN = 8_000; // ms
const LIFETIME_MAX = 15_000;

/* ── color palettes keyed by mode ── */
type ColorMode = 'rose' | 'cyan' | 'emerald' | 'gold';

const PALETTES: Record<ColorMode, [number, number, number][]> = {
  rose: [
    [220, 165, 189],
    [204, 144, 168],
    [232, 181, 206],
  ],
  cyan: [
    [45, 212, 191],
    [56, 189, 248],
    [34, 211, 238],
  ],
  emerald: [
    [74, 222, 128],
    [52, 211, 153],
    [110, 231, 183],
  ],
  gold: [
    [251, 191, 36],
    [245, 158, 11],
    [253, 224, 71],
  ],
};

/* ── connection line color per mode (r,g,b) ── */
const LINE_COLORS: Record<ColorMode, [number, number, number]> = {
  rose: [184, 122, 148],
  cyan: [45, 212, 191],
  emerald: [74, 222, 128],
  gold: [251, 191, 36],
};

/* ── particle struct ── */
interface Particle {
  x: number; // 0..1 normalised
  y: number;
  vx: number;
  vy: number;
  /** Depth layer 0 | 1 | 2 (back → front) */
  layer: number;
  sz: number;
  phase: number;
  hue: [number, number, number];
  /** Timestamp of birth (ms) */
  born: number;
  /** Lifetime in ms */
  life: number;
}

/* ── public handle ── */
export interface HeroParticleFieldHandle {
  pulse: () => void;
}

export interface HeroParticleFieldProps {
  colorMode?: ColorMode;
}

const LAYER_SIZES = [1, 1.5, 2]; // px per layer
const LAYER_SPEEDS = [0.6, 1.0, 1.4]; // velocity multiplier per layer

function makeParticle(
  palette: [number, number, number][],
  now: number,
  index: number,
  staggerBirth?: boolean,
): Particle {
  const layer = index % 3;
  const speed = 0.00012 * LAYER_SPEEDS[layer];
  const life = LIFETIME_MIN + Math.random() * (LIFETIME_MAX - LIFETIME_MIN);
  return {
    x: Math.random(),
    y: Math.random(),
    vx: (Math.random() - 0.5) * speed,
    vy: (Math.random() - 0.5) * speed,
    layer,
    sz: LAYER_SIZES[layer],
    phase: Math.random() * TAU,
    hue: palette[index % palette.length],
    born: staggerBirth ? now - Math.random() * life : now,
    life,
  };
}

const HeroParticleField = forwardRef<
  HeroParticleFieldHandle,
  HeroParticleFieldProps
>(function HeroParticleField({ colorMode = 'rose' }, ref) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const mouseRef = useRef<{ x: number; y: number } | null>(null);
  const pulseRef = useRef<{ time: number; strength: number } | null>(null);
  const colorModeRef = useRef<ColorMode>(colorMode);
  colorModeRef.current = colorMode;

  const pulse = useCallback(() => {
    pulseRef.current = { time: performance.now(), strength: 1 };
  }, []);

  useImperativeHandle(ref, () => ({ pulse }), [pulse]);

  useEffect(() => {
    const can = canvasRef.current;
    if (!can) return;
    const ctx = can.getContext('2d')!;
    const dpr = Math.min(devicePixelRatio, 2);

    /* ── offscreen canvas for connection lines ── */
    const offscreen = document.createElement('canvas');
    const offCtx = offscreen.getContext('2d')!;

    /* ── particles ── */
    const now = performance.now();
    const palette = PALETTES[colorModeRef.current];
    const particles: Particle[] = [];
    for (let i = 0; i < MAX_PARTICLES; i++) {
      particles.push(makeParticle(palette, now, i, true));
    }

    /* ── resize ── */
    let W = 0;
    let H = 0;
    function resize() {
      W = innerWidth;
      H = innerHeight;
      can!.width = W * dpr;
      can!.height = H * dpr;
      can!.style.width = W + 'px';
      can!.style.height = H + 'px';
      offscreen.width = W * dpr;
      offscreen.height = H * dpr;
    }
    resize();
    window.addEventListener('resize', resize);

    /* ── mouse tracking ── */
    function onMouse(e: MouseEvent) {
      mouseRef.current = { x: e.clientX, y: e.clientY };
    }
    function onLeave() {
      mouseRef.current = null;
    }
    can.style.pointerEvents = 'auto';
    can.addEventListener('mousemove', onMouse);
    can.addEventListener('mouseleave', onLeave);

    /* ── animation loop ── */
    let raf: number;
    let t = 0;

    function tick() {
      const cMode = colorModeRef.current;
      const pal = PALETTES[cMode];
      const lineClr = LINE_COLORS[cMode];
      t += 0.005;
      const now = performance.now();
      const w = can!.width;
      const h = can!.height;

      ctx.clearRect(0, 0, w, h);

      /* ── pulse decay ── */
      let pulseStrength = 0;
      let pulseCx = 0.5;
      let pulseCy = 0.5;
      if (pulseRef.current) {
        const elapsed = (now - pulseRef.current.time) / 1000;
        pulseStrength = pulseRef.current.strength * Math.max(0, 1 - elapsed / 1.2);
        if (pulseStrength <= 0.001) pulseRef.current = null;
      }

      /* ── update + birth/death ── */
      const mouse = mouseRef.current;
      for (let i = particles.length - 1; i >= 0; i--) {
        const p = particles[i];
        const age = now - p.born;

        // death: replace with new particle
        if (age >= p.life) {
          particles[i] = makeParticle(pal, now, i);
          continue;
        }

        const layerSpeed = LAYER_SPEEDS[p.layer];

        // mouse repulsion
        if (mouse) {
          const px = p.x * W;
          const py = p.y * H;
          const dx = px - mouse.x;
          const dy = py - mouse.y;
          const dist = Math.sqrt(dx * dx + dy * dy);
          if (dist < MOUSE_RADIUS && dist > 0.1) {
            const force = (1 - dist / MOUSE_RADIUS) * MOUSE_FORCE * layerSpeed;
            p.vx += (dx / dist) * force;
            p.vy += (dy / dist) * force;
          }
        }

        // pulse push
        if (pulseStrength > 0) {
          const px = p.x - pulseCx;
          const py = p.y - pulseCy;
          const dist = Math.sqrt(px * px + py * py);
          if (dist > 0.001) {
            const force = pulseStrength * 0.003 * layerSpeed;
            p.vx += (px / dist) * force;
            p.vy += (py / dist) * force;
          }
        }

        // velocity damping
        p.vx *= 0.995;
        p.vy *= 0.995;

        // integrate
        p.x += p.vx;
        p.y += p.vy;

        // wrap edges
        if (p.x < -0.02) p.x = 1.02;
        else if (p.x > 1.02) p.x = -0.02;
        if (p.y < -0.02) p.y = 1.02;
        else if (p.y > 1.02) p.y = -0.02;
      }

      /* ── draw connection lines on offscreen canvas ── */
      offCtx.clearRect(0, 0, w, h);
      offCtx.lineWidth = 0.5 * dpr;
      for (let i = 0; i < particles.length; i++) {
        const a = particles[i];
        const ax = a.x * W * dpr;
        const ay = a.y * H * dpr;
        const aAge = now - a.born;
        const aFade = fadeAlpha(aAge, a.life);
        for (let j = i + 1; j < particles.length; j++) {
          const b = particles[j];
          // only connect same or adjacent layers
          if (Math.abs(a.layer - b.layer) > 1) continue;
          const bx = b.x * W * dpr;
          const by = b.y * H * dpr;
          const dx = ax - bx;
          const dy = ay - by;
          const dist = Math.sqrt(dx * dx + dy * dy);
          const maxDist = CONNECTION_DIST * dpr;
          if (dist < maxDist) {
            const bAge = now - b.born;
            const bFade = fadeAlpha(bAge, b.life);
            const lineAlpha = (1 - dist / maxDist) * 0.08 * aFade * bFade;
            offCtx.strokeStyle = `rgba(${lineClr[0]},${lineClr[1]},${lineClr[2]},${lineAlpha})`;
            offCtx.beginPath();
            offCtx.moveTo(ax, ay);
            offCtx.lineTo(bx, by);
            offCtx.stroke();
          }
        }
      }
      // blit connections
      ctx.drawImage(offscreen, 0, 0);

      /* ── draw particles ── */
      ctx.shadowBlur = 0;
      for (const p of particles) {
        const age = now - p.born;
        const fade = fadeAlpha(age, p.life);
        const baseAlpha = 0.18 + Math.sin(t * 1.5 + p.phase) * 0.12;
        const alpha = baseAlpha * fade;
        if (alpha < 0.005) continue;

        const [r, g, b] = p.hue;
        ctx.fillStyle = `rgba(${r},${g},${b},${alpha})`;
        ctx.shadowBlur = 8 * dpr * (0.5 + fade * 0.5);
        ctx.shadowColor = `rgba(${r},${g},${b},${alpha * 0.6})`;
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
      can.removeEventListener('mousemove', onMouse);
      can.removeEventListener('mouseleave', onLeave);
    };
  }, []);

  return (
    <div
      style={{
        position: 'fixed',
        inset: 0,
        zIndex: 0,
        willChange: 'transform',
      }}
    >
      <canvas
        ref={canvasRef}
        role="img"
        aria-label="Interactive ambient particle field"
        style={{ display: 'block' }}
      />
    </div>
  );
});

/** Fade-in over first 10% of life, fade-out over last 20% */
function fadeAlpha(age: number, life: number): number {
  const ratio = age / life;
  if (ratio < 0.1) return ratio / 0.1;
  if (ratio > 0.8) return (1 - ratio) / 0.2;
  return 1;
}

export default HeroParticleField;
