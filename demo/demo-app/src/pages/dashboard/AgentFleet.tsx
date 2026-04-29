import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { hexToRgba } from '../../lib/color';
import { ROLE_COLORS } from '../../lib/palette';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import AgentTopology, {
  type TopologyNode,
  type TopologyEdge,
} from '../../components/Spectre/AgentTopology';
import { identityFromAgent } from '../../components/Spectre/AgentIdentity';
import './dashboard.css';
import './AgentFleet.css';

/* ── Types ───────────────────────────────────────────────── */

interface Agent {
  id: string;
  label?: string;
  status: string;
  model?: string;
  model_profile?: { provider?: string; key?: string; context_window?: number };
  capabilities?: string[];
  reputation?: number;
  domain_tags?: string[];
  performance?: { completed_tasks?: number; failed_tasks?: number; active_tasks?: number; reputation?: number };
  costs?: { cumulative_usd?: number | null };
  last_seen_at?: number;
  process_id?: number;
}

function agentDisplayName(a: Agent): string {
  return a.label || a.id;
}

function agentTasks(a: Agent): number {
  return a.performance?.completed_tasks ?? 0;
}

function agentCost(a: Agent): number {
  return a.costs?.cumulative_usd ?? 0;
}

function isAgentActive(a: Agent): boolean {
  return a.status === 'active' || a.status === 'registered';
}

