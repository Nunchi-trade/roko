import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useLiveApi } from '../hooks/useLiveApi';
import { fmtUptime, relativeTime } from '../lib/format';
import { getCssVar } from '../lib/color';
import FlatIcon, { inferIcon } from '../components/FlatIcon';
import './Explorer.css';

/* ═══════════════════════════════════════════════════════════
   TYPES
   ═══════════════════════════════════════════════════════════ */

interface HealthData {
  status: string;
  uptime_secs?: number;
  version?: string;
  active_plans?: number;
  active_agents?: number;
  active_runs?: number;
  providers?: Record<string, { healthy: boolean; latency_ms?: number }>;
}

interface Episode {
  id: string;
  kind: string;
  agent_id?: string;
  task_id?: string;
  model?: string;
  status?: string;
  success?: boolean;
  usage?: { cost_usd?: number; input_tokens?: number; output_tokens?: number };
  timestamp_ms?: number;
  duration_secs?: number;
  turns?: number;
  gate_verdicts?: Array<{ gate: string; passed: boolean }>;
  [key: string]: unknown;
}

interface StateEvent {
  type: string;
  payload: unknown;
  timestamp: string;
}

/* ═══════════════════════════════════════════════════════════
   CONSTANTS
   ═══════════════════════════════════════════════════════════ */

const KIND_COLORS: Record<string, string> = {
  agent_turn: 'var(--rose)',       // --rose
  gate_result: 'var(--success)',   // --success
  tool_call: 'var(--bone)',        // --bone
  plan_step: 'var(--dream)',       // --dream
};

const KIND_LABELS: Record<string, string> = {
  agent_turn: 'Agent Turn',
  gate_result: 'Gate Result',
  tool_call: 'Tool Call',
  plan_step: 'Plan Step',
};

const HERO_HEIGHT = 360;
const LANE_PAD_TOP = 48;
const LANE_PAD_BOTTOM = 40;
const LANE_PAD_LEFT = 120;
const LANE_PAD_RIGHT = 28;
const LEGEND_W = 140;
const LEGEND_H = 100;
const MIN_BLOCK_W = 8;
const BLOCK_H = 22;
const BLOCK_R = 4;

/* ═══════════════════════════════════════════════════════════
   HELPERS
   ═══════════════════════════════════════════════════════════ */

function getProviders(health: HealthData | null): Record<string, { healthy: boolean }> {
  if (!health) return {};
  const prov = health.providers;
  if (prov && typeof prov === 'object') {
    const keys = Object.keys(prov);
    if (keys.length > 0 && keys.some((k) => k !== 'healthy' && k !== 'total' && k !== 'unhealthy')) {
      return prov as Record<string, { healthy: boolean }>;
    }
    return {};
  }
  return {};
}

function safeTimestamp(ts: unknown): string {
  if (!ts) return '';
  try {
    const d = new Date(ts as string | number);
    return isNaN(d.getTime()) ? '' : d.toLocaleTimeString();
  } catch {
    return '';
  }
}

function safePayload(payload: unknown): string {
  try {
    const s = JSON.stringify(payload);
    return s.length > 140 ? s.slice(0, 140) + '...' : s;
  } catch {
    return String(payload ?? '');
  }
}

/** Resolve kind to a CSS var for inline styles. */
function kindColor(kind: string): string {
  return KIND_COLORS[kind] ?? 'var(--dream)';
}

/** Resolve kind to computed color for canvas 2D contexts. */
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

/* ═══════════════════════════════════════════════════════════
   SPARKLINE (tiny inline canvas)
   ═══════════════════════════════════════════════════════════ */

