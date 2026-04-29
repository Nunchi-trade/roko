import { useRef, useEffect, useCallback } from 'react';
import type { AgentIdentity, SpectreArchetype } from './AgentIdentity';
import { ROLE_PALETTES, mulberry32 } from './AgentIdentity';
import './SpectreAvatar.css';

interface SpectreAvatarProps {
  identity: AgentIdentity;
  size?: number;
}

interface Dot {
  x: number;
  y: number;
  baseX: number;
  baseY: number;
  r: number;
  opacity: number;
  color: string;
}

/** Generate dots positioned by archetype-specific pattern */
function generateDots(identity: AgentIdentity, size: number): Dot[] {
  const rng = mulberry32(identity.seed);
  const count = 30 + Math.floor(rng() * 31); // 30-60 dots
  const palette = ROLE_PALETTES[identity.role];
  const cx = size / 2;
  const cy = size / 2;
  const maxR = size * 0.4;
  const dots: Dot[] = [];

  for (let i = 0; i < count; i++) {
    const t = i / count;
    const colorIdx = Math.floor(rng() * 3);
    const color = palette[colorIdx];
    const r = 1 + rng() * 3; // 1-4px
    const opacity = 0.3 + rng() * 0.7; // 0.3-1.0
    const [x, y] = archetypePosition(identity.archetype, t, cx, cy, maxR, rng);
    dots.push({ x, y, baseX: x, baseY: y, r, opacity, color });
  }

  return dots;
}

/** Position a dot according to archetype pattern */
function archetypePosition(
  archetype: SpectreArchetype,
  t: number,
  cx: number,
  cy: number,
  maxR: number,
  rng: () => number,
): [number, number] {
  const jitter = () => (rng() - 0.5) * maxR * 0.3;

  switch (archetype) {
    case 'planner': {
      // Grid pattern
      const cols = 6;
      const col = Math.floor(t * cols * cols) % cols;
      const row = Math.floor((t * cols * cols) / cols) % cols;
      const spacing = maxR * 2 / (cols - 1);
      return [
        cx - maxR + col * spacing + jitter() * 0.5,
        cy - maxR + row * spacing + jitter() * 0.5,
      ];
    }
    case 'executor': {
      // Burst from center
      const angle = rng() * Math.PI * 2;
      const dist = rng() * maxR;
      return [cx + Math.cos(angle) * dist, cy + Math.sin(angle) * dist];
    }
    case 'researcher': {
      // Spiral
      const angle = t * Math.PI * 6 + rng() * 0.3;
      const dist = t * maxR + jitter() * 0.3;
      return [cx + Math.cos(angle) * dist, cy + Math.sin(angle) * dist];
    }
    case 'validator': {
      // Ring
      const angle = t * Math.PI * 2 + rng() * 0.2;
      const ringR = maxR * (0.5 + rng() * 0.5);
      return [cx + Math.cos(angle) * ringR + jitter() * 0.2, cy + Math.sin(angle) * ringR + jitter() * 0.2];
    }
    case 'observer': {
      // Concentric rings
      const ring = Math.floor(rng() * 3);
      const ringR = maxR * (0.3 + ring * 0.3);
      const angle = t * Math.PI * 2 + rng() * 0.4;
      return [cx + Math.cos(angle) * ringR, cy + Math.sin(angle) * ringR];
    }
    case 'orchestrator': {
      // Star / hub-spoke
      const spoke = Math.floor(rng() * 5);
      const angle = (spoke / 5) * Math.PI * 2;
      const dist = rng() * maxR;
      return [cx + Math.cos(angle) * dist + jitter() * 0.2, cy + Math.sin(angle) * dist + jitter() * 0.2];
    }
    case 'guardian': {
      // Shield / dense perimeter
      const angle = t * Math.PI * 2;
      const dist = maxR * (0.7 + rng() * 0.3);
      return [cx + Math.cos(angle) * dist + jitter() * 0.15, cy + Math.sin(angle) * dist + jitter() * 0.15];
    }
    case 'specialist':
    default: {
      // Cluster
      const cluster = Math.floor(rng() * 3);
      const offX = (cluster - 1) * maxR * 0.5;
      const offY = (rng() - 0.5) * maxR * 0.4;
      return [cx + offX + jitter() * 0.6, cy + offY + jitter() * 0.6];
    }
  }
}

export default function SpectreAvatar({ identity, size = 40 }: SpectreAvatarProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const dotsRef = useRef<Dot[]>([]);
  const rafRef = useRef<number>(0);
  const hoveredRef = useRef(false);
  const dpr = typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1;

  const draw = useCallback(
    (ctx: CanvasRenderingContext2D, time: number) => {
      const w = size * dpr;
      ctx.clearRect(0, 0, w, w);

      const breathPhase = (Math.sin((time / 1000) * Math.PI) + 1) / 2; // 0-1 over ~2s
      const breathAmp = hoveredRef.current ? 1.8 : 1.2;

      for (const dot of dotsRef.current) {
        // Breathing: gently shift position
        const dx = Math.sin(dot.baseX * 0.1 + time / 1200) * breathAmp;
        const dy = Math.cos(dot.baseY * 0.1 + time / 1400) * breathAmp;
        dot.x = dot.baseX + dx;
        dot.y = dot.baseY + dy;

        const scale = hoveredRef.current ? 1.3 : 1;
        const glowExtra = hoveredRef.current ? 0.15 : 0;
        const alpha = Math.min(1, dot.opacity + breathPhase * 0.1 + glowExtra);

        ctx.beginPath();
        ctx.arc(dot.x * dpr, dot.y * dpr, dot.r * scale * dpr, 0, Math.PI * 2);
        ctx.fillStyle = dot.color + alphaHex(alpha);
        ctx.fill();
      }

      rafRef.current = requestAnimationFrame((t) => draw(ctx, t));
    },
    [size, dpr],
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    dotsRef.current = generateDots(identity, size);
    rafRef.current = requestAnimationFrame((t) => draw(ctx, t));

    return () => cancelAnimationFrame(rafRef.current);
  }, [identity, size, draw]);

  return (
    <div
      className="spectre-avatar"
      style={{ width: size, height: size }}
      onMouseEnter={() => { hoveredRef.current = true; }}
      onMouseLeave={() => { hoveredRef.current = false; }}
    >
      <canvas
        ref={canvasRef}
        width={size * dpr}
        height={size * dpr}
      />
    </div>
  );
}

/** Convert 0-1 alpha to 2-char hex suffix */
function alphaHex(a: number): string {
  const clamped = Math.max(0, Math.min(1, a));
  const byte = Math.round(clamped * 255);
  return byte.toString(16).padStart(2, '0');
}
