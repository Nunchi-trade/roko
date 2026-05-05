import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import { fmtUptime } from '../../lib/format';
import { getCssVar } from '../../lib/color';
import FlatIcon from '../../components/FlatIcon';
import ExplorerTimeline, { drawSparkline, HeatmapStrip } from './ExplorerTimeline';
import ExplorerCards from './ExplorerCards';
import ExplorerDrawer from './ExplorerDrawer';
import type { HealthData, Episode, StateEvent } from './types';
import { getProviders } from './types';
import './Explorer.css';

/* ── Animated counter hook ── */

function useAnimatedValue(target: number, duration = 800): number {
  const [display, setDisplay] = useState(0);
  const prevTarget = useRef(target);
  const rafRef = useRef(0);

  useEffect(() => {
    const from = prevTarget.current;
    prevTarget.current = target;
    if (from === target) return;

    const start = performance.now();
    const tick = (now: number) => {
      const elapsed = now - start;
      const t = Math.min(elapsed / duration, 1);
      const ease = t >= 1 ? 1 : 1 - Math.pow(2, -10 * t);
      setDisplay(from + (target - from) * ease);
      if (t < 1) rafRef.current = requestAnimationFrame(tick);
    };
    rafRef.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafRef.current);
  }, [target, duration]);

  return display;
}

/* ── Main component ── */

export default function Explorer() {
  /* ── state ── */
  const [health, setHealth] = useState<HealthData | null>(null);
  const [episodes, setEpisodes] = useState<Episode[]>([]);
  const [events, setEvents] = useState<StateEvent[]>([]);
  const [expandedEp, setExpandedEp] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [hoveredEp, setHoveredEp] = useState<Episode | null>(null);
  const [initialLoading, setInitialLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  /* ── refs ── */
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const epSparkRef = useRef<HTMLCanvasElement>(null);
  const costSparkRef = useRef<HTMLCanvasElement>(null);
  const agentSparkRef = useRef<HTMLCanvasElement>(null);
  const gateSparkRef = useRef<HTMLCanvasElement>(null);
  const durSparkRef = useRef<HTMLCanvasElement>(null);

  const { get } = useLiveApi();

  /* ── data fetch ── */
  const refresh = useCallback(async () => {
    setRefreshing(true);
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
    } finally {
      setTimeout(() => setRefreshing(false), 1000);
    }
  }, [get]);

  useEffect(() => {
    refresh().finally(() => setInitialLoading(false));
    pollRef.current = setInterval(refresh, 10_000);
    return () => { if (pollRef.current) clearInterval(pollRef.current); };
  }, [refresh]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(refresh, 2000);
  useContextEventSubscription(
    ['episode', 'task_completed', 'gate_result', 'agent_spawned'],
    debouncedRefetch,
  );

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

  const animEpCount = useAnimatedValue(episodes.length);
  const animCost = useAnimatedValue(stats.totalCost);
  const animAgents = useAnimatedValue(stats.agentCount);
  const animGatePass = useAnimatedValue(stats.gatePass);
  const animDuration = useAnimatedValue(stats.avgDuration);

  const sparkData = useMemo(() => {
    if (episodes.length === 0) return { epBuckets: [], costBuckets: [], durBuckets: [], agentBuckets: [], gateBuckets: [] };

    const timestamps = episodes.map(e => e.timestamp_ms ?? 0).filter(t => t > 0);
    if (timestamps.length === 0) return { epBuckets: [], costBuckets: [], durBuckets: [], agentBuckets: [], gateBuckets: [] };

    const minT = Math.min(...timestamps);
    const maxT = Math.max(...timestamps);
    const bucketMs = 10 * 60 * 1000;
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

    for (let i = 1; i < costBuckets.length; i++) costBuckets[i] += costBuckets[i - 1];

    return {
      epBuckets,
      costBuckets,
      durBuckets,
      agentBuckets: agentSets.map(s => s.size),
      gateBuckets: gateBuckets.map((g, i) => gateTotal[i] > 0 ? (g / gateTotal[i]) * 100 : 100),
    };
  }, [episodes]);

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

  const handleToggleExpand = useCallback((id: string) => {
    setExpandedEp(prev => prev === id ? null : id);
  }, []);

  /* ── Render ── */

  return (
    <div className="explorer-page">

      {/* ── 1. Floating header ── */}
      <header className="expl-header">
        <span className="expl-title text-gradient-warm">
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
            <span className={`expl-pill-value ${health?.status === 'ok' ? 'expl-pill-value--online' : 'expl-pill-value--error'}`}>
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

        <button className="expl-refresh btn-interactive" onClick={refresh} title="Refresh data">
          <FlatIcon name="refresh" size={14} tone="rose" />
        </button>
      </header>

      {/* ── 2. Hero canvas: Swimlane Timeline ── */}
      <ExplorerTimeline
        episodes={episodes}
        hoveredEp={hoveredEp}
        onHoverEpisode={setHoveredEp}
      />

      {/* ── 3. Stat strip with sparklines ── */}
      <div className={`expl-stat-strip${episodes.length > 0 ? ' expl-stat-strip--active' : ''}${refreshing ? ' expl-refreshing' : ''}`}>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="database" size={14} tone="rose" />EPISODES</span>
          <span className="expl-stat-value expl-stat-value--episodes counter-animate">{Math.round(animEpCount)}</span>
          <canvas ref={epSparkRef} className="expl-spark" role="img" aria-label="Episodes sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="cost" size={14} tone="bone" />COST</span>
          <span className="expl-stat-value expl-stat-value--cost counter-animate">${animCost.toFixed(3)}</span>
          <canvas ref={costSparkRef} className="expl-spark" role="img" aria-label="Cost sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="agent" size={14} tone="dream" />AGENTS</span>
          <span className="expl-stat-value expl-stat-value--agents counter-animate">{Math.round(animAgents)}</span>
          <canvas ref={agentSparkRef} className="expl-spark" role="img" aria-label="Agent activity sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="gate" size={14} tone="success" />GATE PASS</span>
          <span className="expl-stat-value expl-stat-value--gate counter-animate">{animGatePass.toFixed(0)}%</span>
          <canvas ref={gateSparkRef} className="expl-spark" role="img" aria-label="Gate pass rate sparkline" />
        </div>
        <div className="expl-stat-pill">
          <span className="expl-stat-label"><FlatIcon name="duration" size={14} tone="rose" />AVG DURATION</span>
          <span className="expl-stat-value expl-stat-value--duration counter-animate">{animDuration.toFixed(1)}s</span>
          <canvas ref={durSparkRef} className="expl-spark" role="img" aria-label="Duration sparkline" />
        </div>
      </div>

      {/* ── 4. Activity heatmap ── */}
      <HeatmapStrip heatmapData={heatmapData} />

      {/* ── 5. Episode card grid ── */}
      <ExplorerCards
        episodes={episodes}
        expandedEp={expandedEp}
        onToggleExpand={handleToggleExpand}
        maxCostInSet={maxCostInSet}
        initialLoading={initialLoading}
        refreshing={refreshing}
      />

      {/* ── 6. Bottom drawer (providers + events) ── */}
      <ExplorerDrawer
        drawerOpen={drawerOpen}
        onToggle={() => setDrawerOpen(!drawerOpen)}
        provEntries={provEntries}
        events={events}
      />
    </div>
  );
}
