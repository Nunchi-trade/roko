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
  score?: number; // 0-1 confidence score
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

/* ── Node pulse state ── */

interface NodePulse {
  startTime: number;
  duration: number;
}

/* ── Sparkle state (new-entry celebration) ── */

interface Sparkle {
  x: number;
  y: number;
  startTime: number;
  duration: number;
  color: string;
  angle: number;
  speed: number;
}

/* ── Category color mapping (rosedust accents) ── */

const CATEGORY_COLORS: Record<string, string> = {
  heuristic: '--dream-bright',    // #a4a4c8 — lavender
  strategy: '--success',          // #8a9c86 — sage
  causal: '--warning',            // #d8a878 — amber
  warning: '--rose-bright',       // #d89ab2 — rose
};

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

/* ── Ease-in-out helper ── */
function easeInOut(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2;
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
  const sparklesRef = useRef<Sparkle[]>([]);
  const nextPidRef = useRef(0);
  const rafRef = useRef(0);
  const dashOffsetRef = useRef(0);
  const lastFrameRef = useRef(performance.now());

  // Node pulse tracking
  const leftPulseRef = useRef<NodePulse | null>(null);
  const rightPulseRef = useRef<NodePulse | null>(null);
  const centerPulseRef = useRef<NodePulse | null>(null);

  // Hover state: which node is hovered
  const hoveredNodeRef = useRef<'left' | 'right' | 'center' | null>(null);
  const [hoveredNode, setHoveredNode] = useState<'left' | 'right' | 'center' | null>(null);

  // "Aha" animation state
  const [ahaActive, setAhaActive] = useState(false);
  const [ahaLabel, setAhaLabel] = useState(false);
  const ahaEdgeRef = useRef(0);
  const feedRef = useRef<HTMLDivElement>(null);

  // Track previous insight count to detect new events
  const prevCountRef = useRef(0);

  // Track new entry IDs for celebration animation
  const [newEntryIds, setNewEntryIds] = useState<Set<string>>(new Set());

  // Continuous animation flag
  const animatingRef = useRef(true);

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

  /* ── Spawn sparkles (celebration) ── */
  const spawnSparkles = useCallback(
    (side: 'left' | 'right' | 'center', color: string) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const lay = getLayout(rect.width, rect.height);

      const cx = side === 'left' ? lay.leftX : side === 'right' ? lay.rightX : lay.centerX;
      const cy = lay.nodeY;
      const now = performance.now();

      for (let i = 0; i < 8; i++) {
        sparklesRef.current.push({
          x: cx,
          y: cy,
          startTime: now,
          duration: 600 + Math.random() * 400,
          color,
          angle: (Math.PI * 2 * i) / 8 + (Math.random() - 0.5) * 0.4,
          speed: 20 + Math.random() * 30,
        });
      }
    },
    [getLayout],
  );

  /* ── Trigger node pulse ── */
  const pulseNode = useCallback((side: 'left' | 'right' | 'center') => {
    const pulse: NodePulse = { startTime: performance.now(), duration: 500 };
    if (side === 'left') leftPulseRef.current = pulse;
    else if (side === 'right') rightPulseRef.current = pulse;
    else centerPulseRef.current = pulse;
  }, []);

  /* ── Mouse move handler for hover detection ── */
  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      const lay = getLayout(rect.width, rect.height);

      const distLeft = Math.hypot(mx - lay.leftX, my - lay.nodeY);
      const distRight = Math.hypot(mx - lay.rightX, my - lay.nodeY);
      const distCenter = Math.max(
        Math.abs(mx - lay.centerX) - lay.centerW / 2,
        Math.abs(my - lay.nodeY) - lay.centerH / 2,
      );

      let next: 'left' | 'right' | 'center' | null = null;
      if (distLeft < lay.nodeR + 12) next = 'left';
      else if (distRight < lay.nodeR + 12) next = 'right';
      else if (distCenter < 12) next = 'center';

      if (next !== hoveredNodeRef.current) {
        hoveredNodeRef.current = next;
        setHoveredNode(next);
      }
    },
    [getLayout],
  );

  const handleMouseLeave = useCallback(() => {
    hoveredNodeRef.current = null;
    setHoveredNode(null);
  }, []);

  /* ── React to new insight events ── */
  useEffect(() => {
    if (insights.length <= prevCountRef.current) {
      prevCountRef.current = insights.length;
      return;
    }

    const newEvents = insights.slice(prevCountRef.current);
    prevCountRef.current = insights.length;

    const celebrateIds = new Set<string>();

    for (const ev of newEvents) {
      const isLeft = ev.agent === leftAgent.name;
      const agentSide: 'left' | 'right' = isLeft ? 'left' : 'right';
      const agentColor = isLeft ? leftAgent.color : rightAgent.color;
      const kindColor = cssVar(CATEGORY_COLORS[ev.kind] ?? '--rose-bright');

      celebrateIds.add(ev.id);

      if (ev.type === 'posted') {
        spawnParticle(agentSide, 'center', agentColor);
        pulseNode(agentSide);
        pulseNode('center');
        spawnSparkles(agentSide, kindColor);
      } else if (ev.type === 'confirmed') {
        spawnParticle('center', agentSide, cssVar('--bone-bright'));
        pulseNode('center');
        pulseNode(agentSide);
        spawnSparkles('center', cssVar('--bone-bright'));
        // Cross-agent confirmation = "aha" moment
        setAhaActive(true);
        setAhaLabel(true);
        ahaEdgeRef.current = performance.now() + 500;
        setTimeout(() => setAhaActive(false), 500);
        setTimeout(() => setAhaLabel(false), 1500);
      } else if (ev.type === 'challenged') {
        spawnParticle(agentSide, 'center', kindColor);
        pulseNode(agentSide);
      }
    }

    // Mark new entries for celebration CSS
    setNewEntryIds(celebrateIds);
    const timer = setTimeout(() => setNewEntryIds(new Set()), 800);

    // Auto-scroll feed
    requestAnimationFrame(() => {
      if (feedRef.current) {
        feedRef.current.scrollTop = feedRef.current.scrollHeight;
      }
    });

    return () => clearTimeout(timer);
  }, [insights, leftAgent, rightAgent, spawnParticle, pulseNode, spawnSparkles]);

  /* ── Get pulse scale for a node ── */
  const getPulseScale = useCallback((pulse: NodePulse | null, now: number): number => {
    if (!pulse) return 1;
    const elapsed = now - pulse.startTime;
    if (elapsed >= pulse.duration) return 1;
    const t = elapsed / pulse.duration;
    // Scale 1 -> 1.15 -> 1
    return 1 + 0.15 * Math.sin(t * Math.PI);
  }, []);

  /* ── Draw a flowing dashed bezier curve ── */
  const drawFlowingEdge = useCallback(
    (
      ctx: CanvasRenderingContext2D,
      x0: number, y0: number,
      cpx: number, cpy: number,
      x1: number, y1: number,
      color: string,
      width: number,
      glow: boolean,
      dimmed: boolean,
      offset: number,
    ) => {
      ctx.save();
      ctx.beginPath();
      ctx.moveTo(x0, y0);
      ctx.quadraticCurveTo(cpx, cpy, x1, y1);
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.globalAlpha = dimmed ? 0.2 : 1;
      ctx.setLineDash([8, 6]);
      ctx.lineDashOffset = -offset;
      ctx.stroke();

      if (glow) {
        ctx.shadowColor = color;
        ctx.shadowBlur = 12;
        ctx.stroke();
        ctx.shadowBlur = 0;
      }

      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
      ctx.restore();
    },
    [],
  );

  /* ── Draw a score arc gauge ── */
  const drawScoreArc = useCallback(
    (ctx: CanvasRenderingContext2D, cx: number, cy: number, radius: number, score: number, color: string) => {
      const startAngle = -Math.PI / 2;
      const endAngle = startAngle + Math.PI * 2 * score;

      // Background arc
      ctx.beginPath();
      ctx.arc(cx, cy, radius, 0, Math.PI * 2);
      ctx.strokeStyle = hexToRgba(color, 0.15);
      ctx.lineWidth = 2;
      ctx.stroke();

      // Score arc
      ctx.beginPath();
      ctx.arc(cx, cy, radius, startAngle, endAngle);
      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      ctx.lineCap = 'round';
      ctx.stroke();
      ctx.lineCap = 'butt';
    },
    [],
  );

  /** Core scene renderer */
  const drawScene = useCallback((ctx: CanvasRenderingContext2D, w: number, h: number) => {
    const lay = getLayout(w, h);
    const now = performance.now();

    // Update dash offset for flowing animation
    const dt = now - lastFrameRef.current;
    lastFrameRef.current = now;
    dashOffsetRef.current += dt * 0.03; // speed of flow

    // Clear
    ctx.clearRect(0, 0, w, h);

    const bgVoid = cssVar('--bg-void');
    ctx.fillStyle = bgVoid;
    ctx.fillRect(0, 0, w, h);

    // Are edges glowing gold?
    const edgeGold = now < ahaEdgeRef.current;
    const edgeColor = edgeGold ? cssVar('--bone-bright') : cssVar('--border-soft');
    const edgeWidth = edgeGold ? 2.5 : 1.5;

    // Hover state for dimming
    const hovered = hoveredNodeRef.current;

    // ── Draw flowing dashed edges ──
    const cpYOffset = -40;
    const lcpX = (lay.leftX + lay.centerX) / 2;
    const lcpY = lay.nodeY + cpYOffset;
    const rcpX = (lay.rightX + lay.centerX) / 2;
    const rcpY = lay.nodeY + cpYOffset;

    // Left -> center edge
    const leftEdgeDim = hovered !== null && hovered !== 'left' && hovered !== 'center';
    const leftEdgeHighlight = hovered === 'left' || hovered === 'center';
    drawFlowingEdge(
      ctx,
      lay.leftX + lay.nodeR, lay.nodeY,
      lcpX, lcpY,
      lay.centerX - lay.centerW / 2, lay.nodeY,
      leftEdgeHighlight ? cssVar('--rose-glow') : edgeColor,
      leftEdgeHighlight ? edgeWidth + 1 : edgeWidth,
      edgeGold || leftEdgeHighlight,
      leftEdgeDim,
      dashOffsetRef.current,
    );

    // Right -> center edge
    const rightEdgeDim = hovered !== null && hovered !== 'right' && hovered !== 'center';
    const rightEdgeHighlight = hovered === 'right' || hovered === 'center';
    drawFlowingEdge(
      ctx,
      lay.rightX - lay.nodeR, lay.nodeY,
      rcpX, rcpY,
      lay.centerX + lay.centerW / 2, lay.nodeY,
      rightEdgeHighlight ? cssVar('--rose-glow') : edgeColor,
      rightEdgeHighlight ? edgeWidth + 1 : edgeWidth,
      edgeGold || rightEdgeHighlight,
      rightEdgeDim,
      dashOffsetRef.current,
    );

    // ── Draw center node (rounded rect) with pulse ──
    const centerScale = getPulseScale(centerPulseRef.current, now);
    const centerDim = hovered !== null && hovered !== 'center';

    ctx.save();
    ctx.translate(lay.centerX, lay.nodeY);
    ctx.scale(centerScale, centerScale);
    ctx.globalAlpha = centerDim ? 0.35 : 1;

    const cw2 = lay.centerW / 2;
    const ch2 = lay.centerH / 2;
    ctx.beginPath();
    ctx.roundRect(-cw2, -ch2, lay.centerW, lay.centerH, 8);
    ctx.fillStyle = cssVar('--border-soft');
    ctx.fill();
    ctx.strokeStyle = cssVar('--bone-dim');
    ctx.lineWidth = 1;
    ctx.stroke();

    // Center pulse glow
    if (centerScale > 1.01) {
      ctx.shadowColor = cssVar('--bone-bright');
      ctx.shadowBlur = 16 * (centerScale - 1) / 0.15;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    // Center label
    const centerLabel = mode === 'chain' ? 'On-Chain Knowledge' : 'Knowledge Store';
    ctx.fillStyle = cssVar('--bone');
    ctx.font = '10px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(centerLabel.toUpperCase(), 0, -8);

    // Center stats
    const totalActive = insights.filter((i) => i.type === 'posted').length;
    const totalConfirms = insights.filter((i) => i.type === 'confirmed').length;
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.fillText(`${totalActive} active  ${totalConfirms} confirmed`, 0, 10);
    ctx.restore();

    // ── Draw left agent node with pulse ──
    const leftScale = getPulseScale(leftPulseRef.current, now);
    const leftDim = hovered !== null && hovered !== 'left';

    ctx.save();
    ctx.translate(lay.leftX, lay.nodeY);
    ctx.scale(leftScale, leftScale);
    ctx.globalAlpha = leftDim ? 0.35 : 1;

    ctx.beginPath();
    ctx.arc(0, 0, lay.nodeR, 0, Math.PI * 2);
    ctx.fillStyle = hexToRgba(leftAgent.color, 0.15);
    ctx.fill();
    ctx.strokeStyle = leftAgent.color;
    ctx.lineWidth = hovered === 'left' ? 2.5 : 1.5;
    ctx.stroke();

    // Pulse glow
    if (leftScale > 1.01) {
      ctx.shadowColor = leftAgent.color;
      ctx.shadowBlur = 16 * (leftScale - 1) / 0.15;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    // Inner dot
    ctx.beginPath();
    ctx.arc(0, 0, 4, 0, Math.PI * 2);
    ctx.fillStyle = leftAgent.color;
    ctx.fill();

    ctx.restore();

    // Left agent labels (not scaled)
    ctx.save();
    ctx.globalAlpha = leftDim ? 0.35 : 1;
    ctx.fillStyle = leftAgent.color;
    ctx.font = '10px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    ctx.fillText(leftAgent.name, lay.leftX, lay.nodeY + lay.nodeR + 18);
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.fillText(
      `${leftAgent.posts}p ${leftAgent.confirms}c`,
      lay.leftX,
      lay.nodeY + lay.nodeR + 32,
    );
    ctx.restore();

    // ── Draw right agent node with pulse ──
    const rightScale = getPulseScale(rightPulseRef.current, now);
    const rightDim = hovered !== null && hovered !== 'right';

    ctx.save();
    ctx.translate(lay.rightX, lay.nodeY);
    ctx.scale(rightScale, rightScale);
    ctx.globalAlpha = rightDim ? 0.35 : 1;

    ctx.beginPath();
    ctx.arc(0, 0, lay.nodeR, 0, Math.PI * 2);
    ctx.fillStyle = hexToRgba(rightAgent.color, 0.15);
    ctx.fill();
    ctx.strokeStyle = rightAgent.color;
    ctx.lineWidth = hovered === 'right' ? 2.5 : 1.5;
    ctx.stroke();

    // Pulse glow
    if (rightScale > 1.01) {
      ctx.shadowColor = rightAgent.color;
      ctx.shadowBlur = 16 * (rightScale - 1) / 0.15;
      ctx.stroke();
      ctx.shadowBlur = 0;
    }

    // Inner dot
    ctx.beginPath();
    ctx.arc(0, 0, 4, 0, Math.PI * 2);
    ctx.fillStyle = rightAgent.color;
    ctx.fill();

    ctx.restore();

    // Right agent labels
    ctx.save();
    ctx.globalAlpha = rightDim ? 0.35 : 1;
    ctx.fillStyle = rightAgent.color;
    ctx.font = '10px "JetBrains Mono", monospace';
    ctx.textAlign = 'center';
    ctx.fillText(rightAgent.name, lay.rightX, lay.nodeY + lay.nodeR + 18);
    ctx.fillStyle = cssVar('--text-dim');
    ctx.font = '9px "JetBrains Mono", monospace';
    ctx.fillText(
      `${rightAgent.posts}p ${rightAgent.confirms}c`,
      lay.rightX,
      lay.nodeY + lay.nodeR + 32,
    );
    ctx.restore();

    // ── Draw score arcs next to agent nodes ──
    // Find last insight per agent to show their latest score
    const latestLeft = [...insights].reverse().find((i) => i.agent === leftAgent.name && i.score != null);
    const latestRight = [...insights].reverse().find((i) => i.agent === rightAgent.name && i.score != null);

    if (latestLeft?.score != null) {
      const kindColor = cssVar(CATEGORY_COLORS[latestLeft.kind] ?? '--rose-bright');
      drawScoreArc(ctx, lay.leftX, lay.nodeY - lay.nodeR - 12, 8, latestLeft.score, kindColor);
    }
    if (latestRight?.score != null) {
      const kindColor = cssVar(CATEGORY_COLORS[latestRight.kind] ?? '--rose-bright');
      drawScoreArc(ctx, lay.rightX, lay.nodeY - lay.nodeR - 12, 8, latestRight.score, kindColor);
    }

    // ── Draw particles ──
    const alive: Particle[] = [];
    for (const p of particlesRef.current) {
      const elapsed = now - p.startTime;
      const t = Math.min(elapsed / p.duration, 1);
      if (t >= 1) continue;
      alive.push(p);

      const cpx = (p.fromX + p.toX) / 2;
      const cpy = lay.nodeY + cpYOffset;
      const [px, py] = quadBezier(p.fromX, p.fromY, cpx, cpy, p.toX, p.toY, easeInOut(t));

      // Ease-in-out alpha
      const alpha = t < 0.1 ? t / 0.1 : t > 0.9 ? (1 - t) / 0.1 : 1;

      ctx.beginPath();
      ctx.arc(px, py, 4, 0, Math.PI * 2);
      ctx.fillStyle = hexToRgba(p.color, alpha * 0.9);
      ctx.fill();

      // Glow trail
      ctx.shadowColor = p.color;
      ctx.shadowBlur = 8;
      ctx.beginPath();
      ctx.arc(px, py, 3, 0, Math.PI * 2);
      ctx.fill();
      ctx.shadowBlur = 0;
    }
    particlesRef.current = alive;

    // ── Draw sparkles ──
    const aliveSparkles: Sparkle[] = [];
    for (const s of sparklesRef.current) {
      const elapsed = now - s.startTime;
      const t = Math.min(elapsed / s.duration, 1);
      if (t >= 1) continue;
      aliveSparkles.push(s);

      const dist = s.speed * easeInOut(t);
      const sx = s.x + Math.cos(s.angle) * dist;
      const sy = s.y + Math.sin(s.angle) * dist;
      const alpha = 1 - t;
      const size = 2 * (1 - t * 0.6);

      ctx.save();
      ctx.globalAlpha = alpha;
      ctx.fillStyle = s.color;
      ctx.shadowColor = s.color;
      ctx.shadowBlur = 6;

      // Diamond shape
      ctx.beginPath();
      ctx.moveTo(sx, sy - size);
      ctx.lineTo(sx + size * 0.6, sy);
      ctx.lineTo(sx, sy + size);
      ctx.lineTo(sx - size * 0.6, sy);
      ctx.closePath();
      ctx.fill();

      ctx.shadowBlur = 0;
      ctx.restore();
    }
    sparklesRef.current = aliveSparkles;

    // Always continue animation for flowing dashes
    if (animatingRef.current) {
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
  }, [getLayout, leftAgent, rightAgent, insights, mode, getPulseScale, drawFlowingEdge, drawScoreArc]);

  /* ── Cleanup animation on unmount ── */
  useEffect(() => {
    animatingRef.current = true;
    return () => {
      animatingRef.current = false;
      cancelAnimationFrame(rafRef.current);
    };
  }, []);

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
        <canvas
          ref={canvasRef}
          role="img"
          aria-label="Knowledge flow network visualization"
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          style={{ cursor: hoveredNode ? 'pointer' : 'default' }}
        />
        <div className={`kf-aha-label${ahaLabel ? ' visible' : ''}`}>Knowledge reused</div>
      </div>

      {/* Insight feed */}
      <div className="kf-feed" ref={feedRef}>
        {insights.length === 0 ? (
          <div className="kf-empty">Waiting for insights...</div>
        ) : (
          insights.map((ev, index) => {
            const isLeft = ev.agent === leftAgent.name;
            const dotColor = isLeft ? leftAgent.color : rightAgent.color;
            const isCelebrating = newEntryIds.has(ev.id);

            return (
              <div
                key={ev.id}
                className={`kf-feed-row${isCelebrating ? ' kf-celebrate' : ''}`}
                style={{ animationDelay: `${index * 40}ms` }}
              >
                <span className={`kf-badge ${ev.kind}`}>{ev.kind}</span>
                <span className="kf-agent-dot" style={{ background: dotColor }} />
                <span className="kf-agent-name">{ev.agent}</span>
                <span className="kf-content">{ev.content}</span>
                {ev.score != null && (
                  <span className={`kf-score ${ev.kind}`}>
                    <svg width="16" height="16" viewBox="0 0 16 16">
                      <circle
                        cx="8" cy="8" r="6"
                        fill="none"
                        stroke="currentColor"
                        strokeOpacity="0.2"
                        strokeWidth="2"
                      />
                      <circle
                        cx="8" cy="8" r="6"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeDasharray={`${ev.score * 37.7} 37.7`}
                        strokeDashoffset="0"
                        transform="rotate(-90 8 8)"
                        className="kf-score-arc"
                      />
                    </svg>
                    <span className="kf-score-value">{Math.round(ev.score * 100)}</span>
                  </span>
                )}
                {ev.txHash && <span className="kf-tx">{ev.txHash.slice(0, 8)}...</span>}
              </div>
            );
          })
        )}
      </div>
    </Pane>
  );
}
