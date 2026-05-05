import { useEffect, useRef, useMemo } from 'react';
import { MetricStrip } from '../layout/MetricStrip';
import Pane from '../Pane';
import { ProviderTable } from './ProviderTable';
import { HealthMosaic } from './HealthMosaic';
import { getCssVar } from '../../lib/color';
import './StatusTab.css';

/* ── Types ────────────────────────────────────────────── */

export interface HealthData {
  status: string;
  uptime_secs?: number;
  version?: string;
  active_plans?: number;
  active_agents?: number;
  active_runs?: number;
  providers?: Record<string, { healthy: boolean; latency_ms?: number }>;
  c_factor?: {
    cost_efficiency: number;
    quality_score: number;
    velocity: number;
    composite: number;
  };
}

interface EpisodeSummary {
  timestamp_ms?: number;
  usage?: { cost_usd?: number };
  duration_secs?: number;
  agent_id?: string;
  gate_verdicts?: Array<{ gate: string; passed: boolean }>;
}

export interface StatusTabProps {
  health: HealthData | null;
  loading?: boolean;
  episodes?: EpisodeSummary[];
}

/* ── Helpers ──────────────────────────────────────────── */

function fmtPct(n: number): string { return `${(n * 100).toFixed(1)}%`; }

function drawSparkline(canvas: HTMLCanvasElement, data: number[], color: string) {
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

  ctx.lineTo((data.length - 1) * step, h);
  ctx.lineTo(0, h);
  ctx.closePath();
  if (color.startsWith('#')) {
    const r = parseInt(color.slice(1, 3), 16);
    const g = parseInt(color.slice(3, 5), 16);
    const b = parseInt(color.slice(5, 7), 16);
    ctx.fillStyle = `rgba(${r},${g},${b},0.10)`;
  } else {
    ctx.fillStyle = color.replace(')', ', 0.08)').replace('rgb', 'rgba');
  }
  ctx.fill();
}

/* ── Derived data ─────────────────────────────────────── */

function computeStats(episodes: EpisodeSummary[]) {
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
}

function computeSparkData(episodes: EpisodeSummary[]) {
  const empty = { epBuckets: [] as number[], costBuckets: [] as number[], durBuckets: [] as number[], agentBuckets: [] as number[], gateBuckets: [] as number[] };
  if (episodes.length === 0) return empty;

  const timestamps = episodes.map(e => e.timestamp_ms ?? 0).filter(t => t > 0);
  if (timestamps.length === 0) return empty;

  const minT = Math.min(...timestamps);
  const maxT = Math.max(...timestamps);
  const bucketMs = 10 * 60 * 1000;
  const numBuckets = Math.max(Math.ceil((maxT - minT) / bucketMs), 8);

  const epBuckets = new Array<number>(numBuckets).fill(0);
  const costBuckets = new Array<number>(numBuckets).fill(0);
  const durBuckets = new Array<number>(numBuckets).fill(0);
  const agentSets: Set<string>[] = Array.from({ length: numBuckets }, () => new Set());
  const gateBuckets = new Array<number>(numBuckets).fill(0);
  const gateTotal = new Array<number>(numBuckets).fill(0);

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
  for (let i = 1; i < costBuckets.length; i++) costBuckets[i] += costBuckets[i - 1];

  return {
    epBuckets,
    costBuckets,
    durBuckets,
    agentBuckets: agentSets.map(s => s.size),
    gateBuckets: gateBuckets.map((g, i) => gateTotal[i] > 0 ? (g / gateTotal[i]) * 100 : 100),
  };
}

function getProviders(health: HealthData): Record<string, { healthy: boolean; latency_ms?: number }> {
  const prov = health.providers;
  if (prov && typeof prov === 'object') {
    const keys = Object.keys(prov);
    if (keys.length > 0 && keys.some((k) => k !== 'healthy' && k !== 'total' && k !== 'unhealthy')) {
      return prov;
    }
  }
  return {};
}

/* ── Component ────────────────────────────────────────── */

export function StatusTab({ health, loading, episodes = [] }: StatusTabProps) {
  const epSparkRef = useRef<HTMLCanvasElement>(null);
  const costSparkRef = useRef<HTMLCanvasElement>(null);
  const agentSparkRef = useRef<HTMLCanvasElement>(null);
  const gateSparkRef = useRef<HTMLCanvasElement>(null);
  const durSparkRef = useRef<HTMLCanvasElement>(null);

  const stats = useMemo(() => computeStats(episodes), [episodes]);
  const sparkData = useMemo(() => computeSparkData(episodes), [episodes]);

  // Draw sparklines
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

  if (loading) return <div className="status-tab__loading">Loading...</div>;
  if (!health) return <div className="status-tab__error">Failed to load health data</div>;

  const providers = getProviders(health);

  return (
    <div className="status-tab">
      {/* Hero mosaic */}
      <HealthMosaic health={health} />

      {/* Stat strip with sparklines */}
      <div className="status-tab__sparkstrip">
        <div className="status-tab__sparkpill">
          <span className="status-tab__sparklabel">EPISODES</span>
          <span className="status-tab__sparkvalue" style={{ color: 'var(--rose-bright)' }}>{episodes.length}</span>
          <canvas ref={epSparkRef} className="status-tab__spark" />
        </div>
        <div className="status-tab__sparkpill">
          <span className="status-tab__sparklabel">COST</span>
          <span className="status-tab__sparkvalue" style={{ color: 'var(--bone)' }}>${stats.totalCost.toFixed(3)}</span>
          <canvas ref={costSparkRef} className="status-tab__spark" />
        </div>
        <div className="status-tab__sparkpill">
          <span className="status-tab__sparklabel">AGENTS</span>
          <span className="status-tab__sparkvalue" style={{ color: 'var(--dream-bright)' }}>{stats.agentCount}</span>
          <canvas ref={agentSparkRef} className="status-tab__spark" />
        </div>
        <div className="status-tab__sparkpill">
          <span className="status-tab__sparklabel">GATE PASS</span>
          <span className="status-tab__sparkvalue" style={{ color: 'var(--success)' }}>{stats.gatePass.toFixed(0)}%</span>
          <canvas ref={gateSparkRef} className="status-tab__spark" />
        </div>
        <div className="status-tab__sparkpill">
          <span className="status-tab__sparklabel">AVG DUR</span>
          <span className="status-tab__sparkvalue" style={{ color: 'var(--rose-glow)' }}>{stats.avgDuration.toFixed(1)}s</span>
          <canvas ref={durSparkRef} className="status-tab__spark" />
        </div>
      </div>

      {/* C-factor strip */}
      {health.c_factor && (
        <MetricStrip
          metrics={[
            { label: 'Cost Eff.', value: health.c_factor.cost_efficiency, format: fmtPct },
            { label: 'Quality', value: health.c_factor.quality_score, format: fmtPct },
            { label: 'Velocity', value: health.c_factor.velocity, format: fmtPct },
            { label: 'Composite', value: health.c_factor.composite, format: fmtPct },
          ]}
          className="status-tab__cfactor"
        />
      )}

      {/* Provider health table */}
      {Object.keys(providers).length > 0 && (
        <Pane title="Providers" className="status-tab__providers" flat>
          <ProviderTable providers={providers} />
        </Pane>
      )}
    </div>
  );
}
