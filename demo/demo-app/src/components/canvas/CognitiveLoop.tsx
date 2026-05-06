import { useRef, useEffect, useCallback } from 'react';

const PHASES = ['SENSE', 'ASSESS', 'COMPOSE', 'ACT', 'VERIFY', 'PERSIST', 'REACT'] as const;

const COLORS = {
  active: '#cc90a8',    // rose-bright
  inactive: '#8a7a6e',  // bone-dim
  pulse: '#c8b890',     // bone
  arc: 'rgba(138,122,110,0.25)', // glass-border approx
};

interface CognitiveLoopProps {
  activePhase?: string;
  height?: number;
  className?: string;
}

export default function CognitiveLoop({
  activePhase,
  height = 200,
  className,
}: CognitiveLoopProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef(0);
  const tRef = useRef(0);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const rect = canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    if (w === 0 || h === 0) {
      rafRef.current = requestAnimationFrame(draw);
      return;
    }

    const dpr = window.devicePixelRatio || 1;
    if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const cx = w / 2;
    const cy = h / 2;
    const r = Math.min(cx, cy) - 28;
    const n = PHASES.length;

    // Angle for each node (start at top, -PI/2)
    const nodeAngle = (i: number) => (i / n) * Math.PI * 2 - Math.PI / 2;

    // Draw arc segments connecting nodes
    ctx.strokeStyle = COLORS.arc;
    ctx.lineWidth = 1.5;
    for (let i = 0; i < n; i++) {
      const a1 = nodeAngle(i);
      const a2 = nodeAngle((i + 1) % n);
      ctx.beginPath();
      ctx.arc(cx, cy, r, a1, a2);
      ctx.stroke();
    }

    // Active phase index
    const activeIdx = activePhase
      ? PHASES.findIndex((p) => p.toLowerCase() === activePhase.toLowerCase())
      : -1;

    // Draw nodes
    const nodeR = 14;
    for (let i = 0; i < n; i++) {
      const a = nodeAngle(i);
      const nx = cx + r * Math.cos(a);
      const ny = cy + r * Math.sin(a);
      const isActive = i === activeIdx;

      // Glow for active node
      if (isActive) {
        ctx.save();
        ctx.shadowColor = COLORS.active;
        ctx.shadowBlur = 12;
      }

      ctx.beginPath();
      ctx.arc(nx, ny, nodeR, 0, Math.PI * 2);
      ctx.fillStyle = isActive ? COLORS.active : COLORS.inactive;
      ctx.globalAlpha = isActive ? 1 : 0.6;
      ctx.fill();
      ctx.globalAlpha = 1;

      if (isActive) ctx.restore();

      // Border
      ctx.beginPath();
      ctx.arc(nx, ny, nodeR, 0, Math.PI * 2);
      ctx.strokeStyle = isActive ? COLORS.active : COLORS.arc;
      ctx.lineWidth = isActive ? 2 : 1;
      ctx.stroke();

      // Label below node
      ctx.font = '600 9px system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillStyle = isActive ? COLORS.active : COLORS.inactive;
      ctx.globalAlpha = isActive ? 1 : 0.7;
      ctx.fillText(PHASES[i], nx, ny + nodeR + 12);
      ctx.globalAlpha = 1;
    }

    // Traveling pulse dot
    const t = tRef.current;
    tRef.current += 0.006;
    const pulsePos = t % 1; // 0..1 around the loop
    const pulseAngle = pulsePos * Math.PI * 2 - Math.PI / 2;

    // Trail dots
    const trailAlphas = [0.8, 0.5, 0.3, 0.15];
    const trailGap = 0.015;
    for (let ti = trailAlphas.length - 1; ti >= 0; ti--) {
      const ta = (pulsePos - (ti + 1) * trailGap) * Math.PI * 2 - Math.PI / 2;
      const tx = cx + r * Math.cos(ta);
      const ty = cy + r * Math.sin(ta);
      ctx.beginPath();
      ctx.arc(tx, ty, 3 - ti * 0.4, 0, Math.PI * 2);
      ctx.fillStyle = COLORS.pulse;
      ctx.globalAlpha = trailAlphas[ti];
      ctx.fill();
      ctx.globalAlpha = 1;
    }

    // Main pulse dot
    const px = cx + r * Math.cos(pulseAngle);
    const py = cy + r * Math.sin(pulseAngle);
    ctx.beginPath();
    ctx.arc(px, py, 4, 0, Math.PI * 2);
    ctx.fillStyle = COLORS.pulse;
    ctx.fill();

    rafRef.current = requestAnimationFrame(draw);
  }, [activePhase]);

  useEffect(() => {
    rafRef.current = requestAnimationFrame(draw);
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [draw]);

  return (
    <canvas
      ref={canvasRef}
      className={className}
      style={{ width: '100%', height, display: 'block' }}
    />
  );
}
