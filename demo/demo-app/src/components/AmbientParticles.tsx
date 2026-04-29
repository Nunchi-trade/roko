import { useEffect, useRef } from 'react';

const TAU = Math.PI * 2;
const N = 30;

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  sz: number;
  phase: number;
  hue: [number, number, number];
}

export default function AmbientParticles() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const can = canvasRef.current;
    if (!can) return;
    const ctx = can.getContext('2d')!;
    const dpr = Math.min(devicePixelRatio, 2);

    const particles: Particle[] = [];
    for (let i = 0; i < N; i++) {
      particles.push({
        x: Math.random(),
        y: Math.random(),
        vx: (Math.random() - 0.5) * 0.00012,
        vy: (Math.random() - 0.5) * 0.00012,
        sz: 0.3 + Math.random() * 1.4,
        phase: Math.random() * TAU,
        hue: Math.random() < 0.5 ? [220, 165, 189] : [200, 184, 144],
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
      t += 0.005;
      const w = can!.width, h = can!.height;
      ctx.clearRect(0, 0, w, h);
      for (const p of particles) {
        p.x += p.vx;
        p.y += p.vy;
        if (p.x < 0 || p.x > 1) p.vx *= -1;
        if (p.y < 0 || p.y > 1) p.vy *= -1;
        const a = 0.18 + Math.sin(t * 1.5 + p.phase) * 0.12;
        ctx.fillStyle = `rgba(${p.hue[0]},${p.hue[1]},${p.hue[2]},${a})`;
        ctx.shadowBlur = 8 * dpr;
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
    <div style={{ position: 'fixed', inset: 0, pointerEvents: 'none', zIndex: 0 }}>
      <canvas ref={canvasRef} role="img" aria-label="Ambient particle field background animation" />
    </div>
  );
}
