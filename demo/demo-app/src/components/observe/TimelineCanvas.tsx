import { useRef, useEffect, useCallback } from 'react';
import { getCssVar } from '../../lib/color';
import './TimelineCanvas.css';

/* ── Types ────────────────────────────────────────────── */

export interface TimelineEntry {
  agent: string;
  start: number;   // timestamp ms
  end: number;      // timestamp ms
  phase: string;
  kind?: string;
  status: 'success' | 'running' | 'failed';
}

export interface TimelineCanvasProps {
  entries: TimelineEntry[];
  height?: number;
  onEntryClick?: (entry: TimelineEntry) => void;
}

/* ── Constants (from Explorer.tsx) ────────────────────── */

const LANE_PAD_TOP = 48;
const LANE_PAD_BOTTOM = 40;
const LANE_PAD_LEFT = 120;
const LANE_PAD_RIGHT = 28;
const BLOCK_H = 22;
const BLOCK_R = 4;
const MIN_BLOCK_W = 8;

/* ── Component ────────────────────────────────────────── */

/**
 * Swimlane timeline canvas -- extracted from Explorer.tsx drawTimeline.
 * Renders agent lanes with task bars, grid lines, and a time axis.
 */
export function TimelineCanvas({ entries, height = 360 }: TimelineCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = container.getBoundingClientRect();
    const w = rect.width;
    const h = height;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    // Background
    ctx.fillStyle = getCssVar('--bg-void') || '#08080c';
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

    if (entries.length === 0) {
      ctx.font = '12px JetBrains Mono, monospace';
      ctx.fillStyle = getCssVar('--text-dim') || '#9a8a98';
      ctx.textAlign = 'center';
      ctx.fillText('No timeline entries', w / 2, h / 2);
      return;
    }

    // Agent lanes
    const agents = [...new Set(entries.map(e => e.agent))];
    const drawH = h - LANE_PAD_TOP - LANE_PAD_BOTTOM;
    const laneH = agents.length > 0 ? Math.min(drawH / agents.length, 48) : 40;

    // Time range
    const allTimes = entries.flatMap(e => [e.start, e.end]).filter(t => t > 0);
    const minT = allTimes.length > 0 ? Math.min(...allTimes) : Date.now() - 3600000;
    const maxT = allTimes.length > 0 ? Math.max(...allTimes) : Date.now();
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
      ctx.fillStyle = getCssVar('--text-dim') || '#9a8a98';
      const label = agents[i].length > 14 ? agents[i].slice(0, 13) + '\u2026' : agents[i];
      ctx.fillText(label, LANE_PAD_LEFT - 10, y);

      // Lane separator
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
    ctx.fillStyle = getCssVar('--text-dim') || '#9a8a98';
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

    // Status colors
    const statusColor: Record<string, string> = {
      success: getCssVar('--success') || '#8a9c86',
      running: getCssVar('--dream') || '#8888a8',
      failed: getCssVar('--rose-bright') || '#d89ab2',
    };

    // Draw entry blocks
    const maxDur = Math.max(...entries.map(e => Math.max(e.end - e.start, 1)), 1);

    for (const entry of entries) {
      const x = tx(entry.start);
      const y = ay(entry.agent) - BLOCK_H / 2;
      const dur = entry.end - entry.start;
      const bw = Math.max((dur / maxDur) * drawW * 0.15, MIN_BLOCK_W);
      const color = statusColor[entry.status] ?? statusColor.running;

      ctx.fillStyle = color + 'cc';
      ctx.beginPath();
      ctx.roundRect(x, y, bw, BLOCK_H, BLOCK_R);
      ctx.fill();

      // Inner shine
      ctx.fillStyle = 'rgba(255,255,255,0.06)';
      ctx.beginPath();
      ctx.roundRect(x, y, bw, BLOCK_H / 2, [BLOCK_R, BLOCK_R, 0, 0]);
      ctx.fill();
    }

    // Legend
    const legendW = 130;
    const legendH = 80;
    const lx = w - LANE_PAD_RIGHT - legendW;
    const ly = 10;
    ctx.fillStyle = 'rgba(6,6,10,0.85)';
    ctx.strokeStyle = 'rgba(255,255,255,0.08)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.roundRect(lx, ly, legendW, legendH, 6);
    ctx.fill();
    ctx.stroke();

    ctx.font = '12px JetBrains Mono, monospace';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const statusLabels = ['success', 'running', 'failed'];
    const statusNames = ['Success', 'Running', 'Failed'];
    for (let i = 0; i < statusLabels.length; i++) {
      const ky = ly + 16 + i * 20;
      ctx.fillStyle = statusColor[statusLabels[i]];
      ctx.beginPath();
      ctx.arc(lx + 14, ky, 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = getCssVar('--text-soft') || '#c8b8c4';
      ctx.fillText(statusNames[i], lx + 26, ky);
    }
  }, [entries, height]);

  // Redraw on mount + resize
  useEffect(() => {
    draw();
    const ro = new ResizeObserver(() => draw());
    const container = containerRef.current;
    if (container) ro.observe(container);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div ref={containerRef} className="timeline-canvas">
      <canvas ref={canvasRef} style={{ cursor: 'crosshair' }} />
    </div>
  );
}
