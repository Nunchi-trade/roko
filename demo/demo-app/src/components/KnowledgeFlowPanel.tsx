import { useRef, useEffect, useCallback, useState } from 'react';
import { getCssVar, hexToRgba } from '../lib/color';
import { useCanvasSetup } from '../hooks/useCanvasSetup';
import Pane from './Pane';
import './KnowledgeFlowPanel.css';

/* ── Types ── */

export interface InsightEvent {
  id: string;
  type: 'posted' | 'confirmed' | 'challenged';
  agent: string;
  kind: 'heuristic' | 'strategy' | 'causal' | 'warning';
  content: string;
  txHash?: string;
  timestamp: number;
}

export interface AgentInfo {
  name: string;
  color: string;
  posts: number;
  confirms: number;
}

export interface KnowledgeFlowProps {
  leftAgent: AgentInfo;
  rightAgent: AgentInfo;
  insights: InsightEvent[];
  mode: 'local' | 'chain';
}

/* ── Particle animation state ── */

interface Particle {
  id: number;
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  startTime: number;
  duration: number;
  color: string;
}

/* ── Resolved CSS variable accessor ── */
const cssVar = getCssVar;

/* ── Bezier helper (quadratic) ── */
function quadBezier(
  x0: number, y0: number,
  cx: number, cy: number,
  x1: number, y1: number,
  t: number,
): [number, number] {
  const u = 1 - t;
  return [
    u * u * x0 + 2 * u * t * cx + t * t * x1,
    u * u * y0 + 2 * u * t * cy + t * t * y1,
  ];
}

/* ── Component ── */