function drawSparkline(
  canvas: HTMLCanvasElement,
  data: number[],
  color: string,
) {
  const ctx = canvas.getContext('2d');
  if (!ctx || data.length < 2) return;

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  canvas.width = w * dpr;
  canvas.height = h * dpr;
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, w, h);

  const max = Math.max(...data, 1);
  const step = w / (data.length - 1);

  ctx.beginPath();
  ctx.moveTo(0, h - (data[0] / max) * h * 0.85);
  for (let i = 1; i < data.length; i++) {
    ctx.lineTo(i * step, h - (data[i] / max) * h * 0.85);
  }
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.lineJoin = 'round';
  ctx.stroke();

  // fill under
  ctx.lineTo((data.length - 1) * step, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  ctx.fillStyle = color.replace(')', ', 0.08)').replace('rgb', 'rgba');
  // handle hex colors
  if (color.startsWith('#')) {
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    ctx.fillStyle = `rgba(${r},${g},${b},0.10)`;
  }
  ctx.fill();
}

/* ═══════════════════════════════════════════════════════════
   COMPONENT
   ═══════════════════════════════════════════════════════════ */

export default function Explorer() {
  /* ── state ── */
  const [health, setHealth] = useState<HealthData | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [events, setEvents] = useState<StateEvent[]>([]);
  const [expandedEp, setExpandedEp] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [hoveredEp, setHoveredEp] = useState<Episode | null>(null);

  /* ── refs ── */
  const heroRef = useRef<HTMLCanvasElement>(null);
  const mouseRef = useRef<{ x: number; y: number }>({ x: -1, y: -1 });
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const epSparkRef = useRef<HTMLCanvasElement>(null);
  const costSparkRef = useRef<HTMLCanvasElement>(null);
  const agentSparkRef = useRef<HTMLCanvasElement>(null);
  const gateSparkRef = useRef<HTMLCanvasElement>(null);
  const durSparkRef = useRef<HTMLCanvasElement>(null);

  // Keep a ref for episode rects so hover detection works outside draw
  const epRectsRef = useRef<Array<{ x: number; y: number; w: number; h: number; ep: Episode }>>([]);

  const { get } = useLiveApi();

  /* ── data fetch ── */
  const refresh = useCallback(async () => {
    try {
      const [h, eps, evts] = await Promise.allSettled([
        get<HealthData>('/api/health'),
        get<Episode[]>('/api/episodes'),
        get<StateEvent[]>('/api/statehub/events'),
      ]);
      if (h.status === 'fulfilled') setHealth(h.value);
      if (eps.status === 'fulfilled') setEpisodes(Array.isArray(eps.value) ? eps.value : []);
      if (evts.status === 'fulfilled') setEvents(Array.isArray(evts.value) ? evts.value : []);
    } catch {
      // API may not be available
    }
  }, [get]);

  useEffect(() => {
    refresh();
    pollRef.current = setInterval(refresh, 10_000);
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, [refresh]);

  /* ── derived data ── */
  const providers = getProviders(health);
  const provEntries = Object.entries(providers);

  const stats = useMemo(() => {
    const totalCost = episodes.reduce((s, e) => s + (e.usage?.cost_usd ?? 0), 0);
    const agents = new Set(episodes.map(e => e.agent_id).filter(Boolean));
    const gateVerdicts = episodes.flatMap(e => e.gate_verdicts ?? []);
    const gatePass = gateVerdicts.length > 0
      ? (gateVerdicts.filter(v => v.passed).length / gateVerdicts.length) * 100
      : 100;
    const durations = episodes.filter(e => e.duration_secs != null).map(e => e.duration_secs!);
    const avgDuration = durations.length > 0
      ? durations.reduce((s, d) => s + d, 0) / durations.length
      : 0;
    return { totalCost, agentCount: agents.size, gatePass, avgDuration };
  }, [episodes]);

  // Sparkline data: bucket episodes into 10-min intervals
  const sparkData = useMemo(() => {
    if (episodes.length === 0) return { epBuckets: [], costBuckets: [], durBuckets: [], agentBuckets: [], gateBuckets: [] };

    const timestamps = episodes.map(e => e.timestamp_ms ?? 0).filter(t => t > 0);
    if (timestamps.length === 0) return { epBuckets: [], costBuckets: [], durBuckets: [], agentBuckets: [], gateBuckets: [] };

    const minT = Math.min(...timestamps);
    const maxT = Math.max(...timestamps);
    const bucketMs = 10 * 60 * 1000; // 10 minutes
    const numBuckets = Math.max(Math.ceil((maxT - minT) / bucketMs), 8);

    const epBuckets = new Array(numBuckets).fill(0);
    const costBuckets = new Array(numBuckets).fill(0);
    const durBuckets = new Array(numBuckets).fill(0);
    const agentSets: Set<string>[] = Array.from({ length: numBuckets }, () => new Set());
    const gateBuckets = new Array(numBuckets).fill(0);
    const gateTotal = new Array(numBuckets).fill(0);

    for (const ep of episodes) {
      const t = ep.timestamp_ms ?? 0;
      if (t <= 0) continue;
      const idx = Math.min(Math.floor((t - minT) / bucketMs), numBuckets - 1);
      epBuckets[idx]++;
      costBuckets[idx] += ep.usage?.cost_usd ?? 0;
      durBuckets[idx] += ep.duration_secs ?? 0;
      if (ep.agent_id) agentSets[idx].add(ep.agent_id);
      for (const v of ep.gate_verdicts ?? []) {
        gateTotal[idx]++;
        if (v.passed) gateBuckets[idx]++;
      }
    }

    // cumulative cost
    for (let i = 1; i < costBuckets.length; i++) costBuckets[i] += costBuckets[i - 1];

    return {
      epBuckets,
      costBuckets,
      durBuckets,
      agentBuckets: agentSets.map(s => s.size),
      gateBuckets: gateBuckets.map((g, i) => gateTotal[i] > 0 ? (g / gateTotal[i]) * 100 : 100),
    };
  }, [episodes]);

  // Heatmap: 24 hours
  const heatmapData = useMemo(() => {
    const hours = new Array(24).fill(0);
    for (const ep of episodes) {
      if (ep.timestamp_ms) {
        const h = new Date(ep.timestamp_ms).getHours();
        hours[h]++;
      }
    }
    return hours;
  }, [episodes]);

  const maxCostInSet = useMemo(() => {
    return Math.max(...episodes.map(e => e.usage?.cost_usd ?? 0), 0.001);
  }, [episodes]);

  /* ── hero canvas draw ── */
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
    const timeSpan = Math.max(maxT - minT, 60000); // at least 1 minute
    const drawW = w - LANE_PAD_LEFT - LANE_PAD_RIGHT;

    // Helper: time to x
    const tx = (t: number) => LANE_PAD_LEFT + ((t - minT) / timeSpan) * drawW;
    // Helper: agent to y
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

      // Glow for hovered
      const isHovered = hoveredEp?.id === ep.id;
      if (isHovered) {
        ctx.shadowColor = color;
        ctx.shadowBlur = 12;
      }

      ctx.fillStyle = isHovered ? color : color + 'cc';
      ctx.beginPath();
      ctx.roundRect(x, y, bw, BLOCK_H, BLOCK_R);
      ctx.fill();

      // Inner shine
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

  /* ── canvas effects ── */
  useEffect(() => {
    drawTimeline();
  }, [drawTimeline]);

  // ResizeObserver for hero canvas
  useEffect(() => {
    const canvas = heroRef.current;
    if (!canvas) return;

    const ro = new ResizeObserver(() => drawTimeline());
    ro.observe(canvas);
    return () => ro.disconnect();
  }, [drawTimeline]);

  // Draw sparklines whenever data changes
  useEffect(() => {
    if (epSparkRef.current && sparkData.epBuckets.length >= 2) {
      drawSparkline(epSparkRef.current, sparkData.epBuckets, getCssVar('--rose'));
    }
    if (costSparkRef.current && sparkData.costBuckets.length >= 2) {
      drawSparkline(costSparkRef.current, sparkData.costBuckets, getCssVar('--bone'));
    }
    if (agentSparkRef.current && sparkData.agentBuckets.length >= 2) {
      drawSparkline(agentSparkRef.current, sparkData.agentBuckets, getCssVar('--dream'));
    }
    if (gateSparkRef.current && sparkData.gateBuckets.length >= 2) {
      drawSparkline(gateSparkRef.current, sparkData.gateBuckets, getCssVar('--success'));
    }
    if (durSparkRef.current && sparkData.durBuckets.length >= 2) {
      drawSparkline(durSparkRef.current, sparkData.durBuckets, getCssVar('--rose-bright'));
    }
  }, [sparkData]);

  /* ── hero mouse tracking ── */
  const handleHeroMouse = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = heroRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    mouseRef.current = { x, y };

    // Hit test
    let found: Episode | null = null;
    for (const r of epRectsRef.current) {
      if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) {
        found = r.ep;
        break;
      }
    }
    setHoveredEp(found);
  }, []);

  const handleHeroLeave = useCallback(() => {
    mouseRef.current = { x: -1, y: -1 };
    setHoveredEp(null);
  }, []);

  /* ── heatmap helpers ── */
  const heatmapMax = Math.max(...heatmapData, 1);
  function heatColor(count: number): string {
    if (count === 0) return 'var(--bg-void)';
    const ratio = count / heatmapMax;
    if (ratio < 0.25) return 'var(--rose-deep)';
    if (ratio < 0.5) return 'var(--rose-dim)';
    if (ratio < 0.75) return 'var(--rose)';
    return 'var(--rose-bright)';
  }

  /* ═══════════════════════════════════════════════════════════
     RENDER
     ═══════════════════════════════════════════════════════════ */

  return (
    <div className="explorer-page">

      {/* ── 1. Floating header ── */}
      <header className="expl-header">
        <span className="expl-title">
          <FlatIcon name="explorer" size={18} tone="rose" />
          Explorer
        </span>
        <span className="expl-live-badge">
          <FlatIcon name="status" size={13} tone={health?.status === 'ok' ? 'success' : 'warning'} />
          LIVE
        </span>

        <div className="expl-header-pills">
          <span className="expl-pill">
            <FlatIcon name="status" size={13} tone={health?.status === 'ok' ? 'success' : 'warning'} />
            <span className="expl-pill-label">Status</span>
            <span className="expl-pill-value" style={{ color: health?.status === 'ok' ? 'var(--success)' : 'var(--rose-bright)' }}>
              {health?.status === 'ok' ? 'online' : (health?.status ?? 'ok')}
            </span>
          </span>
          <span className="expl-pill">
            <FlatIcon name="clock" size={13} tone="dream" />
            <span className="expl-pill-label">Uptime</span>
            <span className="expl-pill-value">{fmtUptime(health?.uptime_secs ?? 0)}</span>
          </span>
          <span className="expl-pill">
            <FlatIcon name="hash" size={13} tone="muted" />
            <span className="expl-pill-label">Version</span>
            <span className="expl-pill-value">{health?.version ?? '0.1.0'}</span>
          </span>
          <span className="expl-pill">
            <FlatIcon name="agent" size={13} tone="dream" />
            <span className="expl-pill-label">Agents</span>
            <span className="expl-pill-value">{health?.active_agents ?? 0}</span>
          </span>
          <span className="expl-pill">
            <FlatIcon name="task" size={13} tone="bone" />
            <span className="expl-pill-label">Plans</span>
            <span className="expl-pill-value">{health?.active_plans ?? 0}</span>
          </span>
        </div>

        <button className="expl-refresh" onClick={refresh} title="Refresh data">
          <FlatIcon name="refresh" size={14} tone="rose" />
        </button>
      </header>

      {/* ── 2. Hero canvas: Swimlane Timeline ── */}
      <div className="expl-hero-wrap">
        <canvas
          ref={heroRef}
          className="expl-hero-canvas"
          role="img"
          aria-label="Signal flow graph"
          style={{ width: '100%', height: HERO_HEIGHT }}
          onMouseMove={handleHeroMouse}
          onMouseLeave={handleHeroLeave}
        />
      </div>

      {/* ── 3. Stat strip with sparklines ── */}
      <div className="expl-stat-strip">
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="database" size={14} tone="rose" />EPISODES</span>
          <span className="expl-stat-value" style={{ color: 'var(--rose-bright)' }}>{episodes.length}</span>
          <canvas ref={epSparkRef} className="expl-spark" role="img" aria-label="Episodes sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="cost" size={14} tone="bone" />COST</span>
          <span className="expl-stat-value" style={{ color: 'var(--bone)' }}>${stats.totalCost.toFixed(3)}</span>
          <canvas ref={costSparkRef} className="expl-spark" role="img" aria-label="Cost sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="agent" size={14} tone="dream" />AGENTS</span>
          <span className="expl-stat-value" style={{ color: 'var(--dream-bright)' }}>{stats.agentCount}</span>
          <canvas ref={agentSparkRef} className="expl-spark" role="img" aria-label="Agent activity sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="gate" size={14} tone="success" />GATE PASS</span>
          <span className="expl-stat-value" style={{ color: 'var(--success)' }}>{stats.gatePass.toFixed(0)}%</span>
          <canvas ref={gateSparkRef} className="expl-spark" role="img" aria-label="Gate pass rate sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="duration" size={14} tone="rose" />AVG DURATION</span>
          <span className="expl-stat-value" style={{ color: 'var(--rose-glow)' }}>{stats.avgDuration.toFixed(1)}s</span>
          <canvas ref={durSparkRef} className="expl-spark" role="img" aria-label="Duration sparkline" />
        </div>
      </div>

      {/* ── 4. Activity heatmap ── */}
      <div className="expl-heatmap-section">
        <span className="expl-section-label"><FlatIcon name="activity" size={15} tone="rose" />ACTIVITY DENSITY</span>
        <div className="expl-heatmap">
          {heatmapData.map((count, hour) => (
            <div
              key={hour}
              className="expl-heat-cell"
              style={{ background: heatColor(count) }}
              title={`${hour}:00 - ${count} episode${count !== 1 ? 's' : ''}`}
            >
              {(hour === 0 || hour === 6 || hour === 12 || hour === 18 || hour === 23) && (
                <span className="expl-heat-label">{hour}</span>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* ── 5. Episode card grid ── */}
      <div className="expl-cards-section">
        {episodes.length === 0 ? (
          <div className="expl-empty">No episodes recorded yet</div>
        ) : (
          <div className="expl-card-grid">
            {episodes.slice(0, 30).map((ep, i) => (
              <div
                key={ep.id}
                className={`expl-card${expandedEp === ep.id ? ' expl-card--expanded' : ''}`}
                style={{ animationDelay: `${i * 40}ms` }}
                role="button"
                tabIndex={0}
                onClick={() => setExpandedEp(expandedEp === ep.id ? null : ep.id)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    setExpandedEp(expandedEp === ep.id ? null : ep.id);
                  }
                }}
              >
                {/* Top row: badges + time */}
                <div className="expl-card-top">
                  <span className="expl-card-kind" style={{ background: kindColor(ep.kind) + '22', color: kindColor(ep.kind) }}>
                    <FlatIcon name={inferIcon(ep.kind)} size={13} tone="muted" />
                    {ep.kind}
                  </span>
                  {ep.model && (
                    <span className="expl-card-model"><FlatIcon name="model" size={13} tone="dream" />{ep.model}</span>
                  )}
                  <span className="expl-card-time">
                    <FlatIcon name="clock" size={13} tone="muted" />
                    {ep.timestamp_ms ? relativeTime(ep.timestamp_ms) : ''}
                  </span>
                </div>

                {/* Agent name */}
                <div className="expl-card-agent"><FlatIcon name="agent" size={14} tone="rose" />{ep.agent_id ?? 'system'}</div>

                {/* Task ID */}
                {ep.task_id && (
                  <div className="expl-card-task"><FlatIcon name="task" size={14} tone="bone" />{ep.task_id}</div>
                )}

                {/* Gate verdict dots */}
                {ep.gate_verdicts && ep.gate_verdicts.length > 0 && (
                  <div className="expl-card-gates">
                    {ep.gate_verdicts.map((v, gi) => (
                      <span
                        key={gi}
                        className={`expl-gate-dot ${v.passed ? 'pass' : 'fail'}`}
                        title={`${v.gate}: ${v.passed ? 'passed' : 'failed'}`}
                      />
                    ))}
                  </div>
                )}

                {/* Meta chips */}
                <div className="expl-card-meta">
                  {ep.usage?.cost_usd != null && (
                    <span className="expl-chip cost"><FlatIcon name="cost" size={12} tone="bone" />${ep.usage.cost_usd.toFixed(3)}</span>
                  )}
                  {ep.duration_secs != null && (
                    <span className="expl-chip dur"><FlatIcon name="duration" size={12} tone="muted" />{ep.duration_secs.toFixed(1)}s</span>
                  )}
                  {ep.turns != null && (
                    <span className="expl-chip turns"><FlatIcon name="route" size={12} tone="dream" />{ep.turns}t</span>
                  )}
                </div>

                {/* Cost bar */}
                <div className="expl-card-bar-wrap">
                  <div
                    className="expl-card-bar"
                    style={{ width: `${Math.max(((ep.usage?.cost_usd ?? 0) / maxCostInSet) * 100, 2)}%` }}
                  />
                </div>

                {/* Expanded detail */}
                {expandedEp === ep.id && (
                  <div className="expl-card-detail">
                    {Object.entries(ep).map(([k, v]) => (
                      <div key={k} className="expl-card-field">
                        <span className="expl-card-field-key">{k}</span>
                        <span className="expl-card-field-val">
                          {typeof v === 'object' ? JSON.stringify(v) : String(v ?? '')}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* ── 6. Bottom drawer (providers + events) ── */}
      <div className={`expl-drawer${drawerOpen ? ' open' : ''}`}>
        <button
          className="expl-drawer-handle"
          onClick={() => setDrawerOpen(!drawerOpen)}
          aria-label={drawerOpen ? 'Collapse drawer' : 'Expand drawer'}
        >
          <span className="expl-drawer-bar" />
          <span className="expl-drawer-hint">{drawerOpen ? 'Collapse' : 'Providers & Events'}</span>
        </button>

        <div className="expl-drawer-body">
          {/* Left: Provider health */}
          <div className="expl-drawer-providers">
            <div className="expl-section-label"><FlatIcon name="provider" size={15} tone="success" />PROVIDER HEALTH</div>
            {provEntries.length === 0 ? (
              <div className="expl-empty">No providers configured</div>
            ) : (
              <div className="expl-provider-list">
                {provEntries.map(([name, info]) => (
                  <div key={name} className={`provider-card${info.healthy ? ' provider-card--healthy' : ''}`}>
                    <FlatIcon name="provider" size={14} tone={info.healthy ? 'success' : 'warning'} />
                    <span className="provider-name">{name}</span>
                    <span className={`provider-badge ${info.healthy ? 'ok' : 'down'}`}>
                      {info.healthy ? 'ok' : 'down'}
                    </span>
                  </div>
                ))}
              </div>
            )}
          </div>

          {/* Right: Event stream */}
          <div className="expl-drawer-events">
            <div className="expl-section-label"><FlatIcon name="event" size={15} tone="dream" />EVENT STREAM</div>
            <div className="expl-event-list">
              {events.length === 0 ? (
                <div className="expl-empty">No events recorded yet</div>
              ) : (
                events.slice(0, 16).map((evt, i) => (
                  <div key={`${evt?.type ?? 'evt'}-${i}`} className="expl-event-item">
                    <span className="expl-event-ts">{safeTimestamp(evt?.timestamp)}</span>
                    <span className="expl-event-badge"><FlatIcon name={inferIcon(evt?.type ?? 'event')} size={12} tone="muted" />{evt?.type ?? 'unknown'}</span>
                    <span className="expl-event-payload">{safePayload(evt?.payload)}</span>
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
