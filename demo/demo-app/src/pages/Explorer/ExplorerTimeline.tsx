import { useEffect, useCallback, useRef } from 'react';
import { getCssVar } from '../../lib/color';
import FlatIcon from '../../components/FlatIcon';
import type { Episode } from './types';

/* ── Constants ── */

const KIND_LABELS: Record<string, string> = {
  agent_turn: 'Agent Turn',
  gate_result: 'Gate Result',
  tool_call: 'Tool Call',
  plan_step: 'Plan Step',
};

const LANE_PAD_TOP = 48;
const LANE_PAD_BOTTOM = 40;
const LANE_PAD_LEFT = 120;
const LANE_PAD_RIGHT = 28;
const LEGEND_W = 140;
const LEGEND_H = 100;
const MIN_BLOCK_W = 8;
const BLOCK_H = 22;
const BLOCK_R = 4;

const KIND_CANVAS_COLORS: Record<string, string> = {
  agent_turn: '--rose',
  gate_result: '--success',
  tool_call: '--bone',
  plan_step: '--dream',
};

function kindCanvasColor(kind: string): string {
  const token = KIND_CANVAS_COLORS[kind];
  return token ? getCssVar(token) : getCssVar('--dream');
}

/* ── Sparkline drawer ── */

export function drawSparkline(
  canvas: HTMLCanvasElement,
  data: number[],
  color: string,
  animate = true,
) {
  const ctx = canvas.getContext('2d');
  if (!ctx || data.length < 2) return;

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  ctx.scale(dpr, dpr);

  const max = Math.max(...data, 1);
  const step = w / (data.length - 1);

  let fillColor: string;
  if (color.startsWith('#')) {
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    fillColor = `rgba(${r},${g},${b},0.10)`;
  } else {
    fillColor = color.replace(')', ', 0.08)').replace('rgb', 'rgba');
  }

  const drawFrame = (progress: number) => {
    ctx.clearRect(0, 0, w, h);
    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, w * progress, h);
    ctx.clip();

    ctx.beginPath();
    ctx.moveTo(0, h - (data[0] / max) * h * 0.85);
    for (let i = 1; i < data.length; i++) {
      ctx.lineTo(i * step, h - (data[i] / max) * h * 0.85);
    }
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.stroke();

    ctx.lineTo((data.length - 1) * step, h);
    ctx.lineTo(0, h);
    ctx.closePath();
    ctx.fillStyle = fillColor;
    ctx.fill();
    ctx.restore();
  };

  if (!animate) {
    drawFrame(1);
    return;
  }

  const duration = 600;
  const start = performance.now();
  const tick = (now: number) => {
    const elapsed = now - start;
    const t = Math.min(elapsed / duration, 1);
    const progress = t >= 1 ? 1 : 1 - Math.pow(2, -10 * t);
    drawFrame(progress);
    if (t < 1) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/* ── Hero Timeline Canvas ── */

interface ExplorerTimelineProps {
  episodes: Episode[];
  hoveredEp: Episode | null;
  onHoverEpisode: (ep: Episode | null) => void;
}

export default function ExplorerTimeline({ episodes, hoveredEp, onHoverEpisode }: ExplorerTimelineProps) {
  const heroRef = useRef<HTMLCanvasElement>(null);
  const mouseRef = useRef<{ x: number; y: number }>({ x: -1, y: -1 });
  const epRectsRef = useRef<Array<{ x: number; y: number; w: number; h: number; ep: Episode }>>([]);

  const drawTimeline = useCallback(() => {
    const canvas = heroRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    // Background
    ctx.fillStyle = getCssVar('--bg-void');
    ctx.fillRect(0, 0, w, h);

    // Grid lines
    ctx.strokeStyle = 'rgba(255,255,255,0.03)';
    ctx.lineWidth = 1;
    for (let y = LANE_PAD_TOP; y < h - LANE_PAD_BOTTOM; y += 40) {
      ctx.beginPath();
      ctx.moveTo(LANE_PAD_LEFT, y);
      ctx.lineTo(w - LANE_PAD_RIGHT, y);
      ctx.stroke();
    }
    for (let x = LANE_PAD_LEFT; x < w - LANE_PAD_RIGHT; x += 80) {
      ctx.beginPath();
      ctx.moveTo(x, LANE_PAD_TOP);
      ctx.lineTo(x, h - LANE_PAD_BOTTOM);
      ctx.stroke();
    }

    if (episodes.length === 0) {
      ctx.font = '12px JetBrains Mono, monospace';
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.textAlign = 'center';
      ctx.fillText('No episodes yet', w / 2, h / 2);
      epRectsRef.current = [];
      return;
    }

    // Agent lanes
    const agents = [...new Set(episodes.map(e => e.agent_id ?? 'system').filter(Boolean))];
    const drawH = h - LANE_PAD_TOP - LANE_PAD_BOTTOM;
    const laneH = agents.length > 0 ? Math.min(drawH / agents.length, 48) : 40;

    // Time range
    const timestamps = episodes.map(e => e.timestamp_ms ?? 0).filter(t => t > 0);
    const minT = timestamps.length > 0 ? Math.min(...timestamps) : Date.now() - 3600000;
    const maxT = timestamps.length > 0 ? Math.max(...timestamps) : Date.now();
    const timeSpan = Math.max(maxT - minT, 60000);
    const drawW = w - LANE_PAD_LEFT - LANE_PAD_RIGHT;

    const tx = (t: number) => LANE_PAD_LEFT + ((t - minT) / timeSpan) * drawW;
    const ay = (agent: string) => {
      const idx = agents.indexOf(agent);
      return LANE_PAD_TOP + (idx >= 0 ? idx : 0) * laneH + laneH / 2;
    };

    // Draw agent labels
    ctx.font = '13px JetBrains Mono, monospace';
    ctx.textAlign = 'right';
    ctx.textBaseline = 'middle';
    for (let i = 0; i < agents.length; i++) {
      const y = LANE_PAD_TOP + i * laneH + laneH / 2;
      ctx.fillStyle = getCssVar('--text-dim');
      const label = agents[i].length > 14 ? agents[i].slice(0, 13) + '\u2026' : agents[i];
      ctx.fillText(label, LANE_PAD_LEFT - 10, y);

      ctx.strokeStyle = 'rgba(255,255,255,0.025)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(LANE_PAD_LEFT, LANE_PAD_TOP + i * laneH);
      ctx.lineTo(w - LANE_PAD_RIGHT, LANE_PAD_TOP + i * laneH);
      ctx.stroke();
    }

    // Time axis
    ctx.textAlign = 'center';
    ctx.textBaseline = 'top';
    ctx.fillStyle = getCssVar('--text-dim');
    ctx.font = '12px JetBrains Mono, monospace';
    const numLabels = Math.max(Math.floor(drawW / 100), 3);
    for (let i = 0; i <= numLabels; i++) {
      const t = minT + (i / numLabels) * timeSpan;
      const x = tx(t);
      const d = new Date(t);
      ctx.fillText(d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }), x, h - LANE_PAD_BOTTOM + 8);

      ctx.strokeStyle = 'rgba(255,255,255,0.04)';
      ctx.beginPath();
      ctx.moveTo(x, h - LANE_PAD_BOTTOM);
      ctx.lineTo(x, h - LANE_PAD_BOTTOM + 4);
      ctx.stroke();
    }

    // Task connection lines (dotted)
    const taskGroups = new Map<string, Episode[]>();
    for (const ep of episodes) {
      if (ep.task_id && ep.timestamp_ms) {
        const arr = taskGroups.get(ep.task_id) ?? [];
        arr.push(ep);
        taskGroups.set(ep.task_id, arr);
      }
    }
    ctx.setLineDash([3, 5]);
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(255,255,255,0.06)';
    for (const [, eps] of taskGroups) {
      if (eps.length < 2) continue;
      const sorted = eps.sort((a, b) => (a.timestamp_ms ?? 0) - (b.timestamp_ms ?? 0));
      ctx.beginPath();
      for (let i = 0; i < sorted.length; i++) {
        const ep = sorted[i];
        const x = tx(ep.timestamp_ms!);
        const y = ay(ep.agent_id ?? 'system');
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // Draw episode blocks
    const maxDur = Math.max(...episodes.map(e => e.duration_secs ?? 1), 1);
    const rects: Array<{ x: number; y: number; w: number; h: number; ep: Episode }> = [];

    for (const ep of episodes) {
      if (!ep.timestamp_ms) continue;
      const x = tx(ep.timestamp_ms);
      const y = ay(ep.agent_id ?? 'system') - BLOCK_H / 2;
      const dur = ep.duration_secs ?? 1;
      const bw = Math.max((dur / maxDur) * drawW * 0.15, MIN_BLOCK_W);
      const color = kindCanvasColor(ep.kind);

      const isHovered = hoveredEp?.id === ep.id;
      if (isHovered) {
        ctx.shadowColor = color;
        ctx.shadowBlur = 12;
      }

      ctx.fillStyle = isHovered ? color : color + 'cc';
      ctx.beginPath();
      ctx.roundRect(x, y, bw, BLOCK_H, BLOCK_R);
      ctx.fill();

      ctx.fillStyle = 'rgba(255,255,255,0.06)';
      ctx.beginPath();
      ctx.roundRect(x, y, bw, BLOCK_H / 2, [BLOCK_R, BLOCK_R, 0, 0]);
      ctx.fill();

      ctx.shadowBlur = 0;
      rects.push({ x, y, w: bw, h: BLOCK_H, ep });
    }
    epRectsRef.current = rects;

    // Legend (top-right)
    const lx = w - LANE_PAD_RIGHT - LEGEND_W;
    const ly = 10;
    ctx.fillStyle = 'rgba(6,6,10,0.85)';
    ctx.strokeStyle = 'rgba(255,255,255,0.08)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(lx, ly, LEGEND_W, LEGEND_H, 6);
    ctx.fill();
    ctx.stroke();

    ctx.font = '12px JetBrains Mono, monospace';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const kinds = Object.keys(KIND_CANVAS_COLORS);
    for (let i = 0; i < kinds.length; i++) {
      const ky = ly + 16 + i * 20;
      ctx.fillStyle = kindCanvasColor(kinds[i]);
      ctx.beginPath();
      ctx.arc(lx + 14, ky, 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = getCssVar('--text-soft');
      ctx.fillText(KIND_LABELS[kinds[i]] ?? kinds[i], lx + 26, ky);
    }

    // Tooltip for hovered episode
    const mouse = mouseRef.current;
    if (hoveredEp && mouse.x >= 0) {
      const tipW = 240;
      const tipH = 90;
      let tipX = mouse.x + 14;
      let tipY = mouse.y - tipH - 8;
      if (tipX + tipW > w) tipX = mouse.x - tipW - 14;
      if (tipY < 0) tipY = mouse.y + 14;

      ctx.fillStyle = 'rgba(6,6,10,0.92)';
      ctx.strokeStyle = 'rgba(184,122,148,0.3)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.roundRect(tipX, tipY, tipW, tipH, 6);
      ctx.fill();
      ctx.stroke();

      ctx.font = '13px JetBrains Mono, monospace';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'top';
      ctx.fillStyle = getCssVar('--text-primary');
      ctx.fillText(hoveredEp.agent_id ?? 'system', tipX + 12, tipY + 10);
      ctx.fillStyle = getCssVar('--text-dim');
      ctx.fillText(`kind: ${hoveredEp.kind}`, tipX + 12, tipY + 28);
      ctx.fillText(`task: ${(hoveredEp.task_id ?? '-').slice(0, 22)}`, tipX + 12, tipY + 44);
      ctx.fillStyle = getCssVar('--bone');
      ctx.fillText(
        `cost: $${(hoveredEp.usage?.cost_usd ?? 0).toFixed(4)}  dur: ${(hoveredEp.duration_secs ?? 0).toFixed(1)}s`,
        tipX + 12,
        tipY + 62,
      );
    }
  }, [episodes, hoveredEp]);

  useEffect(() => {
    drawTimeline();
  }, [drawTimeline]);

  useEffect(() => {
    const canvas = heroRef.current;
    if (!canvas) return;
    const ro = new ResizeObserver(() => drawTimeline());
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [drawTimeline]);

  const handleHeroMouse = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = heroRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    mouseRef.current = { x, y };

    let found: Episode | null = null;
    for (const r of epRectsRef.current) {
      if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) {
        found = r.ep;
        break;
      }
    }
    onHoverEpisode(found);
  }, [onHoverEpisode]);

  const handleHeroLeave = useCallback(() => {
    mouseRef.current = { x: -1, y: -1 };
    onHoverEpisode(null);
  }, [onHoverEpisode]);

  return (
    <div className="expl-hero-wrap">
      <canvas
        ref={heroRef}
        className="expl-hero-canvas"
        role="img"
        aria-label="Signal flow graph"
        onMouseMove={handleHeroMouse}
        onMouseLeave={handleHeroLeave}
      />
    </div>
  );
}

/* ── Heatmap strip ── */

interface HeatmapStripProps {
  heatmapData: number[];
}

export function HeatmapStrip({ heatmapData }: HeatmapStripProps) {
  const heatmapMax = Math.max(...heatmapData, 1);
  function heatColor(count: number): string {
    if (count === 0) return 'var(--bg-void)';
    const ratio = count / heatmapMax;
    if (ratio < 0.25) return 'var(--rose-deep)';
    if (ratio < 0.5) return 'var(--rose-dim)';
    if (ratio < 0.75) return 'var(--rose)';
    return 'var(--rose-bright)';
  }

  return (
    <div className="expl-heatmap-section">
      <span className="expl-section-label"><FlatIcon name="activity" size={15} tone="rose" />ACTIVITY DENSITY</span>
      <div className="expl-heatmap">
        {heatmapData.map((count, hour) => (
          <div
            key={hour}
            className="expl-heat-cell"
            style={{
              background: heatColor(count),
              animationDelay: `${hour * 30}ms`,
            }}
            title={`${hour}:00 - ${count} episode${count !== 1 ? 's' : ''}`}
          >
            {(hour === 0 || hour === 6 || hour === 12 || hour === 18 || hour === 23) && (
              <span className="expl-heat-label">{hour}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