export default function KnowledgeFlowPanel({
  leftAgent,
  rightAgent,
  insights,
  mode,
}: KnowledgeFlowProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const particlesRef = useRef<Particle[]>([]);
  const nextPidRef = useRef(0);
  const rafRef = useRef(0);

  // "Aha" animation state
  const [ahaActive, setAhaActive] = useState(false);
  const [ahaLabel, setAhaLabel] = useState(false);
  const ahaEdgeRef = useRef(0); // timestamp until edges glow gold
  const feedRef = useRef<HTMLDivElement>(null);

  // Track previous insight count to detect new events
  const prevCountRef = useRef(0);

  /* ── Compute node positions (responsive) ── */
  const getLayout = useCallback((w: number, h: number) => {
    const leftX = w * 0.18;
    const rightX = w * 0.82;
    const centerX = w * 0.5;
    const nodeY = h * 0.45;
    const nodeR = Math.min(w * 0.07, 32);
    const centerW = Math.min(w * 0.28, 160);
    const centerH = Math.min(h * 0.32, 56);
    return { leftX, rightX, centerX, nodeY, nodeR, centerW, centerH };
  }, []);

  /* ── Spawn particle ── */
  const spawnParticle = useCallback(
    (fromSide: 'left' | 'right' | 'center', toSide: 'left' | 'right' | 'center', color: string) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const lay = getLayout(rect.width, rect.height);

      const xOf = (side: string) =>
        side === 'left' ? lay.leftX : side === 'right' ? lay.rightX : lay.centerX;

      particlesRef.current.push({
        id: nextPidRef.current++,
        fromX: xOf(fromSide),
        fromY: lay.nodeY,
        toX: xOf(toSide),
        toY: lay.nodeY,
        startTime: performance.now(),
        duration: 1200,
        color,
      });
    },
    [getLayout],
  );

  /* ── React to new insight events ── */
  useEffect(() => {
    if (insights.length <= prevCountRef.current) {
      prevCountRef.current = insights.length;
      return;
    }

    const newEvents = insights.slice(prevCountRef.current);
    prevCountRef.current = insights.length;

    for (const ev of newEvents) {
      const isLeft = ev.agent === leftAgent.name;
      const agentSide = isLeft ? 'left' : 'right';
      const agentColor = isLeft ? leftAgent.color : rightAgent.color;

      if (ev.type === 'posted') {
        spawnParticle(agentSide, 'center', agentColor);
      } else if (ev.type === 'confirmed') {
        spawnParticle('center', agentSide, cssVar('--bone-bright'));
        // Cross-agent confirmation = "aha" moment
        setAhaActive(true);
        setAhaLabel(true);
        ahaEdgeRef.current = performance.now() + 500;
        setTimeout(() => setAhaActive(false), 500);
        setTimeout(() => setAhaLabel(false), 1500);
      }
    }

    // Auto-scroll feed
    requestAnimationFrame(() => {
      if (feedRef.current) {
        feedRef.current.scrollTop = feedRef.current.scrollHeight;
      }
    });
  }, [insights, leftAgent, rightAgent, spawnParticle]);

  /** Core scene renderer — called by useCanvasSetup (DPR-adjusted) and by animation continuation. */
  const drawScene = useCallback((ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const lay = getLayout(w, h);
    const now = performance.now();

    // Clear
    ctx.clearRect(0, 0, w, h);

    const bgVoid = cssVar('--bg-void');
    ctx.fillStyle = bgVoid;
    ctx.fillRect(0, 0, w, h);

    // Are edges glowing gold?
    const edgeGold = now < ahaEdgeRef.current;
    const edgeColor = edgeGold ? cssVar('--bone-bright') : cssVar('--border-soft');
    const edgeWidth = edgeGold ? 2.5 : 1.5;

    // ── Draw edges (bezier curves) ──
    const cpYOffset = -40;
    // Left -> center
    const lcpX = (lay.leftX + lay.centerX) / 2;
    const lcpY = lay.nodeY + cpYOffset;
    ctx.beginPath();
    ctx.moveTo(lay.leftX + lay.nodeR, lay.nodeY);
    ctx.quadraticCurveTo(lcpX, lcpY, lay.centerX - lay.centerW / 2, lay.nodeY);
    ctx.strokeStyle = edgeColor;
    ctx.lineWidth = edgeWidth;
    ctx.stroke();

    if (edgeGold) {
      ctx.shadowColor = cssVar('--bone-bright');
      ctx.shadowBlur = 12;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    // Right -> center
    const rcpX = (lay.rightX + lay.centerX) / 2;
    const rcpY = lay.nodeY + cpYOffset;
    ctx.beginPath();
    ctx.moveTo(lay.rightX - lay.nodeR, lay.nodeY);
    ctx.quadraticCurveTo(rcpX, rcpY, lay.centerX + lay.centerW / 2, lay.nodeY);
    ctx.strokeStyle = edgeColor;
    ctx.lineWidth = edgeWidth;
    ctx.stroke();

    if (edgeGold) {
      ctx.shadowColor = cssVar('--bone-bright');
      ctx.shadowBlur = 12;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    // ── Draw center node (rounded rect) ──
    const cx = lay.centerX - lay.centerW / 2;
    const cy = lay.nodeY - lay.centerH / 2;
    const cr = 8;
    ctx.beginPath();
    ctx.roundRect(cx, cy, lay.centerW, lay.centerH, cr);
    ctx.fillStyle = cssVar('--border-soft');
    ctx.fill();
    ctx.strokeStyle = cssVar('--bone-dim');
    ctx.lineWidth = 1;
    ctx.stroke();

    // Center label
    const centerLabel = mode === 'chain' ? 'On-Chain Knowledge' : 'Knowledge Store';
    ctx.fillStyle = cssVar('--bone');
    ctx.font = `10px "JetBrains Mono", monospace`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(centerLabel.toUpperCase(), lay.centerX, lay.nodeY - 8);

    // Center stats
    const totalActive = insights.filter((i) => i.type === 'posted').length;
    const totalConfirms = insights.filter((i) => i.type === 'confirmed').length;
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = `9px "JetBrains Mono", monospace`;
    ctx.fillText(`${totalActive} active  ${totalConfirms} confirmed`, lay.centerX, lay.nodeY + 10);

    // ── Draw left agent node ──
    ctx.beginPath();
    ctx.arc(lay.leftX, lay.nodeY, lay.nodeR, 0, Math.PI * 2);
    ctx.fillStyle = hexToRgba(leftAgent.color, 0.15);
    ctx.fill();
    ctx.strokeStyle = leftAgent.color;
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Left agent label
    ctx.fillStyle = leftAgent.color;
    ctx.font = `10px "JetBrains Mono", monospace`;
    ctx.textAlign = 'center';
    ctx.fillText(leftAgent.name, lay.leftX, lay.nodeY + lay.nodeR + 18);
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = `9px "JetBrains Mono", monospace`;
    ctx.fillText(
      `${leftAgent.posts}p ${leftAgent.confirms}c`,
      lay.leftX,
      lay.nodeY + lay.nodeR + 32,
    );

    // Left agent icon (small circle inside)
    ctx.beginPath();
    ctx.arc(lay.leftX, lay.nodeY, 4, 0, Math.PI * 2);
    ctx.fillStyle = leftAgent.color;
    ctx.fill();

    // ── Draw right agent node ──
    ctx.beginPath();
    ctx.arc(lay.rightX, lay.nodeY, lay.nodeR, 0, Math.PI * 2);
    ctx.fillStyle = hexToRgba(rightAgent.color, 0.15);
    ctx.fill();
    ctx.strokeStyle = rightAgent.color;
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Right agent label
    ctx.fillStyle = rightAgent.color;
    ctx.font = `10px "JetBrains Mono", monospace`;
    ctx.textAlign = 'center';
    ctx.fillText(rightAgent.name, lay.rightX, lay.nodeY + lay.nodeR + 18);
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = `9px "JetBrains Mono", monospace`;
    ctx.fillText(
      `${rightAgent.posts}p ${rightAgent.confirms}c`,
      lay.rightX,
      lay.nodeY + lay.nodeR + 32,
    );

    // Right agent icon
    ctx.beginPath();
    ctx.arc(lay.rightX, lay.nodeY, 4, 0, Math.PI * 2);
    ctx.fillStyle = rightAgent.color;
    ctx.fill();

    // ── Draw particles ──
    const alive: Particle[] = [];
    for (const p of particlesRef.current) {
      const elapsed = now - p.startTime;
      const t = Math.min(elapsed / p.duration, 1);
      if (t >= 1) continue;
      alive.push(p);

      // Determine control point
      const cpx = (p.fromX + p.toX) / 2;
      const cpy = lay.nodeY + cpYOffset;
      const [px, py] = quadBezier(p.fromX, p.fromY, cpx, cpy, p.toX, p.toY, t);

      // Ease-in-out
      const alpha = t < 0.1 ? t / 0.1 : t > 0.9 ? (1 - t) / 0.1 : 1;

      ctx.beginPath();
      ctx.arc(px, py, 4, 0, Math.PI * 2);
      ctx.fillStyle = hexToRgba(p.color, alpha * 0.9);
      ctx.fill();

      // Glow
      ctx.shadowColor = p.color;
      ctx.shadowBlur = 8;
      ctx.beginPath();
      ctx.arc(px, py, 3, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
    }
    particlesRef.current = alive;

    // Continue animation if particles or gold edges are active
    if (alive.length > 0 || edgeGold) {
      rafRef.current = requestAnimationFrame(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const c = canvas.getContext('2d');
        if (!c) return;
        const dpr = window.devicePixelRatio || 1;
        const rect = canvas.getBoundingClientRect();
        canvas.width = rect.width * dpr;
        canvas.height = rect.height * dpr;
        c.setTransform(dpr, 0, 0, dpr, 0, 0);
        drawScene(c, rect.width, rect.height);
      });
    }
  }, [getLayout, leftAgent, rightAgent, insights, mode]);

  /* ── useCanvasSetup for initial draw + resize handling ── */
  useCanvasSetup(canvasRef, (ctx, w, h) => {
    cancelAnimationFrame(rafRef.current);
    drawScene(ctx, w, h);
  }, [drawScene, insights.length]);

  /* ── Badge for pane header ── */
  const totalInsights = insights.length;
  const badge = (
    <span>
      {totalInsights} event{totalInsights !== 1 ? 's' : ''}
    </span>
  );

  return (
    <Pane
      title="Knowledge Flow"
      badge={badge}
      className={`knowledge-flow-panel${ahaActive ? ' kf-aha-border' : ''}`}
      flat
    >
      {/* Canvas diagram */}
      <div className="kf-canvas-wrap">
        <canvas ref={canvasRef} role="img" aria-label="Knowledge flow network visualization" />
        <div className={`kf-aha-label${ahaLabel ? ' visible' : ''}`}>Knowledge reused</div>
      </div>

      {/* Insight feed */}
      <div className="kf-feed" ref={feedRef}>
        {insights.length === 0 ? (
          <div className="kf-empty">Waiting for insights...</div>
        ) : (
          insights.map((ev) => {
            const isLeft = ev.agent === leftAgent.name;
            const dotColor = isLeft ? leftAgent.color : rightAgent.color;

            return (
              <div key={ev.id} className="kf-feed-row">
                <span className={`kf-badge ${ev.kind}`}>{ev.kind}</span>
                <span className="kf-agent-dot" style={{ background: dotColor }} />
                <span className="kf-agent-name">{ev.agent}</span>
                <span className="kf-content">{ev.content}</span>
                {ev.txHash && <span className="kf-tx">{ev.txHash.slice(0, 8)}...</span>}
              </div>
            );
          })
        )}
      </div>
    </Pane>
  );
}