function fmtLastSeen(a: Agent): string {
  if (!a.last_seen_at) return 'unknown';
  const diff = Math.floor(Date.now() / 1000) - a.last_seen_at;
  if (diff < 60) return 'just now';
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

/* ── Topology types ─────────────────────────────────────── */

interface TopoNode {
  agent_id: string;
  role: string;
  endpoints: string[];
}

interface TopoEdge {
  from: string;
  to: string;
  weight?: number;
}

interface TopoData {
  nodes: TopoNode[];
  edges: TopoEdge[];
}

interface SimNode {
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface SimState {
  positions: SimNode[];
  frame: number;
  width: number;
  height: number;
}

const EMPTY_TOPOLOGY: TopoData = { nodes: [], edges: [] };

function sameTopology(a: TopoData, b: TopoData): boolean {
  if (a === b) return true;
  if (a.nodes.length !== b.nodes.length || a.edges.length !== b.edges.length) return false;

  for (let i = 0; i < a.nodes.length; i++) {
    const left = a.nodes[i];
    const right = b.nodes[i];
    if (
      left.agent_id !== right.agent_id ||
      left.role !== right.role ||
      left.endpoints.length !== right.endpoints.length
    ) {
      return false;
    }

    for (let j = 0; j < left.endpoints.length; j++) {
      if (left.endpoints[j] !== right.endpoints[j]) return false;
    }
  }

  for (let i = 0; i < a.edges.length; i++) {
    const left = a.edges[i];
    const right = b.edges[i];
    if (
      left.from !== right.from ||
      left.to !== right.to ||
      (left.weight ?? 1) !== (right.weight ?? 1)
    ) {
      return false;
    }
  }

  return true;
}

/* ── Force-directed graph canvas ────────────────────────── */

function TopologyGraph({ data, height = 280 }: { data: TopoData; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const simRef = useRef<SimState | null>(null);
  const rafRef = useRef<number | null>(null);
  const runIdRef = useRef(0);

  const stopAnimation = useCallback(() => {
    if (rafRef.current !== null) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  }, []);

  const draw = useCallback(function draw(runId = runIdRef.current) {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;

    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const cx = w / 2;
    const cy = h / 2;
    const nodeRadius = 20;
    const margin = 36;

    if (data.nodes.length === 0) {
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = 'rgba(9, 11, 15, 0.76)';
      ctx.fillRect(0, 0, w, h);
      ctx.fillStyle = 'rgba(194, 184, 201, 0.7)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('topology unavailable', cx, cy);
      return;
    }

    const nodeIndex = new Map<string, number>();
    data.nodes.forEach((node, index) => nodeIndex.set(node.agent_id, index));

    if (
      !simRef.current ||
      simRef.current.positions.length !== data.nodes.length ||
      simRef.current.width !== w ||
      simRef.current.height !== h
    ) {
      simRef.current = {
        width: w,
        height: h,
        frame: 0,
        positions: data.nodes.map((_, index) => {
          const angle = (index / data.nodes.length) * Math.PI * 2 - Math.PI / 2;
          const radius = Math.min(w, h) * 0.35;
          return {
            x: cx + Math.cos(angle) * radius,
            y: cy + Math.sin(angle) * radius,
            vx: 0,
            vy: 0,
          };
        }),
      };
    }

    const sim = simRef.current;
    if (!sim) return;

    const renderFrame = function renderFrame() {
      if (runId !== runIdRef.current) return;

      const positions = sim.positions;

      for (let tick = 0; tick < 3; tick++) {
        const damping = 0.86;
        const springLength = Math.max(70, Math.min(w, h) * 0.25);
        const springStrengthBase = 0.0036;
        const chargeStrength = 2200;
        const centerPull = 0.0018;
        const maxSpeed = 7.5;

        for (const point of positions) {
          point.vx *= damping;
          point.vy *= damping;
        }

        for (let i = 0; i < positions.length; i++) {
          for (let j = i + 1; j < positions.length; j++) {
            let dx = positions[j].x - positions[i].x;
            let dy = positions[j].y - positions[i].y;
            const distSq = Math.max(dx * dx + dy * dy, 36);
            const dist = Math.sqrt(distSq);
            const force = chargeStrength / distSq;
            const fx = (dx / dist) * force;
            const fy = (dy / dist) * force;

            positions[i].vx -= fx;
            positions[i].vy -= fy;
            positions[j].vx += fx;
            positions[j].vy += fy;

            const minDist = nodeRadius * 2.1;
            if (dist < minDist) {
              const overlap = minDist - dist;
              const push = overlap * 0.025;
              positions[i].vx -= (dx / dist) * push;
              positions[i].vy -= (dy / dist) * push;
              positions[j].vx += (dx / dist) * push;
              positions[j].vy += (dy / dist) * push;
            }
          }
        }

        for (const edge of data.edges) {
          const sourceIndex = nodeIndex.get(edge.from);
          const targetIndex = nodeIndex.get(edge.to);
          if (sourceIndex === undefined || targetIndex === undefined) continue;

          const source = positions[sourceIndex];
          const target = positions[targetIndex];
          let dx = target.x - source.x;
          let dy = target.y - source.y;
          const dist = Math.max(Math.sqrt(dx * dx + dy * dy), 1);
          const weight = edge.weight ?? 1;
          const targetLength = Math.max(58, springLength - weight * 4);
          const stiffness = springStrengthBase + weight * 0.00045;
          const force = (dist - targetLength) * stiffness;
          const fx = (dx / dist) * force;
          const fy = (dy / dist) * force;

          source.vx += fx;
          source.vy += fy;
          target.vx -= fx;
          target.vy -= fy;
        }

        for (const point of positions) {
          point.vx += (cx - point.x) * centerPull;
          point.vy += (cy - point.y) * centerPull;

          point.vx = Math.max(-maxSpeed, Math.min(maxSpeed, point.vx));
          point.vy = Math.max(-maxSpeed, Math.min(maxSpeed, point.vy));

          point.x = Math.max(margin, Math.min(w - margin, point.x + point.vx));
          point.y = Math.max(margin, Math.min(h - margin, point.y + point.vy));
        }
      }

      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = 'rgba(9, 11, 15, 0.78)';
      ctx.fillRect(0, 0, w, h);

      const background = ctx.createRadialGradient(cx, cy, 18, cx, cy, Math.max(w, h) * 0.75);
      background.addColorStop(0, 'rgba(220, 165, 189, 0.09)');
      background.addColorStop(0.6, 'rgba(138, 156, 134, 0.04)');
      background.addColorStop(1, 'rgba(9, 11, 15, 0)');
      ctx.fillStyle = background;
      ctx.fillRect(0, 0, w, h);

      ctx.save();
      ctx.lineCap = 'round';
      for (const edge of data.edges) {
        const sourceIndex = nodeIndex.get(edge.from);
        const targetIndex = nodeIndex.get(edge.to);
        if (sourceIndex === undefined || targetIndex === undefined) continue;

        const source = positions[sourceIndex];
        const target = positions[targetIndex];
        const weight = edge.weight ?? 1;
        const alpha = Math.min(0.42, 0.09 + weight * 0.05);

        ctx.beginPath();
        ctx.moveTo(source.x, source.y);
        ctx.lineTo(target.x, target.y);
        ctx.strokeStyle = `rgba(200, 184, 144, ${alpha})`;
        ctx.lineWidth = 0.7 + weight * 0.28;
        ctx.stroke();
      }
      ctx.restore();

      positions.forEach((point, index) => {
        const node = data.nodes[index];
        const color = ROLE_COLORS[node.role] ?? ROLE_COLORS.auditor;

        const halo = ctx.createRadialGradient(point.x, point.y, 2, point.x, point.y, nodeRadius + 12);
        halo.addColorStop(0, hexToRgba(color, 0.36));
        halo.addColorStop(1, hexToRgba(color, 0));

        ctx.beginPath();
        ctx.arc(point.x, point.y, nodeRadius + 12, 0, Math.PI * 2);
        ctx.fillStyle = halo;
        ctx.fill();

        ctx.beginPath();
        ctx.arc(point.x, point.y, nodeRadius, 0, Math.PI * 2);
        ctx.fillStyle = hexToRgba(color, 0.24);
        ctx.fill();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.5;
        ctx.stroke();

        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';

        ctx.font = '10px "JetBrains Mono", monospace';
        ctx.fillStyle = hexToRgba(color, 0.95);
        ctx.fillText(node.role, point.x, point.y - nodeRadius - 12);

        ctx.font = '11px "JetBrains Mono", monospace';
        ctx.fillStyle = 'rgba(232, 226, 234, 0.92)';
        ctx.fillText(node.agent_id, point.x, point.y + nodeRadius + 14);
      });

      sim.frame += 1;
      const energy = sim.positions.reduce((sum, p) => sum + p.vx * p.vx + p.vy * p.vy, 0);
      if ((energy > 0.05 || sim.frame < 30) && sim.frame < 300 && runId === runIdRef.current) {
        rafRef.current = requestAnimationFrame(renderFrame);
      } else if (runId === runIdRef.current) {
        rafRef.current = null;
      }
    };

    renderFrame();
  }, [data]);

  useEffect(() => {
    stopAnimation();
    runIdRef.current += 1;
    simRef.current = null;
    draw(runIdRef.current);

    const ro = new ResizeObserver(() => {
      stopAnimation();
      runIdRef.current += 1;
      simRef.current = null;
      draw(runIdRef.current);
    });

    if (canvasRef.current) ro.observe(canvasRef.current);

    return () => {
      runIdRef.current += 1;
      stopAnimation();
      ro.disconnect();
    };
  }, [draw, stopAnimation]);

  return (
    <div className="dash-canvas-wrap" style={{ height }}>
      <canvas ref={canvasRef} role="img" aria-label="Agent fleet topology network" className="dash-canvas" />
    </div>
  );
}

/* ── Component ───────────────────────────────────────────── */

type FleetView = 'list' | 'topology';

export default function AgentFleet() {
  const { get } = useLiveApi();
  const [agents, setAgents] = useState<Agent[]>([]);
  const [topology, setTopology] = useState<TopoData>(EMPTY_TOPOLOGY);
  const [view, setView] = useState<FleetView>('list');

  const fetchAll = useCallback(async () => {
    const data = await get<Agent[]>('/api/managed-agents');
    setAgents(Array.isArray(data) ? data : []);

    const topo = await get<TopoData>('/api/agents/topology');
    setTopology((prev) => (sameTopology(prev, topo) ? prev : topo));
  }, [get]);

  // Initial fetch + 30s fallback poll
  useEffect(() => {
    fetchAll();
    const id = setInterval(fetchAll, 30_000);
    return () => clearInterval(id);
  }, [fetchAll]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['agent_spawned', 'agent_started', 'agent_stopped'],
    debouncedRefetch,
  );

  /* Derived */
  const active = agents.filter((a) => isAgentActive(a)).length;
  const avgRep = agents.length > 0
    ? Math.round(agents.reduce((s, a) => s + (a.performance?.reputation ?? a.reputation ?? 0), 0) / agents.length)
    : 90;
  const totalTasks = agents.reduce((s, a) => s + agentTasks(a), 0);

  /* Build Spectre topology nodes from API topology + agent list */
  const spectreNodes: TopologyNode[] = useMemo(
    () =>
      topology.nodes.map((tn) => {
        const agent = agents.find((a) => a.id === tn.agent_id);
        const status: TopologyNode['status'] = agent
          ? isAgentActive(agent)
            ? 'active'
            : agent.status === 'idle'
              ? 'idle'
              : 'completed'
          : 'idle';
        return {
          id: tn.agent_id,
          identity: identityFromAgent(tn.agent_id, tn.agent_id, tn.role),
          status,
        };
      }),
    [topology.nodes, agents],
  );

  const spectreEdges: TopologyEdge[] = useMemo(
    () =>
      topology.edges.map((te) => ({
        source: te.from,
        target: te.to,
        type: (te.weight ?? 1) > 3 ? 'dependency' as const : 'communication' as const,
        active: (te.weight ?? 1) > 1,
      })),
    [topology.edges],
  );

  /* View toggle badge */
  const viewToggle = (
    <span className="dash-inline--8">
      <button
        className={`dash-badge${view === 'list' ? '' : ' dash-ghost'}`}
        style={{ cursor: 'pointer', background: 'none', border: 'none', font: 'inherit' }}
        onClick={() => setView('list')}
      >
        list
      </button>
      <span className="dash-ghost">/</span>
      <button
        className={`dash-badge${view === 'topology' ? '' : ' dash-ghost'}`}
        style={{ cursor: 'pointer', background: 'none', border: 'none', font: 'inherit' }}
        onClick={() => setView('topology')}
      >
        topology
      </button>
    </span>
  );

  return (
    <div className="dash-page">
      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={4}>
          <MosaicCell label="TOTAL" value={agents.length} color="bone" mono />
          <MosaicCell label="ACTIVE" value={active} color="success" mono />
          <MosaicCell label="AVG REPUTATION" value={avgRep} color="warning" mono />
          <MosaicCell label="TASKS DONE" value={totalTasks} color="rose" mono />
        </Mosaic>
      </div>

      {/* TOPOLOGY + AGENT CARDS SIDE BY SIDE */}
      <div className="dash-agent-grid dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
        <Pane
          title="AGENT TOPOLOGY"
          badge={viewToggle}
        >
          <div className="dash-chart-enter">
            {view === 'topology' ? (
              <AgentTopology
                nodes={spectreNodes}
                edges={spectreEdges}
                height={280}
              />
            ) : (
              <TopologyGraph data={topology} height={240} />
            )}
          </div>
        </Pane>

        <div className="dash-agent-list">
        {agents.map((agent, agentIdx) => {
          const rep = agent.performance?.reputation ?? agent.reputation ?? 0;
          const active = isAgentActive(agent);
          const isIdle = agent.status === 'idle';
          const tasks = agentTasks(agent);
          const cost = agentCost(agent);
          const failed = agent.performance?.failed_tasks ?? 0;

          return (
            <div
              key={agent.id}
              className="dash-stagger"
              style={{ '--stagger-i': agentIdx + 2 } as React.CSSProperties}
            >
              <Pane
                className="af-agent-card"
                title={agentDisplayName(agent).toUpperCase()}
                badge={
                  <span className="dash-inline">
                    <span className="dash-model-label">
                      {agent.model ?? 'unknown'}
                    </span>
                    <span
                      className={`dash-dot ${active ? 'af-dot--active' : isIdle ? 'af-dot--idle' : 'af-dot--inactive'}`}
                    />
                  </span>
                }
                foot={
                  <span className="dash-ghost">
                    {active ? 'active now' : `last active ${fmtLastSeen(agent)}`}
                  </span>
                }
              >
                <div className="dash-flex-col--gap8">
                  {/* Capability tags */}
                  <div className="dash-tags-wrap">
                    {(agent.capabilities ?? []).map((cap) => (
                      <span key={cap} className="dash-tag">{cap}</span>
                    ))}
                    {(agent.domain_tags ?? []).map((tag) => (
                      <span key={tag} className="dash-tag--rose">{tag}</span>
                    ))}
                  </div>

                  {/* Reputation bar */}
                  <div className="dash-flex-col--gap4">
                    <div className="dash-row-item--between">
                      <span className="dash-label-sans">Reputation</span>
                      <span className="dash-value--glow">{rep}</span>
                    </div>
                    <div className="dash-bar-track">
                      <div
                        className="dash-bar-fill dash-bar-fill--rose dash-bar-animate"
                        style={{ width: `${rep}%`, animationDelay: `${agentIdx * 60 + 200}ms` }}
                      />
                    </div>
                  </div>

                  {/* Stats row */}
                  <div className="dash-stats-row">
                    <StatPill label="tasks" value={String(tasks)} />
                    <StatPill label="cost" value={`$${cost.toFixed(2)}`} />
                    <StatPill label="failed" value={String(failed)} />
                  </div>
                </div>
              </Pane>
            </div>
          );
        })}
        </div>
      </div>
    </div>
  );
}

/* ── Stat pill helper ────────────────────────────────────── */

function StatPill({ label, value }: { label: string; value: string }) {
  return (
    <div className="dash-stat-pill">
      <span className="dash-label-xs">{label}</span>
      <span className="dash-value">{value}</span>
    </div>
  );
}
